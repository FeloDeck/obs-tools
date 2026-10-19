// Messages to OpenDeck (or any host that speaks the Stream Deck SDK protocol). Sending never waits:
// the messages go to a queue, in order, and main.rs passes them on through openaction.
use std::cell::RefCell;
use std::rc::Rc;

use serde_json::{Map, Value};
use tokio::sync::mpsc::UnboundedSender;

use crate::js::{self, iso_time, now};

#[derive(Clone)]
pub struct Sd {
	tx: UnboundedSender<String>,
	uuid: Rc<str>,
	/// Tests read what was sent from here instead of a socket.
	pub sent: Option<Rc<RefCell<Vec<Value>>>>,
}

impl Sd {
	pub fn new(tx: UnboundedSender<String>, uuid: &str) -> Self {
		Sd { tx, uuid: uuid.into(), sent: None }
	}

	/// For tests: keeps every message.
	pub fn recording() -> Self {
		let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
		std::mem::forget(rx);
		Sd { tx, uuid: "test".into(), sent: Some(Rc::new(RefCell::new(Vec::new()))) }
	}

	pub fn send(&self, message: Value) {
		if let Some(sent) = &self.sent {
			sent.borrow_mut().push(message.clone());
		}
		let _ = self.tx.send(js::stringify(&message));
	}

	fn to_context(&self, event: &str, context: &str, payload: Option<Value>) {
		let mut message = Map::new();
		message.insert("event".into(), event.into());
		message.insert("context".into(), context.into());
		if let Some(payload) = payload {
			message.insert("payload".into(), payload);
		}
		self.send(Value::Object(message));
	}

	pub fn set_title(&self, context: &str, title: &str, state: Option<u16>) {
		let mut payload = crate::obj! { "title" => title, "target" => 0 };
		if let Some(state) = state {
			payload["state"] = state.into();
		}
		self.to_context("setTitle", context, Some(payload));
	}

	/// image: data URL, path inside the plugin, or "" to go back to the default image.
	pub fn set_image(&self, context: &str, image: &str, state: Option<u16>) {
		let mut payload = crate::obj! { "image" => image, "target" => 0 };
		if let Some(state) = state {
			payload["state"] = state.into();
		}
		self.to_context("setImage", context, Some(payload));
	}

	pub fn set_state(&self, context: &str, state: u16) {
		self.to_context("setState", context, Some(crate::obj! { "state" => state }));
	}

	pub fn set_feedback(&self, context: &str, payload: Value) {
		self.to_context("setFeedback", context, Some(payload));
	}

	pub fn set_feedback_layout(&self, context: &str, layout: &str) {
		self.to_context("setFeedbackLayout", context, Some(crate::obj! { "layout" => layout }));
	}

	pub fn show_ok(&self, context: &str) {
		self.to_context("showOk", context, None);
	}

	pub fn show_alert(&self, context: &str) {
		self.to_context("showAlert", context, None);
	}

	pub fn set_settings(&self, context: &str, settings: &Map<String, Value>) {
		self.to_context("setSettings", context, Some(Value::Object(settings.clone())));
	}

	pub fn set_global_settings(&self, settings: &Map<String, Value>) {
		let uuid = self.uuid.clone();
		self.to_context("setGlobalSettings", &uuid, Some(Value::Object(settings.clone())));
	}

	pub fn get_global_settings(&self) {
		let uuid = self.uuid.clone();
		self.to_context("getGlobalSettings", &uuid, None);
	}

	pub fn send_to_property_inspector(&self, context: &str, payload: Value) {
		self.to_context("sendToPropertyInspector", context, Some(payload));
	}

	pub fn open_url(&self, url: &str) {
		self.send(crate::obj! { "event" => "openUrl", "payload" => crate::obj! { "url" => url } });
	}

	/// Goes to the plugin log file (OpenDeck catches stdout).
	pub fn log(&self, message: &str) {
		log(message);
	}

	/// Also goes to the OpenDeck log.
	pub fn warn(&self, message: &str) {
		eprintln!("[{}] WARN {message}", iso_time(now()));
		self.send(crate::obj! { "event" => "logMessage", "payload" => crate::obj! { "message" => message } });
	}
}

pub fn log(message: &str) {
	println!("[{}] {message}", iso_time(now()));
}
