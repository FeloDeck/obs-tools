// Dials (Stream Deck +): transition time, scene picker and any number in a filter.
use async_trait::async_trait;
use serde_json::{Map, Value, json};

use super::{Behavior, KeyRc, Kind, Stop, defaults};
use crate::Res;
use crate::i18n::{num, t};
use crate::icons::colors;
use crate::js::{self, truthy};
use crate::render::dial_icon;
use crate::timers::{Timer, set_timeout};

const MIN_DURATION: f64 = 50.0;
const MAX_DURATION: f64 = 20000.0;

// ----- Transition dial: turn for the transition time, press for the next transition. -----

#[derive(Default)]
struct TransitionState {
	transitions: Option<Vec<Value>>,
}

struct TransitionDial;

fn transitions(k: &KeyRc) -> std::cell::RefMut<'_, TransitionState> {
	k.x::<TransitionState>()
}

async fn load_transitions(k: &KeyRc) {
	let list = match k.call("GetSceneTransitionList", json!({})).await {
		Ok(list) => match list.get("transitions") {
			Some(Value::Array(items)) => items.clone(),
			_ => Vec::new(),
		},
		Err(_) => Vec::new(),
	};
	transitions(k).transitions = Some(list);
}

fn current_transition(k: &KeyRc) -> Option<Value> {
	let name = k.data().transition.name;
	transitions(k).transitions.as_ref()?.iter().find(|t| t.get("transitionName").and_then(Value::as_str) == name.as_deref()).cloned()
}

#[async_trait(?Send)]
impl Behavior for TransitionDial {
	fn layout(&self, _k: &KeyRc) -> Option<&'static str> {
		Some("layouts/dial.json")
	}

	async fn on_connected(&self, k: &KeyRc) -> Res<()> {
		load_transitions(k).await;
		Ok(())
	}

	fn render_offline(&self, k: &KeyRc) {
		k.set_feedback(
			json!({ "title": t("Transition"), "icon": dial_icon("transition", false), "value": k.dial_status(), "indicator": { "value": 0 } }),
		);
	}

	fn render(&self, k: &KeyRc) {
		let transition = k.data().transition;
		let fixed = truthy(current_transition(k).as_ref().and_then(|c| c.get("transitionFixed")));
		let duration = transition.duration.filter(|d| js::truthy_num(*d));
		let value = if fixed {
			t("no time")
		} else {
			match duration {
				Some(d) => format!("{} ms", js::fmt(d)),
				None => String::new(),
			}
		};
		let indicator = match duration {
			Some(d) if !fixed => {
				js::min(100.0, js::round((d.log10() - MIN_DURATION.log10()) / (5000f64.log10() - MIN_DURATION.log10()) * 100.0))
			}
			_ => 0.0,
		};
		k.set_feedback(json!({
			"title": transition.name.map_or_else(|| Value::from(t("Transition")), Value::from),
			"icon": dial_icon("transition", false),
			"value": value,
			"indicator": { "value": js::num(indicator) },
		}));
	}

	async fn on_obs_event(&self, k: &KeyRc, kind: &str, _data: &Value) -> Res<()> {
		if matches!(kind, "TransitionCreated" | "TransitionRemoved" | "TransitionNameChanged") {
			let k = k.clone();
			tokio::task::spawn_local(async move {
				load_transitions(&k).await;
				k.render();
			});
		}
		Ok(())
	}

	async fn on_dial_rotate(&self, k: &KeyRc, ticks: f64, _pressed: bool) -> Res<()> {
		let transition = k.data().transition;
		if truthy(current_transition(k).as_ref().and_then(|c| c.get("transitionFixed"))) {
			return Err(
				format!("{} has no duration", js::to_string(transition.name.map(Value::from).as_ref().or(Some(&Value::Null)))).into()
			);
		}
		let step = js::max(10.0, k.s_num_or("step", 50.0));
		let duration = js::min(MAX_DURATION, js::max(MIN_DURATION, transition.duration.unwrap_or(300.0) + ticks * step));
		k.call("SetCurrentSceneTransitionDuration", json!({ "transitionDuration": js::num(duration) })).await?;
		if let Some(state) = k.obs_state() {
			state.update(|d| d.transition.duration = Some(duration));
		}
		self.render(k);
		Ok(())
	}

	async fn on_dial_down(&self, k: &KeyRc) -> Res<()> {
		let action = k.s_str("pressAction");
		if action == "nothing" {
			return Ok(());
		}
		if action == "studio" {
			if !k.data().studio_mode {
				return Err("studio mode is off".into());
			}
			k.call("TriggerStudioModeTransition", json!({})).await?;
			return Ok(());
		}
		if transitions(k).transitions.as_ref().is_none_or(Vec::is_empty) {
			load_transitions(k).await;
		}
		let names: Vec<Value> = transitions(k)
			.transitions
			.clone()
			.unwrap_or_default()
			.iter()
			.map(|t| t.get("transitionName").cloned().unwrap_or(Value::Null))
			.collect();
		if names.is_empty() {
			return Ok(());
		}
		let current = k.data().transition.name.map(Value::from);
		let index = current.and_then(|c| names.iter().position(|n| *n == c)).map_or(-1, |i| i as i64);
		let next = names[((index + 1) % names.len() as i64) as usize].clone();
		k.call("SetCurrentSceneTransition", json!({ "transitionName": next })).await?;
		Ok(())
	}
}

