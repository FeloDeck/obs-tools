// Virtual Stream Deck for OpenDeck: a device plugin that shows the keys in a browser
// and has a small HTTP API, so plugins can be tested without hardware.
//
//   http://127.0.0.1:57190/            the deck (click keys, scroll or drag dials)
//   GET  /api/state                    images and device info as JSON
//   GET  /api/key/<n>.png              image of key n
//   GET  /api/dial/<n>.jpg             image of the touch strip above dial n
//   POST /api/key/<n>/press?ms=100     press and release (down, up also work)
//   POST /api/dial/<n>/rotate?ticks=1  turn (negative = left)
//   POST /api/dial/<n>/press           press and release (down, up also work)
//   POST /api/touch/<n>?hold=0         tap the touch strip
//
// Settings (environment): VIRTUAL_DECK_PORT, VIRTUAL_DECK_ROWS, VIRTUAL_DECK_COLUMNS, VIRTUAL_DECK_DIALS
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc};
use tokio_tungstenite::tungstenite::Message;

fn env_number(name: &str, fallback: u64) -> u64 {
	std::env::var(name).ok().and_then(|v| v.parse().ok()).filter(|n| *n > 0).unwrap_or(fallback)
}

fn log(message: &str) {
	let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
	println!("[{ms}] {message}");
}

struct Deck {
	device: Value,
	keys: Vec<Value>,
	dials: Vec<Value>,
	brightness: Value,
}

#[derive(Clone)]
struct Shared {
	deck: Arc<Mutex<Deck>>,
	events: broadcast::Sender<String>,
	opendeck: mpsc::UnboundedSender<String>,
	id: String,
}

impl Shared {
	fn send(&self, event: &str, payload: Value) {
		let _ = self.opendeck.send(json!({ "event": event, "payload": payload }).to_string());
	}

	fn broadcast(&self, data: Value) {
		let _ = self.events.send(format!("data: {data}\n\n"));
	}
}

// ---------- OpenDeck ----------

fn on_opendeck_message(shared: &Shared, text: &str) {
	let Ok(message) = serde_json::from_str::<Value>(text) else { return };
	if message["device"] != shared.id.as_str() {
		return;
	}
	match message["event"].as_str() {
		Some("setImage") => {
			let image = message.get("image").cloned().unwrap_or(Value::Null);
			let Some(position) = message["position"].as_u64() else {
				{
					let mut guard = shared.deck.lock().expect("deck");
					let deck = &mut *guard;
					deck.keys.iter_mut().chain(deck.dials.iter_mut()).for_each(|i| *i = Value::Null);
				}
				shared.broadcast(json!({ "type": "clear" }));
				return;
			};
			let encoder = message["controller"] == "Encoder";
			{
				let mut deck = shared.deck.lock().expect("deck");
				let list = if encoder { &mut deck.dials } else { &mut deck.keys };
				if let Some(slot) = list.get_mut(position as usize) {
					*slot = image.clone();
				}
			}
			shared.broadcast(json!({ "type": "image", "controller": message["controller"], "position": position, "image": image }));
		}
		Some("setBrightness") => {
			shared.deck.lock().expect("deck").brightness = message["brightness"].clone();
			shared.broadcast(json!({ "type": "brightness", "brightness": message["brightness"] }));
		}
		_ => {}
	}
}

async fn key_action(shared: &Shared, position: u64, what: &str, ms: u64) {
	let payload = json!({ "device": shared.id, "position": position });
	if what == "down" || what == "press" {
		shared.send("keyDown", payload.clone());
	}
	if what == "press" {
		tokio::time::sleep(Duration::from_millis(ms)).await;
	}
	if what == "up" || what == "press" {
		shared.send("keyUp", payload);
	}
}

async fn dial_action(shared: &Shared, position: u64, what: &str, query: &HashMap<String, String>) {
	let number = |key: &str| query.get(key).and_then(|v| v.parse::<i64>().ok()).filter(|n| *n != 0);
	if what == "rotate" {
		shared.send("encoderChange", json!({ "device": shared.id, "position": position, "ticks": number("ticks").unwrap_or(1) }));
	}
	if what == "down" || what == "press" {
		shared.send("encoderDown", json!({ "device": shared.id, "position": position }));
	}
	if what == "press" {
		tokio::time::sleep(Duration::from_millis(number("ms").unwrap_or(100) as u64)).await;
	}
	if what == "up" || what == "press" {
		shared.send("encoderUp", json!({ "device": shared.id, "position": position }));
	}
}

