// Audio: volume and mute of an input, its monitoring, and the volume dial.
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{Behavior, KeyRc, Kind, Press, Stop, defaults, same_name};
use crate::Res;
use crate::i18n::t;
use crate::icons::{colors, volume_glyph};
use crate::js;
use crate::render::{GlyphKey, dial_icon, glyph_key, meter_level};
use crate::settings::{self, read};
use crate::volume::{clamp_db, db_to_fader, fader_to_db, format_volume, read_db, step_volume, volume_request};

fn monitor_type(name: &str) -> Option<&'static str> {
	match name {
		"none" => Some("OBS_MONITORING_TYPE_NONE"),
		"monitorOnly" => Some("OBS_MONITORING_TYPE_MONITOR_ONLY"),
		"monitorAndOutput" => Some("OBS_MONITORING_TYPE_MONITOR_AND_OUTPUT"),
		_ => None,
	}
}

/// The settings of the audio actions. Each one uses a part of them.
#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct AudioSettings {
	input: String,
	/// Source Volume: up, down, set or mute. Mute Toggle: toggle, mute, unmute, talk or holdMute.
	mode: String,
	/// "db" or "percent".
	unit: String,
	step: f64,
	/// What "set" sets, in the unit.
	value: f64,
	allow_boost: bool,
	show_value: bool,
	value_at: String,
	show_meter: bool,
	show_name: bool,
	monitor_type: String,
	press_again_off: bool,
	/// Volume dial: mute, reset or nothing.
	press_action: String,
}

/// Shared by the key and the dial: keeps volume and mute of one input up to date.
#[derive(Default)]
pub struct InputState {
	/// None: not known (null). The value can be -inf.
	db: Option<f64>,
	muted: bool,
	monitor_type: Option<String>,
	level: f64,
	meter_at: i64,
	stop_meter: Option<Stop>,
}

fn new_state() -> Box<dyn std::any::Any> {
	Box::new(InputState::default())
}

fn st(k: &KeyRc) -> std::cell::RefMut<'_, InputState> {
	k.own::<InputState>()
}

fn stop_meter(k: &KeyRc) {
	let stop = st(k).stop_meter.clone();
	if let Some(stop) = stop {
		stop();
	}
}

// Live audio level, at most 10 updates a second and only when it changes visibly.
fn start_meter(k: &KeyRc) {
	stop_meter(k);
	st(k).level = 0.0;
	if !k.settings::<AudioSettings>().show_meter || !k.online() || !k.visible() {
		return;
	}
	let Some(state) = k.obs_state() else { return };
	let unsubscribe = state.subscribe_meters();
	let weak = std::rc::Rc::downgrade(k);
	let id = state.on_meters.on(move |levels| {
		let Some(k) = weak.upgrade() else { return };
		let now = js::now();
		if now - st(&k).meter_at < 100 {
			return;
		}
		let peak = levels.get(&k.settings::<AudioSettings>().input).map_or(f64::NAN, |l| l.peak);
		let level = js::round(meter_level(peak) * 40.0) / 40.0;
		if level == st(&k).level {
			return;
		}
		{
			let mut s = st(&k);
			s.meter_at = now;
			s.level = level;
		}
		k.render();
	});
	let weak = std::rc::Rc::downgrade(k);
	let stop: Stop = std::rc::Rc::new(move || {
		unsubscribe();
		state.on_meters.off(id);
		if let Some(k) = weak.upgrade() {
			let mut s = st(&k);
			s.stop_meter = None;
			s.level = 0.0;
		}
	});
	st(k).stop_meter = Some(stop);
}

async fn load_input(k: &KeyRc) {
	{
		let mut s = st(k);
		s.db = None;
		s.muted = false;
		s.monitor_type = None;
	}
	let input = k.settings::<AudioSettings>().input;
	if input.is_empty() {
		return;
	}
	let volume = k.call("GetInputVolume", json!({ "inputName": input }));
	let mute = k.call("GetInputMute", json!({ "inputName": input }));
	match (volume.await, mute.await) {
		(Ok(volume), Ok(mute)) => {
			let mut s = st(k);
			s.db = Some(read_db(&volume));
			s.muted = mute["inputMuted"] == true;
		}
		_ => st(k).db = None,
	}
	// Monitor Set also needs how the input is monitored.
	if k.kind.uuid == MONITOR_SET.uuid {
		let monitor = k.call("GetInputAudioMonitorType", json!({ "inputName": input })).await;
		st(k).monitor_type = monitor.ok().and_then(|r| r.get("monitorType").and_then(Value::as_str).map(String::from));
	}
}