pub static TRANSITION_DIAL: Kind = Kind {
	uuid: "de.felitendo.obs-tools.transitiondial",
	name: "TransitionDial",
	defaults: || defaults(json!({ "step": 50, "pressAction": "next", "disableTouch": false })),
	press_key: "",
	ops: &[],
	press_only: &[],
	can_repeat: false,
	required: &[],
	behavior: &TransitionDial,
	new_state: || Box::new(TransitionState::default()),
};

// ----- Scene dial: turn to pick a scene (with a picture of it), press to switch. -----

const PICK_TIMEOUT: f64 = 6000.0;

#[derive(Default)]
struct SceneDialState {
	selected: Option<String>,
	stop_shots: Option<Stop>,
	shot_token: u64,
	shot: Option<String>,
	shot_scene: Option<String>,
	pick_timer: Option<Timer>,
	switch_timer: Option<Timer>,
}

struct SceneDial;

fn dial(k: &KeyRc) -> std::cell::RefMut<'_, SceneDialState> {
	k.x::<SceneDialState>()
}

fn target(k: &KeyRc) -> Option<String> {
	let selected = dial(k).selected.clone();
	selected.or_else(|| k.data().program_scene)
}

async fn refresh_scene_shot(k: &KeyRc) {
	let scene = target(k).unwrap_or_default();
	if scene.is_empty() || !k.visible() {
		return;
	}
	let token = {
		let mut d = dial(k);
		d.shot_token += 1;
		d.shot_token
	};
	match k
		.call("GetSourceScreenshot", json!({ "sourceName": scene, "imageFormat": "jpg", "imageWidth": 192, "imageCompressionQuality": 70 }))
		.await
	{
		Ok(result) => {
			if token != dial(k).shot_token {
				return;
			}
			match result.get("imageData").and_then(Value::as_str) {
				Some(data) => {
					let mut d = dial(k);
					d.shot = Some(match data.strip_prefix("data:image/jpg;") {
						Some(rest) => format!("data:image/jpeg;{rest}"),
						None => data.to_string(),
					});
					d.shot_scene = Some(scene);
				}
				None => dial(k).shot = None,
			}
		}
		Err(_) => dial(k).shot = None,
	}
	k.render();
}

fn clear_pick_timer(k: &KeyRc) {
	let timer = dial(k).pick_timer.take();
	if let Some(timer) = timer {
		timer.clear();
	}
}

