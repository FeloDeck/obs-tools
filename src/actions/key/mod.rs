// One key or dial of an action: its settings, its share of the OBS connection, and the hooks of its
// action. gestures.rs turns key presses into press, hold and double press, output.rs sends what the
// key shows to OpenDeck.
mod gestures;
mod output;

use std::any::Any;
use std::cell::{Cell, RefCell, RefMut};
use std::collections::HashMap;
use std::future::Future;
use std::rc::{Rc, Weak};

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use tokio::task::spawn_local;

pub use gestures::{DOUBLE_PRESS_MS, LONG_PRESS_MS};

use super::{Behavior, Controller, Kind, PiReply};
use crate::connections::{ConnHandle, Resolved};
use crate::js::{self, truthy};
use crate::obs::{ObsClient, Reply, Status};
use crate::obs_state::{Change, Data, ObsState};
use crate::plugin::Plugin;
use crate::sd::Sd;
use crate::settings;
use crate::timers::{Timer, set_interval};
use crate::twitch::ChatMessage;
use crate::{Error, Res};

// Settings that hold names of OBS things, so renames in OBS can follow.
const INPUT_FIELDS: &[&str] = &["input", "source", "mediaInput"];
const SCENE_FIELDS: &[&str] = &["scene", "replayScene", "source", "animationScene"];

pub type KeyRc = Rc<Key>;

/// Something to call once to stop a timer, a meter or a subscription.
pub type Stop = Rc<dyn Fn()>;

/// One key or dial of an action (an instance).
pub struct Key {
	pub kind: &'static Kind,
	plugin: Weak<Plugin>,
	pub sd: Sd,
	pub context: String,
	b: RefCell<Base>,
	own: RefCell<Box<dyn Any>>,
	// Counts key ups, so a repeat that is still pressing knows the key was let go meanwhile.
	release_count: Cell<u64>,
	me: Weak<Key>,
}

#[derive(Default)]
struct Base {
	controller: Controller,
	is_in_multi_action: bool,
	state: u16,
	settings: Map<String, Value>,
	visible: bool,
	online: bool,
	conn: Option<Rc<ConnHandle>>,
	unsubscribe: Vec<Box<dyn FnOnce()>>,
	timers: HashMap<u64, Timer>,
	next_timer: u64,
	last_error: Option<String>,
	screen: Screen,
	pending: Pending,
	#[cfg(test)]
	fake_calls: Option<Vec<(String, Value)>>,
}

/// What the key shows. Keeps what was sent to OpenDeck, so the same is not sent twice.
#[derive(Default)]
struct Screen {
	title: Option<String>,
	image: Option<String>,
	state: Option<u16>,
	/// The touch strip content as JSON text. None sends it again.
	feedback: Option<String>,
	/// The key shows an image of its own, not the one from the manifest.
	image_overridden: bool,
	/// The status image, while OBS cannot be reached.
	status_shown: Option<Status>,
	/// The setup image, while the key is not set up.
	setup_shown: bool,
}

/// A press that waits for key up, the long press or a second press, to know which gesture it is.
#[derive(Default)]
struct Pending {
	/// Waiting for key up.
	waiting: bool,
	/// What a double press does.
	double: String,
	/// This press is the second of a double press.
	second: bool,
	/// Runs the first press, unless a second one comes in time.
	single_timer: Option<Timer>,
	/// The long press already ran.
	long_fired: bool,
	long_timer: Option<Timer>,
	repeat_timer: Option<Timer>,
}

/// Another OBS than the one from the setup, for keys that can have their own.
#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Connection {
	custom_connection: bool,
	host: String,
	port: f64,
	password: String,
}

