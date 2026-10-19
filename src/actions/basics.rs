// Everyday OBS actions. On Windows these come with Elgato's own OBS plugin, on Linux they do not.
use std::path::Path;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{Behavior, KeyRc, Kind, Press, Stop, defaults, same_name};
use crate::Res;
use crate::icons::{colors, media_glyph};
use crate::js::{self, truthy};
use crate::obs_state::Change;
use crate::render::{GlyphKey, glyph_key};
use crate::settings;
use crate::time::format_duration;

/// The scene of the key, or the live scene without one.
pub fn scene_or_live(k: &KeyRc, scene: &str) -> Option<String> {
	if scene.is_empty() { k.data().program_scene } else { Some(scene.to_string()) }
}

// ----- Source Visibility: show or hide a source in a scene. -----

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct VisibilitySettings {
	/// Empty: the live scene.
	scene: String,
	source: String,
	show_name: bool,
}

#[derive(Default)]
struct VisibilityState {
	item_id: Option<Value>,
	enabled: bool,
}

struct SourceVisibility;

fn vis(k: &KeyRc) -> std::cell::RefMut<'_, VisibilityState> {
	k.own::<VisibilityState>()
}

async fn load_visibility(k: &KeyRc) {
	{
		let mut s = vis(k);
		s.item_id = None;
		s.enabled = false;
	}
	let s: VisibilitySettings = k.settings();
	let Some(scene) = scene_or_live(k, &s.scene).filter(|_| !s.source.is_empty()) else { return };
	let result: Res<()> = async {
		let item = k.call("GetSceneItemId", json!({ "sceneName": scene, "sourceName": s.source })).await?;
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

	async fn on_state_change(&self, k: &KeyRc, change: Change) -> Res<()> {
		// Without a fixed scene the key follows the live scene.
		if k.settings::<VisibilitySettings>().scene.is_empty() && change == Change::Program {
			load_visibility(k).await;
		}
		self.render(k);
		Ok(())
	}

	async fn on_obs_event(&self, k: &KeyRc, kind: &str, data: &Value) -> Res<()> {
		let s: VisibilitySettings = k.settings();
		let same_scene = scene_or_live(k, &s.scene).is_some_and(|scene| same_name(data, "sceneName", &scene));
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
		let s: VisibilitySettings = k.settings();
		let enabled = vis(k).enabled;
		k.set_state(if enabled { 1 } else { 0 });
		k.glyph_for_title(if s.show_name { &s.source } else { "" });
	}

	async fn on_press(&self, k: &KeyRc, press: Press) -> Res<()> {
		let s: VisibilitySettings = k.settings();
		let scene = scene_or_live(k, &s.scene);
		if vis(k).item_id.is_none() {
			load_visibility(k).await;
		}
		let Some(item_id) = vis(k).item_id.clone() else {
			return Err(format!("\"{}\" is not in scene \"{}\"", s.source, scene.unwrap_or_default()).into());
		};
		let enabled = match press.op.as_str() {
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
	required: &["source"],
	behavior: &SourceVisibility,
	new_state: || Box::new(VisibilityState::default()),
	..Kind::BASE
};

// ----- OBS Hotkey: by its name, or by the keys it is bound to in OBS. -----

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct HotkeySettings {
	/// "name" or "keys".
	mode: String,
	hotkey: String,
	/// The source of a source hotkey, empty for the others.
	context: String,
	/// Like "OBS_KEY_F13".
	key: String,
	shift: bool,
	control: bool,
	alt: bool,
	command: bool,
}

struct Hotkey;

#[async_trait(?Send)]
impl Behavior for Hotkey {
	fn is_set_up(&self, k: &KeyRc) -> bool {
		let s: HotkeySettings = k.settings();
		if s.mode == "keys" { !s.key.is_empty() } else { !s.hotkey.is_empty() }
	}

	async fn on_press(&self, k: &KeyRc, _press: Press) -> Res<()> {
		let s: HotkeySettings = k.settings();
		if s.mode == "keys" {
			if s.key.is_empty() {
				return Err("no key set".into());
			}
			let modifiers = json!({ "shift": s.shift, "control": s.control, "alt": s.alt, "command": s.command });
			k.call("TriggerHotkeyByKeySequence", json!({ "keyId": s.key, "keyModifiers": modifiers })).await?;
		} else {
			if s.hotkey.is_empty() {
				return Err("no hotkey selected".into());
			}
			let mut data = Map::new();
			data.insert("hotkeyName".into(), s.hotkey.into());
			if !s.context.is_empty() {
				data.insert("contextName".into(), s.context.into());
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
	behavior: &Hotkey,
	..Kind::BASE
};

// ----- Screenshot: save a picture of a scene or source as a file. -----

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct ScreenshotSettings {
	/// Empty: the live scene.
	source: String,
	/// Empty: the recording folder of OBS.
	folder: String,
	/// "png" or "jpg".
	format: String,
	/// 0: full size.
	width: f64,
}

struct Screenshot;

#[async_trait(?Send)]
impl Behavior for Screenshot {
	async fn on_press(&self, k: &KeyRc, _press: Press) -> Res<()> {
		let s: ScreenshotSettings = k.settings();
		let source = scene_or_live(k, &s.source);
		let dir = if s.folder.is_empty() {
			js::to_string(k.call("GetRecordDirectory", json!({})).await?.get("recordDirectory"))
		} else {
			s.folder
		};
		let stamp = js::iso_time(js::now())[..19].replace('T', " ").replace(':', "-");
		let ext = if s.format == "jpg" { "jpg" } else { "png" };
		let safe_name: String =
			source.as_deref().unwrap_or_default().chars().map(|c| if r#"\/:*?"<>|"#.contains(c) { '_' } else { c }).collect();
		let file = Path::new(&dir).join(format!("Screenshot {safe_name} {stamp}.{ext}")).to_string_lossy().into_owned();
		let mut data = Map::new();
		data.insert("sourceName".into(), source.into());
		data.insert("imageFormat".into(), ext.into());
		data.insert("imageFilePath".into(), file.clone().into());
		if s.width > 0.0 {
			data.insert("imageWidth".into(), js::num(js::round(s.width)));
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
	behavior: &Screenshot,
	..Kind::BASE
};

// ----- Recording Marker: chapter marker or file split while recording. -----

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct RecordToolsSettings {
	/// "chapter" or "split".
	operation: String,
	chapter_name: String,
}

struct RecordTools;

fn record_tools_glyph(k: &KeyRc) -> &'static str {
	if k.settings::<RecordToolsSettings>().operation == "split" { "split" } else { "chapter" }
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

	async fn on_press(&self, k: &KeyRc, press: Press) -> Res<()> {
		if !k.data().record.active {
			return Err("not recording".into());
		}
		if press.op == "split" {
			k.call("SplitRecordFile", json!({})).await?;
		} else {
			let mut data = Map::new();
			let name = js::trim(&k.settings::<RecordToolsSettings>().chapter_name).to_string();
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
	behavior: &RecordTools,
	..Kind::BASE
};

// ----- Media Control: play, pause, stop or skip in a media source. Shows the time left. -----

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct MediaSettings {
	input: String,
	/// What a press does (one of the ops).
	operation: String,
	/// How far forward and back skip.
	seconds: f64,
	show_time: bool,
	time_at: String,
	/// "left" or "played".
	time_shows: String,
}

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
	k.own::<MediaState>()
}

async fn poll_media(k: &KeyRc) {
	let input = k.settings::<MediaSettings>().input;
	if !k.visible() || input.is_empty() {
		return;
	}
	let status = k.call("GetMediaInputStatus", json!({ "inputName": input })).await.ok();
	media(k).status = status;
	k.render();
}

fn media_playing(k: &KeyRc) -> bool {
	media(k).status.as_ref().and_then(|s| s.get("mediaState")).and_then(Value::as_str) == Some("OBS_MEDIA_STATE_PLAYING")
}

fn media_glyph_name(k: &KeyRc, operation: &str) -> &'static str {
	if operation == "playpause" {
		return if media_playing(k) { "pauseMedia" } else { "play" };
	}
	media_glyph(operation).unwrap_or("playPause")
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
		if same_name(data, "inputName", &k.settings::<MediaSettings>().input) && kind.starts_with("MediaInput") {
			let k = k.clone();
			tokio::task::spawn_local(async move { poll_media(&k).await });
		}
		Ok(())
	}

	fn render_offline(&self, k: &KeyRc) {
		let op = k.settings::<MediaSettings>().operation;
		let glyph = if op == "playpause" { "playPause" } else { media_glyph(&op).unwrap_or("playPause") };
		k.set_image(&glyph_key(glyph, &GlyphKey::default()));
	}

	fn render(&self, k: &KeyRc) {
		let s: MediaSettings = k.settings();
		let show_time = s.show_time && !s.input.is_empty();
		let status = media(k).status.clone();
		let field = |name: &str| status.as_ref().and_then(|s| s.get(name)).cloned();
		let cursor = field("mediaCursor").filter(|c| !c.is_null());
		let duration = js::number(field("mediaDuration").as_ref());
		let mut time = String::new();
		// Time played works without a length too, like for a stream as source.
		if let Some(cursor) = &cursor {
			let cursor = js::number(Some(cursor));
			if s.time_shows == "played" {
				time = format_duration(cursor / 1000.0);
			} else if duration > 0.0 {
				time = format!("-{}", format_duration((duration - cursor) / 1000.0));
			}
		}
		let paused = field("mediaState").as_ref().and_then(Value::as_str) == Some("OBS_MEDIA_STATE_PAUSED");
		let title = if show_time && (media_playing(k) || paused) { time } else { String::new() };
		// The time goes above or below the glyph, so the key draws it into the image.
		k.own_title(&title);
		let color = if status.as_ref().is_some_and(|s| truthy(Some(s))) { colors::WHITE } else { colors::GRAY };
		k.set_image(&glyph_key(
			media_glyph_name(k, &s.operation),
			&GlyphKey { title: &title, title_at: &s.time_at, color, ..Default::default() },
		));
	}

	async fn on_press(&self, k: &KeyRc, press: Press) -> Res<()> {
		let s: MediaSettings = k.settings();
		if s.input.is_empty() {
			return Err("no media source selected".into());
		}
		if press.op == "forward" || press.op == "back" {
			let offset = js::round(settings::or(s.seconds, 10.0) * 1000.0) * if press.op == "back" { -1.0 } else { 1.0 };
			k.call("OffsetMediaInputCursor", json!({ "inputName": s.input, "mediaCursorOffset": js::num(offset) })).await?;
		} else {
			let mut action = media_action(&press.op);
			if press.op == "playpause" {
				let status = k.call("GetMediaInputStatus", json!({ "inputName": s.input })).await?;
				action = match status.get("mediaState").and_then(Value::as_str) {
					Some("OBS_MEDIA_STATE_PLAYING") => media_action("pause"),
					Some("OBS_MEDIA_STATE_ENDED" | "OBS_MEDIA_STATE_STOPPED" | "OBS_MEDIA_STATE_NONE") => media_action("restart"),
					_ => media_action("play"),
				};
			}
			let mut data = Map::new();
			data.insert("inputName".into(), s.input.into());
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
	required: &["input"],
	behavior: &MediaControl,
	new_state: || Box::new(MediaState::default()),
	..Kind::BASE
};

// ----- Profile / Scene Collection -----

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct ProfileSettings {
	/// "profile" or "collection".
	kind: String,
	name: String,
	show_name: bool,
}

impl ProfileSettings {
	fn collection(&self) -> bool {
		self.kind == "collection"
	}

	fn glyph(&self) -> &'static str {
		if self.collection() { "collection" } else { "profile" }
	}

	fn title(&self) -> &str {
		if self.show_name { &self.name } else { "" }
	}
}

struct ProfileSwitch;

fn profile_current(k: &KeyRc, s: &ProfileSettings) -> Option<String> {
	let data = k.data();
	if s.collection() { data.collection } else { data.profile }
}

#[async_trait(?Send)]
impl Behavior for ProfileSwitch {
	fn render_offline(&self, k: &KeyRc) {
		let s: ProfileSettings = k.settings();
		k.own_title(s.title());
		k.set_image(&glyph_key(s.glyph(), &GlyphKey { title: s.title(), color: colors::GRAY, ..Default::default() }));
	}

	fn render(&self, k: &KeyRc) {
		let s: ProfileSettings = k.settings();
		let active = !s.name.is_empty() && profile_current(k, &s).as_deref() == Some(s.name.as_str());
		k.own_title(s.title());
		k.set_image(&glyph_key(
			s.glyph(),
			&GlyphKey { title: s.title(), color: if active { colors::WHITE } else { colors::GRAY }, ..Default::default() },
		));
	}

	async fn on_press(&self, k: &KeyRc, _press: Press) -> Res<()> {
		let s: ProfileSettings = k.settings();
		if s.name.is_empty() {
			return Err(if s.collection() { "no scene collection selected" } else { "no profile selected" }.into());
		}
		if profile_current(k, &s).as_deref() == Some(s.name.as_str()) {
			return Ok(());
		}
		if s.collection() {
			k.call("SetCurrentSceneCollection", json!({ "sceneCollectionName": s.name })).await?;
		} else {
			k.call("SetCurrentProfile", json!({ "profileName": s.name })).await?;
		}
		Ok(())
	}
}

pub static PROFILE_SWITCH: Kind = Kind {
	uuid: "de.felitendo.obs-tools.profile",
	name: "ProfileSwitch",
	defaults: || defaults(json!({ "kind": "profile", "name": "", "showName": true })),
	required: &["name"],
	behavior: &ProfileSwitch,
	..Kind::BASE
};

#[cfg(test)]
mod tests {
	use super::*;
	use crate::settings::testing::assert_defaults;

	#[test]
	fn every_setting_has_a_default() {
		assert_defaults::<VisibilitySettings>(&[&SOURCE_VISIBILITY]);
		assert_defaults::<HotkeySettings>(&[&HOTKEY]);
		assert_defaults::<ScreenshotSettings>(&[&SCREENSHOT]);
		assert_defaults::<RecordToolsSettings>(&[&RECORD_TOOLS]);
		assert_defaults::<MediaSettings>(&[&MEDIA_CONTROL]);
		assert_defaults::<ProfileSettings>(&[&PROFILE_SWITCH]);
	}
}
