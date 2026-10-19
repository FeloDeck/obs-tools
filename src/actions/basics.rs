// Everyday OBS actions. On Windows these come with Elgato's own OBS plugin, on Linux they do not.
use async_trait::async_trait;
use serde_json::{Map, Value, json};

use super::{Behavior, KeyRc, Kind, PressArgs, Stop, defaults, patch};
use crate::Res;
use crate::icons::{colors, media_glyph};
use crate::js::{self, truthy};
use crate::render::{GlyphKey, glyph_key};
use crate::text_gestures::{TEXT_GESTURES, text_gesture, text_kind};
use crate::time::{format_duration, parse_duration};

// path.join of Node: one slash between, "." and ".." resolved.
pub fn path_join(dir: &str, file: &str) -> String {
	let joined = if dir.is_empty() { file.to_string() } else { format!("{dir}/{file}") };
	if joined.is_empty() {
		return ".".into();
	}
	let absolute = joined.starts_with('/');
	let mut parts: Vec<&str> = Vec::new();
	for segment in joined.split('/') {
		match segment {
			"" | "." => {}
			".." => {
				if parts.last().is_some_and(|p| *p != "..") {
					parts.pop();
				} else if !absolute {
					parts.push("..");
				}
			}
			s => parts.push(s),
		}
	}
	let body = parts.join("/");
	match (absolute, body.is_empty()) {
		(true, _) => format!("/{body}"),
		(false, true) => ".".into(),
		(false, false) => body,
	}
}

// ----- Source Visibility: show or hide a source in a scene. -----

#[derive(Default)]
struct VisibilityState {
	item_id: Option<Value>,
	enabled: bool,
}

struct SourceVisibility;

fn vis(k: &KeyRc) -> std::cell::RefMut<'_, VisibilityState> {
	k.x::<VisibilityState>()
}

/// The scene of the key, or the live scene without one.
pub fn scene_name(k: &KeyRc, key: &str) -> Value {
	let scene = k.s_val(key);
	if truthy(Some(&scene)) {
		return scene;
	}
	k.data().program_scene.map_or(Value::Null, Value::from)
}

async fn load_visibility(k: &KeyRc) {
	{
		let mut s = vis(k);
		s.item_id = None;
		s.enabled = false;
	}
	let scene = scene_name(k, "scene");
	if !truthy(Some(&scene)) || !k.s_truthy("source") {
		return;
	}
	let result: Res<()> = async {
		let item = k.call("GetSceneItemId", json!({ "sceneName": scene, "sourceName": k.s_val("source") })).await?;
		let id = item.get("sceneItemId").cloned().unwrap_or(Value::Null);
		vis(k).item_id = Some(id.clone());
		let enabled = k.call("GetSceneItemEnabled", json!({ "sceneName": scene, "sceneItemId": id })).await?;
		vis(k).enabled = truthy(enabled.get("sceneItemEnabled"));
		Ok(())
	}
	.await;
	if result.is_err() {
		vis(k).item_id = None;
	}
}

#[async_trait(?Send)]
impl Behavior for SourceVisibility {
	async fn on_connected(&self, k: &KeyRc) -> Res<()> {
		load_visibility(k).await;
		Ok(())
	}

	async fn on_settings_changed(&self, k: &KeyRc, _old: &Map<String, Value>) -> Res<()> {
		if k.online() {
			load_visibility(k).await;
		}
		Ok(())
	}

	async fn on_state_change(&self, k: &KeyRc, what: &str) -> Res<()> {
		// Without a fixed scene the key follows the live scene.
		if !k.s_truthy("scene") && what == "program" {
			load_visibility(k).await;
		}
		self.render(k);
		Ok(())
	}

	async fn on_obs_event(&self, k: &KeyRc, kind: &str, data: &Value) -> Res<()> {
		let same_scene = data.get("sceneName") == Some(&scene_name(k, "scene"));
		if kind == "SceneItemEnableStateChanged"
			&& same_scene
			&& vis(k).item_id.as_ref().is_some_and(|id| data.get("sceneItemId").is_some_and(|d| js::same(d, id)))
		{
			vis(k).enabled = truthy(data.get("sceneItemEnabled"));
			self.render(k);
		} else if (kind == "SceneItemCreated" || kind == "SceneItemRemoved") && same_scene {
			let k = k.clone();
			tokio::task::spawn_local(async move {
				load_visibility(&k).await;
				k.render();
			});
		}
		Ok(())
	}

