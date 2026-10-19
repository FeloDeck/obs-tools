// Keeps the OBS state that many keys need, so each key does not have to ask for it.
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use serde_json::Value;
use tokio::task::spawn_local;

use crate::emitter::Emitter;
use crate::js::{self, truthy};
use crate::obs::{BatchExecution, EVENTS_ALL, EVENTS_VOLUME_METERS, ObsClient};
use crate::timers::{Timer, set_interval};

const HISTORY_SIZE: usize = 20;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Output {
	pub active: bool,
	pub paused: bool,
	/// Replay buffer only: OBS has one.
	pub available: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Transition {
	pub name: Option<String>,
	pub duration: Option<f64>,
}

/// Loudest peak and magnitude of an input, in dB.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Level {
	pub peak: f64,
	pub magnitude: f64,
}

#[derive(Clone, Debug)]
pub struct Data {
	pub ready: bool,
	pub program_scene: Option<String>,
	pub preview_scene: Option<String>,
	pub studio_mode: bool,
	pub scenes: Vec<String>,
	pub history: Vec<String>,
	pub record: Output,
	pub stream: Output,
	pub replay: Output,
	pub virtualcam: Output,
	pub transition: Transition,
	pub fps: f64,
	pub profile: Option<String>,
	pub collection: Option<String>,
}

impl Default for Data {
	fn default() -> Self {
		Data {
			ready: false,
			program_scene: None,
			preview_scene: None,
			studio_mode: false,
			scenes: Vec::new(),
			history: Vec::new(),
			record: Output::default(),
			stream: Output::default(),
			replay: Output::default(),
			virtualcam: Output::default(),
			transition: Transition::default(),
			fps: 30.0,
			profile: None,
			collection: None,
		}
	}
}

impl Data {
	/// The scene shown before the current one that still exists.
	pub fn previous_scene(&self) -> Option<String> {
		self.history.iter().rev().find(|name| Some(*name) != self.program_scene.as_ref() && self.scenes.contains(name)).cloned()
	}
}

pub struct ObsState {
	pub client: Rc<ObsClient>,
	data: RefCell<Data>,
	last_bytes: Cell<Option<(f64, i64)>>,
	stats_users: Cell<i64>,
	stats_timer: RefCell<Option<Timer>>,
	stats: RefCell<Option<Value>>,
	meter_users: Cell<i64>,
	levels: RefCell<HashMap<String, Level>>,
	me: Weak<ObsState>,
	/// What changed: "all", "program", "preview", "studio", "scenes", "record", ...
	pub on_change: Emitter<str>,
	pub on_meters: Emitter<HashMap<String, Level>>,
	pub on_stats: Emitter<Value>,
}

fn string(v: Option<&Value>) -> Option<String> {
	match v {
		None | Some(Value::Null) => None,
		Some(v) => Some(js::to_string(Some(v))),
	}
}

fn number(v: Option<&Value>) -> Option<f64> {
	match v {
		None | Some(Value::Null) => None,
		Some(v) => Some(js::number(Some(v))),
	}
}

impl ObsState {
	pub fn new(client: Rc<ObsClient>) -> Rc<Self> {
		let state = Rc::new_cyclic(|me: &Weak<ObsState>| ObsState {
			client: client.clone(),
			data: RefCell::new(Data::default()),
			last_bytes: Cell::new(None),
			stats_users: Cell::new(0),
			stats_timer: RefCell::new(None),
			stats: RefCell::new(None),
			meter_users: Cell::new(0),
			levels: RefCell::new(HashMap::new()),
			me: me.clone(),
			on_change: Emitter::default(),
			on_meters: Emitter::default(),
			on_stats: Emitter::default(),
		});
		let me = Rc::downgrade(&state);
		client.on_connected.on({
			let me = me.clone();
			move |_| {
				if let Some(me) = me.upgrade() {
					spawn_local(async move { me.load().await });
				}
			}
		});
		client.on_disconnected.on({
			let me = me.clone();
			move |_| {
				if let Some(me) = me.upgrade() {
					me.reset();
					me.on_change.emit("all");
				}
			}
		});
		client.on_event.on(move |(kind, data)| {
			if let Some(me) = me.upgrade() {
				me.on_event(kind, data);
			}
		});
		state
	}

