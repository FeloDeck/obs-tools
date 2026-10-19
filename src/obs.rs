// Small obs-websocket v5 client.
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::rc::{Rc, Weak};
use std::time::Duration;

use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, oneshot};
use tokio::task::{AbortHandle, spawn_local};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

use crate::emitter::Emitter;
use crate::js::{self, fmt};
use crate::timers::{Timer, set_timeout};
use crate::{Error, Res};

const OP_HELLO: u64 = 0;
const OP_IDENTIFY: u64 = 1;
const OP_IDENTIFIED: u64 = 2;
const OP_REIDENTIFY: u64 = 3;
const OP_EVENT: u64 = 5;
const OP_REQUEST: u64 = 6;
const OP_REQUEST_RESPONSE: u64 = 7;
const OP_REQUEST_BATCH: u64 = 8;
const OP_REQUEST_BATCH_RESPONSE: u64 = 9;

/// Every non high-volume category (2047).
pub const EVENTS_ALL: u32 = (1 << 11) - 1;
/// Audio levels of all inputs, about 20 times a second.
pub const EVENTS_VOLUME_METERS: u32 = 1 << 16;

#[derive(Clone, Copy)]
pub enum BatchExecution {
	SerialRealtime = 0,
	SerialFrame = 1,
	Parallel = 2,
}

const CLOSE_AUTH_FAILED: u16 = 4009;
const CLOSE_RPC_VERSION: u16 = 4010;

const RETRY_DELAYS: [f64; 4] = [1000.0, 2000.0, 3000.0, 5000.0];
const AUTH_RETRY_DELAY: f64 = 15000.0;

pub fn auth_response(password: &str, salt: &str, challenge: &str) -> String {
	let b64 = base64::engine::general_purpose::STANDARD;
	let secret = b64.encode(Sha256::digest(format!("{password}{salt}")));
	b64.encode(Sha256::digest(format!("{secret}{challenge}")))
}

/// The answer to a request, sent right away. Awaiting it waits for OBS.
pub type Reply<T = Value> = Pin<Box<dyn Future<Output = Res<T>>>>;

struct Socket {
	serial: u64,
	tx: mpsc::UnboundedSender<Message>,
	open: Rc<Cell<bool>>,
	task: AbortHandle,
}

// What one connection attempt learned, shared by its handlers.
#[derive(Default)]
struct Attempt {
	identified: Cell<bool>,
	close_code: Cell<u16>,
	error: RefCell<String>,
}

/// How the connection to OBS is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
	/// Not started.
	Idle,
	/// Waiting for the socket or the handshake.
	Connecting,
	/// Identified, requests work.
	Connected,
	/// Nothing listens on host:port (OBS closed or server off).
	Offline,
	/// Wrong or missing password.
	Auth,
	/// Anything else, see last_error.
	Error,
}

impl Status {
	/// The name the settings pages know.
	pub fn name(self) -> &'static str {
		match self {
			Status::Idle => "idle",
			Status::Connecting => "connecting",
			Status::Connected => "connected",
			Status::Offline => "offline",
			Status::Auth => "auth",
			Status::Error => "error",
		}
	}

	/// OBS cannot be reached, and it is not just starting.
	pub fn is_failure(self) -> bool {
		matches!(self, Status::Offline | Status::Auth | Status::Error)
	}
}

pub struct ObsClient {
	pub host: String,
	pub port: f64,
	pub password: String,
	pub name: String,
	status: Cell<Status>,
	last_error: RefCell<String>,
	version: RefCell<Option<Map<String, Value>>>,
	ws: RefCell<Option<Socket>>,
	pending: RefCell<HashMap<String, oneshot::Sender<Res<Value>>>>,
	next_id: Cell<u64>,
	serial: Cell<u64>,
	retry: Cell<usize>,
	retry_timer: RefCell<Option<Timer>>,
	stopped: Cell<bool>,
	subscriptions: Cell<u32>,
	me: Weak<ObsClient>,
	/// (status, error)
	pub on_status: Emitter<(Status, String)>,
	pub on_connected: Emitter<()>,
	pub on_disconnected: Emitter<()>,
	/// (eventType, eventData)
	pub on_event: Emitter<(String, Value)>,
	// Listeners for one event type, called after the ones of on_event.
	on_typed: Emitter<(String, Value)>,
}

