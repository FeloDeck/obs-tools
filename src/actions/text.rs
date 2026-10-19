// Set Text: a fixed text, the next line of a list, a counter or a timer. Press, hold and double press
// can each do any of these (TEXT_ACTIONS).
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{Behavior, Gesture, KeyRc, Kind, Press, Stop, defaults, patch, same_name};
use crate::Res;
use crate::icons::colors;
use crate::js;
use crate::render::{GlyphKey, glyph_key};
use crate::settings::{or, read};
use crate::time::{format_duration, parse_duration};

pub static TEXT_ACTIONS: &[&str] = &["none", "set", "toggle", "next", "previous", "first", "up", "down", "reset", "timer", "clear"];

/// What a text action shows on the key afterwards.
pub static TEXT_KINDS: &[(&str, &str)] = &[
	("set", "text"),
	("toggle", "text"),
	("next", "line"),
	("previous", "line"),
	("first", "line"),
	("up", "count"),
	("down", "count"),
	("reset", "count"),
	("timer", "timer"),
	("clear", "cleared"),
];

pub fn text_kind(action: &str) -> Option<&'static str> {
	TEXT_KINDS.iter().find(|(a, _)| *a == action).map(|(_, k)| *k)
}

/// The action of a gesture. Press sets the text unless something else is picked.
pub fn text_gesture(settings: &Map<String, Value>, gesture: Gesture) -> String {
	if let Some(Value::String(value)) = settings.get(gesture.name())
		&& TEXT_ACTIONS.contains(&value.as_str())
	{
		return value.clone();
	}
	if gesture == Gesture::Press { "set".into() } else { "none".into() }
}

/// Does any gesture use something of this kind ("text", "line", "count" or "timer")?
pub fn text_uses(settings: &Map<String, Value>, kind: &str) -> bool {
	Gesture::ALL.iter().any(|g| text_kind(&text_gesture(settings, *g)) == Some(kind))
}

// ----- the key -----

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct TextSettings {
	input: String,
	text: String,
	/// One line per entry, for next, previous and first.
	lines: String,
	/// The line set last, -1 for none yet.
	index: f64,
	/// The counter text: "{n}" becomes the number.
	template: String,
	start: f64,
	count: f64,
	step: f64,
	/// "{t}" becomes the time left.
	timer_text: String,
	timer_length: String,
	timer_end: String,
	/// What was set last: "text", "line", "count", "timer" or "cleared".
	shown: String,
	show_text: bool,
}

impl TextSettings {
	fn lines(&self) -> Vec<String> {
		self.lines.split('\n').map(|l| l.trim_end_matches(js::is_js_space).to_string()).filter(|l| !l.is_empty()).collect()
	}

	fn counter_text(&self, count: f64) -> String {
		let template = if self.template.is_empty() { "{n}" } else { &self.template };
		if template.contains("{n}") { template.replace("{n}", &js::fmt(count)) } else { format!("{template}{}", js::fmt(count)) }
	}

	/// The timer text: "{t}" becomes the time left. At 0 the end text, if there is one.
	fn timer_text(&self, seconds: f64) -> String {
		if seconds <= 0.0 && !self.timer_end.is_empty() {
			return self.timer_end.clone();
		}
		let template = if self.timer_text.is_empty() { "{t}" } else { &self.timer_text };
		let time = format_duration(seconds);
		if template.contains("{t}") { template.replace("{t}", &time) } else { format!("{template}{time}") }
	}
}

#[derive(Default)]
struct TextState {
	text_on: bool,
	time_left: Option<f64>,
	stop_timer: Option<Stop>,
}

struct SetText;

fn text_state(k: &KeyRc) -> std::cell::RefMut<'_, TextState> {
	k.own::<TextState>()
}

