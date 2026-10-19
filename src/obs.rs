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
use crate::js::{self, fmt, truthy};
use crate::timers::{Timer, set_timeout};
use crate::{Error, Res};

const OP_HELLO: f64 = 0.0;
const OP_IDENTIFY: u8 = 1;
const OP_IDENTIFIED: f64 = 2.0;
const OP_REIDENTIFY: u8 = 3;
const OP_EVENT: f64 = 5.0;
const OP_REQUEST: u8 = 6;
const OP_REQUEST_RESPONSE: f64 = 7.0;
const OP_REQUEST_BATCH: u8 = 8;
const OP_REQUEST_BATCH_RESPONSE: f64 = 9.0;

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

/// Status values:
///   idle        not started
///   connecting  waiting for the socket or the handshake
///   connected   identified, requests work
///   offline     nothing listens on host:port (OBS closed or server off)
///   auth        wrong or missing password
///   error       anything else, see last_error
pub struct ObsClient {
	pub host: String,
	pub port: f64,
	pub password: String,
	pub name: String,
	status: RefCell<String>,
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
	pub on_status: Emitter<(String, String)>,
	pub on_connected: Emitter<()>,
	pub on_disconnected: Emitter<()>,
	/// (eventType, eventData)
	pub on_event: Emitter<(String, Value)>,
	// Listeners for one event type, called after the ones of on_event, like emit(eventType) in Node.
	on_typed: Emitter<(String, Value)>,
}

impl ObsClient {
	pub fn new(host: &str, port: f64, password: &str, name: &str) -> Rc<Self> {
		Rc::new_cyclic(|me| ObsClient {
			host: host.to_string(),
			port: if js::truthy_num(port) { port } else { 4455.0 },
			password: password.to_string(),
			name: name.to_string(),
			status: RefCell::new("idle".into()),
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

	pub fn status(&self) -> String {
		self.status.borrow().clone()
	}

	pub fn last_error(&self) -> String {
		self.last_error.borrow().clone()
	}

	/// { obs, websocket, rpc, platform } once OBS said hello.
	pub fn version(&self) -> Option<Value> {
		self.version.borrow().clone().map(Value::Object)
	}

	pub fn connected(&self) -> bool {
		*self.status.borrow() == "connected"
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
		self.set_status("idle", "");
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

	fn set_status(&self, status: &str, error: &str) {
		let changed = *self.status.borrow() != status || *self.last_error.borrow() != error;
		*self.status.borrow_mut() = status.to_string();
		*self.last_error.borrow_mut() = error.to_string();
		if changed {
			self.on_status.emit(&(status.to_string(), error.to_string()));
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
		if !matches!(self.status.borrow().as_str(), "offline" | "auth" | "error") {
			self.set_status("connecting", "");
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
					me.set_status("offline", &format!("Host {} not found", me.host));
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
				self.set_status("error", &error.to_string());
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
		let op = js::number(message.get("op"));
		let d = message.get("d").cloned().unwrap_or(Value::Null);
		if op == OP_HELLO {
			let mut identify = Map::new();
			identify.insert("rpcVersion".into(), 1.into());
			identify.insert("eventSubscriptions".into(), self.subscriptions.get().into());
			let auth = d.get("authentication");
			if truthy(auth) {
				if self.password.is_empty() {
					attempt.close_code.set(CLOSE_AUTH_FAILED);
					*attempt.error.borrow_mut() = "OBS asks for a password".into();
					self.send_message(Message::Close(Some(CloseFrame { code: CloseCode::Normal, reason: "".into() })));
					return;
				}
				let salt = js::to_string(auth.and_then(|a| a.get("salt")));
				let challenge = js::to_string(auth.and_then(|a| a.get("challenge")));
				identify.insert("authentication".into(), auth_response(&self.password, &salt, &challenge).into());
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
		} else if op == OP_IDENTIFIED {
			// The answer to a Reidentify.
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
				me.set_status("connected", "");
				me.on_connected.emit(&());
			});
		} else if op == OP_EVENT {
			let kind = js::to_string(d.get("eventType"));
			let data = match d.get("eventData") {
				None | Some(Value::Null) => Value::Object(Map::new()),
				Some(data) => data.clone(),
			};
			let event = (kind, data);
			self.on_event.emit(&event);
			self.on_typed.emit(&event);
		} else if op == OP_REQUEST_RESPONSE || op == OP_REQUEST_BATCH_RESPONSE {
			let id = js::to_string(d.get("requestId"));
			let Some(pending) = self.pending.borrow_mut().remove(&id) else { return };
			let result = if op == OP_REQUEST_BATCH_RESPONSE {
				Ok(match d.get("results") {
					None | Some(Value::Null) => Value::Array(Vec::new()),
					Some(results) => results.clone(),
				})
			} else if truthy(d.get("requestStatus").and_then(|s| s.get("result"))) {
				Ok(match d.get("responseData") {
					None | Some(Value::Null) => Value::Object(Map::new()),
					Some(data) => data.clone(),
				})
			} else {
				Err(obs_error(d.get("requestType"), d.get("requestStatus")))
			};
			let _ = pending.send(result);
		}
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
			self.set_status("auth", if error.is_empty() { "Wrong password" } else { &error });
		} else if code == CLOSE_RPC_VERSION {
			self.set_status("error", "This OBS version is not supported (needs obs-websocket 5)");
		} else if !was_connected {
			let nothing = format!("Nothing answers on {}:{}", self.host, fmt(self.port));
			self.set_status("offline", if error.is_empty() { &nothing } else { &error });
		} else {
			self.set_status("offline", if reason.is_empty() { "OBS closed the connection" } else { reason });
		}
		self.schedule_retry();
	}

	fn schedule_retry(&self) {
		if self.stopped.get() {
			return;
		}
		self.clear_retry();
		let delay =
			if *self.status.borrow() == "auth" { AUTH_RETRY_DELAY } else { RETRY_DELAYS[self.retry.get().min(RETRY_DELAYS.len() - 1)] };
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

	fn send(&self, op: u8, mut d: Map<String, Value>, timeout: f64) -> Reply {
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
		// The time starts now, like the setTimeout of the JavaScript version.
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
		let wait = timeout.unwrap_or_else(|| js::max(10000.0, requests.len() as f64 * 100.0));
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

fn obs_error(request_type: Option<&Value>, status: Option<&Value>) -> Error {
	let comment = status.and_then(|s| s.get("comment"));
	if truthy(comment) {
		return js::to_string(comment).into();
	}
	format!("{} failed (code {})", js::to_string(request_type), js::to_string(status.and_then(|s| s.get("code")))).into()
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
