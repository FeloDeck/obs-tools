// The key images from the manifest (images/keys/<name>.svg). `cargo xtask icons` writes them from
// this table, and keys draw them again with a title below (see Key::state_image).
// Gray means "off". The glyph is always in the middle: OpenDeck shows these pictures before the
// plugin draws a key, and then there is no title yet.
use crate::icons::colors::{GRAY as G, GREEN, RED};

pub struct KeyImage {
	pub glyph: &'static str,
	pub color: Option<&'static str>,
}

const fn k(glyph: &'static str) -> KeyImage {
	KeyImage { glyph, color: None }
}

const fn c(glyph: &'static str, color: &'static str) -> KeyImage {
	KeyImage { glyph, color: Some(color) }
}

pub static KEY_IMAGES: &[(&str, KeyImage)] = &[
	("scene", k("scene")),
	("animation", k("animation")),
	("animation-on", c("animation", GREEN)),
	("studio", c("studio", G)),
	("studio-on", k("studio")),
	("video", k("video")),
	("image", k("image")),
	("image-on", c("image", GREEN)),
	("video-on", c("videoStop", GREEN)),
	("volume", k("volumeUp")),
	("unmuted", k("unmuted")),
	("muted", c("mute", RED)),
	("monitor", c("monitor", G)),
	("monitor-on", k("monitor")),
	("dropped", k("dropped")),
	("cpu", k("cpu")),
	("back", k("back")),
	// Profiles save the picture path. Last Scene keys made before the rename still point here.
	("previous", k("back")),
	("next", k("next")),
	("replay", c("replay", G)),
	("replay-on", k("replay")),
	("browser", k("browser")),
	("record", k("record")),
	("record-on", c("record", RED)),
	("stream", k("stream")),
	("stream-on", c("stream", RED)),
	("transition", c("transition", G)),
	("transition-on", k("transition")),
	("filter", c("filter", G)),
	("filter-on", k("filter")),
	("virtualcam", c("virtualcam", G)),
	("virtualcam-on", k("virtualcam")),
	("window", k("window")),
	("visibility", c("eyeOff", G)),
	("visibility-on", k("eye")),
	("hotkey", k("keyboard")),
	("screenshot", k("camera")),
	("chapter", c("chapter", G)),
	("media", k("playPause")),
	("profile", c("profile", G)),
	("text", k("text")),
	("health", k("health")),
];

pub fn key_image(name: &str) -> Option<&'static KeyImage> {
	KEY_IMAGES.iter().find(|(n, _)| *n == name).map(|(_, image)| image)
}