async fn switch_to(k: &KeyRc, scene: Option<String>) -> Res<()> {
	let data = k.data();
	let scene = scene.map_or(Value::Null, Value::from);
	if data.studio_mode && !k.is_in_multi_action() {
		if data.preview_scene.map(Value::from).unwrap_or(Value::Null) == scene {
			k.call("TriggerStudioModeTransition", json!({})).await?;
		} else {
			k.call("SetCurrentPreviewScene", json!({ "sceneName": scene })).await?;
		}
	} else {
		k.call("SetCurrentProgramScene", json!({ "sceneName": scene })).await?;
	}
	clear_pick_timer(k);
	dial(k).selected = None;
	Ok(())
}

#[async_trait(?Send)]
impl Behavior for SceneDial {
	fn layout(&self, _k: &KeyRc) -> Option<&'static str> {
		Some("layouts/scene-dial.json")
	}

	async fn on_connected(&self, k: &KeyRc) -> Res<()> {
		dial(k).selected = None;
		let old = dial(k).stop_shots.take();
		if let Some(stop) = old {
			stop();
		}
		let stop = k.interval(2000.0, |k| {
			Box::pin(async move {
				refresh_scene_shot(&k).await;
				Ok(())
			})
		});
		dial(k).stop_shots = Some(stop);
		refresh_scene_shot(k).await;
		Ok(())
	}

	fn on_disconnected(&self, k: &KeyRc) {
		let stop = dial(k).stop_shots.clone();
		if let Some(stop) = stop {
			stop();
		}
	}

	async fn on_state_change(&self, k: &KeyRc, what: &str) -> Res<()> {
		if matches!(what, "program" | "preview" | "scenes") {
			let selected = dial(k).selected.clone();
			if let Some(selected) = selected
				&& !k.data().scenes.contains(&selected)
			{
				dial(k).selected = None;
			}
			let key = k.clone();
			tokio::task::spawn_local(async move { refresh_scene_shot(&key).await });
		}
		self.render(k);
		Ok(())
	}

	fn render_offline(&self, k: &KeyRc) {
		k.set_feedback(json!({ "title": t("Scene"), "name": "", "info": k.dial_status(), "preview": "" }));
	}

	fn render(&self, k: &KeyRc) {
		let data = k.data();
		let scene = target(k);
		let index = scene.as_ref().and_then(|s| data.scenes.iter().position(|x| x == s)).map_or(-1, |i| i as i64);
		let live = scene == data.program_scene;
		let preview = data.studio_mode && scene == data.preview_scene;
		let info = if live {
			t("LIVE")
		} else if preview {
			t("PREVIEW")
		} else {
			format!("{} / {}", index + 1, data.scenes.len())
		};
		let selected = dial(k).selected.is_some();
		let picture = {
			let d = dial(k);
			if d.shot_scene == scene && d.shot.is_some() { d.shot.clone().unwrap_or_default() } else { String::new() }
		};
		k.set_feedback(json!({
			"title": t(if selected && !live { "Press to switch" } else { "Scene" }),
			"name": { "value": scene.unwrap_or_default(), "color": if live { colors::RED } else if preview { colors::GREEN } else { colors::WHITE } },
			"info": { "value": info, "color": if live { colors::RED } else if preview { colors::GREEN } else { colors::MUTED } },
			"preview": picture,
		}));
	}

	async fn on_dial_rotate(&self, k: &KeyRc, ticks: f64, _pressed: bool) -> Res<()> {
		let scenes = k.data().scenes;
		if scenes.is_empty() {
			return Ok(());
		}
		let len = scenes.len() as i64;
		let from = target(k).and_then(|t| scenes.iter().position(|s| *s == t)).map_or(-1, |i| i as i64);
		let next = scenes[(from + ticks as i64).rem_euclid(len) as usize].clone();
		clear_pick_timer(k);
		if k.s_str("mode") == "direct" {
			// Wait a moment, so a fast turn does not switch through every scene.
			let old = dial(k).switch_timer.take();
			if let Some(timer) = old {
				timer.clear();
			}
			dial(k).selected = Some(next.clone());
			self.render(k);
			let key = std::rc::Rc::downgrade(k);
			let timer = set_timeout(200.0, move || {
				let Some(k) = key.upgrade() else { return };
				tokio::task::spawn_local(async move {
					k.safe(switch_to(&k, Some(next))).await;
				});
			});
			dial(k).switch_timer = Some(timer);
			return Ok(());
		}
		dial(k).selected = Some(next);
		self.render(k);
		let key = k.clone();
		tokio::task::spawn_local(async move { refresh_scene_shot(&key).await });
		let key = std::rc::Rc::downgrade(k);
		let timer = set_timeout(PICK_TIMEOUT, move || {
			let Some(k) = key.upgrade() else { return };
			dial(&k).selected = None;
			tokio::task::spawn_local(async move { refresh_scene_shot(&k).await });
		});
		dial(k).pick_timer = Some(timer);
		Ok(())
	}

	async fn on_dial_down(&self, k: &KeyRc) -> Res<()> {
		switch_to(k, target(k)).await
	}
}

