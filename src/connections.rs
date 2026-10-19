// Shares one OBS connection between all keys that point at the same OBS.
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use serde_json::{Map, Value};

use crate::emitter::Emitter;
use crate::js::{self, fmt};
use crate::obs::{ObsClient, Status};
use crate::obs_state::ObsState;
use crate::timers::{Timer, set_timeout};

const IDLE_STOP_DELAY: f64 = 30000.0;

pub fn default_global() -> Map<String, Value> {
	match crate::obj! { "host" => "127.0.0.1", "port" => 4455, "password" => "" } {
		Value::Object(map) => map,
		_ => Map::new(),
	}
}

fn home() -> PathBuf {
	std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from).or_else(std::env::home_dir).unwrap_or_default()
}

fn config_home() -> PathBuf {
	std::env::var_os("XDG_CONFIG_HOME").filter(|h| !h.is_empty()).map(PathBuf::from).unwrap_or_else(|| home().join(".config"))
}

/// Where OBS keeps the obs-websocket settings on Linux (native and Flatpak).
pub fn local_config_paths() -> Vec<PathBuf> {
	let tail = PathBuf::from("obs-studio").join("plugin_config").join("obs-websocket").join("config.json");
	vec![config_home().join(&tail), home().join(".var").join("app").join("com.obsproject.Studio").join("config").join(&tail)]
}

#[derive(Debug, Clone, PartialEq)]
pub struct LocalObs {
	pub path: PathBuf,
	pub mtime: f64,
	pub enabled: bool,
	pub port: f64,
	pub auth_required: bool,
	pub password: String,
}

/// The newest obs-websocket config file of the given ones.
pub fn read_local_obs_config(paths: &[PathBuf]) -> Option<LocalObs> {
	let mut best: Option<LocalObs> = None;
	for file in paths {
		let Ok(meta) = std::fs::metadata(file) else { continue };
		let Ok(text) = std::fs::read_to_string(file) else { continue };
		let Ok(data) = serde_json::from_str::<Value>(&text) else { continue };
		let mtime =
			meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0.0, |d| d.as_secs_f64() * 1000.0);
		let password = data.get("server_password");
		let entry = LocalObs {
			path: file.clone(),
			mtime,
			enabled: data.get("server_enabled") != Some(&Value::Bool(false)),
			port: js::number_or(data.get("server_port"), 4455.0),
			auth_required: data.get("auth_required") != Some(&Value::Bool(false)),
			password: if js::truthy(password) { js::to_string(password) } else { String::new() },
		};
		if best.as_ref().is_none_or(|b| entry.mtime > b.mtime) {
			best = Some(entry);
		}
	}
	best
}

/// Host, port and password of an OBS.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
	pub host: String,
	pub port: f64,
	pub password: String,
}

/// Turns the saved settings into the host, port and password to use.
pub fn resolve_connection(settings: &Map<String, Value>) -> Resolved {
	let mut merged = default_global();
	js::assign(&mut merged, settings);
	let host = merged.get("host");
	let password = merged.get("password");
	Resolved {
		host: js::trim(&if js::truthy(host) { js::to_string(host) } else { "127.0.0.1".into() }).to_string(),
		port: js::number_or(merged.get("port"), 4455.0),
		password: js::str_or_empty(password),
	}
}

pub fn connection_key(c: &Resolved) -> String {
	format!("{}|{}|{}", c.host.to_lowercase(), fmt(c.port), c.password)
}

struct Entry {
	client: Rc<ObsClient>,
	state: Rc<ObsState>,
	users: Cell<i64>,
	stop_timer: RefCell<Option<Timer>>,
}

/// A key's share of a connection. `release` once the key does not need it any more.
pub struct ConnHandle {
	pub key: String,
	pub client: Rc<ObsClient>,
	pub state: Rc<ObsState>,
	entry: Rc<Entry>,
	released: Cell<bool>,
	manager: std::rc::Weak<ConnectionManager>,
}