async fn on_connected(k: &KeyRc) -> Res<()> {
	load_input(k).await;
	start_meter(k);
	Ok(())
}

async fn on_settings_changed(k: &KeyRc, old: &Map<String, Value>) -> Res<()> {
	let (old, s): (AudioSettings, AudioSettings) = (read(old), k.settings());
	let input_changed = old.input != s.input;
	if input_changed && k.online() {
		load_input(k).await;
	}
	if input_changed || old.show_meter != s.show_meter {
		start_meter(k);
	}
	Ok(())
}

fn on_obs_event(k: &KeyRc, kind: &str, data: &Value) {
	if !same_name(data, "inputName", &k.settings::<AudioSettings>().input) {
		return;
	}
	match kind {
		"InputVolumeChanged" => st(k).db = Some(read_db(data)),
		"InputMuteStateChanged" => st(k).muted = data["inputMuted"] == true,
		"InputAudioMonitorTypeChanged" => st(k).monitor_type = data.get("monitorType").and_then(Value::as_str).map(String::from),
		_ => return,
	}
	k.render();
}

fn require_input(s: &AudioSettings) -> Res<&str> {
	if s.input.is_empty() {
		return Err("no audio source selected".into());
	}
	Ok(&s.input)
}

async fn change_volume(k: &KeyRc, direction: f64, step: f64) -> Res<()> {
	let s: AudioSettings = k.settings();
	let input = require_input(&s)?;
	if st(k).db.is_none() {
		load_input(k).await;
	}
	let db = st(k).db.unwrap_or(f64::NEG_INFINITY);
	let next = step_volume(db, &s.unit, settings::or(step, 1.0), direction, s.allow_boost);
	st(k).db = Some(next);
	k.call("SetInputVolume", volume_request(input, next)).await?;
	k.render();
	Ok(())
}

/// To a value in the unit of the key.
async fn set_volume(k: &KeyRc, value: f64) -> Res<()> {
	let s: AudioSettings = k.settings();
	let input = require_input(&s)?;
	let db = if s.unit == "percent" {
		clamp_db(fader_to_db(js::max(0.0, js::min(100.0, value)) / 100.0), s.allow_boost)
	} else {
		clamp_db(value, s.allow_boost)
	};
	st(k).db = Some(db);
	k.call("SetInputVolume", volume_request(input, db)).await?;
	k.render();
	Ok(())
}

async fn toggle_mute(k: &KeyRc) -> Res<()> {
	let s: AudioSettings = k.settings();
	let result = k.call("ToggleInputMute", json!({ "inputName": require_input(&s)? })).await?;
	st(k).muted = result["inputMuted"] == true;
	k.render();
	Ok(())
}

// ----- Source Volume: up, down, to a fixed value, or mute. Hold keeps going by default. -----

struct SourceVolume;

#[async_trait(?Send)]
impl Behavior for SourceVolume {
	async fn on_connected(&self, k: &KeyRc) -> Res<()> {
		on_connected(k).await
	}

	fn on_disconnected(&self, k: &KeyRc) {
		stop_meter(k);
	}

	fn on_disappear(&self, k: &KeyRc) {
		stop_meter(k);
	}

	async fn on_settings_changed(&self, k: &KeyRc, old: &Map<String, Value>) -> Res<()> {
		on_settings_changed(k, old).await
	}

	async fn on_obs_event(&self, k: &KeyRc, kind: &str, data: &Value) -> Res<()> {
		on_obs_event(k, kind, data);
		Ok(())
	}

	fn render_offline(&self, k: &KeyRc) {
		let s: AudioSettings = k.settings();
		k.set_image(&glyph_key(volume_glyph(&s.mode).unwrap_or("volume"), &GlyphKey::default()));
	}

	// Like the volume keys of the Feishin plugin: the value is the title.
	fn render(&self, k: &KeyRc) {
		let s: AudioSettings = k.settings();
		let input = !s.input.is_empty();
		let (db, muted, level) = {
			let st = st(k);
			(st.db, st.muted, st.level)
		};
		let meter = if s.show_meter && input { Some(if muted { 0.0 } else { level }) } else { None };
		let value = match db {
			None => "?".to_string(),
			Some(_) if muted => t("Muted"),
			Some(db) => format_volume(db, &s.unit),
		};
		let title = if s.show_value && input { value } else { String::new() };
		k.own_title(&title);
		let glyph = if muted { "mute" } else { volume_glyph(&s.mode).unwrap_or("volume") };
		k.set_image(&glyph_key(
			glyph,
			&GlyphKey { color: if muted { colors::RED } else { colors::WHITE }, meter, title: &title, title_at: &s.value_at, setup: false },
		));
	}