/// What the key shows: what was set last, or what a press would set.
fn preview(k: &KeyRc, s: &TextSettings) -> String {
	let kind = if s.shown.is_empty() { text_kind(&k.gesture(Gesture::Press)).unwrap_or("") } else { s.shown.as_str() };
	match kind {
		"count" => s.counter_text(or(s.count, 0.0)),
		"timer" => {
			let left = text_state(k).time_left;
			s.timer_text(left.unwrap_or_else(|| parse_duration(&s.timer_length)))
		}
		"line" => {
			let lines = s.lines();
			if s.index >= 0.0 && s.index.fract() == 0.0 { lines.get(s.index as usize).cloned().unwrap_or_default() } else { String::new() }
		}
		"text" => s.text.clone(),
		_ => String::new(),
	}
}

// Text on or off: the key is white while the source has this key's text, else gray.
fn uses_toggle(k: &KeyRc) -> bool {
	let settings = k.settings_map();
	Gesture::ALL.iter().any(|g| text_gesture(&settings, *g) == "toggle")
}

async fn read_toggle(k: &KeyRc) {
	let s: TextSettings = k.settings();
	if !uses_toggle(k) || s.input.is_empty() {
		return;
	}
	let on = match k.call("GetInputSettings", json!({ "inputName": s.input })).await {
		Ok(r) => !s.text.is_empty() && r.get("inputSettings").and_then(|t| t.get("text")).and_then(Value::as_str) == Some(s.text.as_str()),
		Err(_) => false,
	};
	text_state(k).text_on = on;
}

async fn set_text(k: &KeyRc, text: &str) -> Res<()> {
	let input = k.settings::<TextSettings>().input;
	if input.is_empty() {
		return Err("no text source selected".into());
	}
	k.call("SetInputSettings", json!({ "inputName": input, "inputSettings": { "text": text }, "overlay": true })).await?;
	Ok(())
}

async fn set_count(k: &KeyRc, count: f64) -> Res<()> {
	set_text(k, &k.settings::<TextSettings>().counter_text(count)).await?;
	k.save_settings(patch(vec![("count", js::num(count))]));
	Ok(())
}

async fn set_line(k: &KeyRc, index: f64) -> Res<()> {
	let lines = k.settings::<TextSettings>().lines();
	if lines.is_empty() {
		return Err("the list is empty".into());
	}
	let len = lines.len() as f64;
	let index = ((index % len) + len) % len;
	let line = lines.get(index as usize).cloned().unwrap_or_default();
	set_text(k, &line).await?;
	k.save_settings(patch(vec![("index", js::num(index))]));
	Ok(())
}

// Counts down by itself, once a second. Running: the next start stops it.
async fn toggle_timer(k: &KeyRc) -> Res<()> {
	let running = text_state(k).stop_timer.take();
	if let Some(stop) = running {
		stop();
		return Ok(());
	}
	let length = parse_duration(&k.settings::<TextSettings>().timer_length);
	if length == 0.0 {
		return Err("no timer length set".into());
	}
	let end = js::now() as f64 + length * 1000.0;
	let shown = std::rc::Rc::new(std::cell::Cell::new(f64::NAN));
	let tick = move |k: KeyRc| -> crate::obs::Reply<()> {
		let shown = shown.clone();
		Box::pin(async move {
			let left = js::max(0.0, ((end - js::now() as f64) / 1000.0).ceil());
			if left != shown.get() {
				shown.set(left);
				text_state(&k).time_left = Some(left);
				set_text(&k, &k.settings::<TextSettings>().timer_text(left)).await?;
				k.render();
			}
			if left == 0.0 {
				let stop = text_state(&k).stop_timer.take();
				if let Some(stop) = stop {
					stop();
				}
			}
			Ok(())
		})
	};
	let tick = std::rc::Rc::new(tick);
	let stop = {
		let tick = tick.clone();
		k.interval(200.0, move |k| tick(k))
	};
	text_state(k).stop_timer = Some(stop);
	tick(k.clone()).await
}