pub static SCENE_DIAL: Kind = Kind {
	uuid: "de.felitendo.obs-tools.scenedial",
	name: "SceneDial",
	defaults: || defaults(json!({ "mode": "pick", "disableTouch": false })),
	press_key: "",
	ops: &[],
	press_only: &[],
	can_repeat: false,
	required: &[],
	behavior: &SceneDial,
	new_state: || Box::new(SceneDialState::default()),
};

// ----- Filter dial: turn to change one number of a filter, press to turn the filter on or off. -----

/// Good steps and limits for the numbers in the filters that come with OBS: [min, max, step].
/// [min, max, step] of a number in a filter.
pub type Range = [f64; 3];

pub static FILTER_RANGES: &[(&str, &[(&str, Range)])] = &[
	(
		"color_filter",
		&[
			("gamma", [-3.0, 3.0, 0.05]),
			("contrast", [-4.0, 4.0, 0.05]),
			("brightness", [-1.0, 1.0, 0.01]),
			("saturation", [-1.0, 5.0, 0.05]),
			("hue_shift", [-180.0, 180.0, 2.0]),
			("opacity", [0.0, 1.0, 0.01]),
		],
	),
	("gain_filter", &[("db", [-30.0, 30.0, 0.5])]),
	("sharpness_filter", &[("sharpness", [0.0, 1.0, 0.01])]),
	("noise_suppress_filter", &[("suppress_level", [-60.0, 0.0, 1.0])]),
	("noise_gate_filter", &[("open_threshold", [-96.0, 0.0, 1.0]), ("close_threshold", [-96.0, 0.0, 1.0])]),
	(
		"compressor_filter",
		&[
			("ratio", [1.0, 32.0, 0.5]),
			("threshold", [-60.0, 0.0, 1.0]),
			("output_gain", [-32.0, 32.0, 0.5]),
			("attack_time", [1.0, 500.0, 5.0]),
			("release_time", [1.0, 1000.0, 10.0]),
		],
	),
	("limiter_filter", &[("threshold", [-60.0, 0.0, 1.0]), ("release_time", [1.0, 1000.0, 10.0])]),
	("expander_filter", &[("ratio", [1.0, 20.0, 0.5]), ("threshold", [-60.0, 0.0, 1.0])]),
	("basic_eq_filter", &[("low", [-20.0, 20.0, 0.5]), ("mid", [-20.0, 20.0, 0.5]), ("high", [-20.0, 20.0, 0.5])]),
	("scroll_filter", &[("speed_x", [-500.0, 500.0, 10.0]), ("speed_y", [-500.0, 500.0, 10.0])]),
	(
		"crop_filter",
		&[("left", [0.0, 4000.0, 2.0]), ("right", [0.0, 4000.0, 2.0]), ("top", [0.0, 4000.0, 2.0]), ("bottom", [0.0, 4000.0, 2.0])],
	),
	("gpu_delay", &[("delay_ms", [0.0, 500.0, 10.0])]),
	("async_delay_filter", &[("delay_ms", [0.0, 20000.0, 50.0])]),
	(
		"chroma_key_filter",
		&[
			("similarity", [1.0, 1000.0, 5.0]),
			("smoothness", [1.0, 1000.0, 5.0]),
			("spill", [1.0, 1000.0, 5.0]),
			("opacity", [0.0, 1.0, 0.01]),
			("contrast", [-4.0, 4.0, 0.05]),
			("brightness", [-1.0, 1.0, 0.01]),
			("gamma", [-1.0, 1.0, 0.01]),
		],
	),
	(
		"color_key_filter",
		&[
			("similarity", [1.0, 1000.0, 5.0]),
			("smoothness", [1.0, 1000.0, 5.0]),
			("opacity", [0.0, 1.0, 0.01]),
			("contrast", [-4.0, 4.0, 0.05]),
			("brightness", [-1.0, 1.0, 0.01]),
			("gamma", [-1.0, 1.0, 0.01]),
		],
	),
	(
		"luma_key_filter",
		&[
			("luma_max", [0.0, 1.0, 0.01]),
			("luma_min", [0.0, 1.0, 0.01]),
			("luma_max_smooth", [0.0, 1.0, 0.01]),
			("luma_min_smooth", [0.0, 1.0, 0.01]),
		],
	),
	("mask_filter", &[("opacity", [0.0, 1.0, 0.01])]),
];

