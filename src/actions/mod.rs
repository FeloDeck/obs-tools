// The base of every action: connection, press, hold and double press, the setup and status images,
// and the hooks the actions fill in (see Behavior).
pub mod audio;
pub mod basics;
pub mod dials;
pub mod outputs;
pub mod scenes;
pub mod sources;
pub mod stats;

use std::any::Any;
use std::cell::{Cell, RefCell, RefMut};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use async_trait::async_trait;
use serde_json::{Map, Value};
use tokio::task::spawn_local;

use crate::connections::{ConnHandle, Resolved};
use crate::icons::colors;
use crate::js::{self, truthy};
use crate::key_images::key_image;
use crate::obs::{ObsClient, Reply};
use crate::obs_state::{Data, ObsState};
use crate::plugin::Plugin;
use crate::render::{GlyphKey, glyph_key, status_key, status_label};
use crate::sd::Sd;
use crate::timers::{Timer, set_interval, set_timeout};
use crate::twitch::ChatMessage;
use crate::{Error, Res};

pub const LONG_PRESS_MS: f64 = 700.0;
pub const DOUBLE_PRESS_MS: f64 = 300.0;
const REPEAT_DELAY_MS: f64 = 450.0;
const REPEAT_EVERY_MS: f64 = 140.0;

// Settings that hold names of OBS things, so renames in OBS can follow.
const INPUT_FIELDS: &[&str] = &["input", "source", "mediaInput"];
const SCENE_FIELDS: &[&str] = &["scene", "replayScene", "source", "animationScene"];

pub const ID: &str = crate::manifest::ID;

/// An action, like a class of the JavaScript version.
pub struct Kind {
	pub uuid: &'static str,
	/// For the log, like the class name: "RecordTools: error: not recording".
	pub name: &'static str,
	pub defaults: fn() -> Map<String, Value>,
	/// Press, hold and double press. An action that can do more than one thing lists it in `ops`
	/// and keeps the press choice in the setting `press_key`. The settings `hold` and `double` pick
	/// from the same list. on_press gets the choice as `op`.
	pub press_key: &'static str,
	pub ops: &'static [&'static str],
	/// Press choices that use the key going down and up (push to talk): no hold or double press then.
	pub press_only: &'static [&'static str],
	/// Hold can also repeat the press while the key is held ("repeat").
	pub can_repeat: bool,
	/// Settings a key needs before it can work. Until then it shows a setup image.
	pub required: &'static [&'static str],
	pub behavior: &'static dyn Behavior,
	/// The action's own state, kept per key.
	pub new_state: fn() -> Box<dyn Any>,
}

/// What a press does.
#[derive(Debug, Clone, PartialEq)]
pub struct PressArgs {
	pub op: String,
	pub gesture: String,
	pub repeat: bool,
}

/// Answers a message of the settings page, with the requestId it came with.
pub type Reply2Pi = Rc<dyn Fn(Value)>;

/// The hooks of an action. All are optional.
#[async_trait(?Send)]
pub trait Behavior: Sync {
	/// The touch strip layout of a dial (layouts/*.json).
	fn layout(&self, _k: &KeyRc) -> Option<&'static str> {
		None
	}
	/// A key is set up when its required settings are filled in.
	fn is_set_up(&self, k: &KeyRc) -> bool {
		k.required_set()
	}
	/// What press, hold or double press does.
	fn gesture(&self, k: &KeyRc, name: &str) -> String {
		k.base_gesture(name)
	}
	fn wants_chat(&self, _k: &KeyRc) -> bool {
		false
	}
	fn on_chat_message(&self, _k: &KeyRc, _message: &ChatMessage) {}
	fn render(&self, _k: &KeyRc) {}
	fn render_offline(&self, _k: &KeyRc) {}
	fn on_disappear(&self, _k: &KeyRc) {}
	fn on_disconnected(&self, _k: &KeyRc) {}
	async fn on_connected(&self, _k: &KeyRc) -> Res<()> {
		Ok(())
	}
	async fn on_state_change(&self, k: &KeyRc, _what: &str) -> Res<()> {
		k.draw();
		Ok(())
	}
	async fn on_obs_event(&self, _k: &KeyRc, _kind: &str, _data: &Value) -> Res<()> {
		Ok(())
	}
	async fn on_settings_changed(&self, _k: &KeyRc, _old: &Map<String, Value>) -> Res<()> {
		Ok(())
	}
	async fn on_press(&self, _k: &KeyRc, _args: Option<PressArgs>) -> Res<()> {
		Ok(())
	}
	async fn on_release(&self, _k: &KeyRc) -> Res<()> {
		Ok(())
	}
	async fn on_dial_rotate(&self, _k: &KeyRc, _ticks: f64, _pressed: bool) -> Res<()> {
		Ok(())
	}
	async fn on_dial_down(&self, _k: &KeyRc) -> Res<()> {
		Ok(())
	}
	async fn on_dial_up(&self, _k: &KeyRc) -> Res<()> {
		Ok(())
	}
	/// A touch does what a press does, unless touches are off.
	async fn on_touch_tap(&self, k: &KeyRc) -> Res<()> {
		if !k.s_truthy("disableTouch") {
			return self.on_dial_down(k).await;
		}
		Ok(())
	}
	async fn on_pi_message(&self, _k: &KeyRc, _payload: &Value, _reply: Reply2Pi) -> Res<()> {
		Ok(())
	}
}

