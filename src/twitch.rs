// Twitch: read chat for !replay (no login needed), create clips and post them (login needed).
use std::cell::{Cell, RefCell};
use std::hash::{BuildHasher, Hasher};
use std::rc::{Rc, Weak};
use std::sync::{Arc, LazyLock};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Map, Value};
use tokio::sync::mpsc;
use tokio::task::{AbortHandle, spawn_local};
use tokio_tungstenite::tungstenite::Message;

use crate::emitter::Emitter;
use crate::js::{self, truthy};
use crate::timers::{Timer, set_timeout, sleep};
use crate::{Error, Res};

const IRC_HOST: &str = "irc-ws.chat.twitch.tv";
const IRC_URL: &str = "wss://irc-ws.chat.twitch.tv:443";
const SCOPES: &str = "clips:edit user:write:chat";
const RECONNECT_MS: f64 = 5000.0;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct IrcMessage {
	pub tags: Vec<(String, String)>,
	pub prefix: Option<String>,
	pub command: Option<String>,
	pub params: Vec<String>,
}

impl IrcMessage {
	pub fn tag(&self, name: &str) -> Option<&str> {
		self.tags.iter().rev().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
	}
}

// String.slice(start, end) with a negative end counting from the back, in characters.
fn slice(s: &str, start: usize, end: Option<isize>) -> String {
	let chars: Vec<char> = s.chars().collect();
	let len = chars.len() as isize;
	let end = match end {
		None => len,
		Some(e) if e < 0 => (len + e).max(0),
		Some(e) => e.min(len),
	};
	let start = (start as isize).min(len);
	if start >= end { String::new() } else { chars[start as usize..end as usize].iter().collect() }
}

fn index_of(s: &str, needle: &str) -> isize {
	s.find(needle).map_or(-1, |byte| s[..byte].chars().count() as isize)
}