impl ObsClient {
	pub fn new(host: &str, port: f64, password: &str, name: &str) -> Rc<Self> {
		Rc::new_cyclic(|me| ObsClient {
			host: host.to_string(),
			port: crate::settings::or(port, 4455.0),
			password: password.to_string(),
			name: name.to_string(),
			status: Cell::new(Status::Idle),
			last_error: RefCell::new(String::new()),
			version: RefCell::new(None),
			ws: RefCell::new(None),
			pending: RefCell::new(HashMap::new()),
			next_id: Cell::new(1),
			serial: Cell::new(0),
			retry: Cell::new(0),
			retry_timer: RefCell::new(None),
			stopped: Cell::new(true),
			subscriptions: Cell::new(EVENTS_ALL),
			me: me.clone(),
			on_status: Emitter::default(),
			on_connected: Emitter::default(),
			on_disconnected: Emitter::default(),
			on_event: Emitter::default(),
			on_typed: Emitter::default(),
		})
	}

	fn rc(&self) -> Option<Rc<Self>> {
		self.me.upgrade()
	}

	pub fn status(&self) -> Status {
		self.status.get()
	}

	pub fn last_error(&self) -> String {
		self.last_error.borrow().clone()
	}

	/// { obs, websocket, rpc, platform } once OBS said hello.
	pub fn version(&self) -> Option<Value> {
		self.version.borrow().clone().map(Value::Object)
	}

	pub fn connected(&self) -> bool {
		self.status.get() == Status::Connected
	}

	/// Listens to one event type until `off_type` with the returned id.
	pub fn on_type(&self, kind: &str, listener: impl Fn(&Value) + 'static) -> u64 {
		let kind = kind.to_string();
		self.on_typed.on(move |(t, data)| {
			if *t == kind {
				listener(data);
			}
		})
	}

	pub fn off_type(&self, id: u64) {
		self.on_typed.off(id);
	}

	/// Change which events OBS sends, also on a running connection.
	pub fn set_event_subscriptions(&self, mask: u32) {
		if mask == self.subscriptions.get() {
			return;
		}
		self.subscriptions.set(mask);
		if self.connected() {
			self.send_raw(&crate::obj! { "op" => OP_REIDENTIFY, "d" => crate::obj! { "eventSubscriptions" => mask } });
		}
	}

	pub fn url(&self) -> String {
		let host = if self.host.contains(':') && !self.host.starts_with('[') { format!("[{}]", self.host) } else { self.host.clone() };
		format!("ws://{host}:{}", fmt(self.port))
	}

	pub fn start(&self) {
		if !self.stopped.get() {
			return;
		}
		self.stopped.set(false);
		self.open();
	}

	pub fn stop(&self) {
		self.stopped.set(true);
		self.clear_retry();
		let was_connected = self.connected();
		self.drop_socket();
		self.cleanup("stopped");
		if was_connected {
			self.on_disconnected.emit(&());
		}
		self.set_status(Status::Idle, "");
	}

	/// Reconnect now, for example after the user saved new settings.
	pub fn reconnect(&self) {
		self.clear_retry();
		self.retry.set(0);
		if self.ws.borrow().is_some() {
			let was_connected = self.connected();
			self.drop_socket();
			self.cleanup("reconnect");
			if was_connected {
				self.on_disconnected.emit(&());
			}
		}
		if !self.stopped.get() {
			self.open();
		}
	}

	fn clear_retry(&self) {
		if let Some(timer) = self.retry_timer.borrow_mut().take() {
			timer.clear();
		}
	}

	// Close the socket without running its close handler.
	fn drop_socket(&self) {
		let socket = self.ws.borrow_mut().take();
		if let Some(socket) = socket {
			if socket.open.get() {
				let _ = socket.tx.send(Message::Close(Some(CloseFrame { code: CloseCode::Normal, reason: "".into() })));
			}
			socket.task.abort();
		}
	}

	fn set_status(&self, status: Status, error: &str) {
		let changed = self.status.get() != status || *self.last_error.borrow() != error;
		self.status.set(status);
		*self.last_error.borrow_mut() = error.to_string();
		if changed {
			self.on_status.emit(&(status, error.to_string()));
		}
	}

	fn is_current(&self, serial: u64) -> bool {
		self.ws.borrow().as_ref().is_some_and(|s| s.serial == serial)
	}