// ---------- HTTP ----------

fn data_url(url: &Value) -> Option<(String, Vec<u8>)> {
	let url = url.as_str()?;
	let rest = url.strip_prefix("data:")?;
	let (meta, data) = rest.split_once(',')?;
	let (kind, base64) = match meta.strip_suffix(";base64") {
		Some(kind) => (kind, true),
		None => (meta, false),
	};
	if kind.is_empty() || kind.contains(';') {
		return None;
	}
	let body = if base64 { base64::engine::general_purpose::STANDARD.decode(data).ok()? } else { percent_decode(data) };
	Some((kind.to_string(), body))
}

fn percent_decode(text: &str) -> Vec<u8> {
	let bytes = text.as_bytes();
	let mut out = Vec::with_capacity(bytes.len());
	let mut i = 0;
	while i < bytes.len() {
		if bytes[i] == b'%'
			&& i + 2 < bytes.len()
			&& let Ok(byte) = u8::from_str_radix(&text[i + 1..i + 3], 16)
		{
			out.push(byte);
			i += 3;
			continue;
		}
		out.push(bytes[i]);
		i += 1;
	}
	out
}

// parseInt: the digits at the start.
fn leading_number(text: &str) -> Option<u64> {
	let digits: String = text.chars().take_while(|c| c.is_ascii_digit()).collect();
	digits.parse().ok()
}

fn deck_html() -> Option<Vec<u8>> {
	let mut places = vec![PathBuf::from("deck.html")];
	if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(PathBuf::from)) {
		places.push(dir.join("deck.html"));
	}
	places.iter().find_map(|p| std::fs::read(p).ok())
}