impl ConnHandle {
	pub fn release(&self) {
		if self.released.replace(true) {
			return;
		}
		let Some(manager) = self.manager.upgrade() else { return };
		let entry = self.entry.clone();
		entry.users.set(entry.users.get() - 1);
		if entry.users.get() <= 0 {
			if let Some(timer) = entry.stop_timer.borrow_mut().take() {
				timer.clear();
			}
			let key = self.key.clone();
			let weak = Rc::downgrade(&manager);
			let timer_entry = entry.clone();
			let timer = set_timeout(IDLE_STOP_DELAY, move || {
				let Some(manager) = weak.upgrade() else { return };
				let entry = timer_entry;
				if entry.users.get() > 0 {
					return;
				}
				entry.client.stop();
				manager.entries.borrow_mut().remove(&key);
			});
			*entry.stop_timer.borrow_mut() = Some(timer);
		}
	}
}

pub struct ConnectionManager {
	log: Box<dyn Fn(&str)>,
	entries: RefCell<HashMap<String, Rc<Entry>>>,
	global_settings: RefCell<Map<String, Value>>,
	default_config: RefCell<Resolved>,
	me: std::rc::Weak<ConnectionManager>,
	pub on_default_changed: Emitter<Resolved>,
	/// The status of a connection changed.
	pub on_status: Emitter<()>,
}

/// What testing a connection found, for the setup page.
pub struct TestResult {
	pub ok: bool,
	pub status: Status,
	pub message: String,
	pub version: Option<Value>,
}

impl ConnectionManager {
	pub fn new(log: impl Fn(&str) + 'static) -> Rc<Self> {
		Rc::new_cyclic(|me| ConnectionManager {
			log: Box::new(log),
			entries: RefCell::new(HashMap::new()),
			global_settings: RefCell::new(default_global()),
			default_config: RefCell::new(resolve_connection(&default_global())),
			me: me.clone(),
			on_default_changed: Emitter::default(),
			on_status: Emitter::default(),
		})
	}

	pub fn default_config(&self) -> Resolved {
		self.default_config.borrow().clone()
	}

	pub fn set_global_settings(&self, settings: &Map<String, Value>) {
		let mut merged = default_global();
		js::assign(&mut merged, settings);
		*self.global_settings.borrow_mut() = merged;
		self.check_default(true);
	}

	fn check_default(&self, force: bool) {
		let next = resolve_connection(&self.global_settings.borrow());
		let changed = connection_key(&next) != connection_key(&self.default_config.borrow());
		*self.default_config.borrow_mut() = next.clone();
		if changed || force {
			self.on_default_changed.emit(&next);
		}
		if changed {
			(self.log)(&format!("OBS connection is now {}:{}", next.host, fmt(next.port)));
		}
	}

	/// The connection of this host, port and password, if a key uses it.
	pub fn client(&self, key: &str) -> Option<Rc<ObsClient>> {
		self.entries.borrow().get(key).map(|e| e.client.clone())
	}

	/// Every connection, for a reconnect after the PC woke up.
	pub fn clients(&self) -> Vec<Rc<ObsClient>> {
		self.entries.borrow().values().map(|e| e.client.clone()).collect()
	}

