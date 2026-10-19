// Source animations: phases of transform changes, played frame by frame inside OBS.
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::js::{self, max, number, round as js_round};
use crate::obs::{BatchExecution, ObsClient};

pub static FIELDS: &[&str] = &[
	"positionX",
	"positionY",
	"rotation",
	"scaleX",
	"scaleY",
	"cropLeft",
	"cropRight",
	"cropTop",
	"cropBottom",
	"boundsWidth",
	"boundsHeight",
];
const CROP: &[&str] = &["cropLeft", "cropRight", "cropTop", "cropBottom"];
const BOUNDS: &[&str] = &["boundsWidth", "boundsHeight"];
const MAX_FRAMES: usize = 60 * 120;

pub static EASINGS: &[&str] = &["linear", "easeIn", "easeOut", "easeInOut", "back"];

fn ease(name: &str, t: f64) -> f64 {
	match name {
		"easeIn" => t * t * t,
		"easeOut" => 1.0 - (1.0 - t).powf(3.0),
		"easeInOut" => {
			if t < 0.5 {
				4.0 * t * t * t
			} else {
				1.0 - (-2.0 * t + 2.0).powf(3.0) / 2.0
			}
		}
		"back" => 1.0 + 2.70158 * (t - 1.0).powf(3.0) + 1.70158 * (t - 1.0).powf(2.0),
		_ => t,
	}
}

fn round(value: f64, digits: i32) -> f64 {
	let f = 10f64.powi(digits);
	js_round(value * f) / f
}

pub fn has_bounds(transform: Option<&Value>) -> bool {
	transform.and_then(|t| t["boundsType"].as_str()).is_some_and(|kind| !kind.is_empty() && kind != "OBS_BOUNDS_NONE")
}

fn field<'a>(transform: Option<&'a Value>, name: &str) -> Option<&'a Value> {
	transform.and_then(|t| t.get(name))
}

/// A number of the transform the animation starts from, 0 when it has none.
fn start_value(start: Option<&Value>, name: &str) -> f64 {
	field(start, name).and_then(Value::as_f64).unwrap_or(0.0)
}

/// What changed between two transforms, only the fields that moved.
pub fn diff_transforms(from: Option<&Value>, to: Option<&Value>) -> Map<String, Value> {
	let mut changes = Map::new();
	for name in FIELDS {
		if BOUNDS.contains(name) && !(has_bounds(from) && has_bounds(to)) {
			continue;
		}
		let a = number(field(from, name));
		let b = number(field(to, name));
		if !a.is_finite() || !b.is_finite() {
			continue;
		}
		let delta = round(b - a, 4);
		if delta.abs() > 1e-4 {
			changes.insert(name.to_string(), js::num(delta));
		}
	}
	changes
}

/// A phase with only what is valid: numbers that change something, a known easing.
#[derive(Debug, Clone, PartialEq)]
pub struct Phase {
	pub duration: f64,
	pub delay: f64,
	pub easing: String,
	pub changes: Vec<(&'static str, f64)>,
}

impl Phase {
	fn change(&self, name: &str) -> Option<f64> {
		self.changes.iter().find(|(n, _)| *n == name).map(|(_, v)| *v)
	}

	pub fn to_value(&self) -> Value {
		let changes: Map<String, Value> = self.changes.iter().map(|(n, v)| (n.to_string(), js::num(*v))).collect();
		crate::obj! { "duration" => js::num(self.duration), "delay" => js::num(self.delay), "easing" => self.easing.clone(), "changes" => changes }
	}
}

/// A phase as the settings page saves it.
#[derive(Deserialize, Default)]
#[serde(default)]
struct PhaseSettings {
	duration: f64,
	delay: f64,
	easing: String,
	/// Field name -> how much it changes.
	changes: Value,
}

pub fn normalize_phase(phase: &Value) -> Phase {
	let p: PhaseSettings = crate::settings::read_value(phase);
	let changes =
		FIELDS.iter().map(|name| (*name, number(p.changes.get(*name)))).filter(|(_, value)| value.is_finite() && *value != 0.0).collect();
	Phase {
		duration: max(0.0, crate::settings::or(p.duration, 0.0)),
		delay: max(0.0, crate::settings::or(p.delay, 0.0)),
		easing: if EASINGS.contains(&p.easing.as_str()) { p.easing } else { "linear".into() },
		changes,
	}
}

fn value(field: &str, number: f64) -> f64 {
	if CROP.contains(&field) {
		return max(0.0, js_round(number));
	}
	if BOUNDS.contains(&field) {
		return max(1.0, number);
	}
	if (field == "scaleX" || field == "scaleY") && number.abs() < 0.0001 {
		return if number < 0.0 { -0.0001 } else { 0.0001 };
	}
	round(number, 5)
}

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
	Wait(f64),
	Transform(Map<String, Value>),
}