	/// A copy of the state.
	pub fn data(&self) -> Data {
		self.data.borrow().clone()
	}

	pub fn with<R>(&self, f: impl FnOnce(&Data) -> R) -> R {
		f(&self.data.borrow())
	}

	pub fn update(&self, f: impl FnOnce(&mut Data)) {
		f(&mut self.data.borrow_mut());
	}

	pub fn stats(&self) -> Option<Value> {
		self.stats.borrow().clone()
	}

	fn reset(&self) {
		*self.data.borrow_mut() = Data::default();
		self.last_bytes.set(None);
	}

	pub async fn load(&self) {
		let requests = [
			"GetSceneList",
			"GetStudioModeEnabled",
			"GetRecordStatus",
			"GetStreamStatus",
			"GetReplayBufferStatus",
			"GetVirtualCamStatus",
			"GetCurrentSceneTransition",
			"GetVideoSettings",
			"GetProfileList",
			"GetSceneCollectionList",
		]
		.iter()
		.map(|t| crate::obj! { "requestType" => *t })
		.collect();
		let Ok(results) = self.client.batch(requests, BatchExecution::SerialRealtime, false, None).await else { return };
		let mut data: HashMap<String, Value> = HashMap::new();
		for r in &results {
			let ok = truthy(r.get("requestStatus").and_then(|s| s.get("result")));
			let value = if ok { r.get("responseData").cloned().unwrap_or(Value::Null) } else { Value::Null };
			data.insert(js::to_string(r.get("requestType")), value);
		}
		let get = |request: &str, key: &str| data.get(request).and_then(|d| d.get(key)).cloned();
		let present = |request: &str| data.get(request).is_some_and(truthy_value);
		{
			let mut d = self.data.borrow_mut();
			if let Some(list) = data.get("GetSceneList").filter(|v| truthy_value(v)) {
				set_scenes(&mut d, list);
			}
			d.studio_mode = truthy(get("GetStudioModeEnabled", "studioModeEnabled").as_ref());
			d.preview_scene = if d.studio_mode { string(get("GetSceneList", "currentPreviewSceneName").as_ref()) } else { None };
			d.record = Output {
				active: truthy(get("GetRecordStatus", "outputActive").as_ref()),
				paused: truthy(get("GetRecordStatus", "outputPaused").as_ref()),
				available: false,
			};
			d.stream = Output { active: truthy(get("GetStreamStatus", "outputActive").as_ref()), ..Output::default() };
			d.replay = Output {
				active: truthy(get("GetReplayBufferStatus", "outputActive").as_ref()),
				available: present("GetReplayBufferStatus"),
				..Output::default()
			};
			d.virtualcam = Output { active: truthy(get("GetVirtualCamStatus", "outputActive").as_ref()), ..Output::default() };
			if present("GetCurrentSceneTransition") {
				d.transition = Transition {
					name: string(get("GetCurrentSceneTransition", "transitionName").as_ref()),
					duration: number(get("GetCurrentSceneTransition", "transitionDuration").as_ref()),
				};
			}
			d.profile = string(get("GetProfileList", "currentProfileName").as_ref());
			d.collection = string(get("GetSceneCollectionList", "currentSceneCollectionName").as_ref());
			let denominator = get("GetVideoSettings", "fpsDenominator");
			if truthy(denominator.as_ref()) {
				d.fps = js::number(get("GetVideoSettings", "fpsNumerator").as_ref()) / js::number(denominator.as_ref());
			}
			d.ready = true;
		}
		self.on_change.emit("all");
	}