	fn render(&self, k: &KeyRc) {
		let enabled = vis(k).enabled;
		k.set_state(if enabled { 1 } else { 0 });
		k.show_name_title(&k.s_str("source"));
	}

	async fn on_press(&self, k: &KeyRc, args: Option<PressArgs>) -> Res<()> {
		let scene = scene_name(k, "scene");
		if vis(k).item_id.is_none() {
			load_visibility(k).await;
		}
		let Some(item_id) = vis(k).item_id.clone() else {
			return Err(format!("\"{}\" is not in scene \"{}\"", js::to_string(k.s("source").as_ref()), js::to_string(Some(&scene))).into());
		};
		let mode = k.op(&args);
		let enabled = match mode.as_str() {
			"show" => true,
			"hide" => false,
			_ => !vis(k).enabled,
		};
		k.call("SetSceneItemEnabled", json!({ "sceneName": scene, "sceneItemId": item_id, "sceneItemEnabled": enabled })).await?;
		vis(k).enabled = enabled;
		self.render(k);
		Ok(())
	}
}

pub static SOURCE_VISIBILITY: Kind = Kind {
	uuid: "de.felitendo.obs-tools.sourcevisibility",
	name: "SourceVisibility",
	defaults: || defaults(json!({ "scene": "", "source": "", "mode": "toggle", "hold": "none", "double": "none", "showName": false })),
	press_key: "mode",
	ops: &["toggle", "show", "hide"],
	press_only: &[],
	can_repeat: false,
	required: &["source"],
	behavior: &SourceVisibility,
	new_state: || Box::new(VisibilityState::default()),
};

// ----- OBS Hotkey: by its name, or by the keys it is bound to in OBS. -----

struct Hotkey;

#[async_trait(?Send)]
impl Behavior for Hotkey {
	fn is_set_up(&self, k: &KeyRc) -> bool {
		if k.s_str("mode") == "keys" { k.s_truthy("key") } else { k.s_truthy("hotkey") }
	}

	async fn on_press(&self, k: &KeyRc, _args: Option<PressArgs>) -> Res<()> {
		if k.s_str("mode") == "keys" {
			if !k.s_truthy("key") {
				return Err("no key set".into());
			}
			let modifiers = json!({ "shift": k.s_truthy("shift"), "control": k.s_truthy("control"), "alt": k.s_truthy("alt"), "command": k.s_truthy("command") });
			k.call("TriggerHotkeyByKeySequence", json!({ "keyId": k.s_val("key"), "keyModifiers": modifiers })).await?;
		} else {
			if !k.s_truthy("hotkey") {
				return Err("no hotkey selected".into());
			}
			let mut data = Map::new();
			data.insert("hotkeyName".into(), k.s_val("hotkey"));
			if k.s_truthy("context") {
				data.insert("contextName".into(), k.s_val("context"));
			}
			k.call("TriggerHotkeyByName", Value::Object(data)).await?;
		}
		k.show_ok();
		Ok(())
	}
}

pub static HOTKEY: Kind = Kind {
	uuid: "de.felitendo.obs-tools.hotkey",
	name: "Hotkey",
	defaults: || {
		defaults(
			json!({ "mode": "name", "hotkey": "", "context": "", "key": "OBS_KEY_F13", "shift": false, "control": false, "alt": false, "command": false }),
		)
	},
	press_key: "",
	ops: &[],
	press_only: &[],
	can_repeat: false,
	required: &[],
	behavior: &Hotkey,
	new_state: || Box::new(()),
};

// ----- Screenshot: save a picture of a scene or source as a file. -----

struct Screenshot;

