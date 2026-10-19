use async_trait::async_trait;
use serde_json::{Map, Value, json};

use super::{Behavior, ID, KeyRc, Kind, PressArgs, Stop, defaults};
use crate::Res;
use crate::i18n::t;
use crate::icons::{colors, volume_glyph};
use crate::js::{self, truthy};
use crate::render::{GlyphKey, dial_icon, glyph_key, meter_level};
use crate::volume::{clamp_db, db_to_fader, fader_to_db, format_volume, read_db, step_volume, volume_request};

fn monitor_type(name: &str) -> Option<&'static str> {
	match name {
		"none" => Some("OBS_MONITORING_TYPE_NONE"),
		"monitorOnly" => Some("OBS_MONITORING_TYPE_MONITOR_ONLY"),
		"monitorAndOutput" => Some("OBS_MONITORING_TYPE_MONITOR_AND_OUTPUT"),
		_ => None,
	}
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
	k.x::<InputState>()
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
	if !k.s_truthy("showMeter") || !k.online() || !k.visible() {
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
		let peak = levels.get(&k.s_str("input")).map_or(f64::NAN, |l| l.peak);
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
	let input = k.s_val("input");
	if !truthy(Some(&input)) {
		return;
	}
	let volume = k.call("GetInputVolume", json!({ "inputName": input }));
	let mute = k.call("GetInputMute", json!({ "inputName": input }));
	match (volume.await, mute.await) {
		(Ok(volume), Ok(mute)) => {
			let mut s = st(k);
			s.db = Some(read_db(&volume));
			s.muted = truthy(mute.get("inputMuted"));
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
	let input_changed = old.get("input") != k.s("input").as_ref();
	if input_changed && k.online() {
		load_input(k).await;
	}
	if input_changed || old.get("showMeter") != k.s("showMeter").as_ref() {
		start_meter(k);
	}
	Ok(())
}

fn on_obs_event(k: &KeyRc, kind: &str, data: &Value) {
	if data.get("inputName") != k.s("input").as_ref() {
		return;
	}
	match kind {
		"InputVolumeChanged" => st(k).db = Some(read_db(data)),
		"InputMuteStateChanged" => st(k).muted = truthy(data.get("inputMuted")),
		"InputAudioMonitorTypeChanged" => st(k).monitor_type = data.get("monitorType").and_then(Value::as_str).map(String::from),
		_ => return,
	}
	k.render();
}

fn require_input(k: &KeyRc) -> Res<Value> {
	let input = k.s_val("input");
	if !truthy(Some(&input)) {
		return Err("no audio source selected".into());
	}
	Ok(input)
}

async fn change_volume(k: &KeyRc, direction: f64, step: f64) -> Res<()> {
	let input = require_input(k)?;
	if st(k).db.is_none() {
		load_input(k).await;
	}
	let db = st(k).db.unwrap_or(f64::NEG_INFINITY);
	let next = step_volume(db, &k.s_str("unit"), if js::truthy_num(step) { step } else { 1.0 }, direction, k.s_truthy("allowBoost"));
	st(k).db = Some(next);
	k.call("SetInputVolume", volume_request(&input, next)).await?;
	k.render();
	Ok(())
}

async fn set_volume(k: &KeyRc, value: Option<&Value>) -> Res<()> {
	let input = require_input(k)?;
	let number = js::number(value);
	let boost = k.s_truthy("allowBoost");
	let db = if k.s_str("unit") == "percent" {
		clamp_db(fader_to_db(js::max(0.0, js::min(100.0, number)) / 100.0), boost)
	} else {
		clamp_db(number, boost)
	};
	st(k).db = Some(db);
	k.call("SetInputVolume", volume_request(&input, db)).await?;
	k.render();
	Ok(())
}

async fn toggle_mute(k: &KeyRc) -> Res<()> {
	let input = require_input(k)?;
	let result = k.call("ToggleInputMute", json!({ "inputName": input })).await?;
	st(k).muted = truthy(result.get("inputMuted"));
	k.render();
	Ok(())
}

// ----- Source Volume: up, down, to a fixed value, or mute. Hold keeps going by default. -----

struct SourceVolume;

fn volume_glyph_name(k: &KeyRc) -> &'static str {
	volume_glyph(&k.s_str("mode")).unwrap_or("volume")
}

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
		k.set_image(&glyph_key(volume_glyph_name(k), &GlyphKey::default()));
	}

	// Like the volume keys of the Feishin plugin: the value is the title.
	fn render(&self, k: &KeyRc) {
		let input = k.s_truthy("input");
		let (db, muted, level) = {
			let s = st(k);
			(s.db, s.muted, s.level)
		};
		let meter = if k.s_truthy("showMeter") && input { Some(if muted { 0.0 } else { level }) } else { None };
		let value = match db {
			None => "?".to_string(),
			Some(_) if muted => t("Muted"),
			Some(db) => format_volume(db, &k.s_str("unit")),
		};
		let title = if k.s_truthy("showValue") && input { value } else { String::new() };
		k.own_title(&title);
		let at = k.s_str("valueAt");
		let glyph = if muted { "mute" } else { volume_glyph_name(k) };
		k.set_image(&glyph_key(
			glyph,
			&GlyphKey { color: if muted { colors::RED } else { colors::WHITE }, meter, title: &title, title_at: &at, setup: false },
		));
	}

	async fn on_press(&self, k: &KeyRc, args: Option<PressArgs>) -> Res<()> {
		let mode = k.op(&args);
		let repeat = args.as_ref().is_some_and(|a| a.repeat);
		if mode == "mute" {
			if !repeat {
				toggle_mute(k).await?;
			}
		} else if mode == "set" {
			if !repeat {
				set_volume(k, k.s("value").as_ref()).await?;
			}
		} else {
			change_volume(k, if mode == "down" { -1.0 } else { 1.0 }, k.s_num("step")).await?;
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
	press_only: &[],
	can_repeat: true,
	required: &["input"],
	behavior: &SourceVolume,
	new_state,
};

// ----- Mute Toggle -----

struct MuteToggle;

async fn set_muted(k: &KeyRc, muted: bool) -> Res<()> {
	k.call("SetInputMute", json!({ "inputName": k.s_val("input"), "inputMuted": muted })).await?;
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
		let muted = st(k).muted;
		k.set_state(if muted { 1 } else { 0 });
		k.show_name_title(&k.s_str("input"));
	}

	// talk: on only while the key is held (push to talk). holdMute: off only while held.
	async fn on_press(&self, k: &KeyRc, args: Option<PressArgs>) -> Res<()> {
		let mode = k.op(&args);
		require_input(k)?;
		match mode.as_str() {
			"toggle" => toggle_mute(k).await,
			"talk" => set_muted(k, false).await,
			"holdMute" => set_muted(k, true).await,
			_ => set_muted(k, mode == "mute").await,
		}
	}

	async fn on_release(&self, k: &KeyRc) -> Res<()> {
		if !k.s_truthy("input") || k.is_in_multi_action() {
			return Ok(());
		}
		match k.s_str("mode").as_str() {
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
	can_repeat: false,
	required: &["input"],
	behavior: &MuteToggle,
	new_state,
};

// ----- Monitor Set -----
// set: the chosen monitoring (press again: off, if wanted). off: monitoring off.

struct MonitorSet;

fn monitor_target(k: &KeyRc) -> &'static str {
	monitor_type(&k.s_str("monitorType")).unwrap_or("OBS_MONITORING_TYPE_MONITOR_AND_OUTPUT")
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
		let on = st(k).monitor_type.as_deref() == Some(monitor_target(k));
		k.set_state(if on { 1 } else { 0 });
		k.show_name_title(&k.s_str("input"));
	}

	async fn on_press(&self, k: &KeyRc, args: Option<PressArgs>) -> Res<()> {
		let input = require_input(k)?;
		let target = monitor_target(k);
		let current = st(k).monitor_type.clone();
		let off = k.op(&args) == "off" || (k.s_truthy("pressAgainOff") && current.as_deref() == Some(target) && !k.is_in_multi_action());
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
	press_only: &[],
	can_repeat: false,
	required: &["input"],
	behavior: &MonitorSet,
	new_state,
};

// ----- Volume dial: turn for volume, press or touch to mute (or what pressAction says). -----

struct InputVolume;

#[async_trait(?Send)]
impl Behavior for InputVolume {
	fn layout(&self, k: &KeyRc) -> Option<&'static str> {
		Some(if k.s_truthy("showMeter") { "layouts/volume-meter.json" } else { "layouts/dial.json" })
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
		if old.get("showMeter") != k.s("showMeter").as_ref() {
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
		let title = if k.s_truthy("input") { k.s_val("input") } else { Value::from(t("Volume")) };
		let mut feedback =
			json!({ "title": title, "icon": dial_icon("volume", !k.is_set_up()), "value": k.dial_status(), "indicator": { "value": 0 } });
		if k.s_truthy("showMeter") {
			feedback["meter"] = json!({ "value": 0 });
		}
		k.set_feedback(feedback);
	}

	fn render(&self, k: &KeyRc) {
		if !k.is_set_up() {
			return self.render_offline(k);
		}
		let (db, muted, level) = {
			let s = st(k);
			(s.db, s.muted, s.level)
		};
		let value = match db {
			None => "?".to_string(),
			Some(_) if muted => t("Muted"),
			Some(db) => format_volume(db, &k.s_str("unit")),
		};
		let position = db.map_or(0.0, |db| js::round(db_to_fader(db) * 100.0));
		let mut feedback = json!({
			"title": k.s_val("input"),
			"icon": dial_icon("volume", false),
			"value": { "value": value, "color": if muted { colors::RED } else { colors::WHITE } },
			"indicator": { "value": js::num(position), "bar_fill_c": if muted { colors::GRAY } else { colors::GREEN } },
		});
		if k.s_truthy("showMeter") {
			feedback["meter"] = json!({ "value": js::num(if muted { 0.0 } else { js::round(level * 100.0) }) });
		}
		k.set_feedback(feedback);
	}

	async fn on_dial_rotate(&self, k: &KeyRc, ticks: f64, _pressed: bool) -> Res<()> {
		change_volume(k, ticks.signum() * if ticks == 0.0 { 0.0 } else { 1.0 }, ticks.abs() * k.s_num_or("step", 1.0)).await
	}

	async fn on_dial_down(&self, k: &KeyRc) -> Res<()> {
		let action = k.s_str("pressAction");
		if action == "reset" {
			let full = json!(if k.s_str("unit") == "percent" { 100 } else { 0 });
			set_volume(k, Some(&full)).await
		} else if action != "nothing" {
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
	press_key: "",
	ops: &[],
	press_only: &[],
	can_repeat: false,
	required: &["input"],
	behavior: &InputVolume,
	new_state,
};

const _: &str = ID;

#[cfg(test)]
mod tests {
	use super::super::testing::*;
	use super::*;
	use base64::Engine;

	#[tokio::test]
	async fn a_failed_press_is_kept_for_the_settings_page_until_a_press_works() {
		let (_plugin, k) = key(&MUTE_TOGGLE, json!({ "input": "" }));
		k.act(MuteToggle.on_press(&k, None)).await;
		assert_eq!(k.last_error().as_deref(), Some("no audio source selected"));
		set(&k, "input", json!("Mic"));
		k.act(MuteToggle.on_press(&k, None)).await;
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
			MuteToggle.on_press(&k, None).await.unwrap_or_default();
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
