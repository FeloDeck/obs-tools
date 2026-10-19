use async_trait::async_trait;
use serde_json::{Map, Value, json};

use super::basics::scene_name;
use super::{Behavior, KeyRc, Kind, PiReply, Press, defaults, patch};
use crate::Res;
use crate::animation::{Run, diff_transforms, normalize_phase, run_animation, total_duration};
use crate::desktop::{active_x11_window, session_type, xcomposite_window_value};
use crate::js::{self, truthy};
use crate::media::{PlayOptions, Playback, play_media};
use crate::timers::Timer;

// "/home/me/Videos/Big clip.mp4" -> "Big clip", for a title.
pub fn file_title(file: &str) -> String {
	let trimmed = file.trim_end_matches('/');
	let base = trimmed.rsplit('/').next().unwrap_or("");
	// Like .replace(/\.[^.]+$/, ""): the last dot with something after it, to the end.
	match base.rfind('.') {
		Some(i) if i + 1 < base.len() => base[..i].to_string(),
		_ => base.to_string(),
	}
}

async fn scene_item(k: &KeyRc, scene: &Value, source: &Value) -> Res<Value> {
	if !truthy(Some(scene)) {
		return Err("no scene".into());
	}
	if !truthy(Some(source)) {
		return Err("no source selected".into());
	}
	match k.call("GetSceneItemId", json!({ "sceneName": scene, "sourceName": source })).await {
		Ok(r) => Ok(r.get("sceneItemId").cloned().unwrap_or(Value::Null)),
		Err(_) => Err(format!("\"{}\" is not in scene \"{}\"", js::to_string(Some(source)), js::to_string(Some(scene))).into()),
	}
}

fn phases(k: &KeyRc) -> Vec<Value> {
	match k.s("phases") {
		Some(Value::Array(list)) => list,
		_ => Vec::new(),
	}
}

// ----- Source Animation: moves, scales, rotates or crops a source in phases. -----

struct Recording {
	scene: Value,
	item: Value,
	start: Value,
}

#[derive(Default)]
struct AnimationState {
	running: bool,
	recording: Option<Recording>,
}

struct SourceAnimation;

fn anim(k: &KeyRc) -> std::cell::RefMut<'_, AnimationState> {
	k.own::<AnimationState>()
}

async fn animate(k: &KeyRc) -> Res<()> {
	if anim(k).running {
		return Ok(());
	}
	let source = k.s_val("source");
	let phases: Vec<Value> = phases(k)
		.iter()
		.map(normalize_phase)
		.filter(|p| p.duration > 0.0 || p.delay > 0.0 || !p.changes.is_empty())
		.map(|p| p.to_value())
		.collect();
	if phases.is_empty() {
		return Err("the animation has no phases".into());
	}
	let scene = scene_name(k, "scene");
	let item = scene_item(k, &scene, &source).await?;
	let start =
		k.call("GetSceneItemTransform", json!({ "sceneName": scene, "sceneItemId": item })).await?.get("sceneItemTransform").cloned();
	anim(k).running = true;
	k.render();
	let Some(client) = k.obs() else {
		anim(k).running = false;
		k.render();
		return Err("no connection".into());
	};
	let fps = k.data().fps;
	let options = k.settings();
	let result = run_animation(
		&client,
		&Run {
			scene_name: scene,
			scene_item_id: item,
			source_name: source,
			start: start.as_ref(),
			phases: &phases,
			fps: if js::truthy_num(fps) { fps } else { 30.0 },
			options: &options,
		},
	)
	.await;
	anim(k).running = false;
	k.render();
	result.map(|_| ())
}

#[async_trait(?Send)]
impl Behavior for SourceAnimation {
	fn render(&self, k: &KeyRc) {
		let running = anim(k).running;
		k.set_state(if running { 1 } else { 0 });
	}

	async fn on_press(&self, k: &KeyRc, _press: Press) -> Res<()> {
		animate(k).await
	}