#[async_trait(?Send)]
impl Behavior for Screenshot {
	async fn on_press(&self, k: &KeyRc, _args: Option<PressArgs>) -> Res<()> {
		let source = k.s_val("source");
		let source_name = if truthy(Some(&source)) { source } else { k.data().program_scene.map_or(Value::Null, Value::from) };
		let dir = if k.s_truthy("folder") {
			k.s_str("folder")
		} else {
			js::to_string(k.call("GetRecordDirectory", json!({})).await?.get("recordDirectory"))
		};
		let stamp = js::iso_time(js::now())[..19].replace('T', " ").replace(':', "-");
		let ext = if k.s_str("format") == "jpg" { "jpg" } else { "png" };
		let safe_name: String =
			js::to_string(Some(&source_name)).chars().map(|c| if r#"\/:*?"<>|"#.contains(c) { '_' } else { c }).collect();
		let file = path_join(&dir, &format!("Screenshot {safe_name} {stamp}.{ext}"));
		let mut data = Map::new();
		data.insert("sourceName".into(), source_name);
		data.insert("imageFormat".into(), ext.into());
		data.insert("imageFilePath".into(), file.clone().into());
		let width = k.s_num("width");
		if width > 0.0 {
			data.insert("imageWidth".into(), js::num(js::round(width)));
		}
		if ext == "jpg" {
			data.insert("imageCompressionQuality".into(), 90.into());
		}
		k.call("SaveSourceScreenshot", Value::Object(data)).await?;
		k.log(&format!("screenshot saved: {file}"));
		k.show_ok();
		Ok(())
	}
}

pub static SCREENSHOT: Kind = Kind {
	uuid: "de.felitendo.obs-tools.screenshot",
	name: "Screenshot",
	defaults: || defaults(json!({ "source": "", "folder": "", "format": "png", "width": 0 })),
	press_key: "",
	ops: &[],
	press_only: &[],
	can_repeat: false,
	required: &[],
	behavior: &Screenshot,
	new_state: || Box::new(()),
};

// ----- Recording Marker: chapter marker or file split while recording. -----

struct RecordTools;

fn record_tools_glyph(k: &KeyRc) -> &'static str {
	if k.s_str("operation") == "split" { "split" } else { "chapter" }
}

#[async_trait(?Send)]
impl Behavior for RecordTools {
	fn render_offline(&self, k: &KeyRc) {
		k.set_image(&glyph_key(record_tools_glyph(k), &GlyphKey { color: colors::GRAY, ..Default::default() }));
	}

	fn render(&self, k: &KeyRc) {
		let recording = k.data().record.active;
		k.set_image(&glyph_key(
			record_tools_glyph(k),
			&GlyphKey { color: if recording { colors::WHITE } else { colors::GRAY }, ..Default::default() },
		));
	}

	async fn on_press(&self, k: &KeyRc, args: Option<PressArgs>) -> Res<()> {
		if !k.data().record.active {
			return Err("not recording".into());
		}
		if k.op(&args) == "split" {
			k.call("SplitRecordFile", json!({})).await?;
		} else {
			let mut data = Map::new();
			let name = js::trim(&k.s_str("chapterName")).to_string();
			if !name.is_empty() {
				data.insert("chapterName".into(), name.into());
			}
			k.call("CreateRecordChapter", Value::Object(data)).await?;
		}
		k.show_ok();
		Ok(())
	}
}

pub static RECORD_TOOLS: Kind = Kind {
	uuid: "de.felitendo.obs-tools.recordtools",
	name: "RecordTools",
	defaults: || defaults(json!({ "operation": "chapter", "hold": "none", "double": "none", "chapterName": "" })),
	press_key: "operation",
	ops: &["chapter", "split"],
	press_only: &[],
	can_repeat: false,
	required: &[],
	behavior: &RecordTools,
	new_state: || Box::new(()),
};

// ----- Media Control: play, pause, stop or skip in a media source. Shows the time left. -----

fn media_action(op: &str) -> Option<&'static str> {
	match op {
		"play" => Some("OBS_WEBSOCKET_MEDIA_INPUT_ACTION_PLAY"),
		"pause" => Some("OBS_WEBSOCKET_MEDIA_INPUT_ACTION_PAUSE"),
		"stop" => Some("OBS_WEBSOCKET_MEDIA_INPUT_ACTION_STOP"),
		"restart" => Some("OBS_WEBSOCKET_MEDIA_INPUT_ACTION_RESTART"),
		"next" => Some("OBS_WEBSOCKET_MEDIA_INPUT_ACTION_NEXT"),
		"previous" => Some("OBS_WEBSOCKET_MEDIA_INPUT_ACTION_PREVIOUS"),
		_ => None,
	}
}

#[derive(Default)]
struct MediaState {
	status: Option<Value>,
	stop_poll: Option<Stop>,
}

struct MediaControl;

fn media(k: &KeyRc) -> std::cell::RefMut<'_, MediaState> {
	k.x::<MediaState>()
}