pub fn parse_irc(line: &str) -> IrcMessage {
	let mut message = IrcMessage::default();
	let mut rest = line.to_string();
	if rest.starts_with('@') {
		let space = index_of(&rest, " ");
		for pair in slice(&rest, 1, Some(space)).split(';') {
			match pair.find('=') {
				None => message.tags.push((pair.to_string(), String::new())),
				Some(i) => message.tags.push((pair[..i].to_string(), pair[i + 1..].to_string())),
			}
		}
		rest = slice(&rest, (space + 1) as usize, None);
	}
	if rest.starts_with(':') {
		let space = index_of(&rest, " ");
		message.prefix = Some(slice(&rest, 1, Some(space)));
		rest = slice(&rest, (space + 1) as usize, None);
	}
	let mut trailing = None;
	if let Some(index) = rest.find(" :") {
		trailing = Some(rest[index + 2..].to_string());
		rest = rest[..index].to_string();
	}
	let mut parts: Vec<String> = rest.split(' ').filter(|p| !p.is_empty()).map(String::from).collect();
	message.command = if parts.is_empty() { None } else { Some(parts.remove(0)) };
	if let Some(trailing) = trailing {
		parts.push(trailing);
	}
	message.params = parts;
	message
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChatUser {
	pub login: String,
	pub name: String,
	pub broadcaster: bool,
	pub r#mod: bool,
	pub vip: bool,
	pub sub: bool,
}

pub fn chat_user(message: &IrcMessage) -> ChatUser {
	let badges: Vec<String> = message
		.tag("badges")
		.unwrap_or("")
		.split(',')
		.filter(|b| !b.is_empty())
		.map(|b| b.split('/').next().unwrap_or("").to_string())
		.collect();
	let has = |b: &str| badges.iter().any(|x| x == b);
	let login = message.prefix.as_deref().map(|p| p.split('!').next().unwrap_or("").to_string()).unwrap_or_default();
	let name = message.tag("display-name").filter(|n| !n.is_empty()).map_or_else(|| login.clone(), String::from);
	ChatUser {
		name,
		broadcaster: has("broadcaster"),
		r#mod: has("moderator") || message.tag("mod") == Some("1"),
		vip: has("vip") || message.tag("vip") == Some("1"),
		sub: has("subscriber") || has("founder") || message.tag("subscriber") == Some("1"),
		login,
	}
}

/// permission: everyone, subs, vips, mods
pub fn is_allowed(user: &ChatUser, permission: &str) -> bool {
	if user.broadcaster || permission == "everyone" {
		return true;
	}
	match permission {
		"mods" => user.r#mod,
		"vips" => user.r#mod || user.vip,
		"subs" => user.r#mod || user.vip || user.sub,
		_ => false,
	}
}

pub fn normalize_channel(name: &str) -> String {
	let mut s = js::trim(name).to_string();
	let lower = s.to_lowercase();
	for scheme in ["https://", "http://"] {
		if let Some(after) = lower.strip_prefix(scheme) {
			let www = if after.starts_with("www.") { 4 } else { 0 };
			if after[www..].starts_with("twitch.tv/") {
				s = s[scheme.len() + www + "twitch.tv/".len()..].to_string();
			}
			break;
		}
	}
	let s = s.strip_prefix('#').unwrap_or(&s);
	s.split(['/', '?']).next().unwrap_or("").to_lowercase()
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
	pub user: ChatUser,
	pub text: String,
	pub channel: String,
}

#[derive(Clone)]
struct DeviceFlow {
	id: u64,
	client_id: String,
	device_code: String,
	user_code: Value,
	uri: Value,
	expires: f64,
	interval: f64,
}

struct ChatSocket {
	serial: u64,
	tx: mpsc::UnboundedSender<Message>,
	task: AbortHandle,
}

pub struct Twitch {
	save: Box<dyn Fn(Map<String, Value>)>,
	log: Box<dyn Fn(&str)>,
	config: RefCell<Map<String, Value>>,
	chat_wanted: Cell<bool>,
	ws: RefCell<Option<ChatSocket>>,
	chat_channel: RefCell<Option<String>>,
	chat_connected: Cell<bool>,
	reconnect_timer: RefCell<Option<Timer>>,
	device_flow: RefCell<Option<DeviceFlow>>,
	next_id: Cell<u64>,
	me: Weak<Twitch>,
	pub on_status: Emitter<()>,
	pub on_error: Emitter<Error>,
	pub on_message: Emitter<ChatMessage>,
}

/// A flow the settings page shows: the code to enter and where.
pub struct Flow {
	pub user_code: Value,
	pub uri: Value,
}

fn form(pairs: &[(&str, &str)]) -> String {
	pairs.iter().map(|(k, v)| format!("{}={}", url_encode(k), url_encode(v))).collect::<Vec<_>>().join("&")
}

// Like URLSearchParams: letters, digits and *-._ stay, a space becomes +, the rest is %XX.
fn url_encode(s: &str) -> String {
	let mut out = String::new();
	for b in s.bytes() {
		match b {
			b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => out.push(b as char),
			b' ' => out.push('+'),
			_ => out.push_str(&format!("%{b:02X}")),
		}
	}
	out
}

struct Response {
	status: u16,
	text: String,
}

impl Response {
	fn ok(&self) -> bool {
		(200..300).contains(&self.status)
	}

	/// response.json()
	fn json(&self) -> Res<Value> {
		serde_json::from_str(&self.text).map_err(|e| Error::from(e.to_string()))
	}

	/// response.json().catch(() => ({}))
	fn json_or_empty(&self) -> Value {
		self.json().unwrap_or_else(|_| Value::Object(Map::new()))
	}
}

static AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| ureq::Agent::config_builder().http_status_as_error(false).build().into());