	// The settings page drives recording and import.
	async fn on_pi_message(&self, k: &KeyRc, message: &Value, reply: PiReply) -> Res<()> {
		match message.get("type").and_then(Value::as_str).unwrap_or("") {
			"recordStart" => {
				let scene = scene_name(k, "scene");
				let item = scene_item(k, &scene, &k.s_val("source")).await?;
				let start = k
					.call("GetSceneItemTransform", json!({ "sceneName": scene, "sceneItemId": item }))
					.await?
					.get("sceneItemTransform")
					.cloned()
					.unwrap_or(Value::Null);
				anim(k).recording = Some(Recording { scene, item, start });
				reply(json!({ "type": "recording", "active": true }));
			}
			"recordStop" => {
				let Some(recording) = anim(k).recording.take() else {
					reply(json!({ "type": "recording", "active": false }));
					return Ok(());
				};
				let end = k
					.call("GetSceneItemTransform", json!({ "sceneName": recording.scene, "sceneItemId": recording.item }))
					.await?
					.get("sceneItemTransform")
					.cloned();
				let changes = diff_transforms(Some(&recording.start), end.as_ref());
				if changes.is_empty() {
					reply(
						json!({ "type": "recording", "active": false, "message": "Nothing changed. Move or resize the source in OBS between the two clicks." }),
					);
					return Ok(());
				}
				// Put the source back where it started, so the animation can play from there.
				let back: Map<String, Value> =
					changes.keys().map(|key| (key.clone(), recording.start.get(key).cloned().unwrap_or(Value::Null))).collect();
				k.call(
					"SetSceneItemTransform",
					json!({ "sceneName": recording.scene, "sceneItemId": recording.item, "sceneItemTransform": back }),
				)
				.await?;
				let mut list: Vec<Value> = phases(k).iter().map(|p| normalize_phase(p).to_value()).collect();
				let index = list.iter().position(|p| p.get("changes").and_then(Value::as_object).is_none_or(Map::is_empty));
				let count = changes.len();
				match index {
					Some(i) => list[i]["changes"] = Value::Object(changes),
					None => list.push(json!({ "duration": 1000, "delay": 0, "easing": "easeInOut", "changes": changes })),
				}
				let phase = index.map_or(list.len(), |i| i + 1);
				k.save_settings(patch(vec![("phases", Value::Array(list))]));
				reply(json!({ "type": "recording", "active": false, "message": format!("Recorded {count} changes into phase {phase}.") }));
			}
			"recordCancel" => {
				anim(k).recording = None;
				reply(json!({ "type": "recording", "active": false }));
			}
			"preview" => {
				animate(k).await?;
				reply(json!({ "type": "previewDone", "duration": js::num(total_duration(&phases(k))) }));
			}
			_ => {}
		}
		Ok(())
	}
}

pub static SOURCE_ANIMATION: Kind = Kind {
	uuid: "de.felitendo.obs-tools.sourceanimation",
	name: "SourceAnimation",
	defaults: || {
		defaults(json!({
			"scene": "",
			"source": "",
			"phases": [{ "duration": 1000, "delay": 0, "easing": "easeInOut", "changes": {} }],
			"showAtStart": false,
			"hideAtEnd": false,
			"resetAtEnd": false,
			"filter": "",
			"filterStart": "none",
			"filterEnd": "none",
		}))
	},
	required: &["source"],
	behavior: &SourceAnimation,
	new_state: || Box::new(AnimationState::default()),
	..Kind::BASE
};

// ----- Video Player: one media source, many files. Each key plays its own file. -----

#[derive(Default)]
struct VideoState {
	playback: Option<Playback>,
}

struct VideoPlayer;

fn video(k: &KeyRc) -> std::cell::RefMut<'_, VideoState> {
	k.own::<VideoState>()
}

fn name_or_input(k: &KeyRc, file_key: &str) -> String {
	let title = file_title(&k.s_str(file_key));
	if title.is_empty() { k.s_str("input") } else { title }
}

