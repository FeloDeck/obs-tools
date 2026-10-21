// Small desktop helpers: native file and folder pickers, and the focused window (X11 or macOS).
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};

use crate::Res;

struct Run {
	ok: bool,
	/// The program is not installed.
	missing: bool,
	stdout: String,
}

async fn run(command: &str, args: &[String], timeout_ms: u64) -> Run {
	let child = tokio::process::Command::new(command)
		.args(args)
		.stdin(Stdio::null())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.kill_on_drop(true)
		.spawn();
	let child = match child {
		Ok(child) => child,
		Err(error) => return Run { ok: false, missing: error.kind() == std::io::ErrorKind::NotFound, stdout: String::new() },
	};
	let output = child.wait_with_output();
	let output = if timeout_ms > 0 {
		match tokio::time::timeout(Duration::from_millis(timeout_ms), output).await {
			Ok(output) => output,
			Err(_) => return Run { ok: false, missing: false, stdout: String::new() },
		}
	} else {
		output.await
	};
	match output {
		Ok(output) => Run { ok: output.status.success(), missing: false, stdout: String::from_utf8_lossy(&output.stdout).into_owned() },
		Err(_) => Run { ok: false, missing: false, stdout: String::new() },
	}
}

fn home() -> String {
	std::env::var("HOME")
		.ok()
		.filter(|h| !h.is_empty())
		.or_else(|| std::env::home_dir().map(|p| p.to_string_lossy().into_owned()))
		.unwrap_or_default()
}

// path.dirname
fn dirname(path: &str) -> String {
	match Path::new(path).parent() {
		Some(parent) if !parent.as_os_str().is_empty() => parent.to_string_lossy().into_owned(),
		_ if path.starts_with('/') => "/".into(),
		_ => ".".into(),
	}
}

fn some_path(text: &str) -> Option<String> {
	let text = text.trim();
	if text.is_empty() { None } else { Some(text.to_string()) }
}

const IMAGE_PATTERNS: &[&str] = &["*.png", "*.jpg", "*.jpeg", "*.gif", "*.webp", "*.bmp", "*.tga", "*.psd"];
const VIDEO_PATTERNS: &[&str] =
	&["*.mp4", "*.mkv", "*.mov", "*.webm", "*.flv", "*.avi", "*.ts", "*.m4v", "*.gif", "*.mp3", "*.wav", "*.ogg", "*.flac"];

/// Opens a file dialog. The path, or None when the user cancels.
pub async fn pick_file(title: &str, start: &str, images: bool) -> Res<Option<String>> {
	if cfg!(target_os = "macos") {
		let dir = if start.is_empty() { format!("{}/{}", home(), if images { "Pictures" } else { "Movies" }) } else { dirname(start) };
		// Only pictures get a filter: macOS knows no type for some videos (like .mkv) and would gray them out.
		return Ok(choose(if images { "choose file of type {\"public.image\"}" } else { "choose file" }, title, &dir).await);
	}
	let patterns = if images { IMAGE_PATTERNS } else { VIDEO_PATTERNS }.join(" ");
	let dir = if start.is_empty() { format!("{}/{}", home(), if images { "Pictures" } else { "Videos" }) } else { dirname(start) };
	let kind = if images { "Pictures" } else { "Media files" };
	let begin = if start.is_empty() { dir.clone() } else { start.to_string() };
	let kdialog = run(
		"kdialog",
		&["--title".into(), title.into(), "--getopenfilename".into(), begin, format!("{kind} ({patterns})|All files (*)")],
		0,
	)
	.await;
	if kdialog.ok {
		return Ok(some_path(&kdialog.stdout));
	}
	if !kdialog.missing {
		return Ok(None);
	}
	for tool in ["zenity", "qarma"] {
		let file = if start.is_empty() { format!("{dir}/") } else { start.to_string() };
		let args = vec![
			"--file-selection".into(),
			format!("--title={title}"),
			format!("--filename={file}"),
			format!("--file-filter={kind} | {patterns}"),
			"--file-filter=All files | *".into(),
		];
		let result = run(tool, &args, 0).await;
		if result.ok {
			return Ok(some_path(&result.stdout));
		}
		if !result.missing {
			return Ok(None);
		}
	}
	Err("No file dialog found. Install kdialog or zenity, or type the path.".into())
}