// One HTTPS request, on a thread of its own so the plugin does not wait.
async fn fetch(
	method: &'static str,
	url: String,
	headers: Vec<(&'static str, String)>,
	body: Option<(&'static str, String)>,
) -> Res<Response> {
	tokio::task::spawn_blocking(move || -> Result<Response, ureq::Error> {
		let mut response = match method {
			"GET" => {
				let mut request = AGENT.get(&url);
				for (k, v) in &headers {
					request = request.header(*k, v.as_str());
				}
				request.call()?
			}
			_ => {
				let mut request = AGENT.post(&url);
				for (k, v) in &headers {
					request = request.header(*k, v.as_str());
				}
				match body {
					Some((content_type, body)) => request.header("Content-Type", content_type).send(body.as_str())?,
					None => request.send_empty()?,
				}
			}
		};
		let status = response.status().as_u16();
		let text = response.body_mut().read_to_string()?;
		Ok(Response { status, text })
	})
	.await
	.map_err(|e| Error::from(e.to_string()))?
	.map_err(|e| Error::from(e.to_string()))
}

fn random_nick() -> String {
	let random = std::collections::hash_map::RandomState::new().build_hasher().finish();
	format!("justinfan{}", 10000 + random % 80000)
}

fn tls_config() -> Arc<rustls::ClientConfig> {
	static CONFIG: LazyLock<Arc<rustls::ClientConfig>> = LazyLock::new(|| {
		let mut roots = rustls::RootCertStore::empty();
		roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
		let config = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
			.with_safe_default_protocol_versions()
			.expect("TLS versions")
			.with_root_certificates(roots)
			.with_no_client_auth();
		Arc::new(config)
	});
	CONFIG.clone()
}

async fn connect_chat() -> Res<tokio_tungstenite::WebSocketStream<tokio_rustls::client::TlsStream<tokio::net::TcpStream>>> {
	let tcp = tokio::net::TcpStream::connect((IRC_HOST, 443)).await.map_err(|e| Error::from(e.to_string()))?;
	let name = rustls::pki_types::ServerName::try_from(IRC_HOST).map_err(|e| Error::from(e.to_string()))?;
	let tls = tokio_rustls::TlsConnector::from(tls_config()).connect(name, tcp).await.map_err(|e| Error::from(e.to_string()))?;
	let (ws, _) = tokio_tungstenite::client_async(IRC_URL, tls).await.map_err(|e| Error::from(e.to_string()))?;
	Ok(ws)
}

impl Twitch {
	pub fn new(save: impl Fn(Map<String, Value>) + 'static, log: impl Fn(&str) + 'static) -> Rc<Self> {
		Rc::new_cyclic(|me| Twitch {
			save: Box::new(save),
			log: Box::new(log),
			config: RefCell::new(Map::new()),
			chat_wanted: Cell::new(false),
			ws: RefCell::new(None),
			chat_channel: RefCell::new(None),
			chat_connected: Cell::new(false),
			reconnect_timer: RefCell::new(None),
			device_flow: RefCell::new(None),
			next_id: Cell::new(1),
			me: me.clone(),
			on_status: Emitter::default(),
			on_error: Emitter::default(),
			on_message: Emitter::default(),
		})
	}

	fn get(&self, key: &str) -> Option<Value> {
		self.config.borrow().get(key).cloned()
	}

	fn get_str(&self, key: &str) -> String {
		js::str_or_empty(self.get(key).as_ref())
	}

	pub fn configure(&self, config: Option<&Value>) {
		*self.config.borrow_mut() = js::object(config);
		self.update_chat();
	}

	pub fn channel(&self) -> String {
		let channel = self.get("channel");
		let pick = if truthy(channel.as_ref()) { channel } else { self.get("login") };
		normalize_channel(&js::str_or_empty(pick.as_ref()))
	}

	pub fn status(&self) -> Value {
		let login = self.get("login");
		let scopes = self.get("scopes");
		let flow = self.device_flow.borrow().as_ref().map(|f| crate::obj! { "userCode" => f.user_code.clone(), "uri" => f.uri.clone() });
		crate::obj! {
			"channel" => self.channel(),
			"login" => if truthy(login.as_ref()) { login.unwrap_or(Value::Null) } else { Value::from("") },
			"authorized" => truthy(self.get("token").as_ref()),
			"scopes" => if truthy(scopes.as_ref()) { scopes.unwrap_or(Value::Null) } else { Value::Array(Vec::new()) },
			"chatConnected" => self.chat_connected.get(),
			"chatWanted" => self.chat_wanted.get(),
			"deviceFlow" => flow.unwrap_or(Value::Null),
		}
	}

	pub fn set_chat_wanted(&self, wanted: bool) {
		if wanted == self.chat_wanted.get() {
			return;
		}
		self.chat_wanted.set(wanted);
		self.update_chat();
	}

	fn update_chat(&self) {
		let channel = if self.chat_wanted.get() { self.channel() } else { String::new() };
		if !channel.is_empty() && self.chat_channel.borrow().as_deref() == Some(channel.as_str()) && self.ws.borrow().is_some() {
			return;
		}
		self.close_chat();
		*self.chat_channel.borrow_mut() = if channel.is_empty() { None } else { Some(channel.clone()) };
		if !channel.is_empty() {
			self.open_chat();
		}
	}

	fn close_chat(&self) {
		if let Some(timer) = self.reconnect_timer.borrow_mut().take() {
			timer.clear();
		}
		let socket = self.ws.borrow_mut().take();
		if let Some(socket) = socket {
			let _ = socket.tx.send(Message::Close(None));
			socket.task.abort();
		}
		if self.chat_connected.get() {
			self.chat_connected.set(false);
			self.on_status.emit(&());
		}
	}

	fn open_chat(&self) {
		let Some(channel) = self.chat_channel.borrow().clone() else { return };
		let Some(me) = self.me.upgrade() else { return };
		let serial = self.next_id.get();
		self.next_id.set(serial + 1);
		let (tx, rx) = mpsc::unbounded_channel();
		let task = spawn_local(run_chat(me, serial, channel, tx.clone(), rx));
		*self.ws.borrow_mut() = Some(ChatSocket { serial, tx, task: task.abort_handle() });
	}

	fn chat_current(&self, serial: u64) -> bool {
		self.ws.borrow().as_ref().is_some_and(|s| s.serial == serial)
	}

	fn chat_line(&self, serial: u64, channel: &str, line: &str, tx: &mpsc::UnboundedSender<Message>) {
		let message = parse_irc(line);
		match message.command.as_deref() {
			Some("PING") => {
				let _ = tx.send(Message::Text(format!("PONG :{}", message.params.first().map_or("tmi.twitch.tv", |p| p.as_str())).into()));
			}
			Some("JOIN") if !self.chat_connected.get() => {
				self.chat_connected.set(true);
				(self.log)(&format!("Twitch chat: joined #{channel}"));
				self.on_status.emit(&());
			}
			Some("RECONNECT") => {
				let _ = tx.send(Message::Close(None));
			}
			Some("PRIVMSG") => {
				let _ = serial;
				self.on_message.emit(&ChatMessage {
					user: chat_user(&message),
					text: message.params.get(1).cloned().unwrap_or_default(),
					channel: channel.to_string(),
				});
			}
			_ => {}
		}
	}

	fn chat_closed(&self, serial: u64) {
		if !self.chat_current(serial) {
			return;
		}
		*self.ws.borrow_mut() = None;
		if self.chat_connected.get() {
			self.chat_connected.set(false);
			self.on_status.emit(&());
		}
		let me = self.me.clone();
		let timer = set_timeout(RECONNECT_MS, move || {
			let Some(me) = me.upgrade() else { return };
			if me.ws.borrow().is_none() && me.chat_channel.borrow().is_some() {
				me.open_chat();
			}
		});
		*self.reconnect_timer.borrow_mut() = Some(timer);
	}

	/// Login with a Client ID of a "Public" app from dev.twitch.tv (device code flow).
	pub async fn start_device_flow(&self, client_id: Option<&Value>) -> Res<Flow> {
		let client_id = js::trim(&js::str_or_empty(client_id)).to_string();
		if client_id.is_empty() {
			return Err("Enter the Client ID of your Twitch app first".into());
		}
		let response = fetch(
			"POST",
			"https://id.twitch.tv/oauth2/device".into(),
			vec![],
			Some(("application/x-www-form-urlencoded;charset=UTF-8", form(&[("client_id", &client_id), ("scopes", SCOPES)]))),
		)
		.await?;
		let data = response.json()?;
		if !response.ok() {
			return Err(twitch_error(&data, response.status));
		}
		let id = self.next_id.get();
		self.next_id.set(id + 1);
		let interval = js::number(data.get("interval"));
		let flow = DeviceFlow {
			id,
			client_id,
			device_code: js::to_string(data.get("device_code")),
			user_code: data.get("user_code").cloned().unwrap_or(Value::Null),
			uri: data.get("verification_uri").cloned().unwrap_or(Value::Null),
			expires: js::now() as f64 + js::number(data.get("expires_in")) * 1000.0,
			interval: js::max(1.0, if js::truthy_num(interval) { interval } else { 5.0 }) * 1000.0,
		};
		*self.device_flow.borrow_mut() = Some(flow.clone());
		self.on_status.emit(&());
		if let Some(me) = self.me.upgrade() {
			let poll = flow.clone();
			spawn_local(async move { me.poll_device_flow(poll).await });
		}
		Ok(Flow { user_code: flow.user_code, uri: flow.uri })
	}

	fn flow_is(&self, id: u64) -> bool {
		self.device_flow.borrow().as_ref().is_some_and(|f| f.id == id)
	}

	async fn poll_device_flow(&self, flow: DeviceFlow) {
		while self.flow_is(flow.id) && (js::now() as f64) < flow.expires {
			sleep(flow.interval).await;
			if !self.flow_is(flow.id) {
				return;
			}
			let result: Res<bool> = async {
				let body = form(&[
					("client_id", &flow.client_id),
					("scopes", SCOPES),
					("device_code", &flow.device_code),
					("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
				]);
				let response = fetch(
					"POST",
					"https://id.twitch.tv/oauth2/token".into(),
					vec![],
					Some(("application/x-www-form-urlencoded;charset=UTF-8", body)),
				)
				.await?;
				let data = response.json()?;
				if response.ok() && truthy(data.get("access_token")) {
					*self.device_flow.borrow_mut() = None;
					let refresh = js::str_or_empty(data.get("refresh_token"));
					self.use_token(data.get("access_token"), &refresh, &flow.client_id).await?;
					return Ok(true);
				}
				let message = data.get("message");
				if truthy(message) && !matches!(message.and_then(Value::as_str), Some("authorization_pending" | "slow_down")) {
					return Err(js::to_string(message).into());
				}
				Ok(false)
			}
			.await;
			match result {
				Ok(true) => return,
				Ok(false) => {}
				Err(error) => {
					*self.device_flow.borrow_mut() = None;
					self.on_error.emit(&error);
					self.on_status.emit(&());
					return;
				}
			}
		}
		if self.flow_is(flow.id) {
			*self.device_flow.borrow_mut() = None;
			self.on_error.emit(&Error::from("The Twitch login code expired"));
			self.on_status.emit(&());
		}
	}

	pub fn cancel_device_flow(&self) {
		*self.device_flow.borrow_mut() = None;
		self.on_status.emit(&());
	}

	/// Checks a token and stores it with the user it belongs to.
	pub async fn use_token(&self, token: Option<&Value>, refresh_token: &str, client_id: &str) -> Res<Value> {
		let token = js::trim(&js::str_or_empty(token)).to_string();
		let token = if token.len() >= 6 && token[..6].eq_ignore_ascii_case("oauth:") { token[6..].to_string() } else { token };
		let response =
			fetch("GET", "https://id.twitch.tv/oauth2/validate".into(), vec![("Authorization", format!("OAuth {token}"))], None).await?;
		let data = response.json_or_empty();
		if !response.ok() {
			let message = data.get("message");
			return Err(if truthy(message) { js::to_string(message) } else { "Twitch did not accept the token".into() }.into());
		}
		{
			let mut config = self.config.borrow_mut();
			let channel = config.get("channel").cloned();
			let pick = |key: &str, fallback: Value| match data.get(key) {
				Some(v) if truthy(Some(v)) => v.clone(),
				_ => fallback,
			};
			config.insert("token".into(), token.into());
			config.insert("refreshToken".into(), refresh_token.into());
			config.insert("clientId".into(), pick("client_id", client_id.into()));
			// login and user_id may be missing; JSON then has no such key, like undefined in JS.
			for (key, from) in [("login", "login"), ("userId", "user_id")] {
				match data.get(from) {
					Some(v) => config.insert(key.into(), v.clone()),
					None => config.remove(key),
				};
			}
			config.insert("scopes".into(), pick("scopes", Value::Array(Vec::new())));
			let channel = match channel {
				Some(c) if truthy(Some(&c)) => Some(c),
				_ => data.get("login").cloned(),
			};
			match channel {
				Some(c) => config.insert("channel".into(), c),
				None => config.remove("channel"),
			};
		}
		let config = self.config.borrow().clone();
		(self.save)(config);
		self.update_chat();
		self.on_status.emit(&());
		Ok(self.status())
	}

	pub fn logout(&self) {
		let channel = self.get("channel").filter(|c| truthy(Some(c))).unwrap_or_else(|| Value::from(""));
		let mut config = Map::new();
		config.insert("channel".into(), channel);
		*self.config.borrow_mut() = config.clone();
		(self.save)(config);
		self.on_status.emit(&());
	}

	async fn refresh(&self) -> bool {
		let (refresh_token, client_id) = (self.get_str("refreshToken"), self.get_str("clientId"));
		if refresh_token.is_empty() || client_id.is_empty() {
			return false;
		}
		let body = form(&[("grant_type", "refresh_token"), ("refresh_token", &refresh_token), ("client_id", &client_id)]);
		let Ok(response) = fetch(
			"POST",
			"https://id.twitch.tv/oauth2/token".into(),
			vec![],
			Some(("application/x-www-form-urlencoded;charset=UTF-8", body)),
		)
		.await
		else {
			return false;
		};
		let data = response.json_or_empty();
		if !response.ok() || !truthy(data.get("access_token")) {
			return false;
		}
		{
			let mut config = self.config.borrow_mut();
			config.insert("token".into(), data["access_token"].clone());
			if truthy(data.get("refresh_token")) {
				config.insert("refreshToken".into(), data["refresh_token"].clone());
			}
		}
		let config = self.config.borrow().clone();
		(self.save)(config);
		true
	}

	async fn helix(&self, method: &'static str, path: &str, query: &[(&str, &str)], body: Option<Value>) -> Res<Value> {
		let mut retry = true;
		loop {
			let token = self.get("token");
			if !truthy(token.as_ref()) {
				return Err("Log in to Twitch in the Instant Replay settings first".into());
			}
			let mut url = format!("https://api.twitch.tv/helix/{path}");
			if !query.is_empty() {
				url.push('?');
				url.push_str(&form(query));
			}
			let headers = vec![
				("Authorization", format!("Bearer {}", js::to_string(token.as_ref()))),
				("Client-Id", js::to_string(self.get("clientId").as_ref())),
			];
			let response = fetch(method, url, headers, body.as_ref().map(|b| ("application/json", js::stringify(b)))).await?;
			if response.status == 401 && retry && self.refresh().await {
				retry = false;
				continue;
			}
			let data = response.json_or_empty();
			if !response.ok() {
				return Err(twitch_error(&data, response.status));
			}
			return Ok(data);
		}
	}

	async fn broadcaster_id(&self) -> Res<Value> {
		let channel = self.channel();
		if channel.is_empty() {
			return Err("No Twitch channel set".into());
		}
		if channel == self.get_str("login") && truthy(self.get("userId").as_ref()) {
			return Ok(self.get("userId").unwrap_or(Value::Null));
		}
		let data = self.helix("GET", "users", &[("login", &channel)], None).await?;
		match data.get("data").and_then(|d| d.get(0)) {
			Some(user) if truthy(Some(user)) => Ok(user.get("id").cloned().unwrap_or(Value::Null)),
			_ => Err(format!("Twitch channel {channel} not found").into()),
		}
	}

	/// Clips only work while the channel is live. Returns the URL of the clip.
	pub async fn create_clip(&self) -> Res<String> {
		let id = js::to_string(Some(&self.broadcaster_id().await?));
		let data = self.helix("POST", "clips", &[("broadcaster_id", &id)], None).await?;
		match data.get("data").and_then(|d| d.get(0)) {
			Some(clip) if truthy(Some(clip)) => Ok(format!("https://clips.twitch.tv/{}", js::to_string(clip.get("id")))),
			_ => Err("Twitch did not create a clip".into()),
		}
	}

	pub async fn say(&self, message: &str) -> Res<()> {
		let id = self.broadcaster_id().await?;
		let mut body = Map::new();
		body.insert("broadcaster_id".into(), id);
		if let Some(user) = self.get("userId") {
			body.insert("sender_id".into(), user);
		}
		body.insert("message".into(), message.into());
		let data = self.helix("POST", "chat/messages", &[], Some(Value::Object(body))).await?;
		if let Some(result) = data.get("data").and_then(|d| d.get(0)).filter(|r| truthy(Some(r)))
			&& result.get("is_sent") == Some(&Value::Bool(false))
		{
			let reason = result.get("drop_reason").and_then(|r| r.get("message"));
			return Err(if truthy(reason) { js::to_string(reason) } else { "Twitch dropped the message".into() }.into());
		}
		Ok(())
	}
}

fn twitch_error(data: &Value, status: u16) -> Error {
	let message = data.get("message");
	if truthy(message) { js::to_string(message).into() } else { format!("Twitch said {status}").into() }
}

async fn run_chat(
	me: Rc<Twitch>,
	serial: u64,
	channel: String,
	tx: mpsc::UnboundedSender<Message>,
	mut rx: mpsc::UnboundedReceiver<Message>,
) {
	let stream = match connect_chat().await {
		Ok(stream) => stream,
		Err(_) => {
			me.chat_closed(serial);
			return;
		}
	};
	if !me.chat_current(serial) {
		return;
	}
	let (mut sink, mut stream) = stream.split();
	let nick = random_nick();
	for line in ["CAP REQ :twitch.tv/tags twitch.tv/commands", "PASS SCHMOOPIIE", &format!("NICK {nick}"), &format!("JOIN #{channel}")] {
		if sink.send(Message::Text(line.to_string().into())).await.is_err() {
			me.chat_closed(serial);
			return;
		}
	}
	let writer = spawn_local(async move {
		while let Some(message) = rx.recv().await {
			let close = matches!(message, Message::Close(_));
			if sink.send(message).await.is_err() || close {
				break;
			}
		}
	});
	while let Some(message) = stream.next().await {
		if !me.chat_current(serial) {
			writer.abort();
			return;
		}
		match message {
			Ok(Message::Text(text)) => {
				for line in text.as_str().split("\r\n") {
					if !line.is_empty() {
						me.chat_line(serial, &channel, line, &tx);
					}
				}
			}
			Ok(Message::Close(_)) | Err(_) => break,
			Ok(_) => {}
		}
	}
	writer.abort();
	me.chat_closed(serial);
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn irc_lines_with_tags() {
		let line = "@badge-info=subscriber/12;badges=moderator/1,subscriber/12;display-name=SomeMod;mod=1;subscriber=1 :somemod!somemod@somemod.tmi.twitch.tv PRIVMSG #chan :!replay please";
		let message = parse_irc(line);
		assert_eq!(message.command.as_deref(), Some("PRIVMSG"));
		assert_eq!(message.params, ["#chan", "!replay please"]);
		let user = chat_user(&message);
		assert_eq!(user.login, "somemod");
		assert_eq!(user.name, "SomeMod");
		assert!(user.r#mod && user.sub && !user.vip && !user.broadcaster);
		assert_eq!(parse_irc("PING :tmi.twitch.tv").params[0], "tmi.twitch.tv");
		assert!(is_allowed(&user, "mods"));
		assert!(!is_allowed(&ChatUser { sub: true, ..Default::default() }, "vips"));
		assert!(is_allowed(&ChatUser { broadcaster: true, ..Default::default() }, "mods"));
		assert_eq!(normalize_channel("https://www.twitch.tv/SomeOne/videos"), "someone");
		assert_eq!(normalize_channel("#Chan "), "chan");
	}

	#[test]
	fn form_bodies_like_url_search_params() {
		assert_eq!(form(&[("scopes", SCOPES), ("x", "ä&=")]), "scopes=clips%3Aedit+user%3Awrite%3Achat&x=%C3%A4%26%3D");
	}
}
