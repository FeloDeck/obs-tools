// The extra suite (layout-extra.json): monitoring, set volume, scene above, an OpenDeck multi action,
// image source and a counter.
use obs_tools::volume::fader_to_db;
use serde_json::{Value, json};

use super::{Deck, Obs, Suite, check, sleep, wait_for};
use crate::Result;

pub async fn run(s: &mut Suite, obs: &Obs, deck: &Deck) -> Result {
	let input = "Desktop-Audio";
	obs.call("SetCurrentProgramScene", json!({ "sceneName": "Szene" })).await?;
	obs.call("SetInputAudioMonitorType", json!({ "inputName": input, "monitorType": "OBS_MONITORING_TYPE_NONE" })).await?;
	sleep(500).await;
	let monitor =
		async || -> Result<Value> { Ok(obs.call("GetInputAudioMonitorType", json!({ "inputName": input })).await?["monitorType"].clone()) };

	s.test("monitor set, press again turns it off", async || {
		deck.press(0, 80).await?;
		wait_for("monitor only", 5000, async || Ok(json!(monitor().await? == "OBS_MONITORING_TYPE_MONITOR_ONLY"))).await?;
		sleep(300).await;
		deck.press(0, 80).await?;
		wait_for("monitor off", 5000, async || Ok(json!(monitor().await? == "OBS_MONITORING_TYPE_NONE"))).await.map(|_| ())
	})
	.await;

	s.test("set volume to 50% of the fader", async || {
		obs.call("SetInputVolume", json!({ "inputName": input, "inputVolumeDb": 0 })).await?;
		deck.press(8, 80).await?;
		let target = fader_to_db(0.5);
		wait_for(&format!("{target:.2} dB"), 5000, async || Ok(json!((obs.db(input).await? - target).abs() < 0.05))).await?;
		obs.call("SetInputVolume", json!({ "inputName": input, "inputVolumeDb": 0 })).await.map(|_| ())
	})
	.await;

	s.test("scene above (previous in the list)", async || {
		let order = obs.scene_order().await?;
		let at = order.iter().position(|n| n == "Szene").ok_or("no scene Szene")?;
		let expected = order[(at + order.len() - 1) % order.len()].clone();
		deck.press(5, 80).await?;
		wait_for(&format!("program = {expected}"), 5000, async || Ok(json!(obs.program().await? == expected))).await?;
		obs.call("SetCurrentProgramScene", json!({ "sceneName": "Szene" })).await?;
		sleep(300).await;
		Ok(())
	})
	.await;

	s.test("multi action: studio on, mute, scene goes live directly", async || {
		let studio = async || -> Result<bool> {
			Ok(obs.call("GetStudioModeEnabled", json!({})).await?["studioModeEnabled"].as_bool().unwrap_or(false))
		};
		obs.call("SetStudioModeEnabled", json!({ "studioModeEnabled": false })).await?;
		obs.call("SetInputMute", json!({ "inputName": input, "inputMuted": false })).await?;
		sleep(300).await;
		deck.press(9, 80).await?;
		wait_for("studio mode on", 5000, async || Ok(json!(studio().await?))).await?;
		wait_for("muted", 5000, async || Ok(json!(obs.muted(input).await?))).await?;
		wait_for("program = Scene 2", 6000, async || Ok(json!(obs.program().await? == "Scene 2"))).await?;
		// "enable" and "mute" must not toggle back on a second run.
		sleep(500).await;
		deck.press(9, 80).await?;
		sleep(1200).await;
		check(studio().await?, "second run turned studio mode off")?;
		check(obs.muted(input).await?, "second run unmuted")?;
		obs.call("SetStudioModeEnabled", json!({ "studioModeEnabled": false })).await?;
		obs.call("SetInputMute", json!({ "inputName": input, "inputMuted": false })).await?;
		obs.call("SetCurrentProgramScene", json!({ "sceneName": "Szene" })).await.map(|_| ())
	})
	.await;

	s.test("image source: picture per key, auto hide, press again hides", async || {
		let id = obs.item("Szene", "Picture").await?;
		let file = async || -> Result<String> {
			Ok(obs.call("GetInputSettings", json!({ "inputName": "Picture" })).await?["inputSettings"]["file"]
				.as_str()
				.unwrap_or("")
				.to_string())
		};
		obs.set_shown("Szene", &id, false).await?;
		sleep(300).await;
		deck.press(12, 80).await?;
		wait_for("GG shown", 5000, async || Ok(json!(obs.shown("Szene", &id).await? && file().await?.ends_with("obstools-gg.png"))))
			.await?;
		wait_for("hidden after 2 s", 4000, async || Ok(json!(!obs.shown("Szene", &id).await?))).await?;
		deck.press(13, 80).await?;
		wait_for("BRB shown", 5000, async || Ok(json!(obs.shown("Szene", &id).await? && file().await?.ends_with("obstools-brb.png"))))
			.await?;
		sleep(400).await;
		deck.press(13, 80).await?;
		wait_for("hidden by second press", 5000, async || Ok(json!(!obs.shown("Szene", &id).await?))).await.map(|_| ())
	})
	.await;

	s.test("set text: count down from 3, stops at 0, double press, hold", async || {
		let press_and_wait = async |expected: &str, ms: u64| -> Result {
			deck.press(14, ms).await?;
			wait_for(expected, 5000, async || Ok(json!(obs.text().await? == expected))).await?;
			// A pause, or the next press would make a double press.
			sleep(500).await;
			Ok(())
		};
		press_and_wait("Live in 3", 1000).await?;
		press_and_wait("Live in 2", 80).await?;
		press_and_wait("Live in 1", 80).await?;
		press_and_wait("Live in 0", 80).await?;
		press_and_wait("Live in 0", 80).await?;
		deck.press(14, 80).await?;
		deck.press(14, 80).await?;
		wait_for("Live in 1 (double press counts up)", 5000, async || Ok(json!(obs.text().await? == "Live in 1"))).await?;
		sleep(500).await;
		press_and_wait("Live in 3", 1000).await
	})
	.await;
	Ok(())
}
