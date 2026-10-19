// What press, hold and double press can do on a Set Text key. All three offer the same.
use serde_json::{Map, Value};

pub static TEXT_ACTIONS: &[&str] = &["none", "set", "toggle", "next", "previous", "first", "up", "down", "reset", "timer", "clear"];
pub static TEXT_GESTURES: &[&str] = &["press", "hold", "double"];

/// What a text action shows on the key afterwards.
pub static TEXT_KINDS: &[(&str, &str)] = &[
	("set", "text"),
	("toggle", "text"),
	("next", "line"),
	("previous", "line"),
	("first", "line"),
	("up", "count"),
	("down", "count"),
	("reset", "count"),
	("timer", "timer"),
	("clear", "cleared"),
];

pub fn text_kind(action: &str) -> Option<&'static str> {
	TEXT_KINDS.iter().find(|(a, _)| *a == action).map(|(_, k)| *k)
}

/// The action of a gesture. Press sets the text unless something else is picked.
pub fn text_gesture(settings: &Map<String, Value>, gesture: &str) -> String {
	if let Some(Value::String(value)) = settings.get(gesture)
		&& TEXT_ACTIONS.contains(&value.as_str())
	{
		return value.clone();
	}
	if gesture == "press" { "set".into() } else { "none".into() }
}

/// Does any gesture use something of this kind ("text", "line", "count" or "timer")?
pub fn text_uses(settings: &Map<String, Value>, kind: &str) -> bool {
	TEXT_GESTURES.iter().any(|g| text_kind(&text_gesture(settings, g)) == Some(kind))
}