	/// config: None for the default connection.
	pub fn acquire(&self, config: Option<Resolved>) -> ConnHandle {
		let resolved = config.unwrap_or_else(|| self.default_config());
		let key = connection_key(&resolved);
		let existing = self.entries.borrow().get(&key).cloned();
		let entry = match existing {
			Some(entry) => entry,
			None => {
				let client = ObsClient::new(&resolved.host, resolved.port, &resolved.password, &key);
				let state = ObsState::new(client.clone());
				let weak = self.me.clone();
				let (host, port) = (resolved.host.clone(), resolved.port);
				client.on_status.on(move |(status, error)| {
					let Some(me) = weak.upgrade() else { return };
					let suffix = if error.is_empty() { String::new() } else { format!(": {error}") };
					(me.log)(&format!("OBS {host}:{} {}{suffix}", fmt(port), status.name()));
					me.on_status.emit(&());
				});
				let entry = Rc::new(Entry { client: client.clone(), state, users: Cell::new(0), stop_timer: RefCell::new(None) });
				self.entries.borrow_mut().insert(key.clone(), entry.clone());
				client.start();
				entry
			}
		};
		if let Some(timer) = entry.stop_timer.borrow_mut().take() {
			timer.clear();
		}
		entry.users.set(entry.users.get() + 1);
		ConnHandle {
			key,
			client: entry.client.clone(),
			state: entry.state.clone(),
			entry,
			released: Cell::new(false),
			manager: self.me.clone(),
		}
	}

	/// Used by the setup page to try settings before saving them.
	pub async fn test(&self, config: &Resolved, timeout_ms: u64) -> TestResult {
		let client = ObsClient::new(&config.host, config.port, &config.password, "test");
		let (tx, rx) = tokio::sync::oneshot::channel::<TestResult>();
		let tx = RefCell::new(Some(tx));
		let weak = Rc::downgrade(&client);
		client.on_status.on(move |(status, error)| {
			let result = match status {
				Status::Connected => {
					TestResult { ok: true, status: *status, message: String::new(), version: weak.upgrade().and_then(|c| c.version()) }
				}
				status if status.is_failure() => TestResult { ok: false, status: *status, message: error.clone(), version: None },
				_ => return,
			};
			if let Some(tx) = tx.borrow_mut().take() {
				let _ = tx.send(result);
			}
		});
		client.start();
		let result = match tokio::time::timeout(Duration::from_millis(timeout_ms), rx).await {
			Ok(Ok(result)) => result,
			_ => {
				let error = client.last_error();
				TestResult {
					ok: false,
					status: client.status(),
					message: if error.is_empty() { "No answer from OBS".into() } else { error },
					version: None,
				}
			}
		};
		client.stop();
		result
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	fn map(v: Value) -> Map<String, Value> {
		v.as_object().cloned().unwrap_or_default()
	}

	#[test]
	fn saved_settings_never_read_from_obs_by_itself() {
		let r = |v| resolve_connection(&map(v));
		assert_eq!(
			r(json!({ "host": " localhost ", "password": "typed" })),
			Resolved { host: "localhost".into(), port: 4455.0, password: "typed".into() }
		);
		assert_eq!(
			r(json!({ "host": "192.168.1.5", "port": "4460" })),
			Resolved { host: "192.168.1.5".into(), port: 4460.0, password: "".into() }
		);
		assert_eq!(r(json!({})), Resolved { host: "127.0.0.1".into(), port: 4455.0, password: "".into() });
	}

	#[test]
	fn newest_obs_websocket_config_file_is_used() {
		let dir = std::env::temp_dir().join(format!("obstools-{}", std::process::id()));
		std::fs::create_dir_all(&dir).unwrap_or_default();
		let a = dir.join("a.json");
		let b = dir.join("b.json");
		std::fs::write(&a, r#"{"server_port":1111,"server_password":"old"}"#).unwrap_or_default();
		std::fs::write(&b, r#"{"server_port":2222,"server_password":"new","server_enabled":false}"#).unwrap_or_default();
		let old = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(949_363_200);
		std::fs::File::options().write(true).open(&a).and_then(|f| f.set_modified(old)).unwrap_or_default();
		let found = read_local_obs_config(&[a, b, dir.join("missing.json")]);
		std::fs::remove_dir_all(&dir).unwrap_or_default();
		let found = found.expect("a config");
		assert_eq!(found.port, 2222.0);
		assert_eq!(found.password, "new");
		assert!(!found.enabled);
	}
}
