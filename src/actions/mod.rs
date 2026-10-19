// The actions. Each one is a Kind: its settings, what press, hold and double press can do, and the
// hooks it fills in (Behavior). key/ does the rest for every key: connection, gestures, images.
pub mod audio;
pub mod basics;
pub mod dials;
mod key;
pub mod outputs;
pub mod scenes;
pub mod sources;
pub mod stats;
pub mod text;

use std::any::Any;
use std::rc::Rc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value};

#[cfg(test)]
pub(crate) use key::testing;
pub use key::{Appearance, DOUBLE_PRESS_MS, Key, KeyRc, LONG_PRESS_MS, Stop};

use crate::Res;
use crate::obs_state::Change;
use crate::twitch::ChatMessage;

/// A key (Keypad) or a dial with its touch strip (Encoder).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Controller {
	#[default]
	Keypad,
	Encoder,
}

impl Controller {
	/// From the name OpenDeck uses.
	pub fn from_name(name: &str) -> Controller {
		if name == "Encoder" { Controller::Encoder } else { Controller::Keypad }
	}
}

/// The ways to press a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gesture {
	Press,
	Hold,
	Double,
}

impl Gesture {
	pub const ALL: [Gesture; 3] = [Gesture::Press, Gesture::Hold, Gesture::Double];

	/// Also the setting that holds what hold and double press do.
	pub fn name(self) -> &'static str {
		match self {
			Gesture::Press => "press",
			Gesture::Hold => "hold",
			Gesture::Double => "double",
		}
	}
}

/// A press, once it is clear which gesture it was.
#[derive(Debug, Clone, PartialEq)]
pub struct Press {
	/// What to do: one of the `ops` of the action, or "press" for an action without ops.
	pub op: String,
	/// Again and again while the key is held.
	pub repeat: bool,
}

/// An action: what all its keys have in common.
pub struct Kind {
	pub uuid: &'static str,
	/// For the log: "RecordTools: error: not recording".
	pub name: &'static str,
	pub defaults: fn() -> Map<String, Value>,
	/// Press, hold and double press. An action that can do more than one thing lists it in `ops`
	/// and keeps the press choice in the setting `press_key`. The settings `hold` and `double` pick
	/// from the same list. on_press gets the choice as `op`.
	pub press_key: &'static str,
	pub ops: &'static [&'static str],
	/// Press choices that use the key going down and up (push to talk): no hold or double press then.
	pub press_only: &'static [&'static str],
	/// Hold can also repeat the press while the key is held ("repeat").
	pub can_repeat: bool,
	/// Settings a key needs before it can work. Until then it shows a setup image.
	pub required: &'static [&'static str],
	pub behavior: &'static dyn Behavior,
	/// The action's own state, kept per key (Key::own).
	pub new_state: fn() -> Box<dyn Any>,
}

impl Kind {
	/// What an action does not have to set: one thing to do, nothing required, no state of its own.
	/// `Kind { uuid, name, defaults, behavior, ..Kind::BASE }`
	pub const BASE: Kind = Kind {
		uuid: "",
		name: "",
		defaults: Map::new,
		press_key: "",
		ops: &[],
		press_only: &[],
		can_repeat: false,
		required: &[],
		behavior: &NoHooks,
		new_state: || Box::new(()),
	};
}

struct NoHooks;

impl Behavior for NoHooks {}

/// Answers a message of the settings page, with the requestId it came with.
pub type PiReply = Rc<dyn Fn(Value)>;

