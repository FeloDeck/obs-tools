// Translations for the keys and dials. The code is written in English: t() looks the English text up
// in the table of the language and falls back to it. The settings pages translate the messages the
// plugin sends them, see translate_message.
mod de;

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::LazyLock;

pub use de::DE;

static DE_MAP: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| DE.iter().copied().collect());

// The plugin runs on one thread. Per thread, so tests in other languages do not get in the way.
thread_local! {
	static GERMAN: Cell<bool> = const { Cell::new(false) };
	// Languages that write 1,5 instead of 1.5.
	static COMMA: Cell<bool> = const { Cell::new(false) };
}

/// OpenDeck sends codes like "de" or "zh_CN". Returns the language used: "de" or "en".
pub fn set_language(code: &str) -> &'static str {
	let lang = code.to_lowercase();
	let lang = lang.split(['-', '_']).next().unwrap_or("");
	let german = lang == "de";
	GERMAN.set(german);
	COMMA.set(german);
	if german { "de" } else { "en" }
}

fn table() -> Option<&'static HashMap<&'static str, &'static str>> {
	if GERMAN.get() { Some(&DE_MAP) } else { None }
}

/// The text in the current language.
pub fn t(text: &str) -> String {
	table().and_then(|m| m.get(text)).map_or_else(|| text.to_string(), |s| s.to_string())
}

/// t("Phase {n}", &[("n", "2")]) -> "Phase 2" (or the translation, with {n} filled in).
pub fn t_with(text: &str, vars: &[(&str, &str)]) -> String {
	fill(&t(text), |name| vars.iter().find(|(key, _)| *key == name).map(|(_, value)| value.to_string()))
}

// Replaces {word} with its value. Unknown names stay as they are.
fn fill(text: &str, value: impl Fn(&str) -> Option<String>) -> String {
	let mut out = String::new();
	let mut rest = text;
	while let Some(open) = rest.find('{') {
		out.push_str(&rest[..open]);
		let after = &rest[open + 1..];
		let name_len = after.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(after.len());
		if name_len > 0 && after[name_len..].starts_with('}') {
			let name = &after[..name_len];
			match value(name) {
				Some(v) => out.push_str(&v),
				None => {
					out.push('{');
					out.push_str(name);
					out.push('}');
				}
			}
			rest = &after[name_len + 1..];
		} else {
			out.push('{');
			rest = after;
		}
	}
	out.push_str(rest);
	out
}

/// A number with a fixed count of decimals, in the style of the language: 6.4 or 6,4.
pub fn num(value: f64, digits: usize) -> String {
	let out = crate::js::to_fixed(value, digits);
	if COMMA.get() { out.replacen('.', ",", 1) } else { out }
}

// A text with {placeholders}: the parts in between, and the names.
enum Part {
	Text(String),
	Hole(String),
}

fn parts(text: &str) -> Vec<Part> {
	let mut parts = Vec::new();
	let mut rest = text;
	let mut literal = String::new();
	while let Some(open) = rest.find('{') {
		literal.push_str(&rest[..open]);
		let after = &rest[open + 1..];
		let name_len = after.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(after.len());
		if name_len > 0 && after[name_len..].starts_with('}') {
			parts.push(Part::Text(std::mem::take(&mut literal)));
			parts.push(Part::Hole(after[..name_len].to_string()));
			rest = &after[name_len + 1..];
		} else {
			literal.push('{');
			rest = after;
		}
	}
	literal.push_str(rest);
	parts.push(Part::Text(literal));
	parts
}

// Like the regular expression ^text(.+?)text$ of the JavaScript version: a hole takes at least one
// character but no line break, the shortest that lets the rest match.
fn match_parts<'a>(parts: &[Part], message: &'a str, found: &mut Vec<&'a str>) -> bool {
	match parts.split_first() {
		None => message.is_empty(),
		Some((Part::Text(text), rest)) => message.strip_prefix(text.as_str()).is_some_and(|m| match_parts(rest, m, found)),
		Some((Part::Hole(_), rest)) => {
			for (i, c) in message.char_indices() {
				if matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}') {
					return false;
				}
				let end = i + c.len_utf8();
				found.push(&message[..end]);
				if match_parts(rest, &message[end..], found) {
					return true;
				}
				found.pop();
			}
			false
		}
	}
}

/// For finished English messages like "Nothing answers on 127.0.0.1:4455": finds the text with
/// {placeholders} that fits and translates it with the same values.
pub fn translate_message(message: &str) -> String {
	let Some(table) = table() else { return message.to_string() };
	if let Some(text) = table.get(message) {
		return text.to_string();
	}
	for (text, _) in DE {
		if !text.contains('{') {
			continue;
		}
		let parts = parts(text);
		let mut found = Vec::new();
		if match_parts(&parts, message, &mut found) {
			let names: Vec<&str> = parts.iter().filter_map(|p| if let Part::Hole(n) = p { Some(n.as_str()) } else { None }).collect();
			// With the same name twice, the last value counts (Object.fromEntries).
			return fill(&t(text), |name| names.iter().rposition(|n| *n == name).map(|i| found[i].to_string()));
		}
	}
	message.to_string()
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::collections::HashSet;

	#[test]
	fn plugin_messages_are_translated_with_their_values() {
		set_language("de");
		assert_eq!(translate_message("Nothing answers on 127.0.0.1:4455"), "Unter 127.0.0.1:4455 antwortet nichts");
		assert_eq!(translate_message("Recorded 3 changes into phase 2."), "3 Änderungen in Phase 2 aufgenommen.");
		assert_eq!(translate_message("something we do not know"), "something we do not know");
		assert_eq!(t_with("Phase {n}", &[("n", "4")]), "Phase 4");
		assert_eq!(num(6.43, 1), "6,4");
		assert_eq!(crate::volume::format_volume(-2.0, "db"), "-2,0 dB");
		set_language("en");
		assert_eq!(num(6.43, 1), "6.4");
		assert_eq!(t("Wrong password"), "Wrong password");
	}

	#[test]
	fn table_has_no_text_twice_and_keeps_placeholders() {
		let mut seen = HashSet::new();
		for (en, de) in DE {
			assert!(seen.insert(en), "listed twice: {en}");
			let holes = |text: &str| {
				let mut names: Vec<String> =
					parts(text).into_iter().filter_map(|p| if let Part::Hole(n) = p { Some(n) } else { None }).collect();
				names.sort();
				names
			};
			assert_eq!(holes(en), holes(de), "placeholders differ: {en}");
		}
	}
}