// Readable names for the numbers in the filters above. Others get their key, a bit tidied up.
static FILTER_VALUE_LABELS: &[(&str, &str)] = &[
	("gamma", "Gamma"),
	("contrast", "Contrast"),
	("brightness", "Brightness"),
	("saturation", "Saturation"),
	("hue_shift", "Hue shift"),
	("opacity", "Opacity"),
	("db", "Gain"),
	("sharpness", "Sharpness"),
	("suppress_level", "Suppression"),
	("open_threshold", "Open threshold"),
	("close_threshold", "Close threshold"),
	("ratio", "Ratio"),
	("threshold", "Threshold"),
	("output_gain", "Output gain"),
	("attack_time", "Attack"),
	("release_time", "Release"),
	("low", "Low"),
	("mid", "Mid"),
	("high", "High"),
	("speed_x", "Speed X"),
	("speed_y", "Speed Y"),
	("left", "Left"),
	("right", "Right"),
	("top", "Top"),
	("bottom", "Bottom"),
	("delay_ms", "Delay"),
	("similarity", "Similarity"),
	("smoothness", "Smoothness"),
	("spill", "Spill reduction"),
];

pub fn filter_value_label(key: &str) -> String {
	if let Some((_, label)) = FILTER_VALUE_LABELS.iter().find(|(k, _)| *k == key) {
		return label.to_string();
	}
	let text = key.replace('_', " ");
	let mut chars = text.chars();
	match chars.next() {
		Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
		None => String::new(),
	}
}

pub fn filter_range(kind: &str, key: &str) -> Option<[f64; 3]> {
	// "color_filter_v2" -> "color_filter"
	let base = match kind.rfind("_v") {
		Some(i) if i + 2 < kind.len() && kind[i + 2..].bytes().all(|b| b.is_ascii_digit()) => &kind[..i],
		_ => kind,
	};
	FILTER_RANGES.iter().find(|(k, _)| *k == base)?.1.iter().find(|(k, _)| *k == key).map(|(_, r)| *r)
}

fn format_number(value: f64, step: f64) -> String {
	let step = if js::truthy_num(step) { step } else { 1.0 };
	let decimals = js::min(3.0, js::max(0.0, -step.log10().floor()));
	num(value, if decimals.is_nan() { 0 } else { decimals as usize })
}