#[async_trait(?Send)]
impl Behavior for VideoPlayer {
	fn render(&self, k: &KeyRc) {
		let playing = video(k).playback.is_some();
		k.set_state(if playing { 1 } else { 0 });
		k.show_name_title(&name_or_input(k, "file"));
	}

	async fn on_press(&self, k: &KeyRc, press: Press) -> Res<()> {
		let input = k.s_val("input");
		if !truthy(Some(&input)) {
			return Err("no media source selected".into());
		}
		let op = press.op;
		let current = video(k).playback.clone();
		if op == "stop" {
			match current {
				Some(playback) => playback.stop().await,
				None => {
					k.call(
						"TriggerMediaInputAction",
						json!({ "inputName": input, "mediaAction": "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_STOP" }),
					)
					.await?;
				}
			}
			return Ok(());
		}
		if op == "play" && current.is_some() && k.s_str("pressAgain") == "stop" && !k.is_in_multi_action() {
			if let Some(playback) = current {
				playback.stop().await;
			}
			return Ok(());
		}
		if let Some(playback) = current {
			playback.stop().await;
		}
		let (Some(client), Some(state)) = (k.obs(), k.obs_state()) else { return Err("no connection".into()) };
		let playback = play_media(
			&client,
			&state,
			PlayOptions {
				input,
				file: k.s_val("file"),
				speed: k.s("speed"),
				scene: k.s_val("scene"),
				show_source: k.s("showSource").is_none() || k.s_truthy("showSource"),
				hide_when_done: k.s("hideWhenDone").is_none() || k.s_truthy("hideWhenDone"),
				return_to_scene: k.s_truthy("returnToScene"),
			},
		)
		.await?;
		video(k).playback = Some(playback.clone());
		self.render(k);
		let k = k.clone();
		tokio::task::spawn_local(async move {
			playback.done().await;
			{
				let mut v = video(&k);
				if v.playback.as_ref() == Some(&playback) {
					v.playback = None;
				}
			}
			k.render();
		});
		Ok(())
	}

	fn on_disappear(&self, k: &KeyRc) {
		video(k).playback = None;
	}
}

pub static VIDEO_PLAYER: Kind = Kind {
	uuid: "de.felitendo.obs-tools.videoplayer",
	name: "VideoPlayer",
	// play: start (pressed again while playing: see pressAgain). stop and restart do just that.
	defaults: || {
		defaults(json!({
			"input": "",
			"press": "play",
			"hold": "none",
			"double": "none",
			"file": "",
			"speed": 100,
			"scene": "",
			"showSource": true,
			"hideWhenDone": true,
			"returnToScene": false,
			"pressAgain": "stop",
			"showName": false,
		}))
	},
	press_key: "press",
	ops: &["play", "stop", "restart"],
	required: &["input"],
	behavior: &VideoPlayer,
	new_state: || Box::new(VideoState::default()),
	..Kind::BASE
};

// ----- Image Source: one image source, many pictures. Each key shows its own picture. -----

#[derive(Default)]
struct ImageState {
	showing: bool,
	item_id: Option<Value>,
	hide_timer: Option<Timer>,
}

struct ImageSource;

fn image(k: &KeyRc) -> std::cell::RefMut<'_, ImageState> {
	k.own::<ImageState>()
}

async fn hide_image(k: &KeyRc, scene: &Value, item: &Value) -> Res<()> {
	let timer = image(k).hide_timer.take();
	if let Some(timer) = timer {
		timer.clear();
	}
	k.call("SetSceneItemEnabled", json!({ "sceneName": scene, "sceneItemId": item, "sceneItemEnabled": false })).await?;
	image(k).showing = false;
	k.render();
	Ok(())
}

#[async_trait(?Send)]
impl Behavior for ImageSource {
	fn render(&self, k: &KeyRc) {
		let showing = image(k).showing;
		k.set_state(if showing { 1 } else { 0 });
		k.show_name_title(&name_or_input(k, "file"));
	}

