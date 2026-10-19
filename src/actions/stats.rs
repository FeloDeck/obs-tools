// Stats: OBS CPU usage, dropped frames and stream health.
use std::rc::Rc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{Behavior, KeyRc, Kind, Press, Stop, defaults};
use crate::Res;
use crate::i18n::{num, t, t_with};
use crate::icons::colors;
use crate::js;
use crate::render::{StatKey, is_color, stat_key};
use crate::settings::{or, read};
use crate::time::{format_duration, timecode_seconds};

/// Every stats key: polls the stats of OBS while it is shown.
#[derive(Default)]
struct StatsSubscription {
	unsubscribe: Option<Stop>,
}

trait StatsKey {
	fn sub(k: &KeyRc) -> std::cell::RefMut<'_, StatsSubscription>;
	fn on_stats(k: &KeyRc, stats: &Value);
}

fn subscribe<S: StatsKey + 'static>(k: &KeyRc) {
	unsubscribe::<S>(k);
	let Some(state) = k.obs_state() else { return };
	let off = state.subscribe_stats();
	let weak = Rc::downgrade(k);
	let id = state.on_stats.on(move |stats| {
		if let Some(k) = weak.upgrade() {
			S::on_stats(&k, stats);
		}
	});
	let listening = state.clone();
	S::sub(k).unsubscribe = Some(Rc::new(move || {
		off();
		listening.on_stats.off(id);
	}));
	if let Some(stats) = state.stats() {
		S::on_stats(k, &stats);
	}
}

fn unsubscribe<S: StatsKey>(k: &KeyRc) {
	let stop = S::sub(k).unsubscribe.take();
	if let Some(stop) = stop {
		stop();
	}
}

// ----- OBS CPU Usage -----

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct CpuSettings {
	/// Percent from which the key turns red, 0 for never.
	warn_at: f64,
	show_memory: bool,
}

#[derive(Default)]
struct CpuState {
	sub: StatsSubscription,
	stats: Option<Value>,
}

struct CpuUsage;

impl StatsKey for CpuUsage {
	fn sub(k: &KeyRc) -> std::cell::RefMut<'_, StatsSubscription> {
		std::cell::RefMut::map(k.own::<CpuState>(), |s| &mut s.sub)
	}

	fn on_stats(k: &KeyRc, stats: &Value) {
		k.own::<CpuState>().stats = Some(stats.clone());
		k.render();
	}
}

#[async_trait(?Send)]
impl Behavior for CpuUsage {
	async fn on_connected(&self, k: &KeyRc) -> Res<()> {
		subscribe::<CpuUsage>(k);
		Ok(())
	}

	fn on_disconnected(&self, k: &KeyRc) {
		unsubscribe::<CpuUsage>(k);
	}

	fn on_disappear(&self, k: &KeyRc) {
		unsubscribe::<CpuUsage>(k);
	}

	fn render(&self, k: &KeyRc) {
		let Some(stats) = k.own::<CpuState>().stats.clone() else { return };
		let s: CpuSettings = k.settings();
		let cpu = stats["cpuUsage"].as_f64().unwrap_or(0.0);
		let warn = s.warn_at > 0.0 && cpu >= s.warn_at;
		let value = format!("{}%", num(cpu, if cpu >= 100.0 { 0 } else { 1 }));
		let sub =
			if s.show_memory { format!("{} MB", js::fmt(js::round(stats["memoryUsage"].as_f64().unwrap_or(0.0)))) } else { String::new() };
		let label = t("OBS CPU");
		k.set_image(&stat_key(&StatKey {
			label: &label,
			value: &value,
			sub: &sub,
			bar: Some(js::min(1.0, cpu / 100.0)),
			color: if warn { colors::RED } else { colors::BLUE },
			alert: if warn { Some(colors::RED) } else { None },
			dot: None,
		}));
	}
}

pub static CPU_USAGE: Kind = Kind {
	uuid: "de.felitendo.obs-tools.cpuusage",
	name: "CpuUsage",
	defaults: || defaults(json!({ "warnAt": 50, "showMemory": false })),
	behavior: &CpuUsage,
	new_state: || Box::new(CpuState::default()),
	..Kind::BASE
};

// ----- Dropped Frames Alarm: shows the frame count and flashes when it goes up. -----

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct DroppedSettings {
	/// "dropped" (network), "skipped" (encoder) or "missed" (render).
	frame_type: String,
	/// How many more frames start the alarm.
	threshold: f64,
	alert_color: String,
	alert_seconds: f64,
}

/// The count and the total, or nothing (not live).
type Frames = Option<(Option<f64>, Option<f64>)>;

struct FrameType {
	label: &'static str,
	read: fn(&Value) -> Frames,
}