async fn poll_media(k: &KeyRc) {
	if !k.visible() || !k.s_truthy("input") {
		return;
	}
	let status = k.call("GetMediaInputStatus", json!({ "inputName": k.s_val("input") })).await.ok();
	media(k).status = status;
	k.render();
}

fn media_playing(k: &KeyRc) -> bool {
	media(k).status.as_ref().and_then(|s| s.get("mediaState")).and_then(Value::as_str) == Some("OBS_MEDIA_STATE_PLAYING")
}

fn media_glyph_name(k: &KeyRc) -> &'static str {
	let op = k.s_str("operation");
	if op == "playpause" {
		return if media_playing(k) { "pauseMedia" } else { "play" };
	}
	media_glyph(&op).unwrap_or("playPause")
}

#[async_trait(?Send)]
impl Behavior for MediaControl {
	async fn on_connected(&self, k: &KeyRc) -> Res<()> {
		let old = media(k).stop_poll.take();
		if let Some(stop) = old {
			stop();
		}
		let stop = k.interval(1000.0, |k| {
			Box::pin(async move {
				poll_media(&k).await;
				Ok(())
			})
		});
		media(k).stop_poll = Some(stop);
		poll_media(k).await;
		Ok(())
	}

	fn on_disconnected(&self, k: &KeyRc) {
		let stop = media(k).stop_poll.clone();
		if let Some(stop) = stop {
			stop();
		}
	}

	async fn on_settings_changed(&self, k: &KeyRc, _old: &Map<String, Value>) -> Res<()> {
		if k.online() {
			poll_media(k).await;
		}
		Ok(())
	}

	async fn on_obs_event(&self, k: &KeyRc, kind: &str, data: &Value) -> Res<()> {
		if data.get("inputName") == k.s("input").as_ref() && kind.starts_with("MediaInput") {
			let k = k.clone();
			tokio::task::spawn_local(async move { poll_media(&k).await });
		}
		Ok(())
	}

	fn render_offline(&self, k: &KeyRc) {
		let op = k.s_str("operation");
		let glyph = if op == "playpause" { "playPause" } else { media_glyph(&op).unwrap_or("playPause") };
		k.set_image(&glyph_key(glyph, &GlyphKey::default()));
	}

	fn render(&self, k: &KeyRc) {
		let show_time = k.s_truthy("showTime") && k.s_truthy("input");
		let status = media(k).status.clone();
		let field = |name: &str| status.as_ref().and_then(|s| s.get(name)).cloned();
		let cursor = field("mediaCursor").filter(|c| !c.is_null());
		let duration = js::number(field("mediaDuration").as_ref());
		let mut time = String::new();
		// Time played works without a length too, like for a stream as source.
		if let Some(cursor) = &cursor {
			let cursor = js::number(Some(cursor));
			if k.s_str("timeShows") == "played" {
				time = format_duration(cursor / 1000.0);
			} else if duration > 0.0 {
				time = format!("-{}", format_duration((duration - cursor) / 1000.0));
			}
		}
		let paused = field("mediaState").as_ref().and_then(Value::as_str) == Some("OBS_MEDIA_STATE_PAUSED");
		let title = if show_time && (media_playing(k) || paused) { time } else { String::new() };
		// The time goes above or below the glyph, so the key draws it into the image.
		k.own_title(&title);
		let at = k.s_str("timeAt");
		let color = if status.as_ref().is_some_and(|s| truthy(Some(s))) { colors::WHITE } else { colors::GRAY };
		k.set_image(&glyph_key(media_glyph_name(k), &GlyphKey { title: &title, title_at: &at, color, ..Default::default() }));
	}