/// The hooks of an action. All are optional.
#[async_trait(?Send)]
pub trait Behavior: Sync {
	/// The touch strip layout of a dial (layouts/*.json).
	fn layout(&self, _k: &KeyRc) -> Option<&'static str> {
		None
	}
	/// A key is set up when its required settings are filled in.
	fn is_set_up(&self, k: &KeyRc) -> bool {
		k.required_set()
	}
	/// What press, hold or double press does.
	fn gesture(&self, k: &KeyRc, gesture: Gesture) -> String {
		k.base_gesture(gesture)
	}
	fn wants_chat(&self, _k: &KeyRc) -> bool {
		false
	}
	fn on_chat_message(&self, _k: &KeyRc, _message: &ChatMessage) {}
	fn render(&self, _k: &KeyRc) {}
	fn render_offline(&self, _k: &KeyRc) {}
	fn on_disappear(&self, _k: &KeyRc) {}
	fn on_disconnected(&self, _k: &KeyRc) {}
	async fn on_connected(&self, _k: &KeyRc) -> Res<()> {
		Ok(())
	}
	async fn on_state_change(&self, k: &KeyRc, _change: Change) -> Res<()> {
		k.draw();
		Ok(())
	}
	async fn on_obs_event(&self, _k: &KeyRc, _kind: &str, _data: &Value) -> Res<()> {
		Ok(())
	}
	async fn on_settings_changed(&self, _k: &KeyRc, _old: &Map<String, Value>) -> Res<()> {
		Ok(())
	}
	async fn on_press(&self, _k: &KeyRc, _press: Press) -> Res<()> {
		Ok(())
	}
	async fn on_release(&self, _k: &KeyRc) -> Res<()> {
		Ok(())
	}
	async fn on_dial_rotate(&self, _k: &KeyRc, _ticks: f64, _pressed: bool) -> Res<()> {
		Ok(())
	}
	async fn on_dial_down(&self, _k: &KeyRc) -> Res<()> {
		Ok(())
	}
	async fn on_dial_up(&self, _k: &KeyRc) -> Res<()> {
		Ok(())
	}
	/// A touch does what a press does, unless touches are off.
	async fn on_touch_tap(&self, k: &KeyRc) -> Res<()> {
		if !k.settings::<Touch>().disable_touch {
			return self.on_dial_down(k).await;
		}
		Ok(())
	}
	async fn on_pi_message(&self, _k: &KeyRc, _payload: &Value, _reply: PiReply) -> Res<()> {
		Ok(())
	}
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Touch {
	disable_touch: bool,
}

/// The setting of keys that can show the name of their source, scene or filter.
#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct ShowName {
	pub show_name: bool,
}

/// Is this field of an OBS event this name? For events about the source (or scene, filter) of a key.
pub fn same_name(data: &Value, field: &str, name: &str) -> bool {
	data.get(field).and_then(Value::as_str) == Some(name)
}

/// Makes a map from pairs, for settings patches.
pub fn patch(pairs: Vec<(&str, Value)>) -> Map<String, Value> {
	pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

/// The defaults of an action from a JSON object.
pub fn defaults(value: Value) -> Map<String, Value> {
	match value {
		Value::Object(map) => map,
		_ => Map::new(),
	}
}

/// All actions.
pub fn all() -> Vec<&'static Kind> {
	vec![
		&scenes::SMART_SCENE,
		&sources::SOURCE_ANIMATION,
		&scenes::STUDIO_MODE,
		&sources::VIDEO_PLAYER,
		&sources::IMAGE_SOURCE,
		&audio::SOURCE_VOLUME,
		&audio::INPUT_VOLUME,
		&audio::MUTE_TOGGLE,
		&audio::MONITOR_SET,
		&stats::DROPPED_FRAMES,
		&stats::CPU_USAGE,
		&scenes::PREVIOUS_SCENE,
		&scenes::NEXT_SCENE,
		&outputs::INSTANT_REPLAY,
		&sources::BROWSER_SOURCE,
		&outputs::RECORD_TOGGLE,
		&outputs::STREAM_TOGGLE,
		&scenes::SET_TRANSITION,
		&sources::FILTER_TOGGLE,
		&outputs::VIRTUAL_CAM,
		&sources::FOCUSED_WINDOW,
		&basics::SOURCE_VISIBILITY,
		&basics::HOTKEY,
		&basics::SCREENSHOT,
		&basics::RECORD_TOOLS,
		&basics::MEDIA_CONTROL,
		&basics::PROFILE_SWITCH,
		&text::SET_TEXT,
		&stats::STREAM_HEALTH,
		&dials::TRANSITION_DIAL,
		&dials::SCENE_DIAL,
		&dials::FILTER_DIAL,
	]
}