	fn open(&self) {
		if self.stopped.get() {
			return;
		}
		// While retrying, keep showing why the last try failed.
		if !self.status.get().is_failure() {
			self.set_status(Status::Connecting, "");
		}
		// The WebSocket error does not say why it failed, so check the name first.
		let bare = self.host.strip_prefix('[').unwrap_or(&self.host);
		let bare = bare.strip_suffix(']').unwrap_or(bare);
		if bare.parse::<std::net::IpAddr>().is_err() && self.host != "localhost" {
			let Some(me) = self.rc() else { return };
			spawn_local(async move {
				let found = tokio::net::lookup_host((me.host.as_str(), 0)).await.map(|mut a| a.next().is_some()).unwrap_or(false);
				if !found {
					if me.stopped.get() {
						return;
					}
					me.set_status(Status::Offline, &format!("Host {} not found", me.host));
					me.schedule_retry();
					return;
				}
				if me.stopped.get() || me.ws.borrow().is_some() {
					return;
				}
				me.create_socket();
			});
			return;
		}
		self.create_socket();
	}

	fn create_socket(&self) {
		let Some(me) = self.rc() else { return };
		let mut request = match self.url().into_client_request() {
			Ok(request) => request,
			Err(error) => {
				self.set_status(Status::Error, &error.to_string());
				self.schedule_retry();
				return;
			}
		};
		request.headers_mut().insert("Sec-WebSocket-Protocol", "obswebsocket.json".parse().expect("valid header"));
		let serial = self.serial.get() + 1;
		self.serial.set(serial);
		let (tx, rx) = mpsc::unbounded_channel();
		let open = Rc::new(Cell::new(false));
		let task = spawn_local(run_socket(me, serial, request, rx, open.clone()));
		*self.ws.borrow_mut() = Some(Socket { serial, tx, open, task: task.abort_handle() });
	}

	fn on_message(&self, serial: u64, attempt: &Attempt, text: &str) {
		let Ok(message) = serde_json::from_str::<Value>(text) else { return };
		let d = message.get("d").cloned().unwrap_or(Value::Null);
		match message.get("op").and_then(Value::as_u64) {
			Some(OP_HELLO) => self.on_hello(attempt, &d),
			Some(OP_IDENTIFIED) => self.on_identified(serial, attempt),
			Some(OP_EVENT) => {
				let kind = d["eventType"].as_str().unwrap_or_default().to_string();
				let data = match d.get("eventData") {
					None | Some(Value::Null) => Value::Object(Map::new()),
					Some(data) => data.clone(),
				};
				let event = (kind, data);
				self.on_event.emit(&event);
				self.on_typed.emit(&event);
			}
			Some(op @ (OP_REQUEST_RESPONSE | OP_REQUEST_BATCH_RESPONSE)) => {
				let id = d["requestId"].as_str().unwrap_or_default();
				let Some(pending) = self.pending.borrow_mut().remove(id) else { return };
				let result = if op == OP_REQUEST_BATCH_RESPONSE {
					Ok(match d.get("results") {
						None | Some(Value::Null) => Value::Array(Vec::new()),
						Some(results) => results.clone(),
					})
				} else if d["requestStatus"]["result"] == true {
					Ok(match d.get("responseData") {
						None | Some(Value::Null) => Value::Object(Map::new()),
						Some(data) => data.clone(),
					})
				} else {
					Err(obs_error(&d["requestType"], &d["requestStatus"]))
				};
				let _ = pending.send(result);
			}
			_ => {}
		}
	}

	fn on_hello(&self, attempt: &Attempt, d: &Value) {
		let mut identify = Map::new();
		identify.insert("rpcVersion".into(), 1.into());
		identify.insert("eventSubscriptions".into(), self.subscriptions.get().into());
		if let Some(auth) = d.get("authentication").filter(|a| a.is_object()) {
			if self.password.is_empty() {
				attempt.close_code.set(CLOSE_AUTH_FAILED);
				*attempt.error.borrow_mut() = "OBS asks for a password".into();
				self.send_message(Message::Close(Some(CloseFrame { code: CloseCode::Normal, reason: "".into() })));
				return;
			}
			let (salt, challenge) = (auth["salt"].as_str().unwrap_or_default(), auth["challenge"].as_str().unwrap_or_default());
			identify.insert("authentication".into(), auth_response(&self.password, salt, challenge).into());
		}
		let mut version = Map::new();
		version.insert("obs".into(), Value::Null);
		for (key, from) in [("websocket", "obsWebSocketVersion"), ("rpc", "rpcVersion")] {
			if let Some(value) = d.get(from) {
				version.insert(key.into(), value.clone());
			}
		}
		*self.version.borrow_mut() = Some(version);
		self.send_raw(&crate::obj! { "op" => OP_IDENTIFY, "d" => Value::Object(identify) });
	}