	async fn on_press(&self, k: &KeyRc, args: Option<PressArgs>) -> Res<()> {
		let input = k.s_val("input");
		let operation = k.op(&args);
		if !truthy(Some(&input)) {
			return Err("no media source selected".into());
		}
		if operation == "forward" || operation == "back" {
			let offset = js::round(k.s_num_or("seconds", 10.0) * 1000.0) * if operation == "back" { -1.0 } else { 1.0 };
			k.call("OffsetMediaInputCursor", json!({ "inputName": input, "mediaCursorOffset": js::num(offset) })).await?;
		} else {
			let mut action = media_action(&operation);
			if operation == "playpause" {
				let status = k.call("GetMediaInputStatus", json!({ "inputName": input })).await?;
				action = match status.get("mediaState").and_then(Value::as_str) {
					Some("OBS_MEDIA_STATE_PLAYING") => media_action("pause"),
					Some("OBS_MEDIA_STATE_ENDED" | "OBS_MEDIA_STATE_STOPPED" | "OBS_MEDIA_STATE_NONE") => media_action("restart"),
					_ => media_action("play"),
				};
			}
			let mut data = Map::new();
			data.insert("inputName".into(), input);
			if let Some(action) = action {
				data.insert("mediaAction".into(), action.into());
			}
			k.call("TriggerMediaInputAction", Value::Object(data)).await?;
		}
		let k = k.clone();
		crate::timers::set_timeout(150.0, move || {
			tokio::task::spawn_local(async move { poll_media(&k).await });
		});
		Ok(())
	}
}

pub static MEDIA_CONTROL: Kind = Kind {
	uuid: "de.felitendo.obs-tools.mediacontrol",
	name: "MediaControl",
	defaults: || {
		defaults(
			json!({ "input": "", "operation": "playpause", "hold": "none", "double": "none", "seconds": 10, "showTime": true, "timeAt": "below", "timeShows": "left" }),
		)
	},
	press_key: "operation",
	ops: &["playpause", "play", "pause", "stop", "restart", "forward", "back", "next", "previous"],
	press_only: &[],
	can_repeat: false,
	required: &["input"],
	behavior: &MediaControl,
	new_state: || Box::new(MediaState::default()),
};

// ----- Profile / Scene Collection -----

struct ProfileSwitch;

fn profile_current(k: &KeyRc) -> Option<String> {
	let data = k.data();
	if k.s_str("kind") == "collection" { data.collection } else { data.profile }
}

fn profile_glyph(k: &KeyRc) -> &'static str {
	if k.s_str("kind") == "collection" { "collection" } else { "profile" }
}

fn profile_title(k: &KeyRc) -> String {
	if k.s_truthy("showName") { k.s_str("name") } else { String::new() }
}

#[async_trait(?Send)]
impl Behavior for ProfileSwitch {
	fn render_offline(&self, k: &KeyRc) {
		let title = profile_title(k);
		k.own_title(&title);
		k.set_image(&glyph_key(profile_glyph(k), &GlyphKey { title: &title, color: colors::GRAY, ..Default::default() }));
	}

	fn render(&self, k: &KeyRc) {
		let active = k.s_truthy("name") && profile_current(k).as_deref() == Some(k.s_str("name").as_str());
		let title = profile_title(k);
		k.own_title(&title);
		k.set_image(&glyph_key(
			profile_glyph(k),
			&GlyphKey { title: &title, color: if active { colors::WHITE } else { colors::GRAY }, ..Default::default() },
		));
	}

	async fn on_press(&self, k: &KeyRc, _args: Option<PressArgs>) -> Res<()> {
		let collection = k.s_str("kind") == "collection";
		if !k.s_truthy("name") {
			return Err(if collection { "no scene collection selected" } else { "no profile selected" }.into());
		}
		let name = k.s_val("name");
		if profile_current(k).map(Value::from).as_ref() == Some(&name) {
			return Ok(());
		}
		if collection {
			k.call("SetCurrentSceneCollection", json!({ "sceneCollectionName": name })).await?;
		} else {
			k.call("SetCurrentProfile", json!({ "profileName": name })).await?;
		}
		Ok(())
	}
}

pub static PROFILE_SWITCH: Kind = Kind {
	uuid: "de.felitendo.obs-tools.profile",
	name: "ProfileSwitch",
	defaults: || defaults(json!({ "kind": "profile", "name": "", "showName": true })),
	press_key: "",
	ops: &[],
	press_only: &[],
	can_repeat: false,
	required: &["name"],
	behavior: &ProfileSwitch,
	new_state: || Box::new(()),
};

// ----- Set Text: a fixed text, the next line of a list, a counter or a timer. -----

#[derive(Default)]
struct TextState {
	text_on: bool,
	time_left: Option<f64>,
	stop_timer: Option<Stop>,
}

struct SetText;

fn text_state(k: &KeyRc) -> std::cell::RefMut<'_, TextState> {
	k.x::<TextState>()
}

