// Lists for the drop downs of the settings pages: scenes, sources, filters, hotkeys and more.
use serde_json::{Map, Value, json};

use crate::actions::KeyRc;
use crate::actions::dials::{filter_range, filter_value_label};
use crate::js::{self, truthy};
use crate::obs::ObsClient;
use crate::obs_state::sorted_scene_names;

const AUDIO_CAP: i64 = 1 << 1;
const MEDIA_KINDS: &[&str] = &["ffmpeg_source", "vlc_source"];
const AUDIO_KINDS: &[&str] = &[
	"pulse_input_capture",
	"pulse_output_capture",
	"alsa_input_capture",
	"jack_output_capture",
	"pipewire-audio-capture-source",
	"wasapi_input_capture",
	"wasapi_output_capture",
	"wasapi_process_output_capture",
	"coreaudio_input_capture",
	"coreaudio_output_capture",
	"ffmpeg_source",
	"vlc_source",
	"browser_source",
];

/// The lists asked for, read from the OBS this key talks to.
pub async fn lists(instance: Option<&KeyRc>, names: &[String], payload: &Value) -> Value {
	let mut out = Map::new();
	let Some(client) = instance.and_then(|i| i.obs()) else { return Value::Object(out) };
	if !client.connected() {
		return Value::Object(out);
	}
	let call = |kind: &'static str, data: Value| {
		let reply = client.call(kind, data);
		async move { reply.await.ok() }
	};
	let mut inputs: Option<Vec<Value>> = None;
	async fn get_inputs(client: &ObsClient, inputs: &mut Option<Vec<Value>>) -> Vec<Value> {
		if inputs.is_none() {
			let list = client.call("GetInputList", json!({})).await.ok();
			*inputs = Some(match list.as_ref().and_then(|l| l.get("inputs")) {
				Some(Value::Array(items)) => items.clone(),
				_ => Vec::new(),
			});
		}
		inputs.clone().unwrap_or_default()
	}
	let field = |items: &[Value], key: &str| items.iter().map(|i| i.get(key).cloned().unwrap_or(Value::Null)).collect::<Vec<_>>();
	let kind_of = |i: &Value| js::to_string(i.get("unversionedInputKind"));
	let array = |list: Option<Value>, key: &str| match list.as_ref().and_then(|l| l.get(key)) {
		Some(Value::Array(items)) => items.clone(),
		_ => Vec::new(),
	};
	for name in names {
		let value = match name.as_str() {
			"scenes" => {
				let list = call("GetSceneList", json!({})).await.unwrap_or(Value::Null);
				Value::from(sorted_scene_names(&list))
			}
			"inputs" => Value::from(field(&get_inputs(&client, &mut inputs).await, "inputName")),
			"audioInputs" => {
				let items: Vec<Value> = get_inputs(&client, &mut inputs)
					.await
					.into_iter()
					.filter(|i| match i.get("inputKindCaps") {
						Some(caps) => js::number(Some(caps)) as i64 & AUDIO_CAP != 0,
						None => AUDIO_KINDS.contains(&kind_of(i).as_str()),
					})
					.collect();
				Value::from(field(&items, "inputName"))
			}
			"mediaInputs" | "imageInputs" | "browserInputs" | "windowInputs" | "textInputs" => {
				let wanted = |kind: &str| match name.as_str() {
					"mediaInputs" => MEDIA_KINDS.contains(&kind),
					"imageInputs" => kind == "image_source",
					"browserInputs" => kind == "browser_source",
					"windowInputs" => kind == "xcomposite_input",
					_ => kind.starts_with("text_"),
				};
				let items: Vec<Value> = get_inputs(&client, &mut inputs).await.into_iter().filter(|i| wanted(&kind_of(i))).collect();
				Value::from(field(&items, "inputName"))
			}
			"sources" => {
				let list = call("GetSceneList", json!({})).await.unwrap_or(Value::Null);
				let scenes = sorted_scene_names(&list);
				let mut all = field(&get_inputs(&client, &mut inputs).await, "inputName");
				all.extend(scenes.into_iter().map(Value::from));
				Value::from(all)
			}
			"transitions" => Value::from(field(&array(call("GetSceneTransitionList", json!({})).await, "transitions"), "transitionName")),
			"filters" => {
				let source = payload.get("source").cloned().unwrap_or(Value::Null);
				if truthy(Some(&source)) {
					Value::from(field(&array(call("GetSourceFilterList", json!({ "sourceName": source })).await, "filters"), "filterName"))
				} else {
					Value::from(Vec::<Value>::new())
				}
			}
			"profiles" => Value::from(array(call("GetProfileList", json!({})).await, "profiles")),
			"collections" => Value::from(array(call("GetSceneCollectionList", json!({})).await, "sceneCollections")),
			"hotkeys" => {
				let mut names: Vec<Value> = Vec::new();
				for hotkey in array(call("GetHotkeyList", json!({})).await, "hotkeys") {
					if !names.contains(&hotkey) {
						names.push(hotkey);
					}
				}
				let mut entries: Vec<(String, Value)> = names.into_iter().map(|v| (hotkey_label(&js::to_string(Some(&v))), v)).collect();
				entries.sort_by(|a, b| locale_compare(&a.0, &b.0));
				Value::from(entries.into_iter().map(|(label, value)| json!({ "value": value, "label": label })).collect::<Vec<_>>())
			}
			"filterSettings" => filter_settings(&client, payload.get("source"), payload.get("filter")).await,
			"sceneItems" => {
				let scene = payload
					.get("scene")
					.filter(|s| truthy(Some(s)))
					.cloned()
					.or_else(|| instance.and_then(|i| i.obs_state()).and_then(|s| s.data().program_scene).map(Value::from));
				match scene {
					Some(scene) if truthy(Some(&scene)) => Value::from(field(
						&array(call("GetSceneItemList", json!({ "sceneName": scene })).await, "sceneItems"),
						"sourceName",
					)),
					_ => Value::from(Vec::<Value>::new()),
				}
			}
			_ => continue,
		};
		out.insert(name.clone(), value);
	}
	Value::Object(out)
}

