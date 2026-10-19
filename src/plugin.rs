// Routes OpenDeck events to the keys and answers the settings pages.
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use serde_json::{Map, Value, json};
use tokio::task::spawn_local;

use crate::actions::{Appearance, Controller, Key, KeyRc, Kind, PiReply};
use crate::connections::{
	ConnectionManager, Resolved, connection_key, default_global, local_config_paths, read_local_obs_config, resolve_connection,
};
use crate::desktop::{pick_file, pick_folder, session_type};
use crate::js::{self, truthy};
use crate::lists::lists;
use crate::obs::Status;
use crate::sd::Sd;
use crate::twitch::Twitch;

/// What happened to a key.
#[derive(Debug, Clone)]
pub enum EventKind {
	WillAppear,
	WillDisappear,
	KeyDown,
	KeyUp,
	DialRotate {
		ticks: f64,
		pressed: bool,
	},
	DialDown,
	DialUp,
	TouchTap,
	DidReceiveSettings,
	InspectorDidAppear,
	InspectorDidDisappear,
	/// A message of the settings page.
	SendToPlugin(Value),
}

/// An event from OpenDeck for one key.
#[derive(Debug, Clone)]
pub struct Event {
	pub kind: EventKind,
	pub action: String,
	pub context: String,
	pub controller: Option<Controller>,
	pub is_in_multi_action: bool,
	pub state: Option<u16>,
	pub settings: Option<Value>,
}

pub struct Plugin {
	pub sd: Sd,
	kinds: HashMap<&'static str, &'static Kind>,
	instances: RefCell<HashMap<String, KeyRc>>,
	open_inspectors: RefCell<Vec<String>>,
	global_settings: RefCell<Map<String, Value>>,
	pub connections: Rc<ConnectionManager>,
	pub twitch: Rc<Twitch>,
	me: Weak<Plugin>,
}

impl Plugin {
	pub fn new(sd: Sd, kinds: Vec<&'static Kind>) -> Rc<Self> {
		let plugin = Rc::new_cyclic(|me: &Weak<Plugin>| {
			let log_sd = sd.clone();
			let save = me.clone();
			let twitch_log = sd.clone();
			Plugin {
				kinds: kinds.into_iter().map(|k| (k.uuid, k)).collect(),
				instances: RefCell::new(HashMap::new()),
				open_inspectors: RefCell::new(Vec::new()),
				global_settings: RefCell::new(default_global()),
				connections: ConnectionManager::new(move |m| log_sd.log(m)),
				twitch: Twitch::new(
					move |config| {
						if let Some(plugin) = save.upgrade() {
							plugin.save_global(crate::actions::patch(vec![("twitch", Value::Object(config))]));
						}
					},
					move |m| twitch_log.log(m),
				),
				sd,
				me: me.clone(),
			}
		});
		let me = Rc::downgrade(&plugin);
		plugin.connections.on_default_changed.on({
			let me = me.clone();
			move |config| {
				let Some(plugin) = me.upgrade() else { return };
				let key = connection_key(config);
				for instance in plugin.keys() {
					if instance.connection_config().is_none() && instance.conn().is_some_and(|c| c.key != key) {
						instance.attach();
					}
				}
				plugin.broadcast_state();
			}
		});
		plugin.connections.on_status.on({
			let me = me.clone();
			move |_| {
				if let Some(plugin) = me.upgrade() {
					plugin.broadcast_state();
				}
			}
		});
		plugin.twitch.on_status.on({
			let me = me.clone();
			move |_| {
				if let Some(plugin) = me.upgrade() {
					plugin.broadcast_state();
				}
			}
		});
		plugin.twitch.on_error.on({
			let me = me.clone();
			move |error| {
				if let Some(plugin) = me.upgrade() {
					plugin.sd.warn(&format!("Twitch: {error}"));
					plugin.broadcast(json!({ "type": "twitchError", "message": error.message() }));
				}
			}
		});
		plugin.twitch.on_message.on(move |message| {
			if let Some(plugin) = me.upgrade() {
				for instance in plugin.keys() {
					instance.on_chat_message(message);
				}
			}
		});
		plugin
	}

	fn keys(&self) -> Vec<KeyRc> {
		self.instances.borrow().values().cloned().collect()
	}

	pub fn key(&self, context: &str) -> Option<KeyRc> {
		self.instances.borrow().get(context).cloned()
	}

