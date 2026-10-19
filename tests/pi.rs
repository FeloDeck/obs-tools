// The settings pages are JavaScript. These tests run them with boa (a JavaScript engine in Rust):
// every text needs a German version, and the pictures of keys there must look like the real keys.
use std::path::{Path, PathBuf};
use std::rc::Rc;

use boa_engine::builtins::promise::PromiseState;
use boa_engine::module::SimpleModuleLoader;
use boa_engine::{Context, Module, Source, js_string};
use obs_tools::actions::text::text_uses;
use obs_tools::{i18n, icons, js, pi, render, volume};
use regex::Regex;
use serde_json::{Value, json};

fn root() -> PathBuf {
	PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn pi_dir() -> PathBuf {
	root().join("de.felitendo.obs-tools.sdPlugin").join("pi")
}

/// Runs a module next to the settings page and returns its export `result` as JSON.
fn run_js(code: &str) -> Value {
	let dir = pi_dir();
	let loader = Rc::new(SimpleModuleLoader::new(&dir).expect("pi folder"));
	let mut context = Context::builder().module_loader(loader.clone()).build().expect("context");
	let path = dir.join("__test.js");
	let module = Module::parse(Source::from_bytes(code).with_path(&path), None, &mut context).expect("test module parses");
	loader.insert(path, module.clone());
	let promise = module.load_link_evaluate(&mut context);
	let _ = context.run_jobs();
	match promise.state() {
		PromiseState::Fulfilled(_) => {}
		PromiseState::Rejected(error) => panic!("JavaScript failed: {}", error.display()),
		PromiseState::Pending => panic!("JavaScript did not finish"),
	}
	let namespace = module.namespace(&mut context);
	let result =
		namespace.get(js_string!("result"), &mut context).expect("result").to_string(&mut context).expect("text").to_std_string_escaped();
	serde_json::from_str(&result).expect("result is JSON")
}

#[test]
fn shared_js_is_up_to_date() {
	let file = std::fs::read_to_string(root().join(pi::SHARED_JS)).unwrap_or_default();
	assert!(file == pi::shared_js(), "pi/lib/shared.js is old. Run: make manifest");
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
	for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
		let path = entry.path();
		if path.is_dir() {
			rust_files(&path, out);
		} else if path.extension().is_some_and(|e| e == "rs") {
			out.push(path);
		}
	}
}

// The code of a file without its test module (a #[cfg(test)] field does not end the code).
fn without_tests(code: &str) -> &str {
	let module = code.match_indices("#[cfg(test)]\n").map(|(i, _)| i).find(|&i| {
		let next = &code[i + "#[cfg(test)]\n".len()..];
		next.starts_with("mod ") || next.starts_with("pub(crate) mod ")
	});
	match module {
		Some(i) => &code[..i],
		None => code,
	}
}

// The part between the parentheses that start at `open`.
fn span(code: &str, open: usize) -> &str {
	let mut depth = 0;
	let mut in_string = false;
	let bytes = code.as_bytes();
	let mut i = open;
	while i < bytes.len() {
		match bytes[i] {
			b'\\' if in_string => i += 1,
			b'"' => in_string = !in_string,
			b'(' if !in_string => depth += 1,
			b')' if !in_string => {
				depth -= 1;
				if depth == 0 {
					return &code[open..=i];
				}
			}
			_ => {}
		}
		i += 1;
	}
	&code[open..]
}

fn unescape(literal: &str) -> String {
	serde_json::from_str::<String>(&format!("\"{literal}\"")).unwrap_or_else(|_| literal.to_string())
}

// Names, units, keys and examples that stay as they are.
fn keep(text: &str) -> bool {
	Regex::new(r"^(?:%|\d+:\d\d|[A-Z0-9]{1,3}|F\d+|Num \d|PNG|JPG|dB|ms|px|auto|Alt|Super|URLs?|!replay|https://…|\d+)$")
		.expect("regex")
		.is_match(text)
}