fn lines(k: &KeyRc) -> Vec<String> {
	k.s_str("lines").split('\n').map(|l| l.trim_end_matches(js::is_js_space).to_string()).filter(|l| !l.is_empty()).collect()
}

fn counter_text(k: &KeyRc, count: f64) -> String {
	let template = if k.s_truthy("template") { k.s_str("template") } else { "{n}".into() };
	if template.contains("{n}") { template.replace("{n}", &js::fmt(count)) } else { format!("{template}{}", js::fmt(count)) }
}

/// The timer text: "{t}" becomes the time left. At 0 the end text, if there is one.
fn timer_text(k: &KeyRc, seconds: f64) -> String {
	let end = k.s_str("timerEnd");
	if seconds <= 0.0 && !end.is_empty() {
		return end;
	}
	let template = if k.s_truthy("timerText") { k.s_str("timerText") } else { "{t}".into() };
	let time = format_duration(seconds);
	if template.contains("{t}") { template.replace("{t}", &time) } else { format!("{template}{time}") }
}

/// What the key shows: what was set last, or what a press would set.
fn preview(k: &KeyRc) -> String {
	let shown = k.s_str("shown");
	let kind = if !shown.is_empty() { shown } else { text_kind(&k.gesture("press")).unwrap_or("").to_string() };
	match kind.as_str() {
		"count" => counter_text(k, k.s_num_or("count", 0.0)),
		"timer" => {
			let left = text_state(k).time_left;
			timer_text(k, left.unwrap_or_else(|| parse_duration(&k.s_str("timerLength"))))
		}
		"line" => {
			let index = k.s_num("index");
			let lines = lines(k);
			if index >= 0.0 && index.fract() == 0.0 { lines.get(index as usize).cloned().unwrap_or_default() } else { String::new() }
		}
		"text" => k.s_str("text"),
		_ => String::new(),
	}
}

// Text on or off: the key is white while the source has this key's text, else gray.
fn uses_toggle(k: &KeyRc) -> bool {
	let settings = k.settings();
	TEXT_GESTURES.iter().any(|g| text_gesture(&settings, g) == "toggle")
}

async fn read_toggle(k: &KeyRc) {
	if !uses_toggle(k) || !k.s_truthy("input") {
		return;
	}
	let text = k.s_val("text");
	let on = match k.call("GetInputSettings", json!({ "inputName": k.s_val("input") })).await {
		Ok(r) => truthy(Some(&text)) && r.get("inputSettings").and_then(|s| s.get("text")) == Some(&text),
		Err(_) => false,
	};
	text_state(k).text_on = on;
}

async fn set_text(k: &KeyRc, text: &str) -> Res<()> {
	let input = k.s_val("input");
	if !truthy(Some(&input)) {
		return Err("no text source selected".into());
	}
	k.call("SetInputSettings", json!({ "inputName": input, "inputSettings": { "text": text }, "overlay": true })).await?;
	Ok(())
}

async fn set_count(k: &KeyRc, count: f64) -> Res<()> {
	set_text(k, &counter_text(k, count)).await?;
	k.save_settings(patch(vec![("count", js::num(count))]));
	Ok(())
}

async fn set_line(k: &KeyRc, index: f64) -> Res<()> {
	let lines = lines(k);
	if lines.is_empty() {
		return Err("the list is empty".into());
	}
	let len = lines.len() as f64;
	let index = ((index % len) + len) % len;
	let line = lines.get(index as usize).cloned().unwrap_or_default();
	set_text(k, &line).await?;
	k.save_settings(patch(vec![("index", js::num(index))]));
	Ok(())
}

// Counts down by itself, once a second. Running: the next start stops it.
async fn toggle_timer(k: &KeyRc) -> Res<()> {
	let running = text_state(k).stop_timer.take();
	if let Some(stop) = running {
		stop();
		return Ok(());
	}
	let length = parse_duration(&k.s_str("timerLength"));
	if length == 0.0 {
		return Err("no timer length set".into());
	}
	let end = js::now() as f64 + length * 1000.0;
	let shown = std::rc::Rc::new(std::cell::Cell::new(f64::NAN));
	let tick = move |k: KeyRc| -> crate::obs::Reply<()> {
		let shown = shown.clone();
		Box::pin(async move {
			let left = js::max(0.0, ((end - js::now() as f64) / 1000.0).ceil());
			if left != shown.get() {
				shown.set(left);
				text_state(&k).time_left = Some(left);
				set_text(&k, &timer_text(&k, left)).await?;
				k.render();
			}
			if left == 0.0 {
				let stop = text_state(&k).stop_timer.take();
				if let Some(stop) = stop {
					stop();
				}
			}
			Ok(())
		})
	};
	let tick = std::rc::Rc::new(tick);
	let stop = {
		let tick = tick.clone();
		k.interval(200.0, move |k| tick(k))
	};
	text_state(k).stop_timer = Some(stop);
	tick(k.clone()).await
}