// The number settings of a filter, for the filter dial.
async fn filter_settings(client: &ObsClient, source: Option<&Value>, filter: Option<&Value>) -> Value {
	let empty = Value::Array(Vec::new());
	if !truthy(source) || !truthy(filter) {
		return empty;
	}
	let Ok(found) = client.call("GetSourceFilter", json!({ "sourceName": source, "filterName": filter })).await else { return empty };
	let kind = found.get("filterKind").cloned().unwrap_or(Value::Null);
	let defaults = client
		.call("GetSourceFilterDefaultSettings", json!({ "filterKind": kind }))
		.await
		.ok()
		.and_then(|d| d.get("defaultFilterSettings").cloned());
	let mut all = js::object(defaults.as_ref());
	js::assign(&mut all, &js::object(found.get("filterSettings")));
	let mut keys: Vec<&String> = all.iter().filter(|(k, v)| v.is_number() && !k.starts_with("color")).map(|(k, _)| k).collect();
	keys.sort();
	let kind = js::str_or_empty(Some(&kind));
	Value::from(
		keys.into_iter()
			.map(|key| {
				let range =
					filter_range(&kind, key).map_or(Value::Null, |r| Value::from(r.iter().map(|x| js::num(*x)).collect::<Vec<_>>()));
				json!({ "value": key, "label": filter_value_label(key), "range": range })
			})
			.collect::<Vec<_>>(),
	)
}

/// "OBSBasic.StartRecording" -> "Start recording", "libobs.mute" -> "Mute (source)"
pub fn hotkey_label(name: &str) -> String {
	let mut parts = name.split('.');
	let group = parts.next().unwrap_or("");
	let rest: Vec<&str> = parts.collect();
	let joined = rest.join(" ");
	// ([a-z])([A-Z]) -> "$1 $2"
	let mut spaced = String::new();
	let mut prev: Option<char> = None;
	for c in joined.chars() {
		if prev.is_some_and(|p| p.is_ascii_lowercase()) && c.is_ascii_uppercase() {
			spaced.push(' ');
		}
		spaced.push(c);
		prev = Some(c);
	}
	let words = js::trim(&spaced.replace(['_', '.'], " ").to_lowercase()).to_string();
	let mut chars = words.chars();
	let label = match chars.next() {
		Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
		None => String::new(),
	};
	let label = match label.strip_suffix("virtual cam") {
		Some(start) => format!("{start}virtual camera"),
		None => label,
	};
	let place = match group {
		"OBSBasic" => "",
		"libobs" => "source",
		"MediaSource" => "media source",
		"ReplayBuffer" => "replay buffer",
		other => other,
	};
	if place.is_empty() { label } else { format!("{label} ({place})") }
}

// Like String.localeCompare: letters ignore case and come after digits, punctuation and spaces.
fn locale_compare(a: &str, b: &str) -> std::cmp::Ordering {
	const PUNCTUATION: &str = "_-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$";
	let key = |c: char| -> (u8, u32) {
		if c.is_whitespace() {
			(0, 0)
		} else if let Some(i) = PUNCTUATION.find(c) {
			(1, i as u32)
		} else if c.is_ascii_digit() {
			(2, c as u32)
		} else if c.is_alphabetic() {
			(3, c.to_lowercase().next().unwrap_or(c) as u32)
		} else {
			(1, 1000 + c as u32)
		}
	};
	let primary = a.chars().map(key).cmp(b.chars().map(key));
	// Same letters: lowercase first, like ICU.
	primary.then_with(|| a.chars().map(|c| c.is_uppercase()).cmp(b.chars().map(|c| c.is_uppercase()))).then_with(|| a.cmp(b))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn hotkey_names_become_readable() {
		assert_eq!(hotkey_label("OBSBasic.StartRecording"), "Start recording");
		assert_eq!(hotkey_label("libobs.mute"), "Mute (source)");
		assert_eq!(hotkey_label("MediaSource.Play"), "Play (media source)");
		assert_eq!(hotkey_label("libobs.show_scene_item.1"), "Show scene item 1 (source)");
		assert_eq!(hotkey_label("OBSBasic.StartVirtualCam"), "Start virtual camera");
	}

	#[test]
	fn labels_sort_like_locale_compare() {
		let mut labels = vec!["b", "A", "a", "Push-to-mute (source)", "Push to talk", "item 10", "item 1", "Zoom"];
		labels.sort_by(|a, b| locale_compare(a, b));
		assert_eq!(labels, ["a", "A", "b", "item 1", "item 10", "Push to talk", "Push-to-mute (source)", "Zoom"]);
	}
}