pub type KeyRc = Rc<Key>;

/// Something to call once to stop a timer, a meter or a subscription.
pub type Stop = Rc<dyn Fn()>;

#[derive(Default)]
struct Base {
	controller: String,
	is_in_multi_action: bool,
	state: u16,
	settings: Map<String, Value>,
	visible: bool,
	online: bool,
	conn: Option<Rc<ConnHandle>>,
	unsubscribe: Vec<Box<dyn FnOnce()>>,
	timers: HashMap<u64, Timer>,
	next_timer: u64,
	sent: HashMap<String, Value>,
	image_overridden: bool,
	status_shown: Option<String>,
	setup_shown: bool,
	last_error: Option<String>,
	gesturing: bool,
	double_gesture: String,
	second_press: bool,
	single_timer: Option<Timer>,
	long_fired: bool,
	long_timer: Option<Timer>,
	repeat_timer: Option<Timer>,
	#[cfg(test)]
	fake_calls: Option<Vec<(String, Value)>>,
}

/// One key or dial of an action (an instance).
pub struct Key {
	pub kind: &'static Kind,
	plugin: Weak<Plugin>,
	pub sd: Sd,
	pub context: String,
	pub action: String,
	pub device: String,
	b: RefCell<Base>,
	x: RefCell<Box<dyn Any>>,
	// Counts key ups, so a repeat that is still pressing knows the key was let go meanwhile.
	release_count: Cell<u64>,
	me: Weak<Key>,
}

/// What OpenDeck says about a key when it first shows up.
#[derive(Debug, Clone, Default)]
pub struct Appearance {
	pub controller: Option<String>,
	pub is_in_multi_action: bool,
	pub state: Option<u16>,
	pub settings: Option<Value>,
}

fn with_defaults(kind: &Kind, settings: Option<&Value>) -> Map<String, Value> {
	let mut merged = (kind.defaults)();
	if let Some(Value::Object(settings)) = settings {
		js::assign(&mut merged, settings);
	}
	merged
}

impl Key {
	pub fn new(plugin: &Rc<Plugin>, kind: &'static Kind, context: &str, action: &str, device: &str, a: &Appearance) -> KeyRc {
		Rc::new_cyclic(|me| Key {
			kind,
			plugin: Rc::downgrade(plugin),
			sd: plugin.sd.clone(),
			context: context.to_string(),
			action: action.to_string(),
			device: device.to_string(),
			b: RefCell::new(Base {
				controller: a.controller.clone().unwrap_or_else(|| "Keypad".into()),
				is_in_multi_action: a.is_in_multi_action,
				state: a.state.unwrap_or(0),
				settings: with_defaults(kind, a.settings.as_ref()),
				..Base::default()
			}),
			x: RefCell::new((kind.new_state)()),
			release_count: Cell::new(0),
			me: me.clone(),
		})
	}

	fn rc(&self) -> Option<KeyRc> {
		self.me.upgrade()
	}

	pub fn plugin(&self) -> Option<Rc<Plugin>> {
		self.plugin.upgrade()
	}

