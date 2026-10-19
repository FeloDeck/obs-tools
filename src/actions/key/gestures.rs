// Press, hold and double press. In a multi action there are no gestures, every press is a press.
use std::rc::Rc;

use serde_json::Value;
use tokio::task::spawn_local;

use super::Key;
use crate::Res;
use crate::actions::{Gesture, Press};
use crate::timers::set_timeout;

pub const LONG_PRESS_MS: f64 = 700.0;
pub const DOUBLE_PRESS_MS: f64 = 300.0;
const REPEAT_DELAY_MS: f64 = 450.0;
const REPEAT_EVERY_MS: f64 = 140.0;

impl Key {
	/// What press, hold or double press does: one of `ops`, "none", or "repeat" for hold.
	pub fn gesture(&self, gesture: Gesture) -> String {
		match self.rc() {
			Some(me) => self.behavior().gesture(&me, gesture),
			None => "none".into(),
		}
	}

	pub fn base_gesture(&self, gesture: Gesture) -> String {
		let k = self.kind;
		if k.ops.is_empty() {
			return if gesture == Gesture::Press { "press".into() } else { "none".into() };
		}
		let pick = |v: Option<Value>| match v {
			Some(Value::String(s)) if k.ops.contains(&s.as_str()) => Some(s),
			_ => None,
		};
		let press = pick(self.setting(k.press_key)).unwrap_or_else(|| k.ops[0].to_string());
		if gesture == Gesture::Press {
			return press;
		}
		if k.press_only.contains(&press.as_str()) {
			return "none".into();
		}
		let value = self.setting(gesture.name());
		if gesture == Gesture::Hold && value == Some(Value::from("repeat")) && k.can_repeat {
			return "repeat".into();
		}
		match pick(value) {
			Some(v) if !k.press_only.contains(&v.as_str()) => v,
			_ => "none".into(),
		}
	}

	/// Does what the gesture does, right now.
	pub async fn press(self: &Rc<Self>, gesture: Gesture, repeat: bool) -> Res<()> {
		let press = Press { op: self.gesture(gesture), repeat };
		self.behavior().on_press(self, press).await
	}

	pub async fn key_down(self: &Rc<Self>, is_in_multi_action: bool) {
		self.b.borrow_mut().is_in_multi_action = is_in_multi_action;
		if !self.ensure_online(3000.0).await {
			return;
		}
		let multi = self.is_in_multi_action();
		let hold = if multi { "none".to_string() } else { self.gesture(Gesture::Hold) };
		let double = if multi { "none".to_string() } else { self.gesture(Gesture::Double) };
		if let Some(timer) = self.b.borrow_mut().pending.repeat_timer.take() {
			timer.clear();
		}
		if double == "none" && (hold == "none" || hold == "repeat") {
			// Right away on key down. "repeat" goes on while the key is held.
			self.b.borrow_mut().pending.waiting = false;
			if hold == "repeat" {
				self.start_repeat(REPEAT_DELAY_MS);
			}
			self.act(self.press(Gesture::Press, false)).await;
			return;
		}
		// Wait for key up (or the long press) to know which gesture it is.
		{
			let mut b = self.b.borrow_mut();
			let p = &mut b.pending;
			p.waiting = true;
			p.double = double;
			// A second press while the first one waits for its double press.
			let single = p.single_timer.take();
			p.second = single.is_some();
			if let Some(timer) = single {
				timer.clear();
			}
			p.long_fired = false;
			if let Some(timer) = p.long_timer.take() {
				timer.clear();
			}
		}
		if hold == "none" {
			return;
		}
		let me = self.me.clone();
		let releases = self.release_count.get();
		let delay = if hold == "repeat" { REPEAT_DELAY_MS } else { LONG_PRESS_MS };
		let timer = set_timeout(delay, move || {
			let Some(me) = me.upgrade() else { return };
			spawn_local(async move {
				me.b.borrow_mut().pending.long_fired = true;
				// The first press of the pair still counts.
				if me.b.borrow().pending.second {
					me.act(me.press(Gesture::Press, false)).await;
				}
				if hold == "repeat" {
					me.act(me.press(Gesture::Press, false)).await;
					// Let go meanwhile: no repeat.
					if me.release_count.get() == releases {
						me.start_repeat(REPEAT_EVERY_MS);
					}
				} else if me.act(me.press(Gesture::Hold, false)).await {
					me.show_ok();
				}
			});
		});
		self.b.borrow_mut().pending.long_timer = Some(timer);
	}

	fn start_repeat(&self, delay: f64) {
		let me = self.me.clone();
		let releases = self.release_count.get();
		let timer = set_timeout(delay, move || {
			let Some(me) = me.upgrade() else { return };
			spawn_local(async move {
				me.act(me.press(Gesture::Press, true)).await;
				if me.release_count.get() == releases {
					me.start_repeat(REPEAT_EVERY_MS);
				}
			});
		});
		self.b.borrow_mut().pending.repeat_timer = Some(timer);
	}