	async fn on_obs_event(&self, k: &KeyRc, kind: &str, data: &Value) -> Res<()> {
		// Someone else hid the source or loaded another picture.
		let mine = image(k).item_id.clone();
		let hidden = kind == "SceneItemEnableStateChanged"
			&& mine.is_some_and(|id| data.get("sceneItemId").is_some_and(|d| js::same(d, &id)))
			&& !truthy(data.get("sceneItemEnabled"));
		let other_file = kind == "InputSettingsChanged"
			&& data.get("inputName") == k.s("input").as_ref()
			&& data.get("inputSettings").and_then(|s| s.get("file")) != k.s("file").as_ref();
		if !hidden && !other_file {
			return Ok(());
		}
		image(k).showing = false;
		self.render(k);
		Ok(())
	}

	fn on_disappear(&self, k: &KeyRc) {
		let timer = image(k).hide_timer.take();
		if let Some(timer) = timer {
			timer.clear();
		}
	}

	async fn on_press(&self, k: &KeyRc, press: Press) -> Res<()> {
		let input = k.s_val("input");
		let file = k.s_val("file");
		if !truthy(Some(&input)) {
			return Err("no image source selected".into());
		}
		let mode = press.op;
		let scene = scene_name(k, "scene");
		let item = scene_item(k, &scene, &input).await?;
		image(k).item_id = Some(item.clone());
		let enabled =
			truthy(k.call("GetSceneItemEnabled", json!({ "sceneName": scene, "sceneItemId": item })).await?.get("sceneItemEnabled"));
		let current =
			k.call("GetInputSettings", json!({ "inputName": input })).await?.get("inputSettings").and_then(|s| s.get("file")).cloned();
		let mine = enabled && (!truthy(Some(&file)) || current.as_ref() == Some(&file));
		if mode == "hide" {
			if mine {
				hide_image(k, &scene, &item).await?;
			}
			return Ok(());
		}
		// Toggle, pressed again while its picture is up: hide it.
		if mode == "toggle" && mine && !k.is_in_multi_action() {
			return hide_image(k, &scene, &item).await;
		}
		if truthy(Some(&file)) && current.as_ref() != Some(&file) {
			k.call("SetInputSettings", json!({ "inputName": input, "inputSettings": { "file": file }, "overlay": true })).await?;
		}
		k.call("SetSceneItemEnabled", json!({ "sceneName": scene, "sceneItemId": item, "sceneItemEnabled": true })).await?;
		image(k).showing = true;
		self.render(k);
		let old = image(k).hide_timer.take();
		if let Some(timer) = old {
			timer.clear();
		}
		let auto_hide = k.s_num("autoHide");
		if auto_hide > 0.0 {
			let weak = std::rc::Rc::downgrade(k);
			let timer = crate::timers::set_timeout(auto_hide * 1000.0, move || {
				let Some(k) = weak.upgrade() else { return };
				tokio::task::spawn_local(async move {
					if image(&k).showing {
						k.safe(hide_image(&k, &scene, &item)).await;
					}
				});
			});
			image(k).hide_timer = Some(timer);
		}
		Ok(())
	}
}

pub static IMAGE_SOURCE: Kind = Kind {
	uuid: "de.felitendo.obs-tools.imagesource",
	name: "ImageSource",
	defaults: || {
		defaults(
			json!({ "scene": "", "input": "", "file": "", "mode": "toggle", "hold": "none", "double": "none", "autoHide": 0, "showName": false }),
		)
	},
	press_key: "mode",
	ops: &["toggle", "show", "hide"],
	required: &["input"],
	behavior: &ImageSource,
	new_state: || Box::new(ImageState::default()),
	..Kind::BASE
};

// ----- Browser Source -----

struct BrowserSource;