	fn behavior(&self) -> &'static dyn Behavior {
		self.kind.behavior
	}

	/// The action's own state.
	pub fn x<T: 'static>(&self) -> RefMut<'_, T> {
		RefMut::map(self.x.borrow_mut(), |b| b.downcast_mut::<T>().expect("state of this action"))
	}

	// ----- settings -----

	pub fn with_defaults(&self, settings: Option<&Value>) -> Map<String, Value> {
		with_defaults(self.kind, settings)
	}

	pub fn settings(&self) -> Map<String, Value> {
		self.b.borrow().settings.clone()
	}

	/// A setting as it is (None: not there).
	pub fn s(&self, key: &str) -> Option<Value> {
		self.b.borrow().settings.get(key).cloned()
	}

	/// A setting as text, like `String(value ?? "")`.
	pub fn s_str(&self, key: &str) -> String {
		js::str_or_empty(self.b.borrow().settings.get(key))
	}

	/// A setting as a JSON value for a request, null when missing.
	pub fn s_val(&self, key: &str) -> Value {
		self.s(key).unwrap_or(Value::Null)
	}

	pub fn s_truthy(&self, key: &str) -> bool {
		truthy(self.b.borrow().settings.get(key))
	}

	/// Number(value)
	pub fn s_num(&self, key: &str) -> f64 {
		js::number(self.b.borrow().settings.get(key))
	}

	/// Number(value) || fallback
	pub fn s_num_or(&self, key: &str, fallback: f64) -> f64 {
		js::number_or(self.b.borrow().settings.get(key), fallback)
	}

	pub fn controller(&self) -> String {
		self.b.borrow().controller.clone()
	}

	pub fn is_in_multi_action(&self) -> bool {
		self.b.borrow().is_in_multi_action
	}

	pub fn visible(&self) -> bool {
		self.b.borrow().visible
	}

	pub fn online(&self) -> bool {
		self.b.borrow().online
	}

	pub fn state(&self) -> u16 {
		self.b.borrow().state
	}

	pub fn last_error(&self) -> Option<String> {
		self.b.borrow().last_error.clone()
	}

	// ----- OBS -----

	pub fn conn(&self) -> Option<Rc<ConnHandle>> {
		self.b.borrow().conn.clone()
	}

	pub fn obs(&self) -> Option<Rc<ObsClient>> {
		self.conn().map(|c| c.client.clone())
	}

	pub fn obs_state(&self) -> Option<Rc<ObsState>> {
		self.conn().map(|c| c.state.clone())
	}

	/// A copy of the OBS state, or the empty state without a connection.
	pub fn data(&self) -> Data {
		self.obs_state().map(|s| s.data()).unwrap_or_default()
	}

	pub fn log(&self, message: &str) {
		self.sd.log(&format!("{}: {message}", self.kind.name));
	}

	/// None means the default connection from the setup.
	pub fn connection_config(&self) -> Option<Resolved> {
		let b = self.b.borrow();
		let s = &b.settings;
		let host = js::str_or_empty(s.get("host"));
		if truthy(s.get("customConnection")) && !js::trim(&host).is_empty() {
			return Some(Resolved {
				host: js::trim(&js::to_string(s.get("host"))).to_string(),
				port: js::number_or(s.get("port"), 4455.0),
				password: js::str_or_empty(s.get("password")),
			});
		}
		None
	}

	/// The request is sent right away. Errors bubble up to safe(), which shows the alert.
	pub fn call(&self, request_type: &str, data: Value) -> Reply {
		#[cfg(test)]
		{
			let mut b = self.b.borrow_mut();
			if let Some(calls) = b.fake_calls.as_mut() {
				calls.push((request_type.to_string(), data));
				return Box::pin(async { Ok(Value::Object(Map::new())) });
			}
		}
		match self.obs() {
			Some(client) => client.call(request_type, data),
			None => Box::pin(async { Err(Error::from("no connection")) }),
		}
	}

	pub fn attach(&self) {
		self.detach_connection();
		let Some(plugin) = self.plugin() else { return };
		let conn = Rc::new(plugin.connections.acquire(self.connection_config()));
		self.b.borrow_mut().conn = Some(conn.clone());
		let me = self.me.clone();
		let on_status = conn.client.on_status.on(move |_| {
			if let Some(me) = me.upgrade() {
				me.refresh_online();
			}
		});
		let me = self.me.clone();
		let on_event = conn.client.on_event.on(move |(kind, data)| {
			let Some(me) = me.upgrade() else { return };
			if !me.online() {
				return;
			}
			me.follow_renames(kind, data);
			let (kind, data) = (kind.clone(), data.clone());
			spawn_local(async move {
				let behavior = me.behavior();
				me.safe(behavior.on_obs_event(&me, &kind, &data)).await;
			});
		});
		let me = self.me.clone();
		let on_change = conn.state.on_change.on(move |what| {
			let Some(me) = me.upgrade() else { return };
			me.refresh_online();
			if me.online() && what != "all" {
				let what = what.to_string();
				spawn_local(async move {
					let behavior = me.behavior();
					me.safe(behavior.on_state_change(&me, &what)).await;
				});
			}
		});
		let c = conn.clone();
		self.b.borrow_mut().unsubscribe.push(Box::new(move || {
			c.client.on_status.off(on_status);
			c.client.on_event.off(on_event);
			c.state.on_change.off(on_change);
		}));
		self.b.borrow_mut().online = false;
		self.refresh_online();
	}

	pub fn detach_connection(&self) {
		let unsubscribe = std::mem::take(&mut self.b.borrow_mut().unsubscribe);
		for f in unsubscribe {
			f();
		}
		let was_online = std::mem::replace(&mut self.b.borrow_mut().online, false);
		if was_online && let Some(me) = self.rc() {
			self.behavior().on_disconnected(&me);
		}
		let conn = self.b.borrow_mut().conn.take();
		if let Some(conn) = conn {
			conn.release();
		}
	}

	fn refresh_online(&self) {
		let Some(me) = self.rc() else { return };
		let online = self.conn().is_some_and(|c| c.client.connected() && c.state.with(|d| d.ready));
		let was = self.online();
		if online && !was {
			self.b.borrow_mut().online = true;
			self.clear_status_image();
			spawn_local(async move {
				let behavior = me.behavior();
				me.safe(async {
					behavior.on_connected(&me).await?;
					me.draw();
					Ok(())
				})
				.await;
			});
		} else if !online {
			if was {
				self.b.borrow_mut().online = false;
				self.behavior().on_disconnected(&me);
			}
			self.show_status_image();
		}
	}

	fn show_status_image(&self) {
		let status = self.obs().map(|c| c.status()).unwrap_or_default();
		if !self.visible() || status == "connecting" || status == "idle" || status.is_empty() {
			return;
		}
		self.b.borrow_mut().status_shown = Some(status.clone());
		// Dials show it as text in their offline look (see dial_status).
		if self.controller() == "Encoder" {
			if let Some(me) = self.rc() {
				self.behavior().render_offline(&me);
			}
		} else {
			self.set_image_with(&status_key(&status), None, true);
		}
	}

	fn clear_status_image(&self) {
		if self.b.borrow_mut().status_shown.take().is_none() {
			return;
		}
		// Dials draw again once they are online.
		if self.controller() == "Encoder" {
			return;
		}
		if self.b.borrow().setup_shown {
			self.set_image_with(&self.setup_image(), None, true);
		} else {
			self.reset_image(false);
		}
	}

	/// What a dial shows instead of its value while OBS cannot be reached: the words and colors
	/// of the status image of a key. Empty while connecting.
	pub fn dial_status(&self) -> Value {
		let Some(status) = self.b.borrow().status_shown.clone() else { return Value::from("") };
		crate::obj! { "value" => status_label(&status), "color" => if status == "offline" { colors::MUTED } else { colors::AMBER } }
	}

	pub fn set_layout(&self) {
		let Some(me) = self.rc() else { return };
		let layout = if self.controller() == "Encoder" { self.behavior().layout(&me) } else { None };
		let Some(layout) = layout else { return };
		self.b.borrow_mut().sent.insert("feedback".into(), Value::Null);
		self.sd.set_feedback_layout(&self.context, layout);
		self.resend_feedback_soon();
	}

	pub fn required_set(&self) -> bool {
		self.kind.required.iter().all(|key| !js::trim(&self.s_str(key)).is_empty())
	}

	pub fn is_set_up(&self) -> bool {
		self.rc().is_some_and(|me| self.behavior().is_set_up(&me))
	}

	fn icon(&self) -> String {
		crate::manifest::action_def(&self.action).map_or_else(|| "plugin".to_string(), |a| a.icon.to_string())
	}

	/// The glyph of the action in gray with a yellow dot: "pick something first".
	pub fn setup_image(&self) -> String {
		glyph_key(&self.icon(), &GlyphKey { color: colors::GRAY, setup: true, ..Default::default() })
	}

	/// Draw the key, or the setup image while it is not set up. Dials show this in their feedback.
	pub fn draw(&self) {
		if self.show_setup() {
			return;
		}
		if let Some(me) = self.rc() {
			self.behavior().render(&me);
		}
	}

	pub fn draw_offline(&self) {
		if self.show_setup() {
			return;
		}
		if let Some(me) = self.rc() {
			self.behavior().render_offline(&me);
		}
	}

	fn show_setup(&self) -> bool {
		if self.controller() != "Keypad" {
			return false;
		}
		if !self.is_set_up() {
			self.b.borrow_mut().setup_shown = true;
			if self.b.borrow().status_shown.is_none() {
				self.set_image_with(&self.setup_image(), None, true);
			}
			return true;
		}
		if self.b.borrow().setup_shown {
			self.b.borrow_mut().setup_shown = false;
			self.reset_image(true);
		}
		false
	}

	/// A key image from the manifest, drawn again with a title under the glyph.
	pub fn state_image(&self, state: u16, title: &str) -> String {
		let def = crate::manifest::action_def(&self.action);
		let name = def.and_then(|a| a.state_images().get(state as usize).cloned());
		let (glyph, color) = match name.as_deref().and_then(key_image) {
			Some(image) => (image.glyph.to_string(), image.color.unwrap_or(colors::WHITE)),
			None => (self.icon(), colors::WHITE),
		};
		glyph_key(&glyph, &GlyphKey { color, title, ..Default::default() })
	}

	/// The name of the source (or filter) as title, on keys whose pictures come from the manifest.
	pub fn show_name_title(&self, name: &str) {
		self.glyph_for_title(if self.s_truthy("showName") && !name.is_empty() { name } else { "" });
	}

	// ----- lifecycle from the plugin -----

	pub fn appear(&self, controller: Option<&str>, state: Option<u16>) {
		let multi = self.is_in_multi_action();
		{
			let mut b = self.b.borrow_mut();
			b.visible = !multi;
			if let Some(state) = state {
				b.state = state;
			}
			if let Some(controller) = controller {
				b.controller = controller.to_string();
			}
		}
		// The touch strip layout first, before any feedback is sent.
		self.set_layout();
		self.attach();
		if !self.online() {
			self.draw_offline();
		}
	}

	pub fn disappear(&self) {
		self.b.borrow_mut().visible = false;
		self.clear_timers();
		if let Some(me) = self.rc() {
			self.behavior().on_disappear(&me);
		}
		self.detach_connection();
	}

	pub fn update_settings(&self, settings: Option<&Value>) {
		let old = self.settings();
		let before = self.connection_config();
		self.b.borrow_mut().settings = self.with_defaults(settings);
		if self.connection_config() != before {
			self.attach();
		}
		let Some(me) = self.rc() else { return };
		spawn_local(async move {
			let behavior = me.behavior();
			me.safe(async {
				behavior.on_settings_changed(&me, &old).await?;
				if me.online() {
					me.draw();
				} else {
					me.draw_offline();
				}
				Ok(())
			})
			.await;
		});
	}

	pub fn save_settings(&self, patch: Map<String, Value>) {
		let settings = {
			let mut b = self.b.borrow_mut();
			js::assign(&mut b.settings, &patch);
			b.settings.clone()
		};
		self.sd.set_settings(&self.context, &settings);
	}

	/// What press, hold or double press does: one of `ops`, "none", or "repeat" for hold.
	pub fn gesture(&self, name: &str) -> String {
		match self.rc() {
			Some(me) => self.behavior().gesture(&me, name),
			None => "none".into(),
		}
	}

	pub fn base_gesture(&self, name: &str) -> String {
		let k = self.kind;
		if k.ops.is_empty() {
			return if name == "press" { "press".into() } else { "none".into() };
		}
		let pick = |v: Option<Value>| match v {
			Some(Value::String(s)) if k.ops.contains(&s.as_str()) => Some(s),
			_ => None,
		};
		let press = pick(self.s(k.press_key)).unwrap_or_else(|| k.ops[0].to_string());
		if name == "press" {
			return press;
		}
		if k.press_only.contains(&press.as_str()) {
			return "none".into();
		}
		let value = self.s(name);
		if name == "hold" && value == Some(Value::from("repeat")) && k.can_repeat {
			return "repeat".into();
		}
		match pick(value) {
			Some(v) if !k.press_only.contains(&v.as_str()) => v,
			_ => "none".into(),
		}
	}

	/// The choice for this call of on_press.
	pub fn op(&self, args: &Option<PressArgs>) -> String {
		match args {
			Some(a) => a.op.clone(),
			None => self.gesture("press"),
		}
	}

	pub async fn press(self: &Rc<Self>, name: &str, repeat: bool) -> Res<()> {
		let args = PressArgs { op: self.gesture(name), gesture: name.to_string(), repeat };
		self.behavior().on_press(self, Some(args)).await
	}

	// ----- key handling. In a multi action there are no gestures, every press is a short press. -----

	pub async fn key_down(self: &Rc<Self>, is_in_multi_action: bool) {
		self.b.borrow_mut().is_in_multi_action = is_in_multi_action;
		if !self.ensure_online(3000.0).await {
			return;
		}
		let multi = self.is_in_multi_action();
		let hold = if multi { "none".to_string() } else { self.gesture("hold") };
		let double = if multi { "none".to_string() } else { self.gesture("double") };
		if let Some(timer) = self.b.borrow_mut().repeat_timer.take() {
			timer.clear();
		}
		if double == "none" && (hold == "none" || hold == "repeat") {
			// Right away on key down. "repeat" goes on while the key is held.
			self.b.borrow_mut().gesturing = false;
			if hold == "repeat" {
				self.start_repeat(REPEAT_DELAY_MS);
			}
			self.act(self.press("press", false)).await;
			return;
		}
		// Wait for key up (or the long press) to know which gesture it is.
		{
			let mut b = self.b.borrow_mut();
			b.gesturing = true;
			b.double_gesture = double;
			// A second press while the first one waits for its double press.
			let single = b.single_timer.take();
			b.second_press = single.is_some();
			if let Some(timer) = single {
				timer.clear();
			}
			b.long_fired = false;
			if let Some(timer) = b.long_timer.take() {
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
				me.b.borrow_mut().long_fired = true;
				// The first press of the pair still counts.
				if me.b.borrow().second_press {
					me.act(me.press("press", false)).await;
				}
				if hold == "repeat" {
					me.act(me.press("press", false)).await;
					// Let go meanwhile: no repeat. (The JavaScript version kept repeating then.)
					if me.release_count.get() == releases {
						me.start_repeat(REPEAT_EVERY_MS);
					}
				} else if me.act(me.press("hold", false)).await {
					me.show_ok();
				}
			});
		});
		self.b.borrow_mut().long_timer = Some(timer);
	}

	fn start_repeat(&self, delay: f64) {
		let me = self.me.clone();
		let releases = self.release_count.get();
		let timer = set_timeout(delay, move || {
			let Some(me) = me.upgrade() else { return };
			spawn_local(async move {
				me.act(me.press("press", true)).await;
				if me.release_count.get() == releases {
					me.start_repeat(REPEAT_EVERY_MS);
				}
			});
		});
		self.b.borrow_mut().repeat_timer = Some(timer);
	}

	pub async fn key_up(self: &Rc<Self>) {
		self.release_count.set(self.release_count.get() + 1);
		if let Some(timer) = self.b.borrow_mut().repeat_timer.take() {
			timer.clear();
		}
		if self.b.borrow().gesturing {
			let (long_fired, second_press, double) = {
				let mut b = self.b.borrow_mut();
				b.gesturing = false;
				if let Some(timer) = b.long_timer.take() {
					timer.clear();
				}
				(b.long_fired, b.second_press, b.double_gesture.clone())
			};
			if long_fired || !self.online() {
				return;
			}
			if second_press {
				self.b.borrow_mut().second_press = false;
				self.act(self.press("double", false)).await;
			} else if double != "none" {
				// Wait a moment: a second press makes it a double press.
				let me = self.me.clone();
				let timer = set_timeout(DOUBLE_PRESS_MS, move || {
					let Some(me) = me.upgrade() else { return };
					me.b.borrow_mut().single_timer = None;
					spawn_local(async move {
						me.act(me.press("press", false)).await;
					});
				});
				self.b.borrow_mut().single_timer = Some(timer);
			} else {
				self.act(self.press("press", false)).await;
			}
			return;
		}
		// Not act(): letting go of the key must not clear the error of the press.
		let behavior = self.behavior();
		self.safe(behavior.on_release(self)).await;
	}

	/// A fresh instance (for example in a multi action) may still be connecting.
	pub async fn ensure_online(&self, timeout: f64) -> bool {
		if self.online() {
			return true;
		}
		if self.conn().is_none() {
			self.attach();
		}
		let until = js::now() as f64 + timeout;
		while !self.online() && (js::now() as f64) < until {
			crate::timers::sleep(50.0).await;
		}
		if !self.online() {
			self.show_alert();
		}
		self.online()
	}

	pub async fn safe(&self, f: impl std::future::Future<Output = Res<()>>) {
		if let Err(error) = f.await {
			self.report_error(&error);
		}
	}

	/// Something the user did (press, turn, touch). Success clears the last error.
	pub async fn act(&self, f: impl std::future::Future<Output = Res<()>>) -> bool {
		match f.await {
			Ok(()) => {
				self.clear_error();
				true
			}
			Err(error) => {
				self.report_error(&error);
				false
			}
		}
	}

	/// The settings page shows the last error, so the user sees why the key showed an alert.
	pub fn report_error(&self, error: &Error) {
		let message = error.message().to_string();
		self.log(&format!("error: {message}"));
		self.show_alert();
		if self.b.borrow().last_error.as_deref() == Some(message.as_str()) {
			return;
		}
		self.b.borrow_mut().last_error = Some(message);
		if let Some(plugin) = self.plugin() {
			plugin.state_changed(&self.context);
		}
	}

	pub fn clear_error(&self) {
		if self.b.borrow_mut().last_error.take().is_none() {
			return;
		}
		if let Some(plugin) = self.plugin() {
			plugin.state_changed(&self.context);
		}
	}

	fn follow_renames(&self, kind: &str, data: &Value) {
		let mut patch = Map::new();
		let s = self.settings();
		let follow = |patch: &mut Map<String, Value>, fields: &[&str], old: &str, new: &str| {
			for key in fields {
				if truthy(s.get(*key)) && s.get(*key) == data.get(old) {
					patch.insert(key.to_string(), data.get(new).cloned().unwrap_or(Value::Null));
				}
			}
		};
		match kind {
			"InputNameChanged" => follow(&mut patch, INPUT_FIELDS, "oldInputName", "inputName"),
			"SceneNameChanged" => follow(&mut patch, SCENE_FIELDS, "oldSceneName", "sceneName"),
			"SourceFilterNameChanged" if s.get("filter") == data.get("oldFilterName") && s.get("source") == data.get("sourceName") => {
				patch.insert("filter".into(), data.get("filterName").cloned().unwrap_or(Value::Null));
			}
			_ => {}
		}
		if patch.is_empty() {
			return;
		}
		self.save_settings(patch);
		let Some(me) = self.rc() else { return };
		spawn_local(async move {
			let behavior = me.behavior();
			// Like the JavaScript version: the hook gets the new settings as the old ones.
			let current = me.settings();
			me.safe(async {
				behavior.on_settings_changed(&me, &current).await?;
				me.draw();
				Ok(())
			})
			.await;
		});
	}

	/// A timer that stops with the key. Returns the function that stops it.
	pub fn interval(&self, ms: f64, f: impl Fn(KeyRc) -> crate::obs::Reply<()> + 'static) -> Stop {
		let me = self.me.clone();
		let f = Rc::new(f);
		let timer = set_interval(ms, move || {
			let Some(me) = me.upgrade() else { return };
			let f = f.clone();
			spawn_local(async move {
				let future = f(me.clone());
				me.safe(future).await;
			});
		});
		let id = {
			let mut b = self.b.borrow_mut();
			b.next_timer += 1;
			let id = b.next_timer;
			b.timers.insert(id, timer);
			id
		};
		let me = self.me.clone();
		Rc::new(move || {
			if let Some(me) = me.upgrade()
				&& let Some(timer) = me.b.borrow_mut().timers.remove(&id)
			{
				timer.clear();
			}
		})
	}

	fn clear_timers(&self) {
		let mut b = self.b.borrow_mut();
		for (_, timer) in b.timers.drain() {
			timer.clear();
		}
		for timer in [b.long_timer.take(), b.repeat_timer.take(), b.single_timer.take()].into_iter().flatten() {
			timer.clear();
		}
	}

	// ----- output helpers. They skip sends that would not change anything. -----

	fn set_title(&self, title: &str) {
		if !self.visible() {
			return;
		}
		{
			let mut b = self.b.borrow_mut();
			let key = "title:undefined";
			if b.sent.get(key) == Some(&Value::from(title)) {
				return;
			}
			b.sent.insert(key.into(), title.into());
			for (k, v) in b.sent.iter_mut() {
				if k.starts_with("title:") {
					*v = title.into();
				}
			}
		}
		self.sd.set_title(&self.context, title, None);
	}

	/// Keys draw their titles themselves, close to the glyph. A title in OpenDeck would be a second
	/// one (older versions set it), so it is cleared while the key shows its own. Without one, a title
	/// the user typed stays.
	pub fn own_title(&self, title: &str) {
		if !title.is_empty() {
			self.set_title("");
		}
	}

	pub fn set_image(&self, image: &str) {
		self.set_image_with(image, None, false);
	}

	/// force: also while the setup or status image is shown.
	pub fn set_image_with(&self, image: &str, state: Option<u16>, force: bool) {
		if !self.visible() {
			return;
		}
		{
			let mut b = self.b.borrow_mut();
			if !force && (b.setup_shown || b.status_shown.is_some()) {
				return;
			}
			let key = match state {
				Some(s) => format!("image:{s}"),
				None => "image:undefined".into(),
			};
			if b.sent.get(&key) == Some(&Value::from(image)) {
				return;
			}
			b.sent.insert(key, image.into());
			if state.is_none() {
				for (k, v) in b.sent.iter_mut() {
					if k.starts_with("image:") {
						*v = image.into();
					}
				}
			}
			b.image_overridden = true;
		}
		self.sd.set_image(&self.context, image, state);
	}

	/// The manifest pictures have the glyph in the middle. With a title, draw it with the title below.
	pub fn glyph_for_title(&self, title: &str) {
		self.own_title(title);
		if !title.is_empty() {
			self.set_image(&self.state_image(self.state(), title));
		} else {
			self.reset_image(false);
		}
	}

	/// Back to the image from the manifest (or the one the user picked).
	pub fn reset_image(&self, force: bool) {
		{
			let mut b = self.b.borrow_mut();
			if !force && (b.setup_shown || b.status_shown.is_some()) {
				return;
			}
			if !b.image_overridden {
				return;
			}
			b.image_overridden = false;
			b.sent.retain(|k, _| !k.starts_with("image:"));
		}
		if self.visible() {
			self.sd.set_image(&self.context, "", None);
		}
	}

	pub fn set_state(&self, state: u16) {
		{
			let mut b = self.b.borrow_mut();
			if !b.visible && !b.is_in_multi_action {
				return;
			}
			if b.state == state && b.sent.get("state") == Some(&Value::from(state)) {
				return;
			}
			b.state = state;
			b.sent.insert("state".into(), state.into());
		}
		self.sd.set_state(&self.context, state);
	}

	/// OpenDeck loads a custom touch strip layout again after the key appeared, which drops what
	/// was sent before. So send it again a little later.
	fn resend_feedback_soon(&self) {
		for ms in [700.0, 2500.0] {
			let me = self.me.clone();
			let id = {
				let mut b = self.b.borrow_mut();
				b.next_timer += 1;
				b.next_timer
			};
			let timer = set_timeout(ms, move || {
				let Some(me) = me.upgrade() else { return };
				me.b.borrow_mut().timers.remove(&id);
				if !me.visible() {
					return;
				}
				me.b.borrow_mut().sent.insert("feedback".into(), Value::Null);
				let behavior = me.behavior();
				if me.online() {
					behavior.render(&me);
				} else {
					behavior.render_offline(&me);
				}
			});
			self.b.borrow_mut().timers.insert(id, timer);
		}
	}

	pub fn set_feedback(&self, payload: Value) {
		if !self.visible() {
			return;
		}
		let text = js::stringify(&payload);
		{
			let mut b = self.b.borrow_mut();
			if b.sent.get("feedback") == Some(&Value::from(text.as_str())) {
				return;
			}
			b.sent.insert("feedback".into(), text.into());
		}
		self.sd.set_feedback(&self.context, payload);
	}

	pub fn show_ok(&self) {
		if self.visible() {
			self.sd.show_ok(&self.context);
		}
	}

	pub fn show_alert(&self) {
		if self.visible() {
			self.sd.show_alert(&self.context);
		}
	}

	pub fn send_to_pi(&self, payload: Value) {
		self.sd.send_to_property_inspector(&self.context, payload);
	}

	// ----- hooks, for the plugin -----

	pub fn wants_chat(&self) -> bool {
		self.rc().is_some_and(|me| self.behavior().wants_chat(&me))
	}

	pub fn on_chat_message(&self, message: &ChatMessage) {
		if let Some(me) = self.rc() {
			self.behavior().on_chat_message(&me, message);
		}
	}

	pub async fn dial_rotate(self: &Rc<Self>, ticks: f64, pressed: bool) {
		if self.ensure_online(3000.0).await {
			let behavior = self.behavior();
			self.act(behavior.on_dial_rotate(self, ticks, pressed)).await;
		}
	}

	pub async fn dial_event(self: &Rc<Self>, event: &str) {
		if !self.ensure_online(3000.0).await {
			return;
		}
		let behavior = self.behavior();
		match event {
			"dialDown" => self.act(behavior.on_dial_down(self)).await,
			"dialUp" => self.act(behavior.on_dial_up(self)).await,
			_ => self.act(behavior.on_touch_tap(self)).await,
		};
	}

	pub async fn pi_message(self: &Rc<Self>, payload: &Value, reply: Reply2Pi) -> Res<()> {
		self.behavior().on_pi_message(self, payload, reply).await
	}

	/// Render now, for actions that draw from their own state.
	pub fn render(self: &Rc<Self>) {
		self.behavior().render(self);
	}

	pub fn render_offline(self: &Rc<Self>) {
		self.behavior().render_offline(self);
	}
}