	pub async fn key_up(self: &Rc<Self>) {
		self.release_count.set(self.release_count.get() + 1);
		if let Some(timer) = self.b.borrow_mut().pending.repeat_timer.take() {
			timer.clear();
		}
		if self.b.borrow().pending.waiting {
			let (long_fired, second, double) = {
				let mut b = self.b.borrow_mut();
				let p = &mut b.pending;
				p.waiting = false;
				if let Some(timer) = p.long_timer.take() {
					timer.clear();
				}
				(p.long_fired, p.second, p.double.clone())
			};
			if long_fired || !self.online() {
				return;
			}
			if second {
				self.b.borrow_mut().pending.second = false;
				self.act(self.press(Gesture::Double, false)).await;
			} else if double != "none" {
				// Wait a moment: a second press makes it a double press.
				let me = self.me.clone();
				let timer = set_timeout(DOUBLE_PRESS_MS, move || {
					let Some(me) = me.upgrade() else { return };
					me.b.borrow_mut().pending.single_timer = None;
					spawn_local(async move {
						me.act(me.press(Gesture::Press, false)).await;
					});
				});
				self.b.borrow_mut().pending.single_timer = Some(timer);
			} else {
				self.act(self.press(Gesture::Press, false)).await;
			}
			return;
		}
		// Not act(): letting go of the key must not clear the error of the press.
		let behavior = self.behavior();
		self.safe(behavior.on_release(self)).await;
	}
}

#[cfg(test)]
mod tests {
	use async_trait::async_trait;
	use serde_json::json;

	use super::super::testing::*;
	use super::*;
	use crate::Error;
	use crate::actions::{Behavior, KeyRc, Kind};

	struct Probe;

	#[derive(Default)]
	struct ProbeState {
		seen: Vec<String>,
	}

	#[async_trait(?Send)]
	impl Behavior for Probe {
		async fn on_press(&self, k: &KeyRc, press: Press) -> Res<()> {
			k.own::<ProbeState>().seen.push(press.op);
			Ok(())
		}
	}

	static PROBE: Kind = Kind {
		uuid: "de.felitendo.obs-tools.probe",
		name: "Probe",
		press_key: "mode",
		ops: &["a", "b"],
		behavior: &Probe,
		new_state: || Box::new(ProbeState::default()),
		..Kind::BASE
	};

	fn seen(key: &Key) -> Vec<String> {
		key.own::<ProbeState>().seen.clone()
	}

	struct Broken;

	#[async_trait(?Send)]
	impl Behavior for Broken {
		async fn on_dial_down(&self, _k: &KeyRc) -> Res<()> {
			Err(Error::from("no source selected"))
		}
	}

	static BROKEN: Kind = Kind { name: "Broken", behavior: &Broken, new_state: || Box::new(()), ..PROBE };

	#[tokio::test(start_paused = true)]
	async fn a_second_press_within_a_moment_is_a_double_press() {
		local(async {
			let (_plugin, key) = key(&PROBE, json!({ "mode": "a", "double": "b" }));
			let tap = || async {
				key.key_down(false).await;
				key.key_up().await;
			};
			tap().await;
			crate::timers::sleep(DOUBLE_PRESS_MS + 100.0).await;
			assert_eq!(seen(&key), ["a"]);
			tap().await;
			tap().await;
			crate::timers::sleep(DOUBLE_PRESS_MS + 100.0).await;
			assert_eq!(seen(&key), ["a", "b"]);
		})
		.await;
	}

	#[tokio::test(start_paused = true)]
	async fn holding_runs_the_hold_choice_a_short_press_the_press_choice() {
		local(async {
			let (_plugin, k) = key(&PROBE, json!({ "mode": "a", "hold": "b" }));
			k.key_down(false).await;
			crate::timers::sleep(LONG_PRESS_MS + 100.0).await;
			k.key_up().await;
			k.key_down(false).await;
			k.key_up().await;
			assert_eq!(seen(&k), ["b", "a"]);
			// Without hold and double press the press runs right on key down.
			let (_plugin, plain) = key(&PROBE, json!({ "mode": "b" }));
			plain.key_down(false).await;
			assert_eq!(seen(&plain), ["b"]);
			// In a multi action there are no gestures.
			let (_plugin, multi) = key(&PROBE, json!({ "mode": "a", "hold": "b" }));
			multi.key_down(true).await;
			assert_eq!(seen(&multi), ["a"]);
		})
		.await;
	}

	#[tokio::test]
	async fn letting_go_of_the_dial_keeps_the_error_of_the_press() {
		local(async {
			let (_plugin, k) = key(&BROKEN, json!({}));
			k.dial_down().await;
			k.dial_up().await;
			assert_eq!(k.last_error().as_deref(), Some("no source selected"));
		})
		.await;
	}
}
