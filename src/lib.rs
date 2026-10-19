// OBS Tools for OpenDeck. The plugin runs on one thread: every event from OpenDeck or OBS starts a
// task, and tasks take turns at each await.
pub mod actions;
pub mod animation;
pub mod connections;
pub mod desktop;
pub mod emitter;
pub mod glyph_fit;
pub mod i18n;
pub mod icons;
pub mod js;
pub mod key_images;
pub mod lists;
pub mod manifest;
pub mod media;
pub mod obs;
pub mod obs_state;
pub mod pi;
pub mod plugin;
pub mod render;
pub mod sd;
pub mod time;
pub mod timers;
pub mod twitch;
pub mod volume;

/// An error with a short English sentence. The settings pages translate it.
#[derive(Debug, Clone, PartialEq)]
pub struct Error(pub String);

impl Error {
	pub fn message(&self) -> &str {
		&self.0
	}
}

impl std::fmt::Display for Error {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(&self.0)
	}
}

impl std::error::Error for Error {}

impl From<String> for Error {
	fn from(message: String) -> Self {
		Error(message)
	}
}

impl From<&str> for Error {
	fn from(message: &str) -> Self {
		Error(message.to_string())
	}
}

pub type Res<T> = Result<T, Error>;