fn url_list(k: &KeyRc) -> Vec<String> {
	k.s_str("urls").split('\n').map(|u| js::trim(u).to_string()).filter(|u| !u.is_empty()).collect()
}

#[async_trait(?Send)]
impl Behavior for BrowserSource {
	fn render(&self, k: &KeyRc) {
		k.show_name_title(&k.s_str("input"));
	}

	async fn on_press(&self, k: &KeyRc, press: Press) -> Res<()> {
		let input = k.s_val("input");
		let operation = press.op;
		if !truthy(Some(&input)) {
			return Err("no browser source selected".into());
		}
		match operation.as_str() {
			"refresh" => {
				k.call("PressInputPropertiesButton", json!({ "inputName": input, "propertyName": "refreshnocache" })).await?;
			}
			"url" => {
				let url = k.s_val("url");
				if !truthy(Some(&url)) {
					return Err("no URL set".into());
				}
				k.call("SetInputSettings", json!({ "inputName": input, "inputSettings": { "url": url }, "overlay": true })).await?;
			}
			"cycle" => {
				let urls = url_list(k);
				if urls.is_empty() {
					return Err("no URLs in the list".into());
				}
				let index = (k.s_num_or("index", 0.0) + 1.0) % urls.len() as f64;
				let url = if index >= 0.0 { urls.get(index as usize).cloned() } else { None };
				let mut settings = Map::new();
				if let Some(url) = url {
					settings.insert("url".into(), url.into());
				}
				k.call("SetInputSettings", json!({ "inputName": input, "inputSettings": settings, "overlay": true })).await?;
				k.save_settings(patch(vec![("index", js::num(index))]));
			}
			"visibility" => {
				let scene = k.data().program_scene.map_or(Value::Null, Value::from);
				let item = scene_item(k, &scene, &input).await?;
				let enabled = truthy(
					k.call("GetSceneItemEnabled", json!({ "sceneName": scene, "sceneItemId": item })).await?.get("sceneItemEnabled"),
				);
				k.call("SetSceneItemEnabled", json!({ "sceneName": scene, "sceneItemId": item, "sceneItemEnabled": !enabled })).await?;
			}
			_ => {}
		}
		k.show_ok();
		Ok(())
	}
}

pub static BROWSER_SOURCE: Kind = Kind {
	uuid: "de.felitendo.obs-tools.browsersource",
	name: "BrowserSource",
	defaults: || {
		defaults(
			json!({ "input": "", "operation": "refresh", "hold": "none", "double": "none", "url": "", "urls": "", "index": 0, "showName": false }),
		)
	},
	press_key: "operation",
	ops: &["refresh", "url", "cycle", "visibility"],
	required: &["input"],
	behavior: &BrowserSource,
	..Kind::BASE
};

// ----- Filter Toggle -----

#[derive(Default)]
struct FilterState {
	enabled: bool,
}

struct FilterToggle;

async fn load_filter(k: &KeyRc) {
	k.own::<FilterState>().enabled = false;
	let (source, filter) = (k.s_val("source"), k.s_val("filter"));
	if !truthy(Some(&source)) || !truthy(Some(&filter)) {
		return;
	}
	let enabled = match k.call("GetSourceFilter", json!({ "sourceName": source, "filterName": filter })).await {
		Ok(r) => truthy(r.get("filterEnabled")),
		Err(_) => false,
	};
	k.own::<FilterState>().enabled = enabled;
}

#[async_trait(?Send)]
impl Behavior for FilterToggle {
	async fn on_connected(&self, k: &KeyRc) -> Res<()> {
		load_filter(k).await;
		Ok(())
	}

	async fn on_settings_changed(&self, k: &KeyRc, old: &Map<String, Value>) -> Res<()> {
		if (old.get("source") != k.s("source").as_ref() || old.get("filter") != k.s("filter").as_ref()) && k.online() {
			load_filter(k).await;
		}
		Ok(())
	}