async fn count(k: &KeyRc, direction: f64) -> Res<()> {
	let step = k.s_num_or("step", 1.0);
	set_count(k, js::max(0.0, k.s_num_or("count", 0.0) + direction * step)).await
}

async fn run(k: &KeyRc, what: &str) -> Res<()> {
	let shown_before = k.s("shown");
	match what {
		"set" => set_text(k, &k.s_str("text")).await?,
		// Off when the source shows this text, else on.
		"toggle" => {
			let input = k.s_val("input");
			if !truthy(Some(&input)) {
				return Err("no text source selected".into());
			}
			let text = k.s_str("text");
			let current = k.call("GetInputSettings", json!({ "inputName": input })).await?;
			let on = !text.is_empty() && current.get("inputSettings").and_then(|s| s.get("text")) == Some(&Value::from(text.as_str()));
			set_text(k, if on { "" } else { &text }).await?;
			text_state(k).text_on = !on;
		}
		// index -1 means nothing was set yet, so the first "next" sets line 0.
		"next" => set_line(k, k.s_num_or("index", 0.0) + 1.0).await?,
		"previous" => set_line(k, k.s_num_or("index", 0.0) - 1.0).await?,
		"first" => set_line(k, 0.0).await?,
		"up" => count(k, 1.0).await?,
		"down" => count(k, -1.0).await?,
		"reset" => set_count(k, k.s_num_or("start", 0.0)).await?,
		"timer" => toggle_timer(k).await?,
		"clear" => set_text(k, "").await?,
		_ => return Ok(()),
	}
	let kind = text_kind(what).map(Value::from);
	if shown_before != kind {
		k.save_settings(patch(vec![("shown", kind.unwrap_or(Value::Null))]));
	}
	// Hold or double press can change the text too.
	if what != "toggle" {
		read_toggle(k).await;
	}
	k.render();
	Ok(())
}

#[async_trait(?Send)]
impl Behavior for SetText {
	// Press, hold and double press pick from TEXT_ACTIONS (the base does the rest).
	fn gesture(&self, k: &KeyRc, name: &str) -> String {
		text_gesture(&k.settings(), name)
	}

	// A new start number puts the counter there.
	async fn on_settings_changed(&self, k: &KeyRc, old: &Map<String, Value>) -> Res<()> {
		let start = k.s_num_or("start", 0.0);
		if js::number_or(old.get("start"), 0.0) != start {
			k.save_settings(patch(vec![("count", js::num(start))]));
		}
		if k.online() && (old.get("input") != k.s("input").as_ref() || old.get("text") != k.s("text").as_ref()) {
			read_toggle(k).await;
		}
		Ok(())
	}

	async fn on_connected(&self, k: &KeyRc) -> Res<()> {
		read_toggle(k).await;
		Ok(())
	}

	// The text can also change in OBS or by another key.
	async fn on_obs_event(&self, k: &KeyRc, kind: &str, data: &Value) -> Res<()> {
		if kind != "InputSettingsChanged" || data.get("inputName") != k.s("input").as_ref() || !uses_toggle(k) {
			return Ok(());
		}
		let Some(Value::String(text)) = data.get("inputSettings").and_then(|s| s.get("text")) else { return Ok(()) };
		let mine = k.s_val("text");
		text_state(k).text_on = truthy(Some(&mine)) && mine == text.as_str();
		self.render(k);
		Ok(())
	}

	fn render_offline(&self, k: &KeyRc) {
		self.render(k);
	}

	fn render(&self, k: &KeyRc) {
		let title = if k.s_truthy("showText") { js::trim(preview(k).split('\n').next().unwrap_or("")).to_string() } else { String::new() };
		let off = uses_toggle(k) && !text_state(k).text_on;
		k.own_title(&title);
		k.set_image(&glyph_key(
			"text",
			&GlyphKey { title: &title, color: if off { colors::GRAY } else { colors::WHITE }, ..Default::default() },
		));
	}