/// What OpenDeck says about a key when it first shows up.
#[derive(Debug, Clone, Default)]
pub struct Appearance {
	pub controller: Option<Controller>,
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
	pub fn new(plugin: &Rc<Plugin>, kind: &'static Kind, context: &str, a: &Appearance) -> KeyRc {
		Rc::new_cyclic(|me| Key {
			kind,
			plugin: Rc::downgrade(plugin),
			sd: plugin.sd.clone(),
			context: context.to_string(),
			b: RefCell::new(Base {
				controller: a.controller.unwrap_or_default(),
				is_in_multi_action: a.is_in_multi_action,
				state: a.state.unwrap_or(0),
				settings: with_defaults(kind, a.settings.as_ref()),
				..Base::default()
			}),
			own: RefCell::new((kind.new_state)()),
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

	/// The action's own state (Kind::new_state). Do not hold it across an await.
	pub fn own<T: 'static>(&self) -> RefMut<'_, T> {
		RefMut::map(self.own.borrow_mut(), |b| b.downcast_mut::<T>().expect("state of this action"))
	}

	// ----- settings -----

	pub fn with_defaults(&self, settings: Option<&Value>) -> Map<String, Value> {
		with_defaults(self.kind, settings)
	}

	/// The settings as the action keeps them (with its defaults), for OpenDeck and the settings page.
	pub fn settings_map(&self) -> Map<String, Value> {
		self.b.borrow().settings.clone()
	}

	/// The settings as the struct of the action (see settings.rs).
	pub fn settings<T: DeserializeOwned + Default>(&self) -> T {
		settings::read(&self.b.borrow().settings)
	}

	/// One setting as it is, for settings whose name is not fixed (None: not there).
	pub fn setting(&self, key: &str) -> Option<Value> {
		self.b.borrow().settings.get(key).cloned()
	}

	pub fn required_set(&self) -> bool {
		let b = self.b.borrow();
		self.kind.required.iter().all(|key| !js::trim(&js::str_or_empty(b.settings.get(*key))).is_empty())
	}

	pub fn is_set_up(&self) -> bool {
		self.rc().is_some_and(|me| self.behavior().is_set_up(&me))
	}

	pub fn controller(&self) -> Controller {
		self.b.borrow().controller
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
		let c: Connection = self.settings();
		let host = js::trim(&c.host);
		if !c.custom_connection || host.is_empty() {
			return None;
		}
		Some(Resolved { host: host.to_string(), port: settings::or(c.port, 4455.0), password: c.password })
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
		let on_change = conn.state.on_change.on(move |change| {
			let Some(me) = me.upgrade() else { return };
			me.refresh_online();
			if me.online() && *change != Change::All {
				let change = *change;
				spawn_local(async move {
					let behavior = me.behavior();
					me.safe(behavior.on_state_change(&me, change)).await;
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

	// ----- lifecycle, from the plugin -----

	pub fn appear(&self, controller: Option<Controller>, state: Option<u16>) {
		let multi = self.is_in_multi_action();
		{
			let mut b = self.b.borrow_mut();
			b.visible = !multi;
			if let Some(state) = state {
				b.state = state;
			}
			if let Some(controller) = controller {
				b.controller = controller;
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
		let old = self.settings_map();
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

	fn follow_renames(&self, kind: &str, data: &Value) {
		let mut patch = Map::new();
		let s = self.settings_map();
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
			// The hook gets the new settings as the old ones too: a rename does not change what the key
			// points to, so nothing has to load again.
			let current = me.settings_map();
			me.safe(async {
				behavior.on_settings_changed(&me, &current).await?;
				me.draw();
				Ok(())
			})
			.await;
		});
	}

	// ----- errors -----

	pub async fn safe(&self, f: impl Future<Output = Res<()>>) {
		if let Err(error) = f.await {
			self.report_error(&error);
		}
	}

	/// Something the user did (press, turn, touch). Success clears the last error.
	pub async fn act(&self, f: impl Future<Output = Res<()>>) -> bool {
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

	// ----- timers -----

	/// A timer that stops with the key. Returns the function that stops it.
	pub fn interval(&self, ms: f64, f: impl Fn(KeyRc) -> Reply<()> + 'static) -> Stop {
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
		let id = self.timer_id();
		self.b.borrow_mut().timers.insert(id, timer);
		let me = self.me.clone();
		Rc::new(move || {
			if let Some(me) = me.upgrade()
				&& let Some(timer) = me.b.borrow_mut().timers.remove(&id)
			{
				timer.clear();
			}
		})
	}

	/// A new id for `timers`, which stop when the key goes away.
	fn timer_id(&self) -> u64 {
		let mut b = self.b.borrow_mut();
		b.next_timer += 1;
		b.next_timer
	}

	fn clear_timers(&self) {
		let mut b = self.b.borrow_mut();
		for (_, timer) in b.timers.drain() {
			timer.clear();
		}
		let p = &mut b.pending;
		for timer in [p.long_timer.take(), p.repeat_timer.take(), p.single_timer.take()].into_iter().flatten() {
			timer.clear();
		}
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

	pub async fn dial_down(self: &Rc<Self>) {
		if self.ensure_online(3000.0).await {
			let behavior = self.behavior();
			self.act(behavior.on_dial_down(self)).await;
		}
	}

	pub async fn dial_up(self: &Rc<Self>) {
		if self.ensure_online(3000.0).await {
			let behavior = self.behavior();
			// Not act(): letting go of the dial must not clear the error of the press.
			self.safe(behavior.on_dial_up(self)).await;
		}
	}

	pub async fn touch_tap(self: &Rc<Self>) {
		if self.ensure_online(3000.0).await {
			let behavior = self.behavior();
			self.act(behavior.on_touch_tap(self)).await;
		}
	}

	pub async fn pi_message(self: &Rc<Self>, payload: &Value, reply: PiReply) -> Res<()> {
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

#[cfg(test)]
pub(crate) mod testing {
	use super::*;

	/// A plugin with one key of this action, visible and online, whose OBS calls are only recorded.
	pub fn key(kind: &'static Kind, settings: Value) -> (Rc<Plugin>, KeyRc) {
		let plugin = Plugin::new(Sd::recording(), crate::actions::all());
		let key = Key::new(&plugin, kind, "c", &Appearance { settings: Some(settings), ..Default::default() });
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
		key.b.borrow().screen.setup_shown
	}

	pub async fn local<F: Future>(f: F) -> F::Output {
		tokio::task::LocalSet::new().run_until(f).await
	}
}