/// Opens a folder dialog. The path, or None when the user cancels.
pub async fn pick_folder(title: &str, start: &str) -> Res<Option<String>> {
	let dir = if start.is_empty() { home() } else { start.to_string() };
	if cfg!(target_os = "macos") {
		return Ok(choose("choose folder", title, &dir).await.map(|p| without_slash(&p)));
	}
	let kdialog = run("kdialog", &["--title".into(), title.into(), "--getexistingdirectory".into(), dir.clone()], 0).await;
	if kdialog.ok {
		return Ok(some_path(&kdialog.stdout));
	}
	if !kdialog.missing {
		return Ok(None);
	}
	for tool in ["zenity", "qarma"] {
		let result =
			run(tool, &["--file-selection".into(), "--directory".into(), format!("--title={title}"), format!("--filename={dir}/")], 0)
				.await;
		if result.ok {
			return Ok(some_path(&result.stdout));
		}
		if !result.missing {
			return Ok(None);
		}
	}
	Err("No folder dialog found. Install kdialog or zenity, or type the path.".into())
}

/// An AppleScript dialog: "choose file" or "choose folder". It starts in `dir`, or in the home
/// folder when `dir` is gone. None when the user cancels.
async fn choose(command: &str, title: &str, dir: &str) -> Option<String> {
	let dir = if Path::new(dir).is_dir() { dir.to_string() } else { home() };
	// "activate" brings the dialog to the front: the plugin has no window of its own.
	let script = format!(
		"on run argv\n\tactivate\n\treturn POSIX path of ({command} with prompt (item 1 of argv) default location (POSIX file (item 2 of argv)))\nend run"
	);
	let result = run("osascript", &["-e".into(), script, title.into(), dir], 0).await;
	if result.ok { some_path(&result.stdout) } else { None }
}

// macOS ends folder paths with a slash, the other systems do not.
fn without_slash(path: &str) -> String {
	let trimmed = path.trim_end_matches('/');
	if trimmed.is_empty() { path.to_string() } else { trimmed.to_string() }
}

pub fn session_type() -> String {
	if cfg!(target_os = "macos") {
		return "macos".into();
	}
	let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
	if let Some(kind) = var("XDG_SESSION_TYPE") {
		return kind;
	}
	if var("WAYLAND_DISPLAY").is_some() {
		return "wayland".into();
	}
	if var("DISPLAY").is_some() {
		return "x11".into();
	}
	"unknown".into()
}

fn parse_xprop(text: &str, name: &str) -> Option<String> {
	let line = text.split('\n').find(|l| l.starts_with(name))?;
	let value = match line.find('=') {
		Some(i) => &line[i + 1..],
		None => line,
	};
	Some(crate::js::trim(value).to_string())
}

/// The focused X11 window.
#[derive(Debug, Clone, PartialEq)]
pub struct Window {
	pub id: u64,
	pub name: String,
	pub cls: String,
}

pub async fn active_x11_window() -> Res<Option<Window>> {
	let root = run("xprop", &["-root".into(), "_NET_ACTIVE_WINDOW".into()], 3000).await;
	if !root.ok {
		if root.missing {
			return Err("xprop is missing (package xorg-xprop)".into());
		}
		return Ok(None);
	}
	let lower = root.stdout.to_lowercase();
	let Some(at) = lower.find("window id # 0x") else { return Ok(None) };
	let hex_start = at + "window id # ".len();
	let hex_len = lower[hex_start + 2..].bytes().take_while(u8::is_ascii_hexdigit).count();
	if hex_len == 0 {
		return Ok(None);
	}
	let hex = &root.stdout[hex_start..hex_start + 2 + hex_len];
	if hex == "0x0" {
		return Ok(None);
	}
	let id = u64::from_str_radix(&hex[2..], 16).unwrap_or(0);
	let info = run("xprop", &["-id".into(), hex.into(), "_NET_WM_NAME".into(), "WM_NAME".into(), "WM_CLASS".into()], 3000).await;
	let name = parse_xprop(&info.stdout, "_NET_WM_NAME")
		.filter(|n| !n.is_empty())
		.or_else(|| parse_xprop(&info.stdout, "WM_NAME"))
		.unwrap_or_default();
	let name = name.strip_prefix('"').unwrap_or(&name);
	let name = name.strip_suffix('"').unwrap_or(name).to_string();
	// OBS uses the first WM_CLASS string.
	let classes = parse_xprop(&info.stdout, "WM_CLASS").unwrap_or_default();
	let first = classes.split('"').nth(1).filter(|_| classes.matches('"').count() >= 2).unwrap_or("");
	Ok(Some(Window { id, name, cls: if first.is_empty() { "unknown".into() } else { first.to_string() } }))
}