async fn count(k: &KeyRc, direction: f64) -> Res<()> {
	let s: TextSettings = k.settings();
	set_count(k, js::max(0.0, or(s.count, 0.0) + direction * or(s.step, 1.0))).await
}

async fn run(k: &KeyRc, what: &str) -> Res<()> {
	let s: TextSettings = k.settings();
	match what {
		"set" => set_text(k, &s.text).await?,
		// Off when the source shows this text, else on.
		"toggle" => {
			if s.input.is_empty() {
				return Err("no text source selected".into());
			}
			let current = k.call("GetInputSettings", json!({ "inputName": s.input })).await?;
			let on = !s.text.is_empty()
				&& current.get("inputSettings").and_then(|t| t.get("text")).and_then(Value::as_str) == Some(s.text.as_str());
			set_text(k, if on { "" } else { &s.text }).await?;
			text_state(k).text_on = !on;
		}
		// index -1 means nothing was set yet, so the first "next" sets line 0.
		"next" => set_line(k, or(s.index, 0.0) + 1.0).await?,
		"previous" => set_line(k, or(s.index, 0.0) - 1.0).await?,
		"first" => set_line(k, 0.0).await?,
		"up" => count(k, 1.0).await?,
		"down" => count(k, -1.0).await?,
		"reset" => set_count(k, or(s.start, 0.0)).await?,
		"timer" => toggle_timer(k).await?,
		"clear" => set_text(k, "").await?,
		_ => return Ok(()),
	}
	if let Some(kind) = text_kind(what).filter(|kind| *kind != s.shown) {
		k.save_settings(patch(vec![("shown", kind.into())]));
	}
	// Hold or double press can change the text too.
	if what != "toggle" {
		read_toggle(k).await;
	}
	k.render();
	Ok(())
}

#[async_trait(?Send)]
impl Behavior for SetText {
	// Press, hold and double press pick from TEXT_ACTIONS (the base does the rest).
	fn gesture(&self, k: &KeyRc, gesture: Gesture) -> String {
		text_gesture(&k.settings_map(), gesture)
	}

	// A new start number puts the counter there.
	async fn on_settings_changed(&self, k: &KeyRc, old: &Map<String, Value>) -> Res<()> {
		let (old, s): (TextSettings, TextSettings) = (read(old), k.settings());
		let start = or(s.start, 0.0);
		if or(old.start, 0.0) != start {
			k.save_settings(patch(vec![("count", js::num(start))]));
		}
		if k.online() && (old.input != s.input || old.text != s.text) {
			read_toggle(k).await;
		}
		Ok(())
	}

	async fn on_connected(&self, k: &KeyRc) -> Res<()> {
		read_toggle(k).await;
		Ok(())
	}

	// The text can also change in OBS or by another key.
	async fn on_obs_event(&self, k: &KeyRc, kind: &str, data: &Value) -> Res<()> {
		let s: TextSettings = k.settings();
		if kind != "InputSettingsChanged" || !same_name(data, "inputName", &s.input) || !uses_toggle(k) {
			return Ok(());
		}
		let Some(Value::String(text)) = data.get("inputSettings").and_then(|s| s.get("text")) else { return Ok(()) };
		text_state(k).text_on = !s.text.is_empty() && s.text == *text;
		self.render(k);
		Ok(())
	}

	fn render_offline(&self, k: &KeyRc) {
		self.render(k);
	}

	fn render(&self, k: &KeyRc) {
		let s: TextSettings = k.settings();
		let title = if s.show_text { js::trim(preview(k, &s).split('\n').next().unwrap_or("")).to_string() } else { String::new() };
		let off = uses_toggle(k) && !text_state(k).text_on;
		k.own_title(&title);
		k.set_image(&glyph_key(
			"text",
			&GlyphKey { title: &title, color: if off { colors::GRAY } else { colors::WHITE }, ..Default::default() },
		));
	}

	// The timer stops with the key (another profile), so do not keep a dead one.
	fn on_disappear(&self, k: &KeyRc) {
		text_state(k).stop_timer = None;
	}

