// Builds manifest.json and the translated names (de.json) from src/manifest.rs, and pi/lib/shared.js.
use obs_tools::manifest::{ACTIONS, ActionDef, ID};
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::{Result, plugin_dir, root, write};

/// The targets the release is built for, and where their plugin binary goes.
pub const TARGETS: &[&str] = &["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"];

pub fn binary_path(target: &str) -> String {
	format!("bin/{target}/obs-tools")
}

/// JSON.stringify(value, null, "\t")
pub fn pretty(value: &Value) -> String {
	let mut out = Vec::new();
	let mut serializer = serde_json::Serializer::with_formatter(&mut out, serde_json::ser::PrettyFormatter::with_indent(b"\t"));
	let _ = value.serialize(&mut serializer);
	String::from_utf8(out).unwrap_or_default()
}

fn action(a: &ActionDef) -> Value {
	let mut out = Map::new();
	out.insert("UUID".into(), format!("{ID}.{}", a.id).into());
	out.insert("Name".into(), a.name.into());
	out.insert("Icon".into(), format!("images/actions/{}", a.icon).into());
	out.insert("Tooltip".into(), a.tooltip.into());
	out.insert("Controllers".into(), json!(["Keypad"]));
	out.insert("SupportedInMultiActions".into(), true.into());
	out.insert("DisableAutomaticStates".into(), true.into());
	match &a.dial {
		None => {
			let states: Vec<Value> = a
				.states
				.iter()
				.map(|s| json!({ "Image": format!("images/keys/{s}"), "TitleAlignment": "bottom", "FontSize": 11 }))
				.collect();
			out.insert("States".into(), states.into());
		}
		Some(dial) => {
			out.insert("States".into(), json!([{ "Image": format!("images/encoder/{}-key", dial.icon) }]));
			out.insert("Controllers".into(), json!(["Encoder"]));
			out.insert(
				"Encoder".into(),
				json!({
					"layout": dial.layout,
					"Icon": format!("images/encoder/{}", dial.icon),
					"TriggerDescription": { "Rotate": dial.rotate, "Push": dial.push, "Touch": dial.touch },
				}),
			);
		}
	}
	Value::Object(out)
}

pub fn manifest() -> Value {
	let version = std::fs::read_to_string(root().join("VERSION")).unwrap_or_default().trim().to_string();
	let code_paths: Map<String, Value> = TARGETS.iter().map(|t| (t.to_string(), binary_path(t).into())).collect();
	json!({
		"$schema": "https://schemas.elgato.com/streamdeck/plugins/manifest.json",
		"UUID": ID,
		"Name": "OBS Tools",
		"Version": version,
		"Author": "Felitendo",
		"Description": "Advanced OBS tools for OpenDeck on Linux: smart scene switching with previews, source animations, instant replay, video player, dropped frame alarm and more.",
		"URL": "https://github.com/FeloDeck/obs-tools",
		"Icon": "images/plugin",
		"Category": "OBS Tools",
		"CategoryIcon": "images/category",
		// OpenDeck picks the binary for its CPU from CodePaths. Older versions take CodePath.
		"CodePath": binary_path(TARGETS[0]),
		"CodePaths": code_paths,
		"PropertyInspectorPath": "pi/index.html",
		"SDKVersion": 2,
		"OS": [{ "Platform": "linux" }],
		"Actions": ACTIONS.iter().map(action).collect::<Vec<_>>(),
	})
}

fn localized() -> Value {
	let mut out = Map::new();
	out.insert("Name".into(), "OBS Tools".into());
	out.insert(
		"Description".into(),
		"Erweiterte OBS-Werkzeuge für OpenDeck unter Linux: smarter Szenenwechsel mit Vorschau, Quellen-Animationen, Instant Replay, Videoplayer, Alarm bei Frame-Verlust und mehr.".into(),
	);
	for a in ACTIONS {
		out.insert(format!("{ID}.{}", a.id), json!({ "Name": a.de_name, "Tooltip": a.de_tooltip }));
	}
	Value::Object(out)
}

pub fn run() -> Result {
	let dir = plugin_dir();
	let manifest = manifest();
	write(&dir.join("manifest.json"), pretty(&manifest) + "\n")?;
	write(&dir.join("de.json"), pretty(&localized()) + "\n")?;
	write(&root().join(obs_tools::pi::SHARED_JS), obs_tools::pi::shared_js())?;
	println!(
		"manifest.json and de.json: {} actions, version {}; pi/lib/shared.js",
		ACTIONS.len(),
		manifest["Version"].as_str().unwrap_or("")
	);
	Ok(())
}
