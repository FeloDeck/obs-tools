use async_trait::async_trait;
use serde_json::{Map, Value, json};

use super::{Behavior, KeyRc, Kind, PressArgs, Stop, defaults};
use crate::Res;
use crate::icons::colors;
use crate::js::{self, truthy};
use crate::render::{GlyphKey, SceneKey, glyph_key, is_color, scene_key};

// ----- Smart Scene: press once to put the scene in preview (studio mode), press again to go live. -----

#[derive(Default)]
struct SceneState {
	shot: Option<String>,
	stop_preview: Option<Stop>,
}

struct SmartScene;

fn scene(k: &KeyRc) -> std::cell::RefMut<'_, SceneState> {
	k.x::<SceneState>()
}

fn stop_preview(k: &KeyRc) {
	let stop = scene(k).stop_preview.clone();
	if let Some(stop) = stop {
		stop();
	}
}

fn start_preview(k: &KeyRc) {
	stop_preview(k);
	let seconds = k.s_num("previewInterval");
	if k.s_truthy("showPreview") && seconds > 0.0 {
		let stop = k.interval(js::max(500.0, seconds * 1000.0), |k| {
			Box::pin(async move {
				refresh_shot(&k).await;
				Ok(())
			})
		});
		scene(k).stop_preview = Some(stop);
	}
}

async fn refresh_shot(k: &KeyRc) {
	let scene_name = k.s_val("scene");
	if !k.visible() || !k.online() || !truthy(Some(&scene_name)) || !k.s_truthy("showPreview") {
		return;
	}
	let result = k
		.call(
			"GetSourceScreenshot",
			json!({ "sourceName": scene_name, "imageFormat": "jpg", "imageWidth": 256, "imageCompressionQuality": 70 }),
		)
		.await;
	scene(k).shot = result.ok().and_then(|r| r.get("imageData").and_then(Value::as_str).map(String::from));
	k.render();
}

