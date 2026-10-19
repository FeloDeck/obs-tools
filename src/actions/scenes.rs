use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{Behavior, KeyRc, Kind, Press, ShowName, Stop, defaults};
use crate::Res;
use crate::icons::colors;
use crate::js;
use crate::obs_state::Change;
use crate::render::{GlyphKey, SceneKey, glyph_key, is_color, scene_key};
use crate::settings::{self, read};

// ----- Smart Scene: press once to put the scene in preview (studio mode), press again to go live. -----

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct SceneSettings {
	scene: String,
	title: String,
	show_title: bool,
	show_preview: bool,
	preview_fit: String,
	preview_interval: f64,
	live_color: String,
	preview_color: String,
}

#[derive(Default)]
struct SceneState {
	shot: Option<String>,
	stop_preview: Option<Stop>,
}

struct SmartScene;

fn scene(k: &KeyRc) -> std::cell::RefMut<'_, SceneState> {
	k.own::<SceneState>()
}

fn stop_preview(k: &KeyRc) {
	let stop = scene(k).stop_preview.clone();
	if let Some(stop) = stop {
		stop();
	}
}

fn start_preview(k: &KeyRc) {
	stop_preview(k);
	let s: SceneSettings = k.settings();
	if s.show_preview && s.preview_interval > 0.0 {
		let stop = k.interval((s.preview_interval * 1000.0).max(500.0), |k| {
			Box::pin(async move {
				refresh_shot(&k).await;
				Ok(())
			})
		});
		scene(k).stop_preview = Some(stop);
	}
}

async fn refresh_shot(k: &KeyRc) {
	let s: SceneSettings = k.settings();
	if !k.visible() || !k.online() || s.scene.is_empty() || !s.show_preview {
		return;
	}
	let result = k
		.call(
			"GetSourceScreenshot",
			json!({ "sourceName": s.scene, "imageFormat": "jpg", "imageWidth": 256, "imageCompressionQuality": 70 }),
		)
		.await;
	scene(k).shot = result.ok().and_then(|r| r.get("imageData").and_then(Value::as_str).map(String::from));
	k.render();
}

fn scene_title(s: &SceneSettings) -> String {
	if !s.show_title {
		return String::new();
	}
	if s.title.is_empty() { s.scene.clone() } else { s.title.clone() }
}

#[async_trait(?Send)]
impl Behavior for SmartScene {
	async fn on_connected(&self, k: &KeyRc) -> Res<()> {
		scene(k).shot = None;
		start_preview(k);
		refresh_shot(k).await;
		Ok(())
	}

	fn on_disconnected(&self, k: &KeyRc) {
		stop_preview(k);
	}

	fn on_disappear(&self, k: &KeyRc) {
		stop_preview(k);
	}

	fn render_offline(&self, k: &KeyRc) {
		let title = scene_title(&k.settings());
		k.own_title(&title);
		k.set_image(&scene_key(&SceneKey { title: &title, ..Default::default() }));
	}

	async fn on_settings_changed(&self, k: &KeyRc, old: &Map<String, Value>) -> Res<()> {
		let (old, s): (SceneSettings, SceneSettings) = (read(old), k.settings());
		if old.scene != s.scene || old.show_preview != s.show_preview {
			scene(k).shot = None;
		}
		if !k.online() {
			self.render_offline(k);
			return Ok(());
		}
		start_preview(k);
		refresh_shot(k).await;
		Ok(())
	}

	async fn on_state_change(&self, k: &KeyRc, change: Change) -> Res<()> {
		self.render(k);
		if matches!(change, Change::Program | Change::Preview) {
			let k = k.clone();
			tokio::task::spawn_local(async move { refresh_shot(&k).await });
		}
		Ok(())
	}

	fn render(&self, k: &KeyRc) {
		let s: SceneSettings = k.settings();
		let data = k.data();
		let name = Some(s.scene.as_str()).filter(|n| !n.is_empty());
		let live = name.is_some() && data.program_scene.as_deref() == name;
		let preview = name.is_some() && data.studio_mode && data.preview_scene.as_deref() == name;
		let border = if live {
			Some(if is_color(&s.live_color) { s.live_color.as_str() } else { colors::RED })
		} else if preview {
			Some(if is_color(&s.preview_color) { s.preview_color.as_str() } else { colors::GREEN })
		} else {
			None
		};
		let title = scene_title(&s);
		k.own_title(&title);
		let shot = if s.show_preview { scene(k).shot.clone() } else { None };
		k.set_image(&scene_key(&SceneKey { image: shot.as_deref(), border, fit: &s.preview_fit, title: &title, ..Default::default() }));
	}

