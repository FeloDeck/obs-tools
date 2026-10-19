// The new suite (layout-new.json): the everyday actions, stream health, meters and dials.
use serde_json::{Value, json};

use super::{Deck, Obs, Suite, check, sleep, wait_for};
use crate::Result;

fn plugin_log() -> std::path::PathBuf {
	std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default())
		.join(".local/share/opendeck/logs/plugins/de.felitendo.obs-tools.sdPlugin.log")
}

fn log_size() -> u64 {
	std::fs::metadata(plugin_log()).map(|m| m.len()).unwrap_or(0)
}

fn log_since(size: u64) -> String {
	let text = std::fs::read(plugin_log()).unwrap_or_default();
	String::from_utf8_lossy(text.get(size as usize..).unwrap_or(&[])).into_owned()
}

pub async fn run(s: &mut Suite, obs: &Obs, deck: &Deck) -> Result {
	obs.call("SetCurrentProgramScene", json!({ "sceneName": "Szene" })).await?;
	sleep(500).await;

	s.test("source visibility toggles", async || {
		let id = obs.item("Szene", "Title Text").await?;
		obs.set_shown("Szene", &id, true).await?;
		sleep(300).await;
		deck.press(0, 80).await?;
		wait_for("hidden", 5000, async || Ok(json!(!obs.shown("Szene", &id).await?))).await?;
		deck.press(0, 80).await?;
		wait_for("visible", 5000, async || Ok(json!(obs.shown("Szene", &id).await?))).await.map(|_| ())
	})
	.await;

	s.test("hotkey by name with a source", async || {
		obs.call("SetInputMute", json!({ "inputName": "Desktop-Audio", "inputMuted": false })).await?;
		deck.press(1, 80).await?;
		wait_for("muted by hotkey", 5000, async || Ok(json!(obs.muted("Desktop-Audio").await?))).await?;
		obs.call("SetInputMute", json!({ "inputName": "Desktop-Audio", "inputMuted": false })).await.map(|_| ())
	})
	.await;

	s.test("screenshot saves a file", async || {
		let dir = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join("Pictures/obstools-test");
		std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
		let list = || -> Vec<String> {
			std::fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()
		};
		let before = list();
		deck.press(2, 80).await?;
		let file =
			wait_for("new file", 5000, async || Ok(list().into_iter().find(|f| !before.contains(f)).map_or(Value::Null, Value::from)))
				.await?;
		println!("     {}", file.as_str().unwrap_or(""));
		Ok(())
	})
	.await;

	s.test("set text: fixed, counter, double press, hold to reset, list", async || {
		let text_is = async |want: &str| wait_for(want, 5000, async || Ok(json!(obs.text().await? == want))).await.map(|_| ());
		deck.press(9, 80).await?;
		text_is("Live in 5").await?;
		// This counter counts down on a double press, so single presses need a pause between them.
		deck.press(10, 1000).await?;
		text_is("Deaths: 0").await?;
		deck.press(10, 80).await?;
		sleep(500).await;
		deck.press(10, 80).await?;
		text_is("Deaths: 2").await?;
		sleep(500).await;
		deck.press(10, 80).await?;
		deck.press(10, 80).await?;
		wait_for("Deaths: 1 (double press)", 5000, async || Ok(json!(obs.text().await? == "Deaths: 1"))).await?;
		deck.press(10, 1000).await?;
		text_is("Deaths: 0").await?;
		// The list key goes One, Two, Three, One... from wherever it is now.
		let lines = ["One", "Two", "Three"];
		deck.press(11, 80).await?;
		let first = wait_for("a line of the list", 5000, async || {
			let now = obs.text().await?;
			Ok(if lines.contains(&now.as_str()) { Value::from(now) } else { Value::Null })
		})
		.await?;
		deck.press(11, 80).await?;
		let at = lines.iter().position(|l| first == *l).unwrap_or(0);
		text_is(lines[(at + 1) % lines.len()]).await
	})
	.await;

	s.test("media control: play, pause, stop", async || {
		let id = obs.item("Szene", "Clip Player").await?;
		obs.set_shown("Szene", &id, true).await?;
		// Showing the source restarts it in OBS, so stop it only after that.
		sleep(800).await;
		obs.call("TriggerMediaInputAction", json!({ "inputName": "Clip Player", "mediaAction": "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_STOP" }))
			.await?;
		sleep(500).await;
		deck.press(5, 80).await?;
		wait_for("playing", 5000, async || Ok(json!(obs.media_state().await? == "OBS_MEDIA_STATE_PLAYING"))).await?;
		sleep(700).await;
		deck.press(5, 80).await?;
		wait_for("paused", 5000, async || Ok(json!(obs.media_state().await? == "OBS_MEDIA_STATE_PAUSED"))).await?;
		deck.press(6, 80).await?;
		wait_for("stopped", 5000, async || {
			Ok(json!(["OBS_MEDIA_STATE_STOPPED", "OBS_MEDIA_STATE_ENDED"].contains(&obs.media_state().await?.as_str())))
		})
		.await?;
		obs.set_shown("Szene", &id, false).await
	})
	.await;

	s.test("profile switch and back", async || {
		let profile = async || -> Result<Value> { Ok(obs.call("GetProfileList", json!({})).await?["currentProfileName"].clone()) };
		deck.press(7, 80).await?;
		wait_for("profile Test 2", 8000, async || Ok(json!(profile().await? == "Test 2"))).await?;
		sleep(1500).await;
		deck.press(8, 80).await?;
		wait_for("profile Unbenannt", 8000, async || Ok(json!(profile().await? == "Unbenannt"))).await?;
		sleep(1500).await;
		Ok(())
	})
	.await;

	s.test("recording marker: chapter and split while recording", async || {
		let recording =
			async || -> Result<bool> { Ok(obs.call("GetRecordStatus", json!({})).await?["outputActive"].as_bool().unwrap_or(false)) };
		let size = log_size();
		deck.press(3, 80).await?;
		sleep(500).await;
		check(log_since(size).contains("RecordTools: error: not recording"), "chapter while not recording should fail")?;
		deck.press(14, 80).await?;
		wait_for("recording", 6000, async || Ok(json!(recording().await?))).await?;
		sleep(1500).await;
		let before = log_size();
		deck.press(3, 80).await?;
		sleep(800).await;
		deck.press(4, 80).await?;
		sleep(1500).await;
		let log = log_since(before);
		// The chapter must work. Split only works with file splitting on in OBS, else OBS says so.
		let lower = log.to_lowercase();
		check(!lower.contains("chapter") && !lower.contains("hybrid"), format!("chapter failed: {}", log.trim()))?;
		println!(
			"     chapter ok; split: {}",
			if log.contains("file splitting") { "OBS says file splitting is off (expected here)" } else { "ok" }
		);
		deck.press(14, 80).await?;
		wait_for("stopped", 8000, async || Ok(json!(!recording().await?))).await.map(|_| ())
	})
	.await;

	s.test("source volume key and dial show a live meter", async || {
		let id = obs.item("Szene", "Clip Player").await?;
		let quiet_key = deck.key_image(13).await?;
		let quiet_dial = deck.dial_image(0).await?;
		obs.call("SetInputSettings", json!({ "inputName": "Clip Player", "inputSettings": { "looping": true }, "overlay": true })).await?;
		obs.set_shown("Szene", &id, true).await?;
		sleep(800).await;
		obs.call(
			"TriggerMediaInputAction",
			json!({ "inputName": "Clip Player", "mediaAction": "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_RESTART" }),
		)
		.await?;
		let result = async {
			wait_for("key meter moves", 4000, async || Ok(json!(deck.key_image(13).await? != quiet_key))).await?;
			wait_for("dial meter moves", 4000, async || Ok(json!(deck.dial_image(0).await? != quiet_dial))).await
		}
		.await;
		obs.call("TriggerMediaInputAction", json!({ "inputName": "Clip Player", "mediaAction": "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_STOP" }))
			.await?;
		obs.call("SetInputSettings", json!({ "inputName": "Clip Player", "inputSettings": { "looping": false }, "overlay": true })).await?;
		obs.set_shown("Szene", &id, false).await?;
		result.map(|_| ())
	})
	.await;

	s.test("transition dial: time and next transition", async || {
		let current = async || -> Result<Value> { obs.call("GetCurrentSceneTransition", json!({})).await };
		obs.call("SetCurrentSceneTransition", json!({ "transitionName": "Überblende" })).await?;
		obs.call("SetCurrentSceneTransitionDuration", json!({ "transitionDuration": 300 })).await?;
		sleep(400).await;
		deck.post("/api/dial/1/rotate?ticks=2").await?;
		wait_for("400 ms", 5000, async || Ok(json!(current().await?["transitionDuration"] == 400))).await?;
		deck.post("/api/dial/1/press").await?;
		wait_for("next transition", 5000, async || Ok(json!(current().await?["transitionName"] != "Überblende"))).await?;
		obs.call("SetCurrentSceneTransition", json!({ "transitionName": "Überblende" })).await?;
		obs.call("SetCurrentSceneTransitionDuration", json!({ "transitionDuration": 300 })).await.map(|_| ())
	})
	.await;

	s.test("scene dial: turn picks, press switches", async || {
		obs.call("SetCurrentProgramScene", json!({ "sceneName": "Szene" })).await?;
		sleep(500).await;
		let order = obs.scene_order().await?;
		let at = order.iter().position(|n| n == "Szene").ok_or("no scene Szene")?;
		let target = order[(at + 2) % order.len()].clone();
		deck.post("/api/dial/2/rotate?ticks=2").await?;
		sleep(600).await;
		check(obs.program().await? == "Szene", "turning should not switch in pick mode")?;
		deck.post("/api/dial/2/press").await?;
		wait_for(&format!("program = {target}"), 5000, async || Ok(json!(obs.program().await? == target))).await?;
		obs.call("SetCurrentProgramScene", json!({ "sceneName": "Szene" })).await.map(|_| ())
	})
	.await;

	s.test("filter dial: value, on and off", async || {
		let get = async || -> Result<Value> { obs.call("GetSourceFilter", json!({ "sourceName": "Box", "filterName": "Look" })).await };
		obs.call(
			"SetSourceFilterSettings",
			json!({ "sourceName": "Box", "filterName": "Look", "filterSettings": { "opacity": 1 }, "overlay": true }),
		)
		.await?;
		sleep(400).await;
		deck.post("/api/dial/3/rotate?ticks=-5").await?;
		wait_for("opacity 0.95", 5000, async || {
			Ok(json!((get().await?["filterSettings"]["opacity"].as_f64().unwrap_or(1.0) - 0.95).abs() < 0.001))
		})
		.await?;
		deck.post("/api/dial/3/press").await?;
		wait_for("filter off", 5000, async || Ok(json!(get().await?["filterEnabled"] == false))).await?;
		deck.post("/api/dial/3/press").await?;
		wait_for("filter on", 5000, async || Ok(get().await?["filterEnabled"].clone())).await?;
		obs.call(
			"SetSourceFilterSettings",
			json!({ "sourceName": "Box", "filterName": "Look", "filterSettings": { "opacity": 1 }, "overlay": true }),
		)
		.await
		.map(|_| ())
	})
	.await;

	s.test("stream health while streaming to a local server", async || {
		let streaming = async || -> Result<Value> { obs.call("GetStreamStatus", json!({})).await };
		let old = obs.call("GetStreamServiceSettings", json!({})).await?;
		let mut server = tokio::process::Command::new("ffmpeg")
			.args(["-hide_banner", "-loglevel", "error", "-listen", "1", "-i", "rtmp://127.0.0.1:19350/live/test", "-f", "null", "-"])
			.stdin(std::process::Stdio::null())
			.stdout(std::process::Stdio::null())
			.stderr(std::process::Stdio::null())
			.kill_on_drop(true)
			.spawn()
			.map_err(|e| format!("ffmpeg: {e}"))?;
		let result = async {
			sleep(800).await;
			obs.call(
				"SetStreamServiceSettings",
				json!({ "streamServiceType": "rtmp_custom", "streamServiceSettings": { "server": "rtmp://127.0.0.1:19350/live", "key": "test" } }),
			)
			.await?;
			let before = deck.key_image(12).await?;
			obs.call("StartStream", json!({})).await?;
			wait_for("streaming", 10000, async || Ok(streaming().await?["outputActive"].clone())).await?;
			sleep(4000).await;
			let live = deck.key_image(12).await?;
			check(live != before, "key did not change while live")?;
			let status = streaming().await?;
			println!(
				"     {}, {} bytes, congestion {}",
				status["outputTimecode"].as_str().unwrap_or(""),
				status["outputBytes"],
				status["outputCongestion"]
			);
			obs.call("StopStream", json!({})).await?;
			wait_for("stopped", 10000, async || Ok(json!(streaming().await?["outputActive"] == false))).await.map(|_| ())
		}
		.await;
		let _ = server.kill().await;
		let _ = obs
			.call(
				"SetStreamServiceSettings",
				json!({ "streamServiceType": old["streamServiceType"], "streamServiceSettings": old["streamServiceSettings"] }),
			)
			.await;
		result
	})
	.await;
	Ok(())
}