fn frame_type(name: &str) -> &'static FrameType {
	static DROPPED: FrameType = FrameType {
		label: "DROPPED",
		read: |s| {
			let stream = &s["stream"];
			if stream["outputActive"] == true {
				Some((stream["outputSkippedFrames"].as_f64(), stream["outputTotalFrames"].as_f64()))
			} else {
				None
			}
		},
	};
	static SKIPPED: FrameType =
		FrameType { label: "SKIPPED", read: |s| Some((s["outputSkippedFrames"].as_f64(), s["outputTotalFrames"].as_f64())) };
	static MISSED: FrameType =
		FrameType { label: "MISSED", read: |s| Some((s["renderSkippedFrames"].as_f64(), s["renderTotalFrames"].as_f64())) };
	match name {
		"skipped" => &SKIPPED,
		"missed" => &MISSED,
		_ => &DROPPED,
	}
}

#[derive(Default)]
struct DroppedState {
	sub: StatsSubscription,
	count: Option<f64>,
	total: Option<f64>,
	baseline: Option<f64>,
	alert_until: f64,
	flash: bool,
}

struct DroppedFrames;

fn dropped(k: &KeyRc) -> std::cell::RefMut<'_, DroppedState> {
	k.own::<DroppedState>()
}

impl StatsKey for DroppedFrames {
	fn sub(k: &KeyRc) -> std::cell::RefMut<'_, StatsSubscription> {
		std::cell::RefMut::map(dropped(k), |s| &mut s.sub)
	}

	fn on_stats(k: &KeyRc, stats: &Value) {
		let settings: DroppedSettings = k.settings();
		let kind = frame_type(&settings.frame_type);
		let threshold = js::max(1.0, or(settings.threshold, 1.0));
		let alert_seconds = js::max(1.0, or(settings.alert_seconds, 10.0));
		{
			let mut s = dropped(k);
			match (kind.read)(stats) {
				None => {
					s.count = None;
					s.baseline = None;
					s.alert_until = 0.0;
				}
				Some((count, total)) => {
					let below = matches!((count, s.baseline), (Some(c), Some(b)) if c < b);
					if s.baseline.is_none() || below {
						s.baseline = count;
					}
					if let (Some(c), Some(b)) = (count, s.baseline)
						&& c - b >= threshold
					{
						s.alert_until = js::now() as f64 + alert_seconds * 1000.0;
						s.baseline = count;
					}
					s.count = count;
					s.total = total;
				}
			}
			s.flash = !s.flash;
		}
		k.render();
	}
}

#[async_trait(?Send)]
impl Behavior for DroppedFrames {
	async fn on_connected(&self, k: &KeyRc) -> Res<()> {
		subscribe::<DroppedFrames>(k);
		Ok(())
	}

	fn on_disconnected(&self, k: &KeyRc) {
		unsubscribe::<DroppedFrames>(k);
	}

	fn on_disappear(&self, k: &KeyRc) {
		unsubscribe::<DroppedFrames>(k);
	}

	async fn on_settings_changed(&self, k: &KeyRc, old: &Map<String, Value>) -> Res<()> {
		if read::<DroppedSettings>(old).frame_type != k.settings::<DroppedSettings>().frame_type {
			let mut s = dropped(k);
			s.baseline = None;
			s.alert_until = 0.0;
		}
		Ok(())
	}

	fn render(&self, k: &KeyRc) {
		let s: DroppedSettings = k.settings();
		let kind = frame_type(&s.frame_type);
		let (count, total, alert_until, flash) = {
			let d = dropped(k);
			(d.count, d.total, d.alert_until, d.flash)
		};
		let alerting = alert_until > js::now() as f64;
		let color = if is_color(&s.alert_color) { s.alert_color.as_str() } else { colors::RED };
		let label = t(kind.label);
		let Some(count) = count else {
			let sub = if s.frame_type == "dropped" { t("not live") } else { String::new() };
			k.set_image(&stat_key(&StatKey { label: &label, value: "-", sub: &sub, ..Default::default() }));
			return;
		};
		let total = total.unwrap_or(f64::NAN);
		let pct = if total > 0.0 { (count / total) * 100.0 } else { 0.0 };
		let value = js::fmt(count);
		let sub = format!("{}%", num(pct, if pct >= 10.0 { 0 } else { 1 }));
		k.set_image(&stat_key(&StatKey {
			label: &label,
			value: &value,
			sub: &sub,
			alert: if alerting && flash { Some(color) } else { None },
			..Default::default()
		}));
	}

	// Press: stop the alarm and count from here.
	async fn on_press(&self, k: &KeyRc, _press: Press) -> Res<()> {
		{
			let mut s = dropped(k);
			s.alert_until = 0.0;
			s.baseline = s.count;
		}
		self.render(k);
		Ok(())
	}
}

pub static DROPPED_FRAMES: Kind = Kind {
	uuid: "de.felitendo.obs-tools.droppedframes",
	name: "DroppedFrames",
	defaults: || defaults(json!({ "frameType": "dropped", "threshold": 1, "alertColor": colors::RED, "alertSeconds": 10 })),
	behavior: &DroppedFrames,
	new_state: || Box::new(DroppedState::default()),
	..Kind::BASE
};

// ----- Stream Health: bitrate (or time), recent drops, and a green, yellow or red light. -----

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct HealthSettings {
	/// "bitrate" or "time".
	show: String,
	/// Percent of dropped frames for yellow and for red.
	warn_drops: f64,
	bad_drops: f64,
}