	// The timer stops with the key (another profile), so do not keep a dead one.
	fn on_disappear(&self, k: &KeyRc) {
		text_state(k).stop_timer = None;
	}

	async fn on_press(&self, k: &KeyRc, args: Option<PressArgs>) -> Res<()> {
		run(k, &k.op(&args)).await
	}
}

pub static SET_TEXT: Kind = Kind {
	uuid: "de.felitendo.obs-tools.settext",
	name: "SetText",
	// press, hold and double each pick one of TEXT_ACTIONS. shown: what the key shows ("text", "line" or "count").
	defaults: || {
		defaults(json!({
			"input": "",
			"press": "set",
			"hold": "none",
			"double": "none",
			"text": "",
			"lines": "",
			"index": -1,
			"template": "{n}",
			"start": 0,
			"count": 0,
			"step": 1,
			"timerText": "{t}",
			"timerLength": "5:00",
			"timerEnd": "",
			"shown": "",
			"showText": true,
		}))
	},
	press_key: "",
	ops: &[],
	press_only: &[],
	can_repeat: false,
	required: &["input"],
	behavior: &SetText,
	new_state: || Box::new(TextState::default()),
};

#[cfg(test)]
mod tests {
	use super::super::testing::*;
	use super::*;

	fn written(k: &KeyRc) -> Vec<String> {
		calls(k)
			.iter()
			.filter(|(t, _)| t == "SetInputSettings")
			.map(|(_, d)| d["inputSettings"]["text"].as_str().unwrap_or("").to_string())
			.collect()
	}

	#[tokio::test]
	async fn set_text_press_hold_and_double_press_can_each_do_anything() {
		// A countdown: press counts down and stops at 0, hold resets to the start number.
		let (_p, k) =
			key(&SET_TEXT, json!({ "input": "T", "press": "down", "hold": "reset", "template": "Lives: {n}", "start": 2, "count": 2 }));
		for _ in 0..3 {
			SetText.on_press(&k, None).await.unwrap_or_default();
		}
		assert_eq!(written(&k), ["Lives: 1", "Lives: 0", "Lives: 0"]);
		k.press("hold", false).await.unwrap_or_default();
		assert_eq!(written(&k).last().map(String::as_str), Some("Lives: 2"));
		// Any mix: press sets a text, hold goes to the next line, double press counts up.
		let (_p, mixed) = key(
			&SET_TEXT,
			json!({ "input": "T", "press": "set", "hold": "next", "double": "up", "text": "BRB", "lines": "One\nTwo", "template": "{n}" }),
		);
		assert_eq!([mixed.gesture("hold"), mixed.gesture("double")], ["next", "up"]);
		mixed.press("press", false).await.unwrap_or_default();
		mixed.press("hold", false).await.unwrap_or_default();
		mixed.press("double", false).await.unwrap_or_default();
		assert_eq!(written(&mixed), ["BRB", "One", "1"]);
		// The key shows what was set last.
		assert_eq!(preview(&mixed), "1");
		run(&mixed, "clear").await.unwrap_or_default();
		assert_eq!(preview(&mixed), "");
		// Unknown values fall back: press sets the text, the others do nothing.
		let (_p, bogus) = key(&SET_TEXT, json!({ "press": "bogus", "hold": "bogus" }));
		assert_eq!(bogus.gesture("press"), "set");
		assert_eq!(bogus.gesture("hold"), "none");
	}

	#[test]
	fn timer_texts() {
		let (_p, k) = key(&SET_TEXT, json!({ "timerText": "Live in {t}", "timerEnd": "Live!" }));
		assert_eq!(timer_text(&k, 65.0), "Live in 1:05");
		assert_eq!(timer_text(&k, 0.0), "Live!");
	}

	#[test]
	fn paths_join_like_node() {
		assert_eq!(path_join("/home/me/Videos/", "a b.png"), "/home/me/Videos/a b.png");
		assert_eq!(path_join("/home/me//x/../Videos", "a.png"), "/home/me/Videos/a.png");
		assert_eq!(path_join("", "a.png"), "a.png");
		assert_eq!(path_join("rel", "a.png"), "rel/a.png");
	}
}