/// Makes a map from pairs, for settings patches.
pub fn patch(pairs: Vec<(&str, Value)>) -> Map<String, Value> {
	pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

/// The defaults of an action from a JSON object.
pub fn defaults(value: Value) -> Map<String, Value> {
	match value {
		Value::Object(map) => map,
		_ => Map::new(),
	}
}

/// All actions, in the order of the JavaScript version.
pub fn all() -> Vec<&'static Kind> {
	vec![
		&scenes::SMART_SCENE,
		&sources::SOURCE_ANIMATION,
		&scenes::STUDIO_MODE,
		&sources::VIDEO_PLAYER,
		&sources::IMAGE_SOURCE,
		&audio::SOURCE_VOLUME,
		&audio::INPUT_VOLUME,
		&audio::MUTE_TOGGLE,
		&audio::MONITOR_SET,
		&stats::DROPPED_FRAMES,
		&stats::CPU_USAGE,
		&scenes::PREVIOUS_SCENE,
		&scenes::NEXT_SCENE,
		&outputs::INSTANT_REPLAY,
		&sources::BROWSER_SOURCE,
		&outputs::RECORD_TOGGLE,
		&outputs::STREAM_TOGGLE,
		&scenes::SET_TRANSITION,
		&sources::FILTER_TOGGLE,
		&outputs::VIRTUAL_CAM,
		&sources::FOCUSED_WINDOW,
		&basics::SOURCE_VISIBILITY,
		&basics::HOTKEY,
		&basics::SCREENSHOT,
		&basics::RECORD_TOOLS,
		&basics::MEDIA_CONTROL,
		&basics::PROFILE_SWITCH,
		&basics::SET_TEXT,
		&stats::STREAM_HEALTH,
		&dials::TRANSITION_DIAL,
		&dials::SCENE_DIAL,
		&dials::FILTER_DIAL,
	]
}