/// The value OBS uses for "capture_window" in an xcomposite window capture.
pub fn xcomposite_window_value(w: &Window) -> String {
	format!("{}\r\n{}\r\n{}", w.id, w.name, w.cls)
}

/// The settings that point a window capture at the window that has focus, and that window for the
/// log. `kind` and `current` are the input kind and the settings the source has now.
pub async fn focused_window_settings(kind: &str, current: &Value) -> Res<(Value, String)> {
	if cfg!(target_os = "macos") {
		let Some(window) = active_mac_window().await? else { return Err("no focused window found".into()) };
		return Ok((mac_capture_settings(kind, current, &window)?, format!("{} ({})", window.name, window.owner)));
	}
	let Some(window) = active_x11_window().await? else {
		return Err(if session_type() == "wayland" {
			"no X11 window has focus (Wayland apps cannot be picked)"
		} else {
			"no focused window found"
		}
		.into());
	};
	Ok((json!({ "capture_window": xcomposite_window_value(&window) }), format!("{} ({})", window.name, window.cls)))
}

/// The focused window on macOS.
#[derive(Debug, Clone, PartialEq)]
struct MacWindow {
	id: u64,
	/// Empty when OpenDeck may not record the screen (System Settings, Privacy).
	name: String,
	/// The app.
	owner: String,
	bundle: String,
}

// JavaScript for Automation: the front window of the app that has focus. It needs no permission.
const MAC_FRONT_WINDOW: &str = r#"
ObjC.import("AppKit");
ObjC.import("CoreGraphics");
function run() {
	const app = $.NSWorkspace.sharedWorkspace.frontmostApplication;
	const pid = app.processIdentifier;
	// 17: only windows on screen, without the desktop. The list goes from front to back.
	const windows = ObjC.deepUnwrap(ObjC.castRefToObject($.CGWindowListCopyWindowInfo(17, 0))) || [];
	const w = windows.find((w) => w.kCGWindowOwnerPID === pid && w.kCGWindowLayer === 0);
	if (!w) return "";
	const bundle = ObjC.unwrap(app.bundleIdentifier) || "";
	return JSON.stringify({ id: w.kCGWindowNumber, name: w.kCGWindowName || "", owner: w.kCGWindowOwnerName || "", bundle });
}
"#;

async fn active_mac_window() -> Res<Option<MacWindow>> {
	let result = run("osascript", &["-l".into(), "JavaScript".into(), "-e".into(), MAC_FRONT_WINDOW.into()], 5000).await;
	if !result.ok {
		return Err("could not read the focused window".into());
	}
	Ok(parse_mac_window(&result.stdout))
}

fn parse_mac_window(text: &str) -> Option<MacWindow> {
	let v: Value = serde_json::from_str(text.trim()).ok()?;
	let text = |key: &str| v[key].as_str().unwrap_or_default().to_string();
	Some(MacWindow { id: v["id"].as_u64().filter(|id| *id > 0)?, name: text("name"), owner: text("owner"), bundle: text("bundle") })
}