	fn spawn_refresh_scenes(&self) {
		if let Some(me) = self.me.upgrade() {
			// call() sends right away, like in the JavaScript version.
			let reply = me.client.call("GetSceneList", crate::obj! {});
			spawn_local(async move {
				if let Ok(list) = reply.await {
					set_scenes(&mut me.data.borrow_mut(), &list);
					me.on_change.emit("scenes");
				}
			});
		}
	}

	fn on_event(&self, kind: &str, data: &Value) {
		let s = |key: &str| string(data.get(key));
		match kind {
			"CurrentProgramSceneChanged" => {
				{
					let mut d = self.data.borrow_mut();
					let new = s("sceneName");
					if let Some(old) = d.program_scene.clone()
						&& Some(&old) != new.as_ref()
					{
						d.history.retain(|name| *name != old);
						d.history.push(old);
						if d.history.len() > HISTORY_SIZE {
							d.history.remove(0);
						}
					}
					d.program_scene = new;
				}
				self.on_change.emit("program");
			}
			"CurrentPreviewSceneChanged" => {
				self.data.borrow_mut().preview_scene = s("sceneName");
				self.on_change.emit("preview");
			}
			"StudioModeStateChanged" => {
				let enabled = truthy(data.get("studioModeEnabled"));
				self.data.borrow_mut().studio_mode = enabled;
				if !enabled {
					self.data.borrow_mut().preview_scene = None;
				} else if let Some(me) = self.me.upgrade() {
					let reply = self.client.call("GetCurrentPreviewScene", crate::obj! {});
					spawn_local(async move {
						if let Ok(r) = reply.await {
							me.data.borrow_mut().preview_scene =
								string(r.get("sceneName")).or_else(|| string(r.get("currentPreviewSceneName")));
							me.on_change.emit("preview");
						}
					});
				}
				self.on_change.emit("studio");
			}
			"SceneListChanged" | "SceneCreated" | "SceneRemoved" => self.spawn_refresh_scenes(),
			"SceneNameChanged" => {
				{
					let mut d = self.data.borrow_mut();
					let old = s("oldSceneName");
					let new = s("sceneName");
					for name in d.history.iter_mut() {
						if Some(&*name) == old.as_ref() {
							*name = new.clone().unwrap_or_default();
						}
					}
					if d.program_scene == old {
						d.program_scene = new.clone();
					}
					if d.preview_scene == old {
						d.preview_scene = new;
					}
				}
				self.spawn_refresh_scenes();
			}
			"RecordStateChanged" => {
				self.data.borrow_mut().record = Output {
					active: truthy(data.get("outputActive")),
					paused: data.get("outputState").and_then(Value::as_str) == Some("OBS_WEBSOCKET_OUTPUT_PAUSED"),
					available: false,
				};
				self.on_change.emit("record");
			}
			"StreamStateChanged" => {
				self.data.borrow_mut().stream = Output { active: truthy(data.get("outputActive")), ..Output::default() };
				self.on_change.emit("stream");
			}
			"ReplayBufferStateChanged" => {
				self.data.borrow_mut().replay = Output { active: truthy(data.get("outputActive")), available: true, ..Output::default() };
				self.on_change.emit("replay");
			}
			"VirtualcamStateChanged" => {
				self.data.borrow_mut().virtualcam = Output { active: truthy(data.get("outputActive")), ..Output::default() };
				self.on_change.emit("virtualcam");
			}
			"CurrentSceneTransitionChanged" => {
				self.data.borrow_mut().transition.name = s("transitionName");
				if let Some(me) = self.me.upgrade() {
					let reply = self.client.call("GetCurrentSceneTransition", crate::obj! {});
					spawn_local(async move {
						if let Ok(r) = reply.await {
							me.data.borrow_mut().transition =
								Transition { name: string(r.get("transitionName")), duration: number(r.get("transitionDuration")) };
							me.on_change.emit("transition");
						}
					});
				}
				self.on_change.emit("transition");
			}
			"CurrentSceneTransitionDurationChanged" => {
				self.data.borrow_mut().transition.duration = number(data.get("transitionDuration"));
				self.on_change.emit("transition");
			}
			"CurrentProfileChanged" => {
				self.data.borrow_mut().profile = s("profileName");
				self.on_change.emit("profile");
			}
			"CurrentSceneCollectionChanged" => {
				// Everything about scenes changed, start over.
				self.data.borrow_mut().history.clear();
				if let Some(me) = self.me.upgrade() {
					spawn_local(async move { me.load().await });
				}
			}
			"InputVolumeMeters" => {
				// Inputs that play nothing are left out, so start fresh each time (no stuck meters).
				let mut levels = HashMap::new();
				if let Some(Value::Array(inputs)) = data.get("inputs") {
					for input in inputs {
						levels.insert(js::to_string(input.get("inputName")), level_from_meter(input.get("inputLevelsMul")));
					}
				}
				*self.levels.borrow_mut() = levels.clone();
				self.on_meters.emit(&levels);
			}
			_ => {}
		}
	}

