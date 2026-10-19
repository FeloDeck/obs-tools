// Routes OpenDeck events to the keys and answers the settings pages.
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use serde_json::{Map, Value, json};
use tokio::task::spawn_local;

use crate::actions::dials::{filter_range, filter_value_label};
use crate::actions::{Appearance, Key, KeyRc, Kind, Reply2Pi};
use crate::connections::{
	ConnectionManager, Resolved, connection_key, default_global, local_config_paths, read_local_obs_config, resolve_connection,
};
use crate::desktop::{pick_file, pick_folder, session_type};
use crate::js::{self, truthy};
use crate::obs::ObsClient;
use crate::obs_state::sorted_scene_names;
use crate::sd::Sd;
use crate::twitch::Twitch;

const AUDIO_CAP: i64 = 1 << 1;
const MEDIA_KINDS: &[&str] = &["ffmpeg_source", "vlc_source"];
const AUDIO_KINDS: &[&str] = &[
	"pulse_input_capture",
	"pulse_output_capture",
	"alsa_input_capture",
	"jack_output_capture",
	"pipewire-audio-capture-source",
	"wasapi_input_capture",
	"wasapi_output_capture",
	"wasapi_process_output_capture",
	"coreaudio_input_capture",
	"coreaudio_output_capture",
	"ffmpeg_source",
	"vlc_source",
	"browser_source",
];