	pub fn save_global(&self, patch: Map<String, Value>) {
		let settings = {
			let mut global = self.global_settings.borrow_mut();
			js::assign(&mut global, &patch);
			global.clone()
		};
		self.sd.set_global_settings(&settings);
		self.connections.set_global_settings(&settings);
		self.twitch.configure(settings.get("twitch"));
		self.broadcast_state();
	}

	fn instance_for(&self, e: &Event) -> Option<KeyRc> {
		if let Some(instance) = self.key(&e.context) {
			return Some(instance);
		}
		let kind = self.kinds.get(e.action.as_str())?;
		let plugin = self.me.upgrade()?;
		let appearance = if matches!(e.kind, EventKind::SendToPlugin(_)) {
			Appearance::default()
		} else {
			Appearance { controller: e.controller, is_in_multi_action: e.is_in_multi_action, state: e.state, settings: e.settings.clone() }
		};
		let instance = Key::new(&plugin, kind, &e.context, &appearance);
		self.instances.borrow_mut().insert(e.context.clone(), instance.clone());
		// Multi action children never get willAppear, they only press.
		if !matches!(e.kind, EventKind::WillAppear) {
			instance.attach();
		}
		Some(instance)
	}

	// Press events carry the settings, which matters for multi action children.
	fn with_settings(&self, e: &Event) -> Option<KeyRc> {
		let instance = self.instance_for(e)?;
		if let Some(settings) = e.settings.as_ref().filter(|s| truthy(Some(s))) {
			let merged = Value::Object(instance.with_defaults(Some(settings)));
			if !js::same(&merged, &Value::Object(instance.settings())) {
				instance.update_settings(Some(settings));
			}
		}
		Some(instance)
	}

	fn update_chat_needs(&self) {
		let wanted = self.keys().iter().any(|k| k.wants_chat());
		self.twitch.set_chat_wanted(wanted);
	}

	pub fn on_global_settings(&self, settings: Option<&Value>) {
		let mut global = default_global();
		js::assign(&mut global, &js::object(settings));
		*self.global_settings.borrow_mut() = global.clone();
		self.connections.set_global_settings(&global);
		self.twitch.configure(global.get("twitch"));
		self.broadcast_state();
	}

	pub fn on_wake_up(&self) {
		for client in self.connections.clients() {
			client.reconnect();
		}
	}

	pub async fn on_message(self: Rc<Self>, e: Event) {
		match &e.kind {
			EventKind::WillAppear => {
				if let Some(instance) = self.instance_for(&e) {
					instance.appear(e.controller, e.state);
				}
				self.update_chat_needs();
			}
			EventKind::WillDisappear => {
				let instance = self.instances.borrow_mut().remove(&e.context);
				if let Some(instance) = instance {
					instance.disappear();
				}
				self.update_chat_needs();
			}
			EventKind::KeyDown => {
				if let Some(instance) = self.with_settings(&e) {
					instance.key_down(e.is_in_multi_action).await;
				}
			}
			EventKind::KeyUp => {
				if let Some(instance) = self.with_settings(&e) {
					instance.key_up().await;
				}
			}
			EventKind::DialRotate { ticks, pressed } => {
				if let Some(instance) = self.with_settings(&e) {
					instance.dial_rotate(*ticks, *pressed).await;
				}
			}
			EventKind::DialDown => {
				if let Some(instance) = self.with_settings(&e) {
					instance.dial_down().await;
				}
			}
			EventKind::DialUp => {
				if let Some(instance) = self.with_settings(&e) {
					instance.dial_up().await;
				}
			}
			EventKind::TouchTap => {
				if let Some(instance) = self.with_settings(&e) {
					instance.touch_tap().await;
				}
			}
			EventKind::DidReceiveSettings => {
				if let Some(instance) = self.instance_for(&e) {
					instance.update_settings(e.settings.as_ref());
				}
				self.update_chat_needs();
			}
			EventKind::InspectorDidAppear => {
				self.add_inspector(&e.context);
				self.send_state(&e.context);
			}
			EventKind::InspectorDidDisappear => {
				self.open_inspectors.borrow_mut().retain(|c| *c != e.context);
			}
			EventKind::SendToPlugin(payload) => self.on_inspector_message(&e, payload).await,
		}
	}

