// The main suite (layout-main.json): scenes, studio mode, volume and mute, transitions, filters,
// source animation, video player, instant replay, recording and scene renames.
use serde_json::{Value, json};

use super::{Deck, Obs, Suite, check, sleep, wait_for};
use crate::Result;

pub async fn run(s: &mut Suite, obs: &Obs, deck: &Deck) -> Result {
	obs.call("SetStudioModeEnabled", json!({ "studioModeEnabled": false })).await?;
	obs.call("SetCurrentProgramScene", json!({ "sceneName": "Szene" })).await?;
	sleep(800).await;

	s.test("smart scene switches the live scene", async || {
		let before = deck.key_image(1).await?;
		deck.press(1, 80).await?;
		wait_for("program = Scene 2", 5000, async || Ok(json!(obs.program().await? == "Scene 2"))).await?;
		wait_for("key 1 image changes", 5000, async || Ok(json!(deck.key_image(1).await? != before))).await?;
		deck.shot("smartscene-live").await
	})
	.await;

	s.test("previous scene goes back", async || {
		sleep(300).await;
		deck.press(3, 80).await?;
		wait_for("program = Szene", 5000, async || Ok(json!(obs.program().await? == "Szene"))).await.map(|_| ())
	})
	.await;

	s.test("next scene follows the scene list", async || {
		let order = obs.scene_order().await?;
		let at = order.iter().position(|n| n == "Szene").ok_or("no scene Szene")?;
		let expected = order[(at + 1) % order.len()].clone();
		deck.press(4, 80).await?;
		wait_for(&format!("program = {expected}"), 5000, async || Ok(json!(obs.program().await? == expected))).await?;
		obs.call("SetCurrentProgramScene", json!({ "sceneName": "Szene" })).await?;
		sleep(300).await;
		Ok(())
	})
	.await;

	s.test("studio mode: preview first, then live", async || {
		deck.press(2, 80).await?;
		wait_for("studio mode on", 5000, async || Ok(obs.call("GetStudioModeEnabled", json!({})).await?["studioModeEnabled"].clone()))
			.await?;
		sleep(400).await;
		deck.press(1, 80).await?;
		wait_for("preview = Scene 2", 5000, async || {
			Ok(json!(obs.call("GetCurrentPreviewScene", json!({})).await?["sceneName"] == "Scene 2"))
		})
		.await?;
		sleep(400).await;
		deck.shot("smartscene-preview").await?;
		check(obs.program().await? == "Szene", "program changed too early")?;
		deck.press(1, 80).await?;
		wait_for("program = Scene 2", 6000, async || Ok(json!(obs.program().await? == "Scene 2"))).await?;
		deck.press(2, 80).await?;
		wait_for("studio mode off", 5000, async || {
			Ok(json!(obs.call("GetStudioModeEnabled", json!({})).await?["studioModeEnabled"] == false))
		})
		.await?;
		obs.call("SetCurrentProgramScene", json!({ "sceneName": "Szene" })).await?;
		sleep(300).await;
		Ok(())
	})
	.await;

	s.test("source volume: press and hold", async || {
		obs.call("SetInputVolume", json!({ "inputName": "Desktop-Audio", "inputVolumeDb": 0 })).await?;
		sleep(300).await;
		deck.press(7, 80).await?;
		wait_for("-3 dB", 5000, async || Ok(json!((obs.db("Desktop-Audio").await? + 3.0).abs() < 0.05))).await?;
		deck.press(7, 1100).await?;
		let db = obs.db("Desktop-Audio").await?;
		check(db <= -12.0, format!("holding should repeat, volume is {db} dB"))
	})
	.await;

	s.test("dial turns volume and mutes", async || {
		obs.call("SetInputVolume", json!({ "inputName": "Desktop-Audio", "inputVolumeDb": -10 })).await?;
		sleep(300).await;
		deck.post("/api/dial/0/rotate?ticks=3").await?;
		wait_for("-7 dB", 5000, async || Ok(json!((obs.db("Desktop-Audio").await? + 7.0).abs() < 0.05))).await?;
		deck.post("/api/dial/0/press").await?;
		wait_for("muted", 5000, async || Ok(json!(obs.muted("Desktop-Audio").await?))).await?;
		sleep(500).await;
		deck.shot("dial-muted").await?;
		deck.post("/api/touch/0").await?;
		wait_for("unmuted", 5000, async || Ok(json!(!obs.muted("Desktop-Audio").await?))).await?;
		obs.call("SetInputVolume", json!({ "inputName": "Desktop-Audio", "inputVolumeDb": 0 })).await.map(|_| ())
	})
	.await;

	s.test("mute toggle", async || {
		deck.press(8, 80).await?;
		wait_for("muted", 5000, async || Ok(json!(obs.muted("Desktop-Audio").await?))).await?;
		sleep(400).await;
		deck.shot("muted").await?;
		deck.press(8, 80).await?;
		wait_for("unmuted", 5000, async || Ok(json!(!obs.muted("Desktop-Audio").await?))).await.map(|_| ())
	})
	.await;

	s.test("set transition", async || {
		obs.call("SetCurrentSceneTransition", json!({ "transitionName": "Überblende" })).await?;
		sleep(300).await;
		deck.press(9, 80).await?;
		wait_for("transition = Schnitt", 5000, async || {
			Ok(json!(obs.call("GetCurrentSceneTransition", json!({})).await?["transitionName"] == "Schnitt"))
		})
		.await?;
		obs.call("SetCurrentSceneTransition", json!({ "transitionName": "Überblende" })).await.map(|_| ())
	})
	.await;

	s.test("filter toggle", async || {
		let enabled = async || -> Result<bool> {
			Ok(obs.call("GetSourceFilter", json!({ "sourceName": "Box", "filterName": "Tint" })).await?["filterEnabled"]
				.as_bool()
				.unwrap_or(false))
		};
		obs.call("SetSourceFilterEnabled", json!({ "sourceName": "Box", "filterName": "Tint", "filterEnabled": true })).await?;
		sleep(300).await;
		deck.press(14, 80).await?;
		wait_for("filter off", 5000, async || Ok(json!(!enabled().await?))).await?;
		deck.press(14, 80).await?;
		wait_for("filter on", 5000, async || Ok(json!(enabled().await?))).await.map(|_| ())
	})
	.await;

	s.test("source animation plays and resets", async || {
		let id = obs.item("Szene", "Box").await?;
		let transform = async || -> Result<Value> {
			Ok(obs.call("GetSceneItemTransform", json!({ "sceneName": "Szene", "sceneItemId": id })).await?["sceneItemTransform"].clone())
		};
		let x = |t: &Value, k: &str| t[k].as_f64().unwrap_or(f64::NAN);
		let start = transform().await?;
		deck.press(13, 80).await?;
		sleep(700).await;
		let mid = transform().await?;
		check(
			x(&mid, "positionX") > x(&start, "positionX") + 50.0,
			format!("should be moving (x {} -> {})", x(&start, "positionX"), x(&mid, "positionX")),
		)?;
		check(x(&mid, "positionX") < x(&start, "positionX") + 600.0, format!("should not be done yet (x {})", x(&mid, "positionX")))?;
		sleep(1300).await;
		let peak = transform().await?;
		wait_for("back at start", 4000, async || {
			let t = transform().await?;
			Ok(json!(
				(x(&t, "positionX") - x(&start, "positionX")).abs() < 0.5
					&& (x(&t, "rotation") - x(&start, "rotation")).abs() < 0.5
					&& (x(&t, "scaleX") - x(&start, "scaleX")).abs() < 0.01
			))
		})
		.await?;
		println!(
			"     x: {} -> {:.1} (0.7 s) -> back; peak sample scale {:.2}",
			x(&start, "positionX"),
			x(&mid, "positionX"),
			x(&peak, "scaleX")
		);
		Ok(())
	})
	.await;

	s.test("video player plays, hides when done", async || {
		let id = obs.item("Szene", "Clip Player").await?;
		obs.set_shown("Szene", &id, false).await?;
		deck.press(12, 80).await?;
		wait_for("visible", 5000, async || Ok(json!(obs.shown("Szene", &id).await?))).await?;
		wait_for("playing", 5000, async || Ok(json!(obs.media_state().await? == "OBS_MEDIA_STATE_PLAYING"))).await?;
		sleep(600).await;
		deck.shot("video-playing").await?;
		wait_for("hidden after the end", 9000, async || Ok(json!(!obs.shown("Szene", &id).await?))).await.map(|_| ())
	})
	.await;

	s.test("video player: second press stops", async || {
		let id = obs.item("Szene", "Clip Player").await?;
		deck.press(12, 80).await?;
		wait_for("visible", 5000, async || Ok(json!(obs.shown("Szene", &id).await?))).await?;
		sleep(800).await;
		deck.press(12, 80).await?;
		wait_for("hidden", 3000, async || Ok(json!(!obs.shown("Szene", &id).await?))).await.map(|_| ())
	})
	.await;

	s.test("instant replay: hold starts buffer, press replays in slow motion", async || {
		let active =
			async || -> Result<bool> { Ok(obs.call("GetReplayBufferStatus", json!({})).await?["outputActive"].as_bool().unwrap_or(false)) };
		if active().await? {
			obs.call("StopReplayBuffer", json!({})).await?;
		}
		sleep(500).await;
		deck.press(11, 1000).await?;
		wait_for("replay buffer on", 6000, async || Ok(json!(active().await?))).await?;
		sleep(500).await;
		deck.shot("replay-armed").await?;
		sleep(3000).await;
		obs.call("SetCurrentProgramScene", json!({ "sceneName": "Szene" })).await?;
		sleep(300).await;
		deck.press(11, 80).await?;
		wait_for("switched to Replay", 10000, async || Ok(json!(obs.program().await? == "Replay"))).await?;
		let settings = wait_for("replay file loaded", 5000, async || {
			let s = obs.call("GetInputSettings", json!({ "inputName": "Replay Player" })).await?["inputSettings"].clone();
			Ok(if s["speed_percent"] == 50 { s } else { Value::Null })
		})
		.await?;
		let file = settings["local_file"].as_str().unwrap_or("");
		check(file.contains("Videos/Replay"), format!("replay file not set: {file}"))?;
		println!("     playing {file} at {}%", settings["speed_percent"]);
		wait_for("back to Szene after the replay", 20000, async || Ok(json!(obs.program().await? == "Szene"))).await?;
		deck.press(11, 1000).await?;
		wait_for("replay buffer off", 6000, async || Ok(json!(!active().await?))).await.map(|_| ())
	})
	.await;

	s.test("record toggle shows a timer", async || {
		let status = async || -> Result<Value> { obs.call("GetRecordStatus", json!({})).await };
		deck.press(10, 80).await?;
		wait_for("recording", 6000, async || Ok(status().await?["outputActive"].clone())).await?;
		sleep(2500).await;
		deck.shot("recording").await?;
		deck.press(10, 80).await?;
		wait_for("stopped", 8000, async || Ok(json!(status().await?["outputActive"] == false))).await.map(|_| ())
	})
	.await;

	s.test("scene rename follows into the key settings", async || {
		obs.call("SetSceneName", json!({ "sceneName": "Scene 2", "newSceneName": "Scene Two" })).await?;
		sleep(800).await;
		deck.press(1, 80).await?;
		let result = wait_for("program = Scene Two", 5000, async || Ok(json!(obs.program().await? == "Scene Two"))).await;
		obs.call("SetSceneName", json!({ "sceneName": "Scene Two", "newSceneName": "Scene 2" })).await?;
		sleep(500).await;
		obs.call("SetCurrentProgramScene", json!({ "sceneName": "Szene" })).await?;
		result.map(|_| ())
	})
	.await;
	Ok(())
}