	async fn on_press(&self, k: &KeyRc, press: Press) -> Res<()> {
		let s: SceneSettings = k.settings();
		if s.scene.is_empty() {
			return Err("no scene selected".into());
		}
		let data = k.data();
		if press.op == "preview" {
			if !data.studio_mode {
				return Err("studio mode is off".into());
			}
			k.call("SetCurrentPreviewScene", json!({ "sceneName": s.scene })).await?;
		} else if press.op == "switch" && data.studio_mode && !k.is_in_multi_action() {
			if data.preview_scene.as_deref() == Some(s.scene.as_str()) {
				k.call("TriggerStudioModeTransition", json!({})).await?;
			} else {
				k.call("SetCurrentPreviewScene", json!({ "sceneName": s.scene })).await?;
			}
		} else {
			k.call("SetCurrentProgramScene", json!({ "sceneName": s.scene })).await?;
		}
		Ok(())
	}
}

pub static SMART_SCENE: Kind = Kind {
	uuid: "de.felitendo.obs-tools.smartscene",
	name: "SmartScene",
	// switch: smart (studio mode: preview first, then live). live: straight live. preview: only to preview.
	defaults: || {
		defaults(json!({
			"scene": "",
			"press": "switch",
			"hold": "none",
			"double": "none",
			"title": "",
			"showTitle": true,
			"showPreview": true,
			"previewFit": "fit",
			"previewInterval": 2,
			"liveColor": colors::RED,
			"previewColor": colors::GREEN,
		}))
	},
	press_key: "press",
	ops: &["switch", "live", "preview"],
	required: &["scene"],
	behavior: &SmartScene,
	new_state: || Box::new(SceneState::default()),
	..Kind::BASE
};

// ----- Studio Mode -----

struct StudioMode;

#[async_trait(?Send)]
impl Behavior for StudioMode {
	fn render(&self, k: &KeyRc) {
		k.set_state(if k.data().studio_mode { 1 } else { 0 });
	}

	async fn on_press(&self, k: &KeyRc, press: Press) -> Res<()> {
		let current = k.data().studio_mode;
		// take: the transition button of OBS, the preview goes live.
		if press.op == "take" {
			if !current {
				return Err("studio mode is off".into());
			}
			k.call("TriggerStudioModeTransition", json!({})).await?;
			return Ok(());
		}
		let enabled = match press.op.as_str() {
			"enable" => true,
			"disable" => false,
			_ => !current,
		};
		k.call("SetStudioModeEnabled", json!({ "studioModeEnabled": enabled })).await?;
		Ok(())
	}
}

pub static STUDIO_MODE: Kind = Kind {
	uuid: "de.felitendo.obs-tools.studiomode",
	name: "StudioMode",
	defaults: || defaults(json!({ "mode": "toggle", "hold": "none", "double": "none" })),
	press_key: "mode",
	ops: &["toggle", "enable", "disable", "take"],
	behavior: &StudioMode,
	..Kind::BASE
};

// ----- Last Scene: back to the scene that was live before the current one. -----

struct PreviousScene;

#[async_trait(?Send)]
impl Behavior for PreviousScene {
	fn render(&self, k: &KeyRc) {
		let previous = k.data().previous_scene().unwrap_or_default();
		k.glyph_for_title(if k.settings::<ShowName>().show_name { &previous } else { "" });
	}

	async fn on_press(&self, k: &KeyRc, _press: Press) -> Res<()> {
		let Some(previous) = k.data().previous_scene() else { return Err("no previous scene yet".into()) };
		k.call("SetCurrentProgramScene", json!({ "sceneName": previous })).await?;
		Ok(())
	}
}

pub static PREVIOUS_SCENE: Kind = Kind {
	uuid: "de.felitendo.obs-tools.previousscene",
	name: "PreviousScene",
	defaults: || defaults(json!({ "showName": true })),
	behavior: &PreviousScene,
	..Kind::BASE
};

// ----- Next Scene -----

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct NextSettings {
	direction: String,
	wrap: bool,
	show_name: bool,
}

struct NextScene;

/// The scene after (or before) the live one in the scene list.
fn next_target(scenes: &[String], program: Option<&str>, direction: &str, wrap: bool) -> Option<String> {
	if scenes.is_empty() {
		return None;
	}
	let len = scenes.len() as i64;
	let index = program.and_then(|p| scenes.iter().position(|s| s == p)).map_or(-1, |i| i as i64);
	let mut next = index + if direction == "previous" { -1 } else { 1 };
	if next < 0 || next >= len {
		if !wrap {
			return None;
		}
		next = (next + len).rem_euclid(len);
	}
	scenes.get(next as usize).cloned()
}

// The icon points where a press goes: on in the list, or up.
fn next_glyph(s: &NextSettings) -> &'static str {
	if s.direction == "previous" { "previous" } else { "next" }
}