	fn add_inspector(&self, context: &str) {
		let mut open = self.open_inspectors.borrow_mut();
		if !open.iter().any(|c| c == context) {
			open.push(context.to_string());
		}
	}

	/// State shown at the top of every settings page.
	pub fn state_for(&self, context: &str) -> Value {
		let instance = self.key(context);
		let client = instance.as_ref().and_then(|i| i.obs());
		let resolved = self.connections.default_config();
		let global = self.global_settings.borrow().clone();
		json!({
			"type": "state",
			"connection": {
				"status": client.as_ref().map_or(Status::Idle, |c| c.status()).name(),
				"error": client.as_ref().map(|c| c.last_error()).unwrap_or_default(),
				"version": client.as_ref().and_then(|c| c.version()).unwrap_or(Value::Null),
				"host": client.as_ref().map_or(resolved.host, |c| c.host.clone()),
				"port": js::num(client.as_ref().map_or(resolved.port, |c| c.port)),
				"custom": instance.as_ref().is_some_and(|i| i.connection_config().is_some()),
			},
			"global": {
				"host": global.get("host").cloned().unwrap_or(Value::Null),
				"port": global.get("port").cloned().unwrap_or(Value::Null),
				"hasPassword": truthy(global.get("password")),
			},
			"twitch": self.twitch.status(),
			"session": session_type(),
			"settings": instance.as_ref().map_or(Value::Null, |i| Value::Object(i.settings())),
			"keyError": instance.as_ref().and_then(|i| i.last_error()).map_or(Value::Null, Value::from),
		})
	}

	/// Something on the settings page of this key changed, like its last error.
	pub fn state_changed(&self, context: &str) {
		if self.open_inspectors.borrow().iter().any(|c| c == context) {
			self.send_state(context);
		}
	}

	fn send_state(&self, context: &str) {
		self.sd.send_to_property_inspector(context, self.state_for(context));
	}

	fn broadcast_state(&self) {
		let open = self.open_inspectors.borrow().clone();
		for context in open {
			self.send_state(&context);
		}
	}

	fn broadcast(&self, payload: Value) {
		let open = self.open_inspectors.borrow().clone();
		for context in open {
			self.sd.send_to_property_inspector(&context, payload.clone());
		}
	}

	fn reconnect_default(&self) {
		if let Some(client) = self.connections.client(&connection_key(&self.connections.default_config())) {
			client.reconnect();
		}
	}