pub struct Frames {
	pub fields: Vec<&'static str>,
	pub steps: Vec<Step>,
}

/// Every transform to set, one per frame. Delays become empty frames.
pub fn build_frames(start: Option<&Value>, phases: &[Value], fps: f64) -> Frames {
	let normalized: Vec<Phase> = phases.iter().map(normalize_phase).collect();
	let fields: Vec<&'static str> = FIELDS
		.iter()
		.copied()
		.filter(|f| normalized.iter().any(|p| p.change(f).is_some()) && (!BOUNDS.contains(f) || has_bounds(start)))
		.collect();
	let mut current: Vec<f64> = fields.iter().map(|f| start_value(start, f)).collect();
	let mut steps = Vec::new();
	for phase in &normalized {
		let wait = js_round((phase.delay / 1000.0) * fps);
		if wait > 0.0 {
			steps.push(Step::Wait(wait));
		}
		let frames = max(1.0, js_round((phase.duration / 1000.0) * fps));
		let from = current.clone();
		let mut i = 1.0;
		while i <= frames {
			let t = ease(&phase.easing, i / frames);
			let mut transform = Map::new();
			for (k, f) in fields.iter().enumerate() {
				transform.insert(f.to_string(), js::num(value(f, from[k] + phase.change(f).unwrap_or(0.0) * t)));
			}
			steps.push(Step::Transform(transform));
			i += 1.0;
		}
		for (k, f) in fields.iter().enumerate() {
			current[k] = from[k] + phase.change(f).unwrap_or(0.0);
		}
	}
	steps.truncate(MAX_FRAMES);
	Frames { fields, steps }
}

pub fn total_duration(phases: &[Value]) -> f64 {
	phases.iter().map(normalize_phase).fold(0.0, |sum, p| sum + p.delay + p.duration)
}

/// What happens before and after the phases. Settings of the Source Animation key.
#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Options {
	pub show_at_start: bool,
	pub hide_at_end: bool,
	/// Back to where the source was before the animation.
	pub reset_at_end: bool,
	/// A filter of the source to turn on or off at the start or the end.
	pub filter: String,
	/// "enable", "disable" or "none".
	pub filter_start: String,
	pub filter_end: String,
}

pub struct Run<'a> {
	pub scene_name: &'a str,
	pub scene_item_id: Value,
	pub source_name: &'a str,
	pub start: Option<&'a Value>,
	pub phases: &'a [Value],
	pub fps: f64,
	pub options: &'a Options,
}

fn request(kind: &str, data: Value) -> Value {
	crate::obj! { "requestType" => kind, "requestData" => data }
}

/// The requests for one run, played by OBS on its own render frames.
pub fn build_requests(run: &Run) -> (Vec<Value>, usize) {
	let Frames { fields, steps } = build_frames(run.start, run.phases, run.fps);
	let o = run.options;
	let mut requests = Vec::new();
	let item = |extra: (&str, Value)| {
		let mut data = Map::new();
		data.insert("sceneName".into(), run.scene_name.into());
		data.insert("sceneItemId".into(), run.scene_item_id.clone());
		data.insert(extra.0.into(), extra.1);
		Value::Object(data)
	};
	let filter = |enabled: bool| {
		request(
			"SetSourceFilterEnabled",
			crate::obj! { "sourceName" => run.source_name, "filterName" => o.filter.as_str(), "filterEnabled" => enabled },
		)
	};
	let filter_at = |when: &str| !o.filter.is_empty() && !when.is_empty() && when != "none";
	if filter_at(&o.filter_start) {
		requests.push(filter(o.filter_start == "enable"));
	}
	if o.show_at_start {
		requests.push(request("SetSceneItemEnabled", item(("sceneItemEnabled", Value::Bool(true)))));
	}
	for step in &steps {
		match step {
			Step::Wait(wait) => requests.push(request("Sleep", crate::obj! { "sleepFrames" => js::num(*wait) })),
			Step::Transform(transform) => {
				requests.push(request("SetSceneItemTransform", item(("sceneItemTransform", Value::Object(transform.clone())))));
				requests.push(request("Sleep", crate::obj! { "sleepFrames" => 1 }));
			}
		}
	}
	if o.hide_at_end {
		requests.push(request("SetSceneItemEnabled", item(("sceneItemEnabled", Value::Bool(false)))));
	}
	if o.reset_at_end && !fields.is_empty() {
		if o.hide_at_end {
			requests.push(request("Sleep", crate::obj! { "sleepFrames" => 1 }));
		}
		let reset: Map<String, Value> = fields.iter().map(|f| (f.to_string(), js::num(value(f, start_value(run.start, f))))).collect();
		requests.push(request("SetSceneItemTransform", item(("sceneItemTransform", Value::Object(reset)))));
	}
	if filter_at(&o.filter_end) {
		requests.push(filter(o.filter_end == "enable"));
	}
	(requests, steps.len())
}

