// Everyday OBS actions. On Windows these come with Elgato's own OBS plugin, on Linux they do not.
use std::path::Path;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use super::{Behavior, KeyRc, Kind, Press, Stop, defaults};
use crate::Res;
use crate::icons::{colors, media_glyph};
use crate::js::{self, truthy};
use crate::obs_state::Change;
use crate::render::{GlyphKey, glyph_key};
use crate::time::format_duration;

// ----- Source Visibility: show or hide a source in a scene. -----

#[derive(Default)]
struct VisibilityState {
	item_id: Option<Value>,
	enabled: bool,
}

struct SourceVisibility;

fn vis(k: &KeyRc) -> std::cell::RefMut<'_, VisibilityState> {
	k.own::<VisibilityState>()
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

	async fn on_state_change(&self, k: &KeyRc, change: Change) -> Res<()> {
		// Without a fixed scene the key follows the live scene.
		if !k.s_truthy("scene") && change == Change::Program {
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

	async fn on_press(&self, k: &KeyRc, press: Press) -> Res<()> {
		let scene = scene_name(k, "scene");
		if vis(k).item_id.is_none() {
			load_visibility(k).await;
		}
		let Some(item_id) = vis(k).item_id.clone() else {
			return Err(format!("\"{}\" is not in scene \"{}\"", js::to_string(k.s("source").as_ref()), js::to_string(Some(&scene))).into());
		};
		let mode = press.op;
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
	required: &["source"],
	behavior: &SourceVisibility,
	new_state: || Box::new(VisibilityState::default()),
	..Kind::BASE
};

// ----- OBS Hotkey: by its name, or by the keys it is bound to in OBS. -----

struct Hotkey;

#[async_trait(?Send)]
impl Behavior for Hotkey {
	fn is_set_up(&self, k: &KeyRc) -> bool {
		if k.s_str("mode") == "keys" { k.s_truthy("key") } else { k.s_truthy("hotkey") }
	}

	async fn on_press(&self, k: &KeyRc, _press: Press) -> Res<()> {
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
	behavior: &Hotkey,
	..Kind::BASE
};

// ----- Screenshot: save a picture of a scene or source as a file. -----

struct Screenshot;

#[async_trait(?Send)]
impl Behavior for Screenshot {
	async fn on_press(&self, k: &KeyRc, _press: Press) -> Res<()> {
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
		let file = Path::new(&dir).join(format!("Screenshot {safe_name} {stamp}.{ext}")).to_string_lossy().into_owned();
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
	behavior: &Screenshot,
	..Kind::BASE
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

	async fn on_press(&self, k: &KeyRc, press: Press) -> Res<()> {
		if !k.data().record.active {
			return Err("not recording".into());
		}
		if press.op == "split" {
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
	behavior: &RecordTools,
	..Kind::BASE
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
	k.own::<MediaState>()
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

	async fn on_press(&self, k: &KeyRc, press: Press) -> Res<()> {
		let input = k.s_val("input");
		let operation = press.op;
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
	required: &["input"],
	behavior: &MediaControl,
	new_state: || Box::new(MediaState::default()),
	..Kind::BASE
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

	async fn on_press(&self, k: &KeyRc, _press: Press) -> Res<()> {
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
	required: &["name"],
	behavior: &ProfileSwitch,
	..Kind::BASE
};