	/// Audio levels only come while at least one key shows them, they are a lot of traffic.
	/// Returns the unsubscribe function.
	pub fn subscribe_meters(&self) -> impl Fn() + 'static {
		self.meter_users.set(self.meter_users.get() + 1);
		self.client.set_event_subscriptions(EVENTS_ALL | EVENTS_VOLUME_METERS);
		let done = Cell::new(false);
		let me = self.me.clone();
		move || {
			if done.replace(true) {
				return;
			}
			let Some(me) = me.upgrade() else { return };
			me.meter_users.set((me.meter_users.get() - 1).max(0));
			if me.meter_users.get() == 0 {
				me.client.set_event_subscriptions(EVENTS_ALL);
				me.levels.borrow_mut().clear();
			}
		}
	}

	/// Stats are polled once a second while at least one key shows them.
	pub fn subscribe_stats(&self) -> impl Fn() + 'static {
		self.stats_users.set(self.stats_users.get() + 1);
		if self.stats_timer.borrow().is_none()
			&& let Some(me) = self.me.upgrade()
		{
			me.spawn_poll_stats();
			let weak = self.me.clone();
			let timer = set_interval(1000.0, move || {
				if let Some(me) = weak.upgrade() {
					me.spawn_poll_stats();
				}
			});
			*self.stats_timer.borrow_mut() = Some(timer);
		}
		let done = Cell::new(false);
		let me = self.me.clone();
		move || {
			if done.replace(true) {
				return;
			}
			let Some(me) = me.upgrade() else { return };
			me.stats_users.set(me.stats_users.get() - 1);
			if me.stats_users.get() <= 0 {
				me.stats_users.set(0);
				if let Some(timer) = me.stats_timer.borrow_mut().take() {
					timer.clear();
				}
			}
		}
	}

	fn spawn_poll_stats(self: Rc<Self>) {
		if !self.client.connected() {
			return;
		}
		let reply = self.client.batch(
			vec![crate::obj! { "requestType" => "GetStats" }, crate::obj! { "requestType" => "GetStreamStatus" }],
			BatchExecution::SerialRealtime,
			false,
			None,
		);
		spawn_local(async move {
			let Ok(results) = reply.await else { return };
			let pick = |r: Option<&Value>| match r {
				Some(r) if truthy(r.get("requestStatus").and_then(|s| s.get("result"))) => r.get("responseData").cloned(),
				_ => None,
			};
			let Some(Value::Object(mut stats)) = pick(results.first()) else { return };
			let stream = pick(results.get(1)).map(|mut stream| {
				let kbps = self.bitrate(&stream);
				if let Value::Object(map) = &mut stream {
					map.insert("kbps".into(), kbps.map_or(Value::Null, js::num));
				}
				stream
			});
			stats.insert("stream".into(), stream.unwrap_or(Value::Null));
			let stats = Value::Object(stats);
			*self.stats.borrow_mut() = Some(stats.clone());
			self.on_stats.emit(&stats);
		});
	}

	/// Upload speed of the stream in kbit/s, from the bytes sent between two polls.
	pub fn bitrate(&self, stream: &Value) -> Option<f64> {
		self.bitrate_at(stream, js::now())
	}

	pub fn bitrate_at(&self, stream: &Value, now: i64) -> Option<f64> {
		let bytes = match stream.get("outputBytes") {
			None | Some(Value::Null) => 0.0,
			b => js::number(b),
		};
		let active = truthy(stream.get("outputActive"));
		let last = self.last_bytes.get();
		self.last_bytes.set(if active { Some((bytes, now)) } else { None });
		let (last_bytes, last_now) = last?;
		if !active || bytes < last_bytes || now == last_now {
			return None;
		}
		Some(((bytes - last_bytes) * 8.0) / (now - last_now) as f64)
	}
}