fn mac_capture_settings(kind: &str, current: &Value, window: &MacWindow) -> Res<Value> {
	match kind {
		// macOS Screen Capture. "type": 0 a display, 1 a window, 2 an app. An app capture stays one.
		"screen_capture" if current["type"].as_i64() == Some(2) && !window.bundle.is_empty() => Ok(json!({ "application": window.bundle })),
		"screen_capture" => Ok(json!({ "type": 1, "window": window.id })),
		// The old Window Capture. It finds the window again by its names.
		"window_capture" => Ok(json!({ "window": window.id, "owner_name": window.owner, "window_name": window.name })),
		_ => Err("this source is not a window capture".into()),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn xcomposite_window_value_uses_the_obs_format() {
		assert_eq!(
			xcomposite_window_value(&Window { id: 123, name: "Firefox".into(), cls: "Navigator".into() }),
			"123\r\nFirefox\r\nNavigator"
		);
	}

	#[test]
	fn mac_windows() {
		let text = r#"{"id":4711,"name":"Inbox","owner":"Mail","bundle":"com.apple.mail"}"#;
		let window = parse_mac_window(&format!("{text}\n")).expect("window");
		assert_eq!(window, MacWindow { id: 4711, name: "Inbox".into(), owner: "Mail".into(), bundle: "com.apple.mail".into() });
		assert_eq!(parse_mac_window(""), None);
		assert_eq!(parse_mac_window(r#"{"id":0}"#), None);

		let display = json!({});
		assert_eq!(mac_capture_settings("screen_capture", &display, &window).ok(), Some(json!({ "type": 1, "window": 4711 })));
		let app = json!({ "type": 2, "application": "com.apple.Safari" });
		assert_eq!(mac_capture_settings("screen_capture", &app, &window).ok(), Some(json!({ "application": "com.apple.mail" })));
		assert_eq!(
			mac_capture_settings("window_capture", &display, &window).ok(),
			Some(json!({ "window": 4711, "owner_name": "Mail", "window_name": "Inbox" }))
		);
		assert!(mac_capture_settings("image_source", &display, &window).is_err());
	}

	// The script for macOS, run with a fake macOS: the front window of the app that has focus.
	#[test]
	fn mac_script_finds_the_front_window() {
		let fake = r#"
			const ObjC = { import() {}, deepUnwrap: (x) => x, castRefToObject: (x) => x, unwrap: (x) => x };
			const $ = {
				NSWorkspace: { sharedWorkspace: { frontmostApplication: { processIdentifier: 42, bundleIdentifier: "com.apple.mail" } } },
				CGWindowListCopyWindowInfo: () => [
					{ kCGWindowOwnerPID: 7, kCGWindowLayer: 0, kCGWindowNumber: 1 },
					{ kCGWindowOwnerPID: 42, kCGWindowLayer: 25, kCGWindowNumber: 2 },
					{ kCGWindowOwnerPID: 42, kCGWindowLayer: 0, kCGWindowNumber: 3, kCGWindowName: "Inbox", kCGWindowOwnerName: "Mail" },
				],
			};
		"#;
		let mut context = boa_engine::Context::default();
		let out = context.eval(boa_engine::Source::from_bytes(&format!("{fake}{MAC_FRONT_WINDOW}\nrun()"))).expect("script runs");
		let text = out.to_string(&mut context).expect("text").to_std_string_escaped();
		let mail = MacWindow { id: 3, name: "Inbox".into(), owner: "Mail".into(), bundle: "com.apple.mail".into() };
		assert_eq!(parse_mac_window(&text), Some(mail));
	}

	#[test]
	fn folder_paths_lose_the_slash() {
		assert_eq!(without_slash("/Users/me/Movies/"), "/Users/me/Movies");
		assert_eq!(without_slash("/Users/me"), "/Users/me");
		assert_eq!(without_slash("/"), "/");
	}

	#[test]
	fn xprop_values() {
		let text = "_NET_WM_NAME(UTF8_STRING) = \"Firefox\"\nWM_NAME(STRING) = \"x\"\nWM_CLASS(STRING) = \"Navigator\", \"firefox\"\n";
		assert_eq!(parse_xprop(text, "_NET_WM_NAME").as_deref(), Some("\"Firefox\""));
		assert_eq!(parse_xprop(text, "WM_CLASS").as_deref(), Some("\"Navigator\", \"firefox\""));
		assert_eq!(parse_xprop(text, "NOPE"), None);
	}
}