#[async_trait(?Send)]
impl Behavior for NextScene {
	fn render_offline(&self, k: &KeyRc) {
		k.set_image(&glyph_key(next_glyph(&k.settings()), &GlyphKey::default()));
	}

	fn render(&self, k: &KeyRc) {
		let s: NextSettings = k.settings();
		let data = k.data();
		let title = if s.show_name {
			next_target(&data.scenes, data.program_scene.as_deref(), &s.direction, s.wrap).unwrap_or_default()
		} else {
			String::new()
		};
		k.own_title(&title);
		k.set_image(&glyph_key(next_glyph(&s), &GlyphKey { title: &title, ..Default::default() }));
	}

	async fn on_press(&self, k: &KeyRc, press: Press) -> Res<()> {
		// Ask OBS, the cached scene can lag behind a fast double press.
		let current = k.call("GetCurrentProgramScene", json!({})).await?;
		let program = current.get("sceneName").and_then(Value::as_str);
		let wrap = k.settings::<NextSettings>().wrap;
		let Some(target) = next_target(&k.data().scenes, program, &press.op, wrap) else { return Err("no next scene".into()) };
		k.call("SetCurrentProgramScene", json!({ "sceneName": target })).await?;
		Ok(())
	}
}

pub static NEXT_SCENE: Kind = Kind {
	uuid: "de.felitendo.obs-tools.nextscene",
	name: "NextScene",
	defaults: || defaults(json!({ "direction": "next", "hold": "none", "double": "none", "wrap": true, "showName": true })),
	press_key: "direction",
	ops: &["next", "previous"],
	behavior: &NextScene,
	..Kind::BASE
};

// ----- Set Transition -----

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct TransitionSettings {
	transition: String,
	/// In ms. Empty: keep the duration the transition has.
	#[serde(deserialize_with = "settings::optional_number")]
	duration: Option<f64>,
	show_name: bool,
}

struct SetTransition;

fn transition_active(k: &KeyRc, s: &TransitionSettings) -> bool {
	let current = k.data().transition;
	if s.transition.is_empty() || current.name.as_deref() != Some(s.transition.as_str()) {
		return false;
	}
	match (s.duration, current.duration) {
		(Some(wanted), Some(now)) => wanted == now,
		_ => true,
	}
}

fn transition_title(k: &KeyRc, s: &TransitionSettings) {
	k.glyph_for_title(if s.show_name { &s.transition } else { "" });
}

#[async_trait(?Send)]
impl Behavior for SetTransition {
	// Offline it is not the active transition, so it is gray.
	fn render_offline(&self, k: &KeyRc) {
		k.set_state(0);
		transition_title(k, &k.settings());
	}

	fn render(&self, k: &KeyRc) {
		let s: TransitionSettings = k.settings();
		k.set_state(if transition_active(k, &s) { 1 } else { 0 });
		transition_title(k, &s);
	}

	async fn on_press(&self, k: &KeyRc, _press: Press) -> Res<()> {
		let s: TransitionSettings = k.settings();
		if s.transition.is_empty() {
			return Err("no transition selected".into());
		}
		k.call("SetCurrentSceneTransition", json!({ "transitionName": s.transition })).await?;
		if let Some(duration) = s.duration.filter(|d| *d >= 50.0) {
			// Fixed transitions like Cut have no duration.
			if let Err(error) =
				k.call("SetCurrentSceneTransitionDuration", json!({ "transitionDuration": js::num(js::round(duration)) })).await
			{
				k.log(&format!("duration not set: {error}"));
			}
		}
		self.render(k);
		Ok(())
	}
}

pub static SET_TRANSITION: Kind = Kind {
	uuid: "de.felitendo.obs-tools.settransition",
	name: "SetTransition",
	defaults: || defaults(json!({ "transition": "", "duration": "", "showName": true })),
	required: &["transition"],
	behavior: &SetTransition,
	..Kind::BASE
};

#[cfg(test)]
mod tests {
	use super::*;
	use crate::settings::testing::assert_defaults;

	#[test]
	fn every_setting_has_a_default() {
		assert_defaults::<SceneSettings>(&[&SMART_SCENE]);
		assert_defaults::<ShowName>(&[&PREVIOUS_SCENE]);
		assert_defaults::<NextSettings>(&[&NEXT_SCENE]);
		assert_defaults::<TransitionSettings>(&[&SET_TRANSITION]);
	}

	#[test]
	fn next_scene_wraps_only_when_asked() {
		let scenes = ["A".to_string(), "B".to_string()];
		assert_eq!(next_target(&scenes, Some("B"), "next", false), None);
		assert_eq!(next_target(&scenes, Some("B"), "next", true).as_deref(), Some("A"));
		assert_eq!(next_target(&scenes, Some("A"), "previous", true).as_deref(), Some("B"));
		assert_eq!(next_target(&scenes, None, "next", false).as_deref(), Some("A"));
	}
}