async fn respond(stream: &mut TcpStream, status: &str, kind: &str, body: &[u8]) {
	let head = format!("HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
	let _ = stream.write_all(head.as_bytes()).await;
	let _ = stream.write_all(body).await;
}

async fn serve(mut stream: TcpStream, shared: Shared) {
	let mut request = Vec::new();
	let mut buffer = [0u8; 4096];
	while !request.windows(4).any(|w| w == b"\r\n\r\n") {
		match stream.read(&mut buffer).await {
			Ok(0) | Err(_) => return,
			Ok(n) => request.extend_from_slice(&buffer[..n]),
		}
		if request.len() > 65536 {
			return;
		}
	}
	let text = String::from_utf8_lossy(&request);
	let mut first = text.lines().next().unwrap_or("").split(' ');
	let method = first.next().unwrap_or("").to_string();
	let target = first.next().unwrap_or("/").to_string();
	let (path, query) = target.split_once('?').unwrap_or((&target, ""));
	let query: HashMap<String, String> =
		query.split('&').filter_map(|p| p.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect();
	let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
	let (keys, dials) = {
		let deck = shared.deck.lock().expect("deck");
		(deck.keys.len() as u64, deck.dials.len() as u64)
	};
	if method == "GET" && path == "/" {
		match deck_html() {
			Some(html) => respond(&mut stream, "200 OK", "text/html; charset=utf-8", &html).await,
			None => respond(&mut stream, "404 Not Found", "text/plain", b"deck.html not found").await,
		}
	} else if method == "GET" && path == "/events" {
		let mut events = shared.events.subscribe();
		let state = {
			let deck = shared.deck.lock().expect("deck");
			json!({ "type": "state", "device": deck.device, "keys": deck.keys, "dials": deck.dials, "brightness": deck.brightness })
		};
		let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n";
		if stream.write_all(head.as_bytes()).await.is_err() || stream.write_all(format!("data: {state}\n\n").as_bytes()).await.is_err() {
			return;
		}
		while let Ok(event) = events.recv().await {
			if stream.write_all(event.as_bytes()).await.is_err() {
				return;
			}
		}
	} else if method == "GET" && path == "/api/state" {
		let state = {
			let deck = shared.deck.lock().expect("deck");
			json!({ "device": deck.device, "brightness": deck.brightness, "keys": deck.keys, "dials": deck.dials })
		};
		respond(&mut stream, "200 OK", "application/json", state.to_string().as_bytes()).await;
	} else if method == "GET" && parts.len() >= 3 && parts[0] == "api" && (parts[1] == "key" || parts[1] == "dial") {
		let image = leading_number(parts[2]).and_then(|i| {
			let deck = shared.deck.lock().expect("deck");
			let list = if parts[1] == "key" { &deck.keys } else { &deck.dials };
			list.get(i as usize).cloned()
		});
		match image.as_ref().and_then(data_url) {
			Some((kind, body)) => respond(&mut stream, "200 OK", &kind, &body).await,
			None => respond(&mut stream, "404 Not Found", "text/plain", b"no image").await,
		}
	} else if method == "POST" && parts.len() >= 3 && parts[0] == "api" {
		let index = leading_number(parts[2]);
		let what = parts.get(3).copied().unwrap_or("press");
		match (parts[1], index) {
			("key", Some(i)) if i < keys => {
				let ms = query.get("ms").and_then(|v| v.parse::<u64>().ok()).filter(|n| *n > 0).unwrap_or(100);
				key_action(&shared, i, what, ms).await;
			}
			("dial", Some(i)) if i < dials => dial_action(&shared, i, what, &query).await,
			("touch", Some(i)) if i < dials => {
				shared.send(
					"touchscreenPress",
					json!({ "device": shared.id, "position": i, "x": 100, "y": 50, "hold": query.get("hold").map(String::as_str) == Some("1") }),
				);
			}
			_ => {
				respond(&mut stream, "400 Bad Request", "text/plain", b"bad request").await;
				return;
			}
		}
		let _ = stream.write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n").await;
	} else {
		respond(&mut stream, "404 Not Found", "text/plain", b"not found").await;
	}
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
	let args: Vec<String> = std::env::args().collect();
	let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned().unwrap_or_default();
	let http_port = env_number("VIRTUAL_DECK_PORT", 57190);
	let (rows, columns, encoders) =
		(env_number("VIRTUAL_DECK_ROWS", 3), env_number("VIRTUAL_DECK_COLUMNS", 5), env_number("VIRTUAL_DECK_DIALS", 4));
	let id = "vd-virtual-deck".to_string();
	let device = json!({ "id": id, "name": "Virtual Stream Deck", "rows": rows, "columns": columns, "encoders": encoders, "touchpoints": 0, "type": 7 });

	let (opendeck_tx, mut opendeck_rx) = mpsc::unbounded_channel::<String>();
	let (events, _) = broadcast::channel(256);
	let shared = Shared {
		deck: Arc::new(Mutex::new(Deck {
			device: device.clone(),
			keys: vec![Value::Null; (rows * columns) as usize],
			dials: vec![Value::Null; encoders as usize],
			brightness: json!(100),
		})),
		events,
		opendeck: opendeck_tx,
		id,
	};

	let socket = match tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{}", arg("-port"))).await {
		Ok((socket, _)) => socket,
		Err(error) => {
			log(&format!("cannot reach OpenDeck: {error}"));
			std::process::exit(1);
		}
	};
	let (mut sink, mut stream) = socket.split();
	let _ = sink.send(Message::Text(json!({ "event": arg("-registerEvent"), "uuid": arg("-pluginUUID") }).to_string().into())).await;
	shared.send("registerDevice", device);
	log(&format!("registered vd-virtual-deck ({rows}x{columns}, {encoders} dials), UI on http://127.0.0.1:{http_port}/"));
	tokio::spawn(async move {
		while let Some(text) = opendeck_rx.recv().await {
			if sink.send(Message::Text(text.into())).await.is_err() {
				break;
			}
		}
	});

	match TcpListener::bind(("127.0.0.1", http_port as u16)).await {
		Ok(listener) => {
			let shared = shared.clone();
			tokio::spawn(async move {
				while let Ok((connection, _)) = listener.accept().await {
					tokio::spawn(serve(connection, shared.clone()));
				}
			});
		}
		Err(error) => log(&format!("HTTP server: {error}")),
	}

	while let Some(message) = stream.next().await {
		match message {
			Ok(Message::Text(text)) => on_opendeck_message(&shared, text.as_str()),
			Ok(Message::Close(_)) | Err(_) => break,
			Ok(_) => {}
		}
	}
	std::process::exit(0);
}