#[derive(Default)]
struct FilterDialState {
	filter: Option<Map<String, Value>>,
	defaults: Option<Map<String, Value>>,
	read_at: i64,
	stop_poll: Option<Stop>,
}

struct FilterDial;

fn fd(k: &KeyRc) -> std::cell::RefMut<'_, FilterDialState> {
	k.x::<FilterDialState>()
}

async fn load_filter(k: &KeyRc) {
	fd(k).filter = None;
	let (source, filter) = (k.s_val("source"), k.s_val("filter"));
	if !truthy(Some(&source)) || !truthy(Some(&filter)) {
		return;
	}
	let result: Res<()> = async {
		let found = k.call("GetSourceFilter", json!({ "sourceName": source, "filterName": filter })).await?;
		let kind = found.get("filterKind").cloned().unwrap_or(Value::Null);
		fd(k).filter = Some(js::object(Some(&found)));
		let defaults = k.call("GetSourceFilterDefaultSettings", json!({ "filterKind": kind })).await?;
		fd(k).defaults = Some(js::object(defaults.get("defaultFilterSettings")));
		Ok(())
	}
	.await;
	if result.is_err() {
		fd(k).filter = None;
	}
}

// Fresh settings from OBS. Skipped while the dial is turned quickly.
async fn refresh_filter(k: &KeyRc, max_age: i64) {
	let has = fd(k).filter.is_some();
	if !has || !k.visible() || js::now() - fd(k).read_at < max_age {
		return;
	}
	let Ok(fresh) = k.call("GetSourceFilter", json!({ "sourceName": k.s_val("source"), "filterName": k.s_val("filter") })).await else {
		return;
	};
	let changed = {
		let mut s = fd(k);
		s.read_at = js::now();
		let old = s.filter.clone().unwrap_or_default();
		let settings_changed = js::stringify(fresh.get("filterSettings").unwrap_or(&Value::Null))
			!= js::stringify(old.get("filterSettings").unwrap_or(&Value::Null));
		let changed = settings_changed || fresh.get("filterEnabled") != old.get("filterEnabled");
		s.filter = Some(js::object(Some(&fresh)));
		changed
	};
	if changed {
		k.render();
	}
}

// [min, max, step]: from the settings page, the table above, or a guess.
fn range(k: &KeyRc) -> (f64, f64, f64) {
	let setting = k.s_str("setting");
	let (kind, default) = {
		let s = fd(k);
		(js::str_or_empty(s.filter.as_ref().and_then(|f| f.get("filterKind"))), s.defaults.as_ref().and_then(|d| d.get(&setting)).cloned())
	};
	let known = filter_range(&kind, &setting);
	let guess_max = if js::number_or(default.as_ref(), 0.0).abs() <= 1.0 { 1.0 } else { 100.0 };
	let given = |key: &str| match k.s(key) {
		Some(Value::String(s)) if s.is_empty() => None,
		Some(Value::Null) => None,
		v => Some(js::number(v.as_ref())),
	};
	let min = given("min").unwrap_or_else(|| known.map_or(0.0, |r| r[0]));
	let max = given("max").unwrap_or_else(|| known.map_or(guess_max, |r| r[1]));
	let step_setting = k.s_num("step");
	let step = if step_setting > 0.0 { step_setting } else { known.map_or((max - min) / 100.0, |r| r[2]) };
	(min, max, step)
}

fn filter_value(k: &KeyRc) -> f64 {
	let key = k.s_str("setting");
	let s = fd(k);
	let v = s
		.filter
		.as_ref()
		.and_then(|f| f.get("filterSettings"))
		.and_then(|fs| fs.get(&key))
		.filter(|v| !v.is_null())
		.or_else(|| s.defaults.as_ref().and_then(|d| d.get(&key)));
	match v {
		Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
		v => js::number_or(v, 0.0),
	}
}