/// An event from OpenDeck for one key.
#[derive(Debug, Clone, Default)]
pub struct Event {
	/// "willAppear", "keyDown", "sendToPlugin", ...
	pub event: String,
	pub action: String,
	pub context: String,
	pub device: String,
	pub controller: Option<String>,
	pub is_in_multi_action: bool,
	pub state: Option<u16>,
	pub settings: Option<Value>,
	pub ticks: f64,
	pub pressed: bool,
	/// The message of the settings page (sendToPlugin).
	pub payload: Value,
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
		let appearance = if e.event == "sendToPlugin" {
			Appearance::default()
		} else {
			Appearance {
				controller: e.controller.clone(),
				is_in_multi_action: e.is_in_multi_action,
				state: e.state,
				settings: e.settings.clone(),
			}
		};
		let instance = Key::new(&plugin, kind, &e.context, &e.action, &e.device, &appearance);
		self.instances.borrow_mut().insert(e.context.clone(), instance.clone());
		// Multi action children never get willAppear, they only press.
		if e.event != "willAppear" {
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
		match e.event.as_str() {
			"willAppear" => {
				if let Some(instance) = self.instance_for(&e) {
					instance.appear(e.controller.as_deref(), e.state);
				}
				self.update_chat_needs();
			}
			"willDisappear" => {
				let instance = self.instances.borrow_mut().remove(&e.context);
				if let Some(instance) = instance {
					instance.disappear();
				}
				self.update_chat_needs();
			}
			"keyDown" => {
				if let Some(instance) = self.with_settings(&e) {
					instance.key_down(e.is_in_multi_action).await;
				}
			}
			"keyUp" => {
				if let Some(instance) = self.with_settings(&e) {
					instance.key_up().await;
				}
			}
			"dialRotate" => {
				if let Some(instance) = self.with_settings(&e) {
					instance.dial_rotate(e.ticks, e.pressed).await;
				}
			}
			"dialDown" | "dialUp" | "touchTap" => {
				if let Some(instance) = self.with_settings(&e) {
					instance.dial_event(&e.event).await;
				}
			}
			"didReceiveSettings" => {
				if let Some(instance) = self.instance_for(&e) {
					instance.update_settings(e.settings.as_ref());
				}
				self.update_chat_needs();
			}
			"propertyInspectorDidAppear" => {
				self.add_inspector(&e.context);
				self.send_state(&e.context);
			}
			"propertyInspectorDidDisappear" => {
				self.open_inspectors.borrow_mut().retain(|c| *c != e.context);
			}
			"sendToPlugin" => self.on_inspector_message(e).await,
			_ => {}
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
				"status": client.as_ref().map_or_else(|| "idle".to_string(), |c| c.status()),
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

	async fn on_inspector_message(self: &Rc<Self>, e: Event) {
		let context = e.context.clone();
		let payload = if e.payload.is_object() { e.payload.clone() } else { json!({}) };
		let instance = self.instance_for(&e);
		let sd = self.sd.clone();
		let request_id = payload.get("requestId").cloned();
		let reply_context = context.clone();
		let reply: Reply2Pi = Rc::new(move |data: Value| {
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
				answer.insert("status".into(), result.status.into());
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
			kind => {
				let Some(instance) = instance else { return };
				if let Err(error) = instance.pi_message(&payload, reply.clone()).await {
					instance.log(&format!("settings page: {error}"));
					reply(json!({ "type": "error", "for": get("type").cloned().unwrap_or(Value::Null), "message": error.message() }));
				}
				let _ = kind;
			}
		}
	}
}

/// Starts the handling of an event, without waiting for it, like the JavaScript version.
pub fn dispatch(plugin: &Rc<Plugin>, event: Event) {
	spawn_local(plugin.clone().on_message(event));
}

// Lists for the drop downs, read from the OBS this key talks to.
async fn lists(instance: Option<&KeyRc>, names: &[String], payload: &Value) -> Value {
	let mut out = Map::new();
	let Some(client) = instance.and_then(|i| i.obs()) else { return Value::Object(out) };
	if !client.connected() {
		return Value::Object(out);
	}
	let call = |kind: &'static str, data: Value| {
		let reply = client.call(kind, data);
		async move { reply.await.ok() }
	};
	let mut inputs: Option<Vec<Value>> = None;
	async fn get_inputs(client: &ObsClient, inputs: &mut Option<Vec<Value>>) -> Vec<Value> {
		if inputs.is_none() {
			let list = client.call("GetInputList", json!({})).await.ok();
			*inputs = Some(match list.as_ref().and_then(|l| l.get("inputs")) {
				Some(Value::Array(items)) => items.clone(),
				_ => Vec::new(),
			});
		}
		inputs.clone().unwrap_or_default()
	}
	let field = |items: &[Value], key: &str| items.iter().map(|i| i.get(key).cloned().unwrap_or(Value::Null)).collect::<Vec<_>>();
	let kind_of = |i: &Value| js::to_string(i.get("unversionedInputKind"));
	let array = |list: Option<Value>, key: &str| match list.as_ref().and_then(|l| l.get(key)) {
		Some(Value::Array(items)) => items.clone(),
		_ => Vec::new(),
	};
	for name in names {
		let value = match name.as_str() {
			"scenes" => {
				let list = call("GetSceneList", json!({})).await.unwrap_or(Value::Null);
				Value::from(sorted_scene_names(&list))
			}
			"inputs" => Value::from(field(&get_inputs(&client, &mut inputs).await, "inputName")),
			"audioInputs" => {
				let items: Vec<Value> = get_inputs(&client, &mut inputs)
					.await
					.into_iter()
					.filter(|i| match i.get("inputKindCaps") {
						Some(caps) => js::number(Some(caps)) as i64 & AUDIO_CAP != 0,
						None => AUDIO_KINDS.contains(&kind_of(i).as_str()),
					})
					.collect();
				Value::from(field(&items, "inputName"))
			}
			"mediaInputs" | "imageInputs" | "browserInputs" | "windowInputs" | "textInputs" => {
				let wanted = |kind: &str| match name.as_str() {
					"mediaInputs" => MEDIA_KINDS.contains(&kind),
					"imageInputs" => kind == "image_source",
					"browserInputs" => kind == "browser_source",
					"windowInputs" => kind == "xcomposite_input",
					_ => kind.starts_with("text_"),
				};
				let items: Vec<Value> = get_inputs(&client, &mut inputs).await.into_iter().filter(|i| wanted(&kind_of(i))).collect();
				Value::from(field(&items, "inputName"))
			}
			"sources" => {
				let list = call("GetSceneList", json!({})).await.unwrap_or(Value::Null);
				let scenes = sorted_scene_names(&list);
				let mut all = field(&get_inputs(&client, &mut inputs).await, "inputName");
				all.extend(scenes.into_iter().map(Value::from));
				Value::from(all)
			}
			"transitions" => Value::from(field(&array(call("GetSceneTransitionList", json!({})).await, "transitions"), "transitionName")),
			"filters" => {
				let source = payload.get("source").cloned().unwrap_or(Value::Null);
				if truthy(Some(&source)) {
					Value::from(field(&array(call("GetSourceFilterList", json!({ "sourceName": source })).await, "filters"), "filterName"))
				} else {
					Value::from(Vec::<Value>::new())
				}
			}
			"profiles" => Value::from(array(call("GetProfileList", json!({})).await, "profiles")),
			"collections" => Value::from(array(call("GetSceneCollectionList", json!({})).await, "sceneCollections")),
			"hotkeys" => {
				let mut names: Vec<Value> = Vec::new();
				for hotkey in array(call("GetHotkeyList", json!({})).await, "hotkeys") {
					if !names.contains(&hotkey) {
						names.push(hotkey);
					}
				}
				let mut entries: Vec<(String, Value)> = names.into_iter().map(|v| (hotkey_label(&js::to_string(Some(&v))), v)).collect();
				entries.sort_by(|a, b| locale_compare(&a.0, &b.0));
				Value::from(entries.into_iter().map(|(label, value)| json!({ "value": value, "label": label })).collect::<Vec<_>>())
			}
			"filterSettings" => filter_settings(&client, payload.get("source"), payload.get("filter")).await,
			"sceneItems" => {
				let scene = payload
					.get("scene")
					.filter(|s| truthy(Some(s)))
					.cloned()
					.or_else(|| instance.and_then(|i| i.obs_state()).and_then(|s| s.data().program_scene).map(Value::from));
				match scene {
					Some(scene) if truthy(Some(&scene)) => Value::from(field(
						&array(call("GetSceneItemList", json!({ "sceneName": scene })).await, "sceneItems"),
						"sourceName",
					)),
					_ => Value::from(Vec::<Value>::new()),
				}
			}
			_ => continue,
		};
		out.insert(name.clone(), value);
	}
	Value::Object(out)
}

// The number settings of a filter, for the filter dial.
async fn filter_settings(client: &ObsClient, source: Option<&Value>, filter: Option<&Value>) -> Value {
	let empty = Value::Array(Vec::new());
	if !truthy(source) || !truthy(filter) {
		return empty;
	}
	let Ok(found) = client.call("GetSourceFilter", json!({ "sourceName": source, "filterName": filter })).await else { return empty };
	let kind = found.get("filterKind").cloned().unwrap_or(Value::Null);
	let defaults = client
		.call("GetSourceFilterDefaultSettings", json!({ "filterKind": kind }))
		.await
		.ok()
		.and_then(|d| d.get("defaultFilterSettings").cloned());
	let mut all = js::object(defaults.as_ref());
	js::assign(&mut all, &js::object(found.get("filterSettings")));
	let mut keys: Vec<&String> = all.iter().filter(|(k, v)| v.is_number() && !k.starts_with("color")).map(|(k, _)| k).collect();
	keys.sort();
	let kind = js::str_or_empty(Some(&kind));
	Value::from(
		keys.into_iter()
			.map(|key| {
				let range =
					filter_range(&kind, key).map_or(Value::Null, |r| Value::from(r.iter().map(|x| js::num(*x)).collect::<Vec<_>>()));
				json!({ "value": key, "label": filter_value_label(key), "range": range })
			})
			.collect::<Vec<_>>(),
	)
}

/// "OBSBasic.StartRecording" -> "Start recording", "libobs.mute" -> "Mute (source)"
pub fn hotkey_label(name: &str) -> String {
	let mut parts = name.split('.');
	let group = parts.next().unwrap_or("");
	let rest: Vec<&str> = parts.collect();
	let joined = rest.join(" ");
	// ([a-z])([A-Z]) -> "$1 $2"
	let mut spaced = String::new();
	let mut prev: Option<char> = None;
	for c in joined.chars() {
		if prev.is_some_and(|p| p.is_ascii_lowercase()) && c.is_ascii_uppercase() {
			spaced.push(' ');
		}
		spaced.push(c);
		prev = Some(c);
	}
	let words = js::trim(&spaced.replace(['_', '.'], " ").to_lowercase()).to_string();
	let mut chars = words.chars();
	let label = match chars.next() {
		Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
		None => String::new(),
	};
	let label = match label.strip_suffix("virtual cam") {
		Some(start) => format!("{start}virtual camera"),
		None => label,
	};
	let place = match group {
		"OBSBasic" => "",
		"libobs" => "source",
		"MediaSource" => "media source",
		"ReplayBuffer" => "replay buffer",
		other => other,
	};
	if place.is_empty() { label } else { format!("{label} ({place})") }
}

// Like String.localeCompare: letters ignore case and come after digits, punctuation and spaces.
fn locale_compare(a: &str, b: &str) -> std::cmp::Ordering {
	const PUNCTUATION: &str = "_-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$";
	let key = |c: char| -> (u8, u32) {
		if c.is_whitespace() {
			(0, 0)
		} else if let Some(i) = PUNCTUATION.find(c) {
			(1, i as u32)
		} else if c.is_ascii_digit() {
			(2, c as u32)
		} else if c.is_alphabetic() {
			(3, c.to_lowercase().next().unwrap_or(c) as u32)
		} else {
			(1, 1000 + c as u32)
		}
	};
	let primary = a.chars().map(key).cmp(b.chars().map(key));
	// Same letters: lowercase first, like ICU.
	primary.then_with(|| a.chars().map(|c| c.is_uppercase()).cmp(b.chars().map(|c| c.is_uppercase()))).then_with(|| a.cmp(b))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn hotkey_names_become_readable() {
		assert_eq!(hotkey_label("OBSBasic.StartRecording"), "Start recording");
		assert_eq!(hotkey_label("libobs.mute"), "Mute (source)");
		assert_eq!(hotkey_label("MediaSource.Play"), "Play (media source)");
		assert_eq!(hotkey_label("libobs.show_scene_item.1"), "Show scene item 1 (source)");
		assert_eq!(hotkey_label("OBSBasic.StartVirtualCam"), "Start virtual camera");
	}

	#[test]
	fn labels_sort_like_locale_compare() {
		let mut labels = vec!["b", "A", "a", "Push-to-mute (source)", "Push to talk", "item 10", "item 1", "Zoom"];
		labels.sort_by(|a, b| locale_compare(a, b));
		assert_eq!(labels, ["a", "A", "b", "item 1", "item 10", "Push to talk", "Push-to-mute (source)", "Zoom"]);
	}
}