	async fn on_obs_event(&self, k: &KeyRc, kind: &str, data: &Value) -> Res<()> {
		if kind == "SourceFilterEnableStateChanged"
			&& data.get("sourceName") == k.s("source").as_ref()
			&& data.get("filterName") == k.s("filter").as_ref()
		{
			k.own::<FilterState>().enabled = truthy(data.get("filterEnabled"));
			self.render(k);
		}
		Ok(())
	}

	fn render(&self, k: &KeyRc) {
		let enabled = k.own::<FilterState>().enabled;
		k.set_state(if enabled { 1 } else { 0 });
		k.show_name_title(&k.s_str("filter"));
	}

	async fn on_press(&self, k: &KeyRc, press: Press) -> Res<()> {
		let (source, filter) = (k.s_val("source"), k.s_val("filter"));
		let mode = press.op;
		if !truthy(Some(&source)) || !truthy(Some(&filter)) {
			return Err("no filter selected".into());
		}
		let enabled = match mode.as_str() {
			"enable" => true,
			"disable" => false,
			_ => !truthy(k.call("GetSourceFilter", json!({ "sourceName": source, "filterName": filter })).await?.get("filterEnabled")),
		};
		k.call("SetSourceFilterEnabled", json!({ "sourceName": source, "filterName": filter, "filterEnabled": enabled })).await?;
		k.own::<FilterState>().enabled = enabled;
		self.render(k);
		Ok(())
	}
}

pub static FILTER_TOGGLE: Kind = Kind {
	uuid: "de.felitendo.obs-tools.filtertoggle",
	name: "FilterToggle",
	defaults: || defaults(json!({ "source": "", "filter": "", "mode": "toggle", "hold": "none", "double": "none", "showName": false })),
	press_key: "mode",
	ops: &["toggle", "enable", "disable"],
	required: &["source", "filter"],
	behavior: &FilterToggle,
	new_state: || Box::new(FilterState::default()),
	..Kind::BASE
};

// ----- Focused Window Capture: points a Window Capture (Xcomposite) source at the focused window. -----

struct FocusedWindow;

#[async_trait(?Send)]
impl Behavior for FocusedWindow {
	async fn on_press(&self, k: &KeyRc, _press: Press) -> Res<()> {
		let input = k.s_val("input");
		if !truthy(Some(&input)) {
			return Err("no window capture source selected".into());
		}
		// Fails early when the source is missing.
		k.call("GetInputSettings", json!({ "inputName": input })).await?;
		let delay = k.s_num("delay");
		if delay > 0.0 {
			crate::timers::sleep(delay * 1000.0).await;
		}
		let Some(window) = active_x11_window().await? else {
			return Err(if session_type() == "wayland" {
				"no X11 window has focus (Wayland apps cannot be picked)"
			} else {
				"no focused window found"
			}
			.into());
		};
		k.call(
			"SetInputSettings",
			json!({ "inputName": input, "inputSettings": { "capture_window": xcomposite_window_value(&window) }, "overlay": true }),
		)
		.await?;
		k.log(&format!("window capture \"{}\" -> {} ({})", js::to_string(Some(&input)), window.name, window.cls));
		k.show_ok();
		Ok(())
	}
}

pub static FOCUSED_WINDOW: Kind = Kind {
	uuid: "de.felitendo.obs-tools.focusedwindow",
	name: "FocusedWindow",
	defaults: || defaults(json!({ "input": "", "delay": 0 })),
	required: &["input"],
	behavior: &FocusedWindow,
	..Kind::BASE
};

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn file_titles() {
		assert_eq!(file_title("/home/me/Videos/Big clip.mp4"), "Big clip");
		assert_eq!(file_title("/a/b.tar.gz"), "b.tar");
		assert_eq!(file_title(".bashrc"), "");
		assert_eq!(file_title("noext"), "noext");
		assert_eq!(file_title("/a/dir/"), "dir");
		assert_eq!(file_title("x."), "x.");
		assert_eq!(file_title(""), "");
	}
}