fn scene_title(k: &KeyRc) -> String {
	if !k.s_truthy("showTitle") {
		return String::new();
	}
	if k.s_truthy("title") { k.s_str("title") } else { k.s_str("scene") }
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
		let title = scene_title(k);
		k.own_title(&title);
		k.set_image(&scene_key(&SceneKey { title: &title, ..Default::default() }));
	}

	async fn on_settings_changed(&self, k: &KeyRc, old: &Map<String, Value>) -> Res<()> {
		if old.get("scene") != k.s("scene").as_ref() || old.get("showPreview") != k.s("showPreview").as_ref() {
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

	async fn on_state_change(&self, k: &KeyRc, what: &str) -> Res<()> {
		self.render(k);
		if what == "program" || what == "preview" {
			let k = k.clone();
			tokio::task::spawn_local(async move { refresh_shot(&k).await });
		}
		Ok(())
	}

	fn render(&self, k: &KeyRc) {
		let name = k.s_val("scene");
		let has = truthy(Some(&name));
		let data = k.data();
		let name = js::str_or_empty(Some(&name));
		let live = has && data.program_scene.as_deref() == Some(name.as_str());
		let preview = has && data.studio_mode && data.preview_scene.as_deref() == Some(name.as_str());
		let (live_color, preview_color) = (k.s_str("liveColor"), k.s_str("previewColor"));
		let border = if live {
			Some(if is_color(&live_color) { live_color.as_str() } else { colors::RED })
		} else if preview {
			Some(if is_color(&preview_color) { preview_color.as_str() } else { colors::GREEN })
		} else {
			None
		};
		let title = scene_title(k);
		k.own_title(&title);
		let shot = if k.s_truthy("showPreview") { scene(k).shot.clone() } else { None };
		let fit = k.s_str("previewFit");
		k.set_image(&scene_key(&SceneKey { image: shot.as_deref(), border, fit: &fit, title: &title, ..Default::default() }));
	}

	async fn on_press(&self, k: &KeyRc, args: Option<PressArgs>) -> Res<()> {
		let name = k.s_val("scene");
		if !truthy(Some(&name)) {
			return Err("no scene selected".into());
		}
		let data = k.data();
		let op = k.op(&args);
		if op == "preview" {
			if !data.studio_mode {
				return Err("studio mode is off".into());
			}
			k.call("SetCurrentPreviewScene", json!({ "sceneName": name })).await?;
		} else if op == "switch" && data.studio_mode && !k.is_in_multi_action() {
			if data.preview_scene.map(Value::from).as_ref() == Some(&name) {
				k.call("TriggerStudioModeTransition", json!({})).await?;
			} else {
				k.call("SetCurrentPreviewScene", json!({ "sceneName": name })).await?;
			}
		} else {
			k.call("SetCurrentProgramScene", json!({ "sceneName": name })).await?;
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
	press_only: &[],
	can_repeat: false,
	required: &["scene"],
	behavior: &SmartScene,
	new_state: || Box::new(SceneState::default()),
};

// ----- Studio Mode -----

struct StudioMode;

#[async_trait(?Send)]
impl Behavior for StudioMode {
	fn render(&self, k: &KeyRc) {
		k.set_state(if k.data().studio_mode { 1 } else { 0 });
	}

	async fn on_press(&self, k: &KeyRc, args: Option<PressArgs>) -> Res<()> {
		let current = k.data().studio_mode;
		let mode = k.op(&args);
		// take: the transition button of OBS, the preview goes live.
		if mode == "take" {
			if !current {
				return Err("studio mode is off".into());
			}
			k.call("TriggerStudioModeTransition", json!({})).await?;
			return Ok(());
		}
		let enabled = match mode.as_str() {
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
	press_only: &[],
	can_repeat: false,
	required: &[],
	behavior: &StudioMode,
	new_state: || Box::new(()),
};

// ----- Last Scene: back to the scene that was live before the current one. -----

struct PreviousScene;

#[async_trait(?Send)]
impl Behavior for PreviousScene {
	fn render(&self, k: &KeyRc) {
		let previous = k.data().previous_scene().unwrap_or_default();
		k.glyph_for_title(if k.s_truthy("showName") { &previous } else { "" });
	}

	async fn on_press(&self, k: &KeyRc, _args: Option<PressArgs>) -> Res<()> {
		let Some(previous) = k.data().previous_scene() else { return Err("no previous scene yet".into()) };
		k.call("SetCurrentProgramScene", json!({ "sceneName": previous })).await?;
		Ok(())
	}
}

pub static PREVIOUS_SCENE: Kind = Kind {
	uuid: "de.felitendo.obs-tools.previousscene",
	name: "PreviousScene",
	defaults: || defaults(json!({ "showName": true })),
	press_key: "",
	ops: &[],
	press_only: &[],
	can_repeat: false,
	required: &[],
	behavior: &PreviousScene,
	new_state: || Box::new(()),
};

// ----- Next Scene -----

struct NextScene;

fn next_target(k: &KeyRc, program: Option<&str>, direction: &str) -> Option<String> {
	let scenes = k.data().scenes;
	if scenes.is_empty() {
		return None;
	}
	let len = scenes.len() as i64;
	let index = program.and_then(|p| scenes.iter().position(|s| s == p)).map_or(-1, |i| i as i64);
	let mut next = index + if direction == "previous" { -1 } else { 1 };
	if next < 0 || next >= len {
		if !k.s_truthy("wrap") {
			return None;
		}
		next = (next + len).rem_euclid(len);
	}
	scenes.get(next as usize).cloned()
}

// The icon points where a press goes: on in the list, or up.
fn next_glyph(k: &KeyRc) -> &'static str {
	if k.s_str("direction") == "previous" { "previous" } else { "next" }
}

#[async_trait(?Send)]
impl Behavior for NextScene {
	fn render_offline(&self, k: &KeyRc) {
		k.set_image(&glyph_key(next_glyph(k), &GlyphKey::default()));
	}

	fn render(&self, k: &KeyRc) {
		let program = k.data().program_scene;
		let title = if k.s_truthy("showName") {
			next_target(k, program.as_deref(), &k.s_str("direction")).unwrap_or_default()
		} else {
			String::new()
		};
		k.own_title(&title);
		k.set_image(&glyph_key(next_glyph(k), &GlyphKey { title: &title, ..Default::default() }));
	}

	async fn on_press(&self, k: &KeyRc, args: Option<PressArgs>) -> Res<()> {
		// Ask OBS, the cached scene can lag behind a fast double press.
		let current = k.call("GetCurrentProgramScene", json!({})).await?;
		let program = current.get("sceneName").and_then(Value::as_str).map(String::from);
		let Some(target) = next_target(k, program.as_deref(), &k.op(&args)) else { return Err("no next scene".into()) };
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
	press_only: &[],
	can_repeat: false,
	required: &[],
	behavior: &NextScene,
	new_state: || Box::new(()),
};

// ----- Set Transition -----

struct SetTransition;

fn transition_active(k: &KeyRc) -> bool {
	let transition = k.s_val("transition");
	let duration = k.s_val("duration");
	let current = k.data().transition;
	if !truthy(Some(&transition)) || current.name.map(Value::from).as_ref() != Some(&transition) {
		return false;
	}
	duration == "" || duration.is_null() || current.duration.is_none() || Some(js::number(Some(&duration))) == current.duration
}

fn transition_title(k: &KeyRc) {
	let title = if k.s_truthy("showName") { k.s_str("transition") } else { String::new() };
	k.glyph_for_title(&title);
}

#[async_trait(?Send)]
impl Behavior for SetTransition {
	// Offline it is not the active transition, so it is gray.
	fn render_offline(&self, k: &KeyRc) {
		k.set_state(0);
		transition_title(k);
	}

	fn render(&self, k: &KeyRc) {
		k.set_state(if transition_active(k) { 1 } else { 0 });
		transition_title(k);
	}

	async fn on_press(&self, k: &KeyRc, _args: Option<PressArgs>) -> Res<()> {
		let transition = k.s_val("transition");
		if !truthy(Some(&transition)) {
			return Err("no transition selected".into());
		}
		k.call("SetCurrentSceneTransition", json!({ "transitionName": transition })).await?;
		let duration = k.s("duration");
		let number = js::number(duration.as_ref());
		if duration.as_ref().is_some_and(|d| *d != "" && !d.is_null()) && number >= 50.0 {
			// Fixed transitions like Cut have no duration.
			if let Err(error) =
				k.call("SetCurrentSceneTransitionDuration", json!({ "transitionDuration": js::num(js::round(number)) })).await
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
	press_key: "",
	ops: &[],
	press_only: &[],
	can_repeat: false,
	required: &["transition"],
	behavior: &SetTransition,
	new_state: || Box::new(()),
};