	async fn on_press(&self, k: &KeyRc, press: Press) -> Res<()> {
		run(k, &press.op).await
	}
}

pub static SET_TEXT: Kind = Kind {
	uuid: "de.felitendo.obs-tools.settext",
	name: "SetText",
	// press, hold and double each pick one of TEXT_ACTIONS. shown: what the key shows ("text", "line" or "count").
	defaults: || {
		defaults(json!({
			"input": "",
			"press": "set",
			"hold": "none",
			"double": "none",
			"text": "",
			"lines": "",
			"index": -1,
			"template": "{n}",
			"start": 0,
			"count": 0,
			"step": 1,
			"timerText": "{t}",
			"timerLength": "5:00",
			"timerEnd": "",
			"shown": "",
			"showText": true,
		}))
	},
	required: &["input"],
	behavior: &SetText,
	new_state: || Box::new(TextState::default()),
	..Kind::BASE
};

#[cfg(test)]
mod tests {
	use super::super::testing::*;
	use super::*;
	use crate::settings::testing::assert_defaults;

	#[test]
	fn every_setting_has_a_default() {
		assert_defaults::<TextSettings>(&[&SET_TEXT]);
	}

	fn written(k: &KeyRc) -> Vec<String> {
		calls(k)
			.iter()
			.filter(|(t, _)| t == "SetInputSettings")
			.map(|(_, d)| d["inputSettings"]["text"].as_str().unwrap_or("").to_string())
			.collect()
	}

	#[tokio::test]
	async fn set_text_press_hold_and_double_press_can_each_do_anything() {
		// A countdown: press counts down and stops at 0, hold resets to the start number.
		let (_p, k) =
			key(&SET_TEXT, json!({ "input": "T", "press": "down", "hold": "reset", "template": "Lives: {n}", "start": 2, "count": 2 }));
		for _ in 0..3 {
			k.press(Gesture::Press, false).await.unwrap_or_default();
		}
		assert_eq!(written(&k), ["Lives: 1", "Lives: 0", "Lives: 0"]);
		k.press(Gesture::Hold, false).await.unwrap_or_default();
		assert_eq!(written(&k).last().map(String::as_str), Some("Lives: 2"));
		// Any mix: press sets a text, hold goes to the next line, double press counts up.
		let (_p, mixed) = key(
			&SET_TEXT,
			json!({ "input": "T", "press": "set", "hold": "next", "double": "up", "text": "BRB", "lines": "One\nTwo", "template": "{n}" }),
		);
		assert_eq!([mixed.gesture(Gesture::Hold), mixed.gesture(Gesture::Double)], ["next", "up"]);
		mixed.press(Gesture::Press, false).await.unwrap_or_default();
		mixed.press(Gesture::Hold, false).await.unwrap_or_default();
		mixed.press(Gesture::Double, false).await.unwrap_or_default();
		assert_eq!(written(&mixed), ["BRB", "One", "1"]);
		// The key shows what was set last.
		assert_eq!(preview(&mixed, &mixed.settings()), "1");
		run(&mixed, "clear").await.unwrap_or_default();
		assert_eq!(preview(&mixed, &mixed.settings()), "");
		// Unknown values fall back: press sets the text, the others do nothing.
		let (_p, bogus) = key(&SET_TEXT, json!({ "press": "bogus", "hold": "bogus" }));
		assert_eq!(bogus.gesture(Gesture::Press), "set");
		assert_eq!(bogus.gesture(Gesture::Hold), "none");
	}

	#[test]
	fn timer_texts() {
		let (_p, k) = key(&SET_TEXT, json!({ "timerText": "Live in {t}", "timerEnd": "Live!" }));
		assert_eq!(k.settings::<TextSettings>().timer_text(65.0), "Live in 1:05");
		assert_eq!(k.settings::<TextSettings>().timer_text(0.0), "Live!");
	}
}
