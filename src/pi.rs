// The settings pages are JavaScript (they run in OpenDeck's web view). The data they share with the
// plugin comes from here: `cargo xtask manifest` writes it to pi/lib/shared.js.
use serde::Serialize;
use serde_json::{Map, Value};

use crate::glyph_fit::FIT;
use crate::i18n::DE;
use crate::icons::{COLORS, GLYPHS, MEDIA_GLYPHS, VOLUME_GLYPHS};
use crate::text_gestures::{TEXT_ACTIONS, TEXT_GESTURES, TEXT_KINDS};

pub const SHARED_JS: &str = "de.felitendo.obs-tools.sdPlugin/pi/lib/shared.js";

fn pretty(value: &Value) -> String {
	let mut out = Vec::new();
	let mut serializer = serde_json::Serializer::with_formatter(&mut out, serde_json::ser::PrettyFormatter::with_indent(b"\t"));
	let _ = value.serialize(&mut serializer);
	String::from_utf8(out).unwrap_or_default()
}

fn pairs(list: &[(&str, &str)]) -> Value {
	Value::Object(list.iter().map(|(k, v)| (k.to_string(), Value::from(*v))).collect())
}

/// The content of pi/lib/shared.js.
pub fn shared_js() -> String {
	let fit: Map<String, Value> = FIT
		.iter()
		.map(|(name, f)| {
			(
				name.to_string(),
				serde_json::json!({ "cx": crate::js::num(f.cx), "cy": crate::js::num(f.cy), "w": crate::js::num(f.w), "h": crate::js::num(f.h) }),
			)
		})
		.collect();
	let glyphs: Map<String, Value> = GLYPHS.iter().map(|(name, body)| (name.to_string(), Value::from(body.as_str()))).collect();
	let parts = [
		("DE", pairs(DE)),
		("GLYPHS", Value::Object(glyphs)),
		("FIT", Value::Object(fit)),
		("COLORS", Value::Object(COLORS.iter().map(|(k, v)| (k.to_string(), Value::from(*v))).collect())),
		("VOLUME_GLYPHS", pairs(VOLUME_GLYPHS)),
		("MEDIA_GLYPHS", pairs(MEDIA_GLYPHS)),
		("TEXT_ACTIONS", Value::from(TEXT_ACTIONS.to_vec())),
		("TEXT_GESTURES", Value::from(TEXT_GESTURES.to_vec())),
		("TEXT_KINDS", pairs(TEXT_KINDS)),
	];
	let mut out = String::from("// Written by `cargo xtask manifest` from the tables of the plugin (src/). Do not edit.\n");
	for (name, value) in parts {
		out += &format!("export const {name} = {};\n", pretty(&value));
	}
	out
}