	fn on_identified(&self, serial: u64, attempt: &Attempt) {
		// Identified already: this is the answer to a Reidentify.
		if attempt.identified.get() {
			return;
		}
		attempt.identified.set(true);
		self.retry.set(0);
		let Some(me) = self.rc() else { return };
		let reply = self.call("GetVersion", crate::obj! {});
		spawn_local(async move {
			if let Ok(version) = reply.await {
				let mut current = me.version.borrow().clone().unwrap_or_default();
				current.insert("obs".into(), version.get("obsVersion").cloned().unwrap_or(Value::Null));
				if let Some(platform) = version.get("platformDescription") {
					current.insert("platform".into(), platform.clone());
				}
				*me.version.borrow_mut() = Some(current);
			}
			if !me.is_current(serial) {
				return;
			}
			me.set_status(Status::Connected, "");
			me.on_connected.emit(&());
		});
	}

	fn closed(&self, serial: u64, attempt: &Attempt, event_code: u16, reason: &str) {
		if !self.is_current(serial) {
			return;
		}
		let code = if attempt.close_code.get() != 0 { attempt.close_code.get() } else { event_code };
		*self.ws.borrow_mut() = None;
		let was_connected = attempt.identified.get();
		self.cleanup("connection closed");
		if was_connected {
			self.on_disconnected.emit(&());
		}
		let error = attempt.error.borrow().clone();
		if code == CLOSE_AUTH_FAILED {
			self.set_status(Status::Auth, if error.is_empty() { "Wrong password" } else { &error });
		} else if code == CLOSE_RPC_VERSION {
			self.set_status(Status::Error, "This OBS version is not supported (needs obs-websocket 5)");
		} else if !was_connected {
			let nothing = format!("Nothing answers on {}:{}", self.host, fmt(self.port));
			self.set_status(Status::Offline, if error.is_empty() { &nothing } else { &error });
		} else {
			self.set_status(Status::Offline, if reason.is_empty() { "OBS closed the connection" } else { reason });
		}
		self.schedule_retry();
	}

	fn schedule_retry(&self) {
		if self.stopped.get() {
			return;
		}
		self.clear_retry();
		let delay =
			if self.status.get() == Status::Auth { AUTH_RETRY_DELAY } else { RETRY_DELAYS[self.retry.get().min(RETRY_DELAYS.len() - 1)] };
		self.retry.set(self.retry.get() + 1);
		let me = self.me.clone();
		let timer = set_timeout(delay, move || {
			if let Some(me) = me.upgrade() {
				me.open();
			}
		});
		*self.retry_timer.borrow_mut() = Some(timer);
	}

	fn cleanup(&self, reason: &str) {
		let pending: Vec<_> = self.pending.borrow_mut().drain().collect();
		for (_, sender) in pending {
			let _ = sender.send(Err(format!("OBS not connected ({reason})").into()));
		}
	}

	fn send_message(&self, message: Message) -> bool {
		match self.ws.borrow().as_ref() {
			Some(socket) if socket.open.get() => socket.tx.send(message).is_ok(),
			_ => false,
		}
	}

	fn send_raw(&self, message: &Value) -> bool {
		self.send_message(Message::Text(js::stringify(message).into()))
	}

	fn send(&self, op: u64, mut d: Map<String, Value>, timeout: f64) -> Reply {
		let open = self.ws.borrow().as_ref().is_some_and(|s| s.open.get());
		if !open {
			return Box::pin(async { Err(Error::from("OBS not connected")) });
		}
		let id = self.next_id.get().to_string();
		self.next_id.set(self.next_id.get() + 1);
		let (tx, rx) = oneshot::channel();
		self.pending.borrow_mut().insert(id.clone(), tx);
		d.insert("requestId".into(), id.clone().into());
		self.send_raw(&crate::obj! { "op" => op, "d" => Value::Object(d) });
		// The time starts now, not when the reply is first awaited.
		let deadline = tokio::time::Instant::now() + Duration::from_secs_f64(timeout.max(0.0) / 1000.0);
		let me = self.me.clone();
		Box::pin(async move {
			match tokio::time::timeout_at(deadline, rx).await {
				Ok(Ok(result)) => result,
				Ok(Err(_)) => Err(Error::from("OBS not connected")),
				Err(_) => {
					if let Some(me) = me.upgrade() {
						me.pending.borrow_mut().remove(&id);
					}
					Err(Error::from("OBS did not answer in time"))
				}
			}
		})
	}

	pub fn call(&self, request_type: &str, request_data: Value) -> Reply {
		self.call_with_timeout(request_type, request_data, 10000.0)
	}