#[cfg(test)]
pub(crate) mod testing {
	use super::*;

	/// A plugin with one key of this action, visible and online, whose OBS calls are only recorded.
	pub fn key(kind: &'static Kind, settings: Value) -> (Rc<Plugin>, KeyRc) {
		let plugin = Plugin::new(Sd::recording(), all());
		let key = Key::new(&plugin, kind, "c", kind.uuid, "", &Appearance { settings: Some(settings), ..Default::default() });
		{
			let mut b = key.b.borrow_mut();
			b.visible = true;
			b.online = true;
			b.fake_calls = Some(Vec::new());
		}
		(plugin, key)
	}

	pub fn calls(key: &Key) -> Vec<(String, Value)> {
		key.b.borrow().fake_calls.clone().unwrap_or_default()
	}

	pub fn set(key: &Key, name: &str, value: Value) {
		key.b.borrow_mut().settings.insert(name.into(), value);
	}

	pub fn sent(plugin: &Plugin, event: &str) -> Vec<Value> {
		plugin
			.sd
			.sent
			.as_ref()
			.map(|s| s.borrow().iter().filter(|m| m["event"] == event).map(|m| m["payload"].clone()).collect())
			.unwrap_or_default()
	}

	pub fn setup_shown(key: &Key) -> bool {
		key.b.borrow().setup_shown
	}
}

#[cfg(test)]
mod tests {
	use super::testing::*;
	use super::*;
	use serde_json::json;

	struct Probe;

	#[derive(Default)]
	struct ProbeState {
		seen: Vec<String>,
	}

	#[async_trait(?Send)]
	impl Behavior for Probe {
		async fn on_press(&self, k: &KeyRc, args: Option<PressArgs>) -> Res<()> {
			let op = k.op(&args);
			k.x::<ProbeState>().seen.push(op);
			Ok(())
		}
	}

	static PROBE: Kind = Kind {
		uuid: "de.felitendo.obs-tools.probe",
		name: "Probe",
		defaults: Map::new,
		press_key: "mode",
		ops: &["a", "b"],
		press_only: &[],
		can_repeat: false,
		required: &[],
		behavior: &Probe,
		new_state: || Box::new(ProbeState::default()),
	};

	fn seen(key: &Key) -> Vec<String> {
		key.x::<ProbeState>().seen.clone()
	}

	async fn local<F: std::future::Future>(f: F) -> F::Output {
		tokio::task::LocalSet::new().run_until(f).await
	}

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
}