	async fn on_press(&self, k: &KeyRc, press: Press) -> Res<()> {
		let s: AudioSettings = k.settings();
		if press.op == "mute" {
			if !press.repeat {
				toggle_mute(k).await?;
			}
		} else if press.op == "set" {
			if !press.repeat {
				set_volume(k, s.value).await?;
			}
		} else {
			change_volume(k, if press.op == "down" { -1.0 } else { 1.0 }, s.step).await?;
		}
		Ok(())
	}
}

pub static SOURCE_VOLUME: Kind = Kind {
	uuid: "de.felitendo.obs-tools.sourcevolume",
	name: "SourceVolume",
	defaults: || {
		defaults(
			json!({ "input": "", "mode": "up", "hold": "repeat", "double": "none", "unit": "db", "step": 2, "value": -10, "allowBoost": false, "showValue": true, "valueAt": "below", "showMeter": true }),
		)
	},
	press_key: "mode",
	ops: &["up", "down", "set", "mute"],
	can_repeat: true,
	required: &["input"],
	behavior: &SourceVolume,
	new_state,
	..Kind::BASE
};

// ----- Mute Toggle -----

struct MuteToggle;

async fn set_muted(k: &KeyRc, muted: bool) -> Res<()> {
	let input = k.settings::<AudioSettings>().input;
	k.call("SetInputMute", json!({ "inputName": input, "inputMuted": muted })).await?;
	st(k).muted = muted;
	k.render();
	Ok(())
}

#[async_trait(?Send)]
impl Behavior for MuteToggle {
	async fn on_connected(&self, k: &KeyRc) -> Res<()> {
		on_connected(k).await
	}

	fn on_disconnected(&self, k: &KeyRc) {
		stop_meter(k);
	}

	fn on_disappear(&self, k: &KeyRc) {
		stop_meter(k);
	}

	async fn on_settings_changed(&self, k: &KeyRc, old: &Map<String, Value>) -> Res<()> {
		on_settings_changed(k, old).await
	}

	async fn on_obs_event(&self, k: &KeyRc, kind: &str, data: &Value) -> Res<()> {
		on_obs_event(k, kind, data);
		Ok(())
	}

	fn render(&self, k: &KeyRc) {
		let s: AudioSettings = k.settings();
		let muted = st(k).muted;
		k.set_state(if muted { 1 } else { 0 });
		k.glyph_for_title(if s.show_name { &s.input } else { "" });
	}

	// talk: on only while the key is held (push to talk). holdMute: off only while held.
	async fn on_press(&self, k: &KeyRc, press: Press) -> Res<()> {
		require_input(&k.settings())?;
		match press.op.as_str() {
			"toggle" => toggle_mute(k).await,
			"talk" => set_muted(k, false).await,
			"holdMute" => set_muted(k, true).await,
			op => set_muted(k, op == "mute").await,
		}
	}

	async fn on_release(&self, k: &KeyRc) -> Res<()> {
		let s: AudioSettings = k.settings();
		if s.input.is_empty() || k.is_in_multi_action() {
			return Ok(());
		}
		match s.mode.as_str() {
			"talk" => set_muted(k, true).await,
			"holdMute" => set_muted(k, false).await,
			_ => Ok(()),
		}
	}
}

pub static MUTE_TOGGLE: Kind = Kind {
	uuid: "de.felitendo.obs-tools.mutetoggle",
	name: "MuteToggle",
	defaults: || defaults(json!({ "input": "", "mode": "toggle", "hold": "none", "double": "none", "showName": false })),
	press_key: "mode",
	ops: &["toggle", "mute", "unmute", "talk", "holdMute"],
	press_only: &["talk", "holdMute"],
	required: &["input"],
	behavior: &MuteToggle,
	new_state,
	..Kind::BASE
};

// ----- Monitor Set -----
// set: the chosen monitoring (press again: off, if wanted). off: monitoring off.

struct MonitorSet;

fn monitor_target(s: &AudioSettings) -> &'static str {
	monitor_type(&s.monitor_type).unwrap_or("OBS_MONITORING_TYPE_MONITOR_AND_OUTPUT")
}

#[async_trait(?Send)]
impl Behavior for MonitorSet {
	async fn on_connected(&self, k: &KeyRc) -> Res<()> {
		on_connected(k).await
	}

	fn on_disconnected(&self, k: &KeyRc) {
		stop_meter(k);
	}