fn truthy_value(v: &Value) -> bool {
	truthy(Some(v))
}

fn set_scenes(d: &mut crate::obs_state::Data, list: &Value) {
	// OBS lists scenes bottom first. Keys want the order of the scene dock.
	d.scenes = sorted_scene_names(list);
	let current = list.get("currentProgramSceneName");
	if truthy(current) {
		d.program_scene = Some(js::to_string(current));
	}
}

/// Scene names in the order of the scene dock (OBS lists them bottom first).
pub fn sorted_scene_names(list: &Value) -> Vec<String> {
	let mut scenes: Vec<&Value> = match list.get("scenes") {
		Some(Value::Array(scenes)) => scenes.iter().collect(),
		_ => Vec::new(),
	};
	// Stable sort by sceneIndex, highest first, like the JavaScript sort.
	scenes.sort_by(|a, b| {
		let (a, b) = (js::number(a.get("sceneIndex")), js::number(b.get("sceneIndex")));
		b.partial_cmp(&a).unwrap_or(std::cmp::Ordering::Equal)
	});
	scenes.iter().map(|s| js::to_string(s.get("sceneName"))).collect()
}

/// OBS sends [magnitude, peak, input peak] per channel as multipliers. Keys show the loudest peak in dB.
pub fn level_from_meter(channels: Option<&Value>) -> Level {
	let mut peak: f64 = 0.0;
	let mut magnitude: f64 = 0.0;
	if let Some(Value::Array(channels)) = channels {
		for channel in channels {
			let at = |i: usize| match channel.get(i) {
				None => 0.0,
				v => js::number(v),
			};
			peak = js::max(peak, at(1));
			magnitude = js::max(magnitude, at(0));
		}
	}
	let db = |v: f64| if v > 0.0 { 20.0 * v.log10() } else { f64::NEG_INFINITY };
	Level { peak: db(peak), magnitude: db(magnitude) }
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	#[test]
	fn loudest_peak_in_db() {
		let level = level_from_meter(Some(&json!([[0.1, 0.5, 0.5], [0.2, 0.25, 0.25]])));
		assert!((level.peak - 20.0 * 0.5f64.log10()).abs() < 1e-9);
		assert_eq!(level_from_meter(Some(&json!([]))).peak, f64::NEG_INFINITY);
	}

	#[test]
	fn stream_bitrate_from_bytes_between_two_polls() {
		let client = ObsClient::new("127.0.0.1", 4455.0, "", "test");
		let state = ObsState::new(client);
		assert_eq!(state.bitrate_at(&json!({ "outputActive": true, "outputBytes": 0 }), 10_000), None);
		let kbps = state.bitrate_at(&json!({ "outputActive": true, "outputBytes": 750000 }), 11_000).unwrap_or(0.0);
		assert!((kbps - 6000.0).abs() < 1.0);
		assert_eq!(state.bitrate_at(&json!({ "outputActive": false, "outputBytes": 0 }), 12_000), None);
		assert_eq!(state.last_bytes.get(), None);
	}
}
