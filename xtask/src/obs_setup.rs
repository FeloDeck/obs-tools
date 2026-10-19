// The scenes, sources and media the end-to-end tests expect, in the OBS on this PC.
use std::process::Command;

use serde_json::{Value, json};

use crate::Result;
use crate::e2e::{Obs, local};

/// Creates the scenes and sources the end-to-end tests use (needs the media of make-video).
pub fn setup() -> Result {
	local(async {
		let obs = Obs::connect().await?;
		let home = std::env::var("HOME").unwrap_or_default();
		// A step that fails (because it is there already) is skipped.
		let call = async |kind: &str, data: Value| match obs.call(kind, data).await {
			Ok(value) => Some(value),
			Err(error) => {
				println!("  skip {kind} {error}");
				None
			}
		};
		let scenes: Vec<String> = call("GetSceneList", json!({}))
			.await
			.and_then(|l| l["scenes"].as_array().cloned())
			.unwrap_or_default()
			.iter()
			.map(|s| s["sceneName"].as_str().unwrap_or("").to_string())
			.collect();
		for scene in ["Szene", "Scene 2", "Replay"] {
			if !scenes.iter().any(|s| s == scene) {
				call("CreateScene", json!({ "sceneName": scene })).await;
			}
		}
		call("CreateInput", json!({ "sceneName": "Scene 2", "inputName": "Blue BG", "inputKind": "color_source_v3", "inputSettings": { "color": 0xffd08030_u32, "width": 1280, "height": 800 } })).await;
		call("CreateInput", json!({ "sceneName": "Szene", "inputName": "Dark BG", "inputKind": "color_source_v3", "inputSettings": { "color": 0xff302020_u32, "width": 1280, "height": 800 } })).await;
		call("CreateInput", json!({ "sceneName": "Szene", "inputName": "Box", "inputKind": "color_source_v3", "inputSettings": { "color": 0xff3030e0_u32, "width": 200, "height": 200 } })).await;
		call("CreateInput", json!({ "sceneName": "Szene", "inputName": "Clip Player", "inputKind": "ffmpeg_source", "inputSettings": { "local_file": format!("{home}/Videos/obstools-test.mp4"), "is_local_file": true } })).await;
		call(
			"CreateInput",
			json!({ "sceneName": "Replay", "inputName": "Replay Player", "inputKind": "ffmpeg_source", "inputSettings": {} }),
		)
		.await;
		call("CreateSourceFilter", json!({ "sourceName": "Box", "filterName": "Tint", "filterKind": "color_filter_v2", "filterSettings": { "color_multiply": 0xff00ff00_u32 } })).await;
		call(
			"CreateSourceFilter",
			json!({ "sourceName": "Box", "filterName": "Look", "filterKind": "color_filter_v2", "filterSettings": { "opacity": 1 } }),
		)
		.await;
		call("CreateInput", json!({ "sceneName": "Szene", "inputName": "Picture", "inputKind": "image_source", "inputSettings": { "file": format!("{home}/Pictures/obstools-brb.png") } })).await;
		call(
			"CreateInput",
			json!({ "sceneName": "Szene", "inputName": "Title Text", "inputKind": "text_ft2_source_v2", "inputSettings": { "text": "Hello" } }),
		)
		.await;
		let profiles = call("GetProfileList", json!({})).await.unwrap_or(Value::Null);
		if !profiles["profiles"].as_array().is_some_and(|p| p.iter().any(|p| p == "Test 2")) {
			let current = profiles["currentProfileName"].clone();
			call("CreateProfile", json!({ "profileName": "Test 2" })).await;
			call("SetCurrentProfile", json!({ "profileName": current })).await;
		}
		// Order: Dark BG at the bottom.
		let items = call("GetSceneItemList", json!({ "sceneName": "Szene" }))
			.await
			.and_then(|l| l["sceneItems"].as_array().cloned())
			.unwrap_or_default();
		println!(
			"Szene items: {}",
			items.iter().map(|i| format!("{}:{}", i["sceneItemId"], i["sourceName"].as_str().unwrap_or(""))).collect::<Vec<_>>().join(", ")
		);
		let item = |name: &str| items.iter().find(|i| i["sourceName"] == name).map(|i| i["sceneItemId"].clone());
		if let Some(dark) = item("Dark BG") {
			call("SetSceneItemIndex", json!({ "sceneName": "Szene", "sceneItemId": dark, "sceneItemIndex": 0 })).await;
		}
		if let Some(box_item) = item("Box") {
			call(
				"SetSceneItemTransform",
				json!({ "sceneName": "Szene", "sceneItemId": box_item, "sceneItemTransform": { "positionX": 100, "positionY": 100 } }),
			)
			.await;
		}
		if let Some(clip) = item("Clip Player") {
			call("SetSceneItemEnabled", json!({ "sceneName": "Szene", "sceneItemId": clip, "sceneItemEnabled": false })).await;
			call(
				"SetSceneItemTransform",
				json!({ "sceneName": "Szene", "sceneItemId": clip, "sceneItemTransform": { "positionX": 600, "positionY": 380 } }),
			)
			.await;
		}
		let list = call("GetSceneList", json!({})).await.unwrap_or(Value::Null);
		println!(
			"scenes: {}",
			list["scenes"]
				.as_array()
				.cloned()
				.unwrap_or_default()
				.iter()
				.map(|s| format!("{}:{}", s["sceneIndex"], s["sceneName"].as_str().unwrap_or("")))
				.collect::<Vec<_>>()
				.join(", ")
		);
		println!("replay: {}", call("GetReplayBufferStatus", json!({})).await.unwrap_or(Value::Null));
		obs.client.stop();
		Ok(())
	})
}

fn run(program: &str, args: &[&str]) -> Result {
	let status = Command::new(program).args(args).status().map_err(|e| format!("{program}: {e}"))?;
	if status.success() { Ok(()) } else { Err(format!("{program} failed")) }
}

/// A 4 second test video with sound for the Video Player tests, and two pictures for Image Source.
pub fn make_video() -> Result {
	let home = std::env::var("HOME").unwrap_or_default();
	std::fs::create_dir_all(format!("{home}/Videos")).map_err(|e| e.to_string())?;
	let video = format!("{home}/Videos/obstools-test.mp4");
	run(
		"ffmpeg",
		&[
			"-hide_banner",
			"-loglevel",
			"error",
			"-y",
			"-f",
			"lavfi",
			"-i",
			"testsrc2=size=640x360:rate=30:duration=4",
			"-f",
			"lavfi",
			"-i",
			"sine=frequency=440:duration=4",
			"-c:v",
			"libx264",
			"-pix_fmt",
			"yuv420p",
			"-c:a",
			"aac",
			"-shortest",
			&video,
		],
	)?;
	std::fs::create_dir_all(format!("{home}/Pictures")).map_err(|e| e.to_string())?;
	for (color, text, name) in [("#2b7fff", "BRB", "obstools-brb.png"), ("#30a46c", "GG", "obstools-gg.png")] {
		let file = format!("{home}/Pictures/{name}");
		run(
			"magick",
			&[
				"-size",
				"400x225",
				&format!("xc:{color}"),
				"-gravity",
				"center",
				"-pointsize",
				"48",
				"-fill",
				"white",
				"-annotate",
				"0",
				text,
				&file,
			],
		)?;
	}
	println!("{video}, ~/Pictures/obstools-brb.png and obstools-gg.png");
	Ok(())
}