fn format_bitrate(kbps: Option<f64>) -> String {
	match kbps {
		Some(k) if k.is_finite() => {
			if k >= 1000.0 {
				format!("{} Mb/s", num(k / 1000.0, 1))
			} else {
				format!("{} kb/s", js::fmt(js::round(k)))
			}
		}
		_ => "...".into(),
	}
}

struct Health {
	level: &'static str,
	drops: f64,
	kbps: Option<f64>,
	timecode: String,
	reconnecting: bool,
}

#[derive(Default)]
struct HealthState {
	sub: StatsSubscription,
	samples: Vec<(f64, f64)>,
	health: Option<Health>,
}

struct StreamHealth;

fn health(k: &KeyRc) -> std::cell::RefMut<'_, HealthState> {
	k.own::<HealthState>()
}

impl StatsKey for StreamHealth {
	fn sub(k: &KeyRc) -> std::cell::RefMut<'_, StatsSubscription> {
		std::cell::RefMut::map(health(k), |s| &mut s.sub)
	}

	fn on_stats(k: &KeyRc, stats: &Value) {
		let settings: HealthSettings = k.settings();
		let (warn, bad) = (or(settings.warn_drops, 1.0), or(settings.bad_drops, 5.0));
		{
			let stream = &stats["stream"];
			let number = |key: &str| stream[key].as_f64().unwrap_or(0.0);
			let mut s = health(k);
			if stream["outputActive"] != true {
				s.samples.clear();
				s.health = None;
			} else {
				// Drops of the last 10 seconds, so an old problem does not keep the key red.
				s.samples.push((number("outputSkippedFrames"), number("outputTotalFrames")));
				if s.samples.len() > 11 {
					s.samples.remove(0);
				}
				let (first, last) = (s.samples[0], s.samples[s.samples.len() - 1]);
				let frames = last.1 - first.1;
				let drops = if frames > 0.0 { ((last.0 - first.0) / frames) * 100.0 } else { 0.0 };
				let congestion = number("outputCongestion");
				let reconnecting = stream["outputReconnecting"] == true;
				let level = if reconnecting || drops >= bad || congestion >= 0.5 {
					"bad"
				} else if drops >= warn || congestion >= 0.15 {
					"warn"
				} else {
					"good"
				};
				s.health = Some(Health {
					level,
					drops: js::max(0.0, drops),
					kbps: stream["kbps"].as_f64(),
					timecode: stream["outputTimecode"].as_str().unwrap_or_default().to_string(),
					reconnecting,
				});
			}
		}
		k.render();
	}
}

#[async_trait(?Send)]
impl Behavior for StreamHealth {
	async fn on_connected(&self, k: &KeyRc) -> Res<()> {
		subscribe::<StreamHealth>(k);
		Ok(())
	}

	fn on_disconnected(&self, k: &KeyRc) {
		unsubscribe::<StreamHealth>(k);
	}

	fn on_disappear(&self, k: &KeyRc) {
		unsubscribe::<StreamHealth>(k);
	}

	fn render(&self, k: &KeyRc) {
		let show_time = k.settings::<HealthSettings>().show == "time";
		let (label, value, sub, dot) = {
			let s = health(k);
			let Some(h) = &s.health else {
				drop(s);
				let (label, value) = (t("STREAM"), t("off"));
				k.set_image(&stat_key(&StatKey { label: &label, value: &value, dot: Some(colors::GRAY), ..Default::default() }));
				return;
			};
			let dot = match h.level {
				"good" => colors::GREEN,
				"warn" => colors::AMBER,
				_ => colors::RED,
			};
			let value = if h.reconnecting {
				"...".to_string()
			} else if show_time {
				format_duration(timecode_seconds(&h.timecode))
			} else {
				format_bitrate(h.kbps)
			};
			let sub = if h.reconnecting {
				t("reconnecting")
			} else if show_time {
				format_bitrate(h.kbps)
			} else {
				t_with("drops {pct}%", &[("pct", &num(h.drops, if h.drops >= 10.0 { 0 } else { 1 }))])
			};
			(t("LIVE"), value, sub, dot)
		};
		k.set_image(&stat_key(&StatKey { label: &label, value: &value, sub: &sub, dot: Some(dot), ..Default::default() }));
	}
}

pub static STREAM_HEALTH: Kind = Kind {
	uuid: "de.felitendo.obs-tools.streamhealth",
	name: "StreamHealth",
	defaults: || defaults(json!({ "show": "bitrate", "warnDrops": 1, "badDrops": 5 })),
	behavior: &StreamHealth,
	new_state: || Box::new(HealthState::default()),
	..Kind::BASE
};

#[cfg(test)]
mod tests {
	use super::*;
	use crate::settings::testing::assert_defaults;

	#[test]
	fn every_setting_has_a_default() {
		assert_defaults::<CpuSettings>(&[&CPU_USAGE]);
		assert_defaults::<DroppedSettings>(&[&DROPPED_FRAMES]);
		assert_defaults::<HealthSettings>(&[&STREAM_HEALTH]);
	}
}
