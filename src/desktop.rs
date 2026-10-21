// Small desktop helpers: native file and folder pickers, and the focused X11 window.
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

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