	fn on_disappear(&self, k: &KeyRc) {
		stop_meter(k);
	}

	async fn on_settings_changed(&self, k: &KeyRc, old: &Map<String, Value>) -> Res<()> {
		on_settings_changed(k, old).await
	}

	async fn on_obs_event(&self, k: &KeyRc, kind: &str, data: &Value) -> Res<()> {
		on_obs_event(k, kind, data);
		Ok(())
	}

	fn render(&self, k: &KeyRc) {
		let s: AudioSettings = k.settings();
		let on = st(k).monitor_type.as_deref() == Some(monitor_target(&s));
		k.set_state(if on { 1 } else { 0 });
		k.glyph_for_title(if s.show_name { &s.input } else { "" });
	}

	async fn on_press(&self, k: &KeyRc, press: Press) -> Res<()> {
		let s: AudioSettings = k.settings();
		let input = require_input(&s)?;
		let target = monitor_target(&s);
		let current = st(k).monitor_type.clone();
		let off = press.op == "off" || (s.press_again_off && current.as_deref() == Some(target) && !k.is_in_multi_action());
		let next = if off { "OBS_MONITORING_TYPE_NONE" } else { target };
		k.call("SetInputAudioMonitorType", json!({ "inputName": input, "monitorType": next })).await?;
		st(k).monitor_type = Some(next.to_string());
		k.render();
		Ok(())
	}
}

pub static MONITOR_SET: Kind = Kind {
	uuid: "de.felitendo.obs-tools.monitorset",
	name: "MonitorSet",
	defaults: || {
		defaults(
			json!({ "input": "", "press": "set", "hold": "none", "double": "none", "monitorType": "monitorAndOutput", "pressAgainOff": true, "showName": false }),
		)
	},
	press_key: "press",
	ops: &["set", "off"],
	required: &["input"],
	behavior: &MonitorSet,
	new_state,
	..Kind::BASE
};

// ----- Volume dial: turn for volume, press or touch to mute (or what pressAction says). -----

struct InputVolume;

#[async_trait(?Send)]
impl Behavior for InputVolume {
	fn layout(&self, k: &KeyRc) -> Option<&'static str> {
		Some(if k.settings::<AudioSettings>().show_meter { "layouts/volume-meter.json" } else { "layouts/dial.json" })
	}

	async fn on_connected(&self, k: &KeyRc) -> Res<()> {
		on_connected(k).await
	}

	fn on_disconnected(&self, k: &KeyRc) {
		stop_meter(k);
	}

	fn on_disappear(&self, k: &KeyRc) {
		stop_meter(k);
	}

	async fn on_settings_changed(&self, k: &KeyRc, old: &Map<String, Value>) -> Res<()> {
		if read::<AudioSettings>(old).show_meter != k.settings::<AudioSettings>().show_meter {
			k.set_layout();
		}
		on_settings_changed(k, old).await
	}

	async fn on_obs_event(&self, k: &KeyRc, kind: &str, data: &Value) -> Res<()> {
		on_obs_event(k, kind, data);
		Ok(())
	}

	// Without OBS or settings: the name, the status, and the setup dot on the icon.
	fn render_offline(&self, k: &KeyRc) {
		let s: AudioSettings = k.settings();
		let title = if s.input.is_empty() { t("Volume") } else { s.input };
		let mut feedback =
			json!({ "title": title, "icon": dial_icon("volume", !k.is_set_up()), "value": k.dial_status(), "indicator": { "value": 0 } });
		if s.show_meter {
			feedback["meter"] = json!({ "value": 0 });
		}
		k.set_feedback(feedback);
	}

	fn render(&self, k: &KeyRc) {
		if !k.is_set_up() {
			return self.render_offline(k);
		}
		let s: AudioSettings = k.settings();
		let (db, muted, level) = {
			let st = st(k);
			(st.db, st.muted, st.level)
		};
		let value = match db {
			None => "?".to_string(),
			Some(_) if muted => t("Muted"),
			Some(db) => format_volume(db, &s.unit),
		};
		let position = db.map_or(0.0, |db| js::round(db_to_fader(db) * 100.0));
		let mut feedback = json!({
			"title": s.input,
			"icon": dial_icon("volume", false),
			"value": { "value": value, "color": if muted { colors::RED } else { colors::WHITE } },
			"indicator": { "value": js::num(position), "bar_fill_c": if muted { colors::GRAY } else { colors::GREEN } },
		});
		if s.show_meter {
			feedback["meter"] = json!({ "value": js::num(if muted { 0.0 } else { js::round(level * 100.0) }) });
		}
		k.set_feedback(feedback);
	}

	async fn on_dial_rotate(&self, k: &KeyRc, ticks: f64, _pressed: bool) -> Res<()> {
		let step = settings::or(k.settings::<AudioSettings>().step, 1.0);
		change_volume(k, ticks.signum() * if ticks == 0.0 { 0.0 } else { 1.0 }, ticks.abs() * step).await
	}

	async fn on_dial_down(&self, k: &KeyRc) -> Res<()> {
		let s: AudioSettings = k.settings();
		if s.press_action == "reset" {
			set_volume(k, if s.unit == "percent" { 100.0 } else { 0.0 }).await
		} else if s.press_action != "nothing" {
			toggle_mute(k).await
		} else {
			Ok(())
		}
	}
}