pub async fn run_animation(client: &ObsClient, run: &Run<'_>) -> crate::Res<usize> {
	let (requests, frames) = build_requests(run);
	let timeout = (frames as f64 / max(1.0, run.fps)) * 1000.0 + 15000.0;
	let results = client.batch(requests, BatchExecution::SerialFrame, true, Some(timeout)).await?;
	if let Some(failed) = results.iter().find(|r| r["requestStatus"]["result"] != true) {
		return Err(match failed["requestStatus"]["comment"].as_str() {
			Some(comment) if !comment.is_empty() => comment.into(),
			_ => format!("{} failed", failed["requestType"].as_str().unwrap_or_default()).into(),
		});
	}
	Ok(results.len())
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	#[test]
	fn diff_only_keeps_fields_that_moved() {
		let a = json!({ "positionX": 100, "positionY": 50, "rotation": 0, "scaleX": 1, "scaleY": 1, "cropLeft": 0, "boundsType": "OBS_BOUNDS_NONE", "boundsWidth": 0 });
		let mut b = a.clone();
		b["positionX"] = json!(400);
		b["scaleX"] = json!(1.5);
		b["cropLeft"] = json!(20);
		b["boundsWidth"] = json!(300);
		assert_eq!(Value::Object(diff_transforms(Some(&a), Some(&b))), json!({ "positionX": 300, "scaleX": 0.5, "cropLeft": 20 }));
	}

	#[test]
	fn frames_per_phase_final_values_and_easing_ends() {
		let start = json!({ "positionX": 0, "positionY": 0, "rotation": 0, "scaleX": 1, "scaleY": 1, "cropLeft": 0 });
		let phases = vec![
			json!({ "duration": 1000, "easing": "easeInOut", "changes": { "positionX": 300 } }),
			json!({ "duration": 500, "delay": 200, "easing": "easeOut", "changes": { "cropLeft": 10.6, "scaleX": -1 } }),
		];
		let Frames { mut fields, steps } = build_frames(Some(&start), &phases, 30.0);
		fields.sort();
		assert_eq!(fields, ["cropLeft", "positionX", "scaleX"]);
		let frames: Vec<&Map<String, Value>> =
			steps.iter().filter_map(|s| if let Step::Transform(t) = s { Some(t) } else { None }).collect();
		assert_eq!(frames.len(), 30 + 15);
		assert_eq!(steps.iter().filter(|s| matches!(s, Step::Wait(_))).cloned().collect::<Vec<_>>(), vec![Step::Wait(6.0)]);
		assert_eq!(frames[29]["positionX"], json!(300));
		assert_eq!(frames[frames.len() - 1]["cropLeft"], json!(11), "crop is a whole number");
		assert_eq!(frames[frames.len() - 1]["scaleX"], json!(0.0001), "scale never hits 0");
		assert!(frames[0]["positionX"].as_f64().unwrap_or(99.0) < 10.0, "ease in starts slow");
		assert_eq!(total_duration(&phases), 1700.0);
	}

	#[test]
	fn requests_include_start_and_end_options() {
		let start = json!({ "positionX": 5 });
		let options = crate::settings::read_value(
			&json!({ "showAtStart": true, "hideAtEnd": true, "resetAtEnd": true, "filter": "Blur", "filterStart": "enable", "filterEnd": "disable" }),
		);
		let phases = vec![json!({ "duration": 100, "changes": { "positionX": 10 } })];
		let (requests, frames) = build_requests(&Run {
			scene_name: "S",
			scene_item_id: json!(3),
			source_name: "Box",
			start: Some(&start),
			phases: &phases,
			fps: 30.0,
			options: &options,
		});
		assert_eq!(frames, 3);
		let types: Vec<&str> = requests.iter().map(|r| r["requestType"].as_str().unwrap_or("")).collect();
		assert_eq!(types[0], "SetSourceFilterEnabled");
		assert_eq!(types[1], "SetSceneItemEnabled");
		assert_eq!(types[types.len() - 1], "SetSourceFilterEnabled");
		assert_eq!(requests[requests.len() - 1]["requestData"]["filterEnabled"], json!(false));
		assert_eq!(requests[requests.len() - 2]["requestData"]["sceneItemTransform"], json!({ "positionX": 5 }), "reset to the start");
		assert_eq!(normalize_phase(&json!({ "easing": "nope" })).easing, "linear");
	}
}