// Every text a user can see needs a German translation, with the same {placeholders}.
#[test]
fn every_text_has_a_german_translation() {
	let mut texts: Vec<String> = Vec::new();
	// The settings pages: labels, hints, options and sections of every field.
	let schema_texts = run_js(
		r#"
		import { SCHEMAS } from "./schemas.js";
		const texts = new Set();
		const add = (value) => typeof value === "string" && value && texts.add(value);
		for (const schema of Object.values(SCHEMAS)) {
			add(schema.help);
			add(schema.hint);
			add(schema.lookTitle);
			for (const field of schema.fields ?? []) {
				for (const key of ["label", "text", "hint", "placeholder", "empty", "unit"]) add(field[key]);
				// Section titles. "look" is a name in the code, the page shows "On the key" or lookTitle.
				if (field.section !== "look") add(field.section);
				if (typeof field.unit === "function") for (const unit of ["db", "percent"]) add(field.unit({ unit }));
				for (const [, label] of field.options ?? []) add(label);
			}
		}
		export const result = JSON.stringify([...texts]);
		"#,
	);
	texts.extend(schema_texts.as_array().expect("texts").iter().filter_map(|t| t.as_str().map(String::from)));
	// t("...") in the settings page and in the plugin.
	let t_call = Regex::new(r#"\bt(?:_with)?\("((?:[^"\\]|\\.)+)""#).expect("regex");
	let page = std::fs::read_to_string(pi_dir().join("pi.js")).expect("pi.js");
	texts.extend(t_call.captures_iter(&page).map(|c| unescape(&c[1])));
	let mut files = Vec::new();
	rust_files(&root().join("src"), &mut files);
	let literal = Regex::new(r#""((?:[^"\\]|\\.)+)""#).expect("regex");
	let placeholder = Regex::new(r"\{[^{}]*\}").expect("regex");
	let label = Regex::new(r#"label: "([^"]+)""#).expect("regex");
	let mut errors = Vec::new();
	for file in files.iter().filter(|f| !f.ends_with("de.rs")) {
		let code = std::fs::read_to_string(file).expect("source");
		let code = without_tests(&code);
		texts.extend(t_call.captures_iter(code).map(|c| unescape(&c[1])));
		// Errors and statuses reach the settings page, which translates them. Messages are the texts
		// with a space, any {value} can be any value.
		for start in ["Err(", "Error::from(", "set_status("] {
			for (at, _) in code.match_indices(start) {
				let open = at + start.len() - 1;
				for capture in literal.captures_iter(span(code, open)) {
					let text = placeholder.replace_all(&unescape(&capture[1]), "{x}").to_string();
					if text.contains(' ') {
						errors.push(text);
					}
				}
			}
		}
		// The labels of the frame counts on the dropped frames key.
		if file.ends_with("stats.rs") {
			texts.extend(label.captures_iter(code).map(|c| c[1].to_string()));
		}
	}
	let de: std::collections::HashMap<&str, &str> = i18n::DE.iter().copied().collect();
	let mut missing: Vec<&String> = texts.iter().filter(|t| !de.contains_key(t.as_str()) && !keep(t)).collect();
	missing.sort();
	missing.dedup();
	assert!(missing.is_empty(), "missing German texts: {missing:#?}");
	let holes = Regex::new(r"\{\w+\}").expect("regex");
	let known: Vec<String> = i18n::DE.iter().map(|(en, _)| holes.replace_all(en, "{x}").to_string()).collect();
	let mut unknown: Vec<&String> = errors.iter().filter(|e| !known.contains(e)).collect();
	unknown.sort();
	unknown.dedup();
	assert!(unknown.is_empty(), "errors without a German text: {unknown:#?}");
}

// The settings pages draw small pictures of keys with pi/lib/render.js. They must look like the keys.
#[test]
fn settings_page_pictures_match_the_keys() {
	let names: Vec<&str> = icons::GLYPHS.iter().map(|(n, _)| *n).collect();
	let mut glyph_cases = Vec::new();
	for name in &names {
		for options in [
			json!({}),
			json!({ "color": "#6B6B6B", "setup": true }),
			json!({ "title": "Mic" }),
			json!({ "title": "12:34", "titleAt": "above", "meter": 0.7 }),
			json!({ "meter": null, "title": "Bildschirmaufnahme (PipeWire)" }),
		] {
			glyph_cases.push(json!([name, options]));
		}
	}
	let stat_cases = json!([
		{ "label": "OBS CPU", "value": "12.5%", "sub": "820 MB", "bar": 0.125 },
		{ "label": "LIVE", "value": "6.0 Mb/s", "sub": "drops 0.0%", "dot": "#30A46C" },
		{ "label": "AUSGELASSEN", "value": "12", "alert": "#E5484D" },
		{ "label": "x", "value": "y", "bar": 0 },
	]);
	let image = "data:image/svg+xml,%3Csvg%3E";
	let scene_cases = json!([
		{ "image": null, "fit": "icon", "title": "Szene" },
		{ "image": image, "fit": "fit", "title": "Scene 2" },
		{ "image": image, "fit": "fill", "title": "A long scene name" },
		{ "image": image, "fit": "fit", "title": "" },
	]);
	let result = run_js(&format!(
		r#"
		import {{ glyphKeySvg, sceneKeySvg, statKeySvg }} from "./lib/render.js";
		const glyphs = {glyph_cases}.map(([name, options]) => glyphKeySvg(name, options));
		const stats = {stat_cases}.map((options) => statKeySvg(options));
		const scenes = {scene_cases}.map((options) => sceneKeySvg(options));
		export const result = JSON.stringify({{ glyphs, stats, scenes }});
		"#,
		glyph_cases = Value::Array(glyph_cases.clone()),
	));
	let mut differences = Vec::new();
	for (case, js_svg) in glyph_cases.iter().zip(result["glyphs"].as_array().expect("glyphs")) {
		let o = &case[1];
		let options = render::GlyphKey {
			color: o.get("color").and_then(Value::as_str).unwrap_or(icons::colors::WHITE),
			meter: o.get("meter").and_then(Value::as_f64),
			setup: o.get("setup").and_then(Value::as_bool).unwrap_or(false),
			title: o.get("title").and_then(Value::as_str).unwrap_or(""),
			title_at: o.get("titleAt").and_then(Value::as_str).unwrap_or("below"),
		};
		if render::glyph_key_svg(case[0].as_str().unwrap_or(""), &options) != js_svg.as_str().unwrap_or("") {
			differences.push(case.to_string());
		}
	}
	for (o, js_svg) in stat_cases.as_array().expect("cases").iter().zip(result["stats"].as_array().expect("stats")) {
		let get = |k: &str| o.get(k).and_then(Value::as_str);
		let options = render::StatKey {
			label: get("label").unwrap_or(""),
			value: get("value").unwrap_or(""),
			sub: get("sub").unwrap_or(""),
			bar: o.get("bar").and_then(Value::as_f64),
			alert: get("alert"),
			dot: get("dot"),
			..Default::default()
		};
		if render::stat_key_svg(&options) != js_svg.as_str().unwrap_or("") {
			differences.push(o.to_string());
		}
	}
	for (o, js_svg) in scene_cases.as_array().expect("cases").iter().zip(result["scenes"].as_array().expect("scenes")) {
		let get = |k: &str| o.get(k).and_then(Value::as_str);
		let options = render::SceneKey {
			image: get("image"),
			fit: get("fit").unwrap_or("fit"),
			title: get("title").unwrap_or(""),
			..Default::default()
		};
		if render::scene_key_svg(&options) != js_svg.as_str().unwrap_or("") {
			differences.push(o.to_string());
		}
	}
	assert!(differences.is_empty(), "pictures differ from the keys: {differences:#?}");
}

// The settings pages translate the messages of the plugin and show numbers like the keys.
#[test]
fn settings_page_texts_match_the_plugin() {
	let messages = [
		"Nothing answers on 127.0.0.1:4455",
		"Recorded 3 changes into phase 2.",
		"\"Mic\" is not in scene \"Szene\"",
		"something else",
		"Wrong password",
		"Twitch said 401",
	];
	let numbers = [0.0, 6.43, -2.25, 99.95, 1234.5678, -0.04, 0.125];
	let result = run_js(&format!(
		r#"
		import {{ num, setLanguage, t, translateMessage }} from "./lib/i18n.js";
		import {{ formatVolume }} from "./lib/volume.js";
		import {{ textUses }} from "./lib/text-gestures.js";
		const out = [];
		for (const lang of ["en", "de"]) {{
			setLanguage(lang);
			out.push({messages}.map((m) => translateMessage(m)));
			out.push({numbers}.flatMap((n) => [num(n, 0), num(n, 1), num(n, 2), formatVolume(n), formatVolume(n, "percent")]));
			out.push([t("Phase {{n}}", {{ n: 4 }}), t("OBS offline")]);
		}}
		out.push(["text", "line", "count", "timer"].map((k) => textUses({{ press: "up", hold: "next" }}, k)));
		export const result = JSON.stringify(out);
		"#,
		messages = serde_json::to_string(&messages).unwrap_or_default(),
		numbers = serde_json::to_string(&numbers).unwrap_or_default(),
	));
	let mut expected = Vec::new();
	for lang in ["en", "de"] {
		i18n::set_language(lang);
		expected.push(json!(messages.iter().map(|m| i18n::translate_message(m)).collect::<Vec<_>>()));
		expected.push(json!(
			numbers
				.iter()
				.flat_map(|n| [
					i18n::num(*n, 0),
					i18n::num(*n, 1),
					i18n::num(*n, 2),
					volume::format_volume(*n, "db"),
					volume::format_volume(*n, "percent")
				])
				.collect::<Vec<_>>()
		));
		expected.push(json!([i18n::t_with("Phase {n}", &[("n", "4")]), i18n::t("OBS offline")]));
	}
	i18n::set_language("en");
	let settings = js::object(Some(&json!({ "press": "up", "hold": "next" })));
	expected.push(json!(["text", "line", "count", "timer"].map(|k| text_uses(&settings, k))));
	assert_eq!(result, Value::Array(expected));
}