pub static INPUT_VOLUME: Kind = Kind {
	uuid: "de.felitendo.obs-tools.inputvolume",
	name: "InputVolume",
	defaults: || {
		defaults(
			json!({ "input": "", "unit": "db", "step": 1, "allowBoost": false, "pressAction": "mute", "disableTouch": false, "showMeter": true }),
		)
	},
	required: &["input"],
	behavior: &InputVolume,
	new_state,
	..Kind::BASE
};

#[cfg(test)]
mod tests {
	use base64::Engine;

	use super::super::testing::*;
	use super::*;
	use crate::actions::Gesture;
	use crate::settings::testing::assert_defaults;

	#[test]
	fn every_setting_has_a_default() {
		assert_defaults::<AudioSettings>(&[&SOURCE_VOLUME, &MUTE_TOGGLE, &MONITOR_SET, &INPUT_VOLUME]);
	}

	#[tokio::test]
	async fn a_failed_press_is_kept_for_the_settings_page_until_a_press_works() {
		let (_plugin, k) = key(&MUTE_TOGGLE, json!({ "input": "" }));
		k.act(k.press(Gesture::Press, false)).await;
		assert_eq!(k.last_error().as_deref(), Some("no audio source selected"));
		set(&k, "input", json!("Mic"));
		k.act(k.press(Gesture::Press, false)).await;
		assert_eq!(k.last_error(), None);
	}

	#[test]
	fn a_key_that_is_not_set_up_shows_a_gray_glyph_with_a_yellow_dot() {
		let (plugin, k) = key(&MUTE_TOGGLE, json!({ "input": "" }));
		assert!(!k.is_set_up());
		k.draw();
		let images = sent(&plugin, "setImage");
		let last = images.last().and_then(|p| p["image"].as_str()).unwrap_or("").to_string();
		let svg =
			String::from_utf8(base64::engine::general_purpose::STANDARD.decode(last.split(',').nth(1).unwrap_or("")).unwrap_or_default())
				.unwrap_or_default();
		assert!(svg.contains("#F5A524"), "yellow dot");
		// Normal images wait until it is set up.
		k.set_image("data:other");
		assert_ne!(sent(&plugin, "setImage").last().and_then(|p| p["image"].as_str()), Some("data:other"));
		set(&k, "input", json!("Mic"));
		k.draw();
		assert!(!setup_shown(&k));
	}

	#[tokio::test]
	async fn push_to_talk_on_while_held_off_again_on_release() {
		for (mode, expected) in [("talk", [false, true]), ("holdMute", [true, false])] {
			let (_plugin, k) = key(&MUTE_TOGGLE, json!({ "input": "Mic", "mode": mode }));
			k.press(Gesture::Press, false).await.unwrap_or_default();
			MuteToggle.on_release(&k).await.unwrap_or_default();
			let muted: Vec<bool> = calls(&k).iter().map(|(_, d)| d["inputMuted"].as_bool().unwrap_or(false)).collect();
			assert_eq!(muted, expected, "{mode}");
		}
	}

	#[test]
	fn the_source_name_is_drawn_under_the_glyph_not_as_an_opendeck_title() {
		let (plugin, k) = key(&MUTE_TOGGLE, json!({ "input": "Mic", "showName": true }));
		MuteToggle.render(&k);
		assert_eq!(sent(&plugin, "setTitle").last().map(|p| p["title"].clone()), Some(json!("")), "no second title from OpenDeck");
		let image = sent(&plugin, "setImage").last().and_then(|p| p["image"].as_str().map(String::from)).unwrap_or_default();
		let svg =
			String::from_utf8(base64::engine::general_purpose::STANDARD.decode(image.split(',').nth(1).unwrap_or("")).unwrap_or_default())
				.unwrap_or_default();
		assert!(svg.contains(">Mic</text>"));
	}
}