	pub fn call_with_timeout(&self, request_type: &str, request_data: Value, timeout: f64) -> Reply {
		let mut d = Map::new();
		d.insert("requestType".into(), request_type.into());
		d.insert("requestData".into(), request_data);
		self.send(OP_REQUEST, d, timeout)
	}

	/// requests: [{ requestType, requestData }]. Resolves with the raw results.
	pub fn batch(&self, requests: Vec<Value>, execution: BatchExecution, halt_on_failure: bool, timeout: Option<f64>) -> Reply<Vec<Value>> {
		let wait = timeout.unwrap_or_else(|| (requests.len() as f64 * 100.0).max(10000.0));
		let mut d = Map::new();
		d.insert("executionType".into(), (execution as u8).into());
		d.insert("haltOnFailure".into(), halt_on_failure.into());
		d.insert("requests".into(), Value::Array(requests));
		let reply = self.send(OP_REQUEST_BATCH, d, wait);
		Box::pin(async move {
			match reply.await? {
				Value::Array(results) => Ok(results),
				_ => Ok(Vec::new()),
			}
		})
	}
}

fn obs_error(request_type: &Value, status: &Value) -> Error {
	match status["comment"].as_str() {
		Some(comment) if !comment.is_empty() => comment.into(),
		_ => format!("{} failed (code {})", request_type.as_str().unwrap_or_default(), status["code"]).into(),
	}
}

// Why a connection failed, in the words the settings page knows.
fn attempt_error(client: &ObsClient, error: &tokio_tungstenite::tungstenite::Error) -> String {
	use tokio_tungstenite::tungstenite::Error as E;
	match error {
		E::Io(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => format!("Nothing answers on {}:{}", client.host, fmt(client.port)),
		E::Io(e) if e.to_string().contains("failed to lookup address") => format!("Host {} not found", client.host),
		E::Io(e) => without_os_error(&e.to_string()),
		// Something answers, but it is not obs-websocket. "Nothing answers" says it better.
		E::Http(_) => String::new(),
		other => without_os_error(&other.to_string()),
	}
}

// "Connection reset by peer (os error 104)" -> "Connection reset by peer"
fn without_os_error(message: &str) -> String {
	match message.rfind(" (os error ") {
		Some(i) if message.ends_with(')') => message[..i].to_string(),
		_ => message.to_string(),
	}
}

async fn run_socket(
	me: Rc<ObsClient>,
	serial: u64,
	request: tokio_tungstenite::tungstenite::handshake::client::Request,
	mut rx: mpsc::UnboundedReceiver<Message>,
	open: Rc<Cell<bool>>,
) {
	let attempt = Attempt::default();
	let stream = match tokio_tungstenite::connect_async(request).await {
		Ok((stream, _)) => stream,
		Err(error) => {
			if !me.is_current(serial) {
				return;
			}
			let message = attempt_error(&me, &error);
			if !message.is_empty() {
				*attempt.error.borrow_mut() = message;
			}
			me.closed(serial, &attempt, 1006, "");
			return;
		}
	};
	if !me.is_current(serial) {
		return;
	}
	open.set(true);
	let (mut sink, mut stream) = stream.split();
	let writer = spawn_local(async move {
		while let Some(message) = rx.recv().await {
			if sink.send(message).await.is_err() {
				break;
			}
		}
	});
	let mut code = 1006;
	let mut reason = String::new();
	while let Some(message) = stream.next().await {
		if !me.is_current(serial) {
			writer.abort();
			return;
		}
		match message {
			Ok(Message::Text(text)) => me.on_message(serial, &attempt, &text),
			Ok(Message::Close(frame)) => {
				match frame {
					Some(frame) => {
						code = frame.code.into();
						reason = frame.reason.to_string();
					}
					None => code = 1005,
				}
				break;
			}
			Ok(_) => {}
			Err(error) => {
				let message = attempt_error(&me, &error);
				if !message.is_empty() {
					*attempt.error.borrow_mut() = message;
				}
				break;
			}
		}
	}
	open.set(false);
	writer.abort();
	me.closed(serial, &attempt, code, &reason);
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn auth_matches_the_example_from_the_protocol_docs() {
		assert_eq!(
			auth_response(
				"supersecretpassword",
				"lM1GncleQOaCu9lT1yeUZhFYnqhsLLP1G5lAGo3ixaI=",
				"+IxH4CnCiqpX1rM9scsNynZzbOe4KhDeYcTNS3PDaeY="
			),
			"1Ct943GAT+6YQUUX47Ia/ncufilbe6+oD6lY+5kaCu4="
		);
	}
}