	async fn on_inspector_message(self: &Rc<Self>, e: &Event, payload: &Value) {
		let context = e.context.clone();
		let payload = if payload.is_object() { payload.clone() } else { json!({}) };
		let instance = self.instance_for(e);
		let sd = self.sd.clone();
		let request_id = payload.get("requestId").cloned();
		let reply_context = context.clone();
		let reply: PiReply = Rc::new(move |data: Value| {
			let mut data = js::object(Some(&data));
			if let Some(id) = &request_id {
				data.insert("requestId".into(), id.clone());
			}
			sd.send_to_property_inspector(&reply_context, Value::Object(data));
		});
		self.add_inspector(&context);
		let get = |key: &str| payload.get(key);
		match get("type").and_then(Value::as_str).unwrap_or("") {
			"hello" => self.send_state(&context),
			"getLists" => {
				let names: Vec<String> = match get("lists") {
					Some(Value::Array(names)) => names.iter().map(|n| js::to_string(Some(n))).collect(),
					_ => Vec::new(),
				};
				let lists = lists(instance.as_ref(), &names, &payload).await;
				reply(json!({ "type": "lists", "lists": lists }));
			}
			"saveConnection" => {
				let settings = js::object(get("settings"));
				let host = js::trim(&js::str_or_empty(settings.get("host"))).to_string();
				let mut patch = Map::new();
				patch.insert("host".into(), if host.is_empty() { "127.0.0.1".into() } else { host.into() });
				patch.insert("port".into(), js::num(js::number_or(settings.get("port"), 4455.0)));
				if let Some(password) = settings.get("password").filter(|p| !p.is_null()) {
					patch.insert("password".into(), password.clone());
				}
				self.save_global(patch);
				self.reconnect_default();
				reply(json!({ "type": "saved" }));
			}
			// Only when the user asks for it: take port and password from OBS on this PC.
			"readLocalObs" => {
				let local = read_local_obs_config(&local_config_paths());
				if let Some(local) = &local {
					let password = if local.auth_required { local.password.clone() } else { String::new() };
					self.save_global(crate::actions::patch(vec![
						("host", "127.0.0.1".into()),
						("port", js::num(local.port)),
						("password", password.into()),
					]));
					self.reconnect_default();
				}
				let mut answer = Map::new();
				answer.insert("type".into(), "localObs".into());
				answer.insert("found".into(), local.is_some().into());
				if let Some(local) = &local {
					answer.insert("port".into(), js::num(local.port));
					answer.insert("authRequired".into(), local.auth_required.into());
					answer.insert("enabled".into(), local.enabled.into());
				}
				reply(Value::Object(answer));
			}
			"testConnection" => {
				let given = js::object(get("settings"));
				let mut settings = self.global_settings.borrow().clone();
				js::assign(&mut settings, &given);
				if given.get("password").is_none_or(Value::is_null) {
					match self.global_settings.borrow().get("password") {
						Some(p) => settings.insert("password".into(), p.clone()),
						None => settings.remove("password"),
					};
				}
				let resolved = if truthy(get("direct")) {
					Resolved {
						host: match settings.get("host") {
							None => "127.0.0.1".into(),
							host => js::to_string(host),
						},
						port: js::number_or(settings.get("port"), 4455.0),
						password: js::str_or_empty(settings.get("password")),
					}
				} else {
					resolve_connection(&settings)
				};
				let result = self.connections.test(&resolved, 4000).await;
				let mut answer = Map::new();
				answer.insert("type".into(), "testResult".into());
				answer.insert("ok".into(), result.ok.into());
				answer.insert("status".into(), result.status.name().into());
				if result.ok {
					answer.insert("version".into(), result.version.unwrap_or(Value::Null));
				} else {
					answer.insert("message".into(), result.message.into());
				}
				answer.insert("host".into(), resolved.host.into());
				answer.insert("port".into(), js::num(resolved.port));
				reply(Value::Object(answer));
			}
			"pickFile" => {
				let title = |fallback: &str| if get("title").is_none() { fallback.to_string() } else { js::to_string(get("title")) };
				let start = js::str_or_empty(get("start"));
				let picked = if truthy(get("folder")) {
					pick_folder(&title("Choose a folder"), &start).await
				} else {
					pick_file(&title("Choose a file"), &start, truthy(get("images"))).await
				};
				let key = get("key").cloned().unwrap_or(Value::Null);
				match picked {
					Ok(file) => reply(json!({ "type": "pickedFile", "key": key, "file": file })),
					Err(error) => reply(json!({ "type": "pickedFile", "key": key, "file": null, "error": error.message() })),
				}
			}
			"openUrl" => {
				let url = js::str_or_empty(get("url"));
				if url.starts_with("http://") || url.starts_with("https://") {
					self.sd.open_url(&url);
				}
			}
			"twitchSave" => {
				let mut twitch = js::object(self.global_settings.borrow().get("twitch"));
				twitch.insert("channel".into(), js::trim(&js::str_or_empty(get("channel"))).into());
				self.save_global(crate::actions::patch(vec![("twitch", Value::Object(twitch))]));
			}
			"twitchDeviceFlow" => match self.twitch.start_device_flow(get("clientId")).await {
				Ok(flow) => {
					self.sd.open_url(&js::to_string(Some(&flow.uri)));
					reply(json!({ "type": "twitchFlow", "userCode": flow.user_code, "uri": flow.uri }));
				}
				Err(error) => reply(json!({ "type": "twitchError", "message": error.message() })),
			},
			"twitchToken" => {
				if let Err(error) = self.twitch.use_token(get("token"), "", "").await {
					reply(json!({ "type": "twitchError", "message": error.message() }));
				}
			}
			"twitchCancel" => self.twitch.cancel_device_flow(),
			"twitchLogout" => self.twitch.logout(),
			_ => {
				let Some(instance) = instance else { return };
				if let Err(error) = instance.pi_message(&payload, reply.clone()).await {
					instance.log(&format!("settings page: {error}"));
					reply(json!({ "type": "error", "for": get("type").cloned().unwrap_or(Value::Null), "message": error.message() }));
				}
			}
		}
	}
}

/// Starts the handling of an event as a task of its own, without waiting for it.
pub fn dispatch(plugin: &Rc<Plugin>, event: Event) {
	spawn_local(plugin.clone().on_message(event));
}