fn set_filter_value(k: &KeyRc, setting: &str, value: Value) {
	if let Some(filter) = fd(k).filter.as_mut() {
		let mut settings = js::object(filter.get("filterSettings"));
		settings.insert(setting.to_string(), value);
		filter.insert("filterSettings".into(), Value::Object(settings));
	}
}

#[async_trait(?Send)]
impl Behavior for FilterDial {
	fn layout(&self, _k: &KeyRc) -> Option<&'static str> {
		Some("layouts/dial.json")
	}

	async fn on_connected(&self, k: &KeyRc) -> Res<()> {
		load_filter(k).await;
		// OBS sends no event when filter settings change, so check now and then.
		let old = fd(k).stop_poll.take();
		if let Some(stop) = old {
			stop();
		}
		let stop = k.interval(2000.0, |k| {
			Box::pin(async move {
				refresh_filter(&k, 0).await;
				Ok(())
			})
		});
		fd(k).stop_poll = Some(stop);
		Ok(())
	}

	fn on_disconnected(&self, k: &KeyRc) {
		let stop = fd(k).stop_poll.clone();
		if let Some(stop) = stop {
			stop();
		}
	}

	async fn on_settings_changed(&self, k: &KeyRc, old: &Map<String, Value>) -> Res<()> {
		let changed = ["source", "filter", "setting"].iter().any(|key| old.get(*key) != k.s(key).as_ref());
		if k.online() && changed {
			load_filter(k).await;
		}
		Ok(())
	}

	async fn on_obs_event(&self, k: &KeyRc, kind: &str, data: &Value) -> Res<()> {
		if data.get("sourceName") != k.s("source").as_ref() || data.get("filterName") != k.s("filter").as_ref() || fd(k).filter.is_none() {
			return Ok(());
		}
		match kind {
			"SourceFilterSettingsChanged" => {
				if let Some(filter) = fd(k).filter.as_mut() {
					filter.insert("filterSettings".into(), data.get("filterSettings").cloned().unwrap_or(Value::Null));
				}
			}
			"SourceFilterEnableStateChanged" => {
				if let Some(filter) = fd(k).filter.as_mut() {
					filter.insert("filterEnabled".into(), data.get("filterEnabled").cloned().unwrap_or(Value::Null));
				}
			}
			_ => return Ok(()),
		}
		self.render(k);
		Ok(())
	}

	// Without OBS or settings: the name, the status, and the setup dot on the icon.
	fn render_offline(&self, k: &KeyRc) {
		let title = if k.s_truthy("filter") { k.s_val("filter") } else { Value::from(t("Filter")) };
		k.set_feedback(
			json!({ "title": title, "icon": dial_icon("filter", !k.is_set_up()), "value": k.dial_status(), "indicator": { "value": 0 } }),
		);
	}

	fn render(&self, k: &KeyRc) {
		if !k.is_set_up() {
			return self.render_offline(k);
		}
		let filter_name = k.s_str("filter");
		let filter = fd(k).filter.clone();
		let Some(filter) = filter else {
			// The filter is not in OBS (any more).
			k.set_feedback(
				json!({ "title": k.s_val("filter"), "icon": dial_icon("filter", false), "value": "?", "indicator": { "value": 0 } }),
			);
			return;
		};
		let (min, max, step) = range(k);
		let value = filter_value(k);
		let on = truthy(filter.get("filterEnabled"));
		let indicator = if max > min { js::round(((value - min) / (max - min)) * 100.0) } else { 0.0 };
		k.set_feedback(json!({
			"title": format!("{filter_name} · {}", t(&filter_value_label(&k.s_str("setting")))),
			"icon": dial_icon("filter", false),
			"value": { "value": if on { format_number(value, step) } else { t("off") }, "color": if on { colors::WHITE } else { colors::GRAY } },
			"indicator": { "value": js::num(indicator), "bar_fill_c": if on { colors::WHITE } else { colors::GRAY } },
		}));
	}

	async fn on_dial_rotate(&self, k: &KeyRc, ticks: f64, _pressed: bool) -> Res<()> {
		let setting = k.s_str("setting");
		if fd(k).filter.is_none() {
			load_filter(k).await;
		}
		if fd(k).filter.is_none() || setting.is_empty() {
			return Err("no filter setting selected".into());
		}
		refresh_filter(k, 300).await;
		let (min, max, step) = range(k);
		let value = js::min(max, js::max(min, js::round((filter_value(k) + ticks * step) / step) * step));
		let rounded = js::round(value * 10000.0) / 10000.0;
		let default = fd(k).defaults.as_ref().and_then(|d| d.get(&setting)).cloned();
		let is_int = default.as_ref().and_then(Value::as_f64).is_some_and(|d| d.is_finite() && d.fract() == 0.0)
			&& step.is_finite()
			&& step.fract() == 0.0;
		let mut settings = Map::new();
		settings.insert(setting.clone(), js::num(if is_int { js::round(rounded) } else { rounded }));
		k.call(
			"SetSourceFilterSettings",
			json!({ "sourceName": k.s_val("source"), "filterName": k.s_val("filter"), "filterSettings": settings, "overlay": true }),
		)
		.await?;
		set_filter_value(k, &setting, js::num(rounded));
		fd(k).read_at = js::now();
		self.render(k);
		Ok(())
	}

	async fn on_dial_down(&self, k: &KeyRc) -> Res<()> {
		let setting = k.s_str("setting");
		let action = k.s_str("pressAction");
		if fd(k).filter.is_none() {
			load_filter(k).await;
		}
		let Some(filter) = fd(k).filter.clone() else { return Err("no filter selected".into()) };
		if action == "reset" && !setting.is_empty() {
			let value = fd(k).defaults.as_ref().and_then(|d| d.get(&setting)).filter(|v| !v.is_null()).cloned().unwrap_or(json!(0));
			let mut settings = Map::new();
			settings.insert(setting.clone(), value.clone());
			k.call(
				"SetSourceFilterSettings",
				json!({ "sourceName": k.s_val("source"), "filterName": k.s_val("filter"), "filterSettings": settings, "overlay": true }),
			)
			.await?;
			set_filter_value(k, &setting, value);
		} else if action == "toggle" {
			let enabled = !truthy(filter.get("filterEnabled"));
			k.call(
				"SetSourceFilterEnabled",
				json!({ "sourceName": k.s_val("source"), "filterName": k.s_val("filter"), "filterEnabled": enabled }),
			)
			.await?;
			if let Some(filter) = fd(k).filter.as_mut() {
				filter.insert("filterEnabled".into(), enabled.into());
			}
		}
		self.render(k);
		Ok(())
	}
}

pub static FILTER_DIAL: Kind = Kind {
	uuid: "de.felitendo.obs-tools.filterdial",
	name: "FilterDial",
	defaults: || {
		defaults(
			json!({ "source": "", "filter": "", "setting": "", "step": "", "min": "", "max": "", "pressAction": "toggle", "disableTouch": false }),
		)
	},
	press_key: "",
	ops: &[],
	press_only: &[],
	can_repeat: false,
	required: &["source", "filter", "setting"],
	behavior: &FilterDial,
	new_state: || Box::new(FilterDialState::default()),
};

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn filter_ranges_know_versioned_filter_kinds() {
		assert_eq!(filter_range("color_filter_v2", "opacity"), Some([0.0, 1.0, 0.01]));
		assert_eq!(filter_range("gain_filter", "db"), Some([-30.0, 30.0, 0.5]));
		assert_eq!(filter_range("unknown_filter", "x"), None);
	}

	#[test]
	fn readable_names_for_filter_values() {
		assert_eq!(filter_value_label("opacity"), "Opacity");
		assert_eq!(filter_value_label("some_new_value"), "Some new value");
	}
}
