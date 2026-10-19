// The more suite (layout-more.json): push to talk, take preview live, the text timer, setup images,
// names as titles, and press, hold and double press on other keys.
use serde_json::json;

use super::{Deck, Obs, Suite, check, sleep, wait_for};
use crate::Result;

pub async fn run(s: &mut Suite, obs: &Obs, deck: &Deck) -> Result {
	s.test("push to talk: on while held, muted again after", async || {
		obs.call("SetInputMute", json!({ "inputName": "Desktop-Audio", "inputMuted": true })).await?;
		sleep(300).await;
		let holding = deck.post_in_background("/api/key/0/press?ms=1200");
		wait_for("on while held", 5000, async || Ok(json!(!obs.muted("Desktop-Audio").await?))).await?;
		holding.await.map_err(|e| e.to_string())??;
		wait_for("muted after release", 5000, async || Ok(json!(obs.muted("Desktop-Audio").await?))).await.map(|_| ())
	})
	.await;

	s.test("hold to mute: muted while held, on again after", async || {
		obs.call("SetInputMute", json!({ "inputName": "Desktop-Audio", "inputMuted": false })).await?;
		sleep(300).await;
		let holding = deck.post_in_background("/api/key/1/press?ms=1200");
		wait_for("muted while held", 5000, async || Ok(json!(obs.muted("Desktop-Audio").await?))).await?;
		holding.await.map_err(|e| e.to_string())??;
		wait_for("on after release", 5000, async || Ok(json!(!obs.muted("Desktop-Audio").await?))).await.map(|_| ())
	})
	.await;

	s.test("take preview live", async || {
		obs.call("SetCurrentProgramScene", json!({ "sceneName": "Szene" })).await?;
		obs.call("SetStudioModeEnabled", json!({ "studioModeEnabled": true })).await?;
		sleep(500).await;
		obs.call("SetCurrentPreviewScene", json!({ "sceneName": "Scene 2" })).await?;
		sleep(300).await;
		deck.press(2, 80).await?;
		wait_for("Scene 2 live", 6000, async || Ok(json!(obs.program().await? == "Scene 2"))).await?;
		obs.call("SetStudioModeEnabled", json!({ "studioModeEnabled": false })).await?;
		obs.call("SetCurrentProgramScene", json!({ "sceneName": "Szene" })).await?;
		sleep(500).await;
		Ok(())
	})
	.await;

	s.test("text timer counts down to the end text, a second press stops it", async || {
		deck.press(3, 80).await?;
		wait_for("Live in 0:02", 5000, async || Ok(json!(obs.text().await? == "Live in 0:02"))).await?;
		wait_for("Live!", 5000, async || Ok(json!(obs.text().await? == "Live!"))).await?;
		deck.press(3, 80).await?;
		wait_for("Live in 0:03", 5000, async || Ok(json!(obs.text().await? == "Live in 0:03"))).await?;
		sleep(400).await;
		deck.press(3, 80).await?;
		sleep(2500).await;
		check(obs.text().await? != "Live!", "the timer did not stop")
	})
	.await;

	s.test("keys that are not set up show the yellow dot", async || {
		let yellow = |color: &str| {
			let Some(rgb) = color.strip_prefix("srgb(").and_then(|c| c.strip_suffix(')')) else { return false };
			let parts: Vec<u32> = rgb.split(',').filter_map(|p| p.trim().parse().ok()).collect();
			parts.len() == 3 && (220..=255).contains(&parts[0]) && (140..=199).contains(&parts[1]) && parts[2] < 100
		};
		for key in [4, 8] {
			let color = deck.pixel(key, 120, 24).await?;
			check(yellow(&color), format!("key {key}: {color}"))?;
		}
		// A set up key has no dot there.
		let color = deck.pixel(5, 120, 24).await?;
		let rg: Vec<u32> = color.trim_start_matches("srgb(").split(',').take(2).filter_map(|p| p.trim().parse().ok()).collect();
		check(!(rg.len() == 2 && (220..=259).contains(&rg[0]) && (140..=199).contains(&rg[1])), "key 5 has a dot")
	})
	.await;

	s.test("press, hold and double press on a visibility key", async || {
		let id = obs.item("Szene", "Title Text").await?;
		obs.set_shown("Szene", &id, true).await?;
		sleep(400).await;
		deck.press(10, 1000).await?;
		wait_for("hidden by hold", 5000, async || Ok(json!(!obs.shown("Szene", &id).await?))).await?;
		sleep(500).await;
		deck.press(10, 80).await?;
		deck.press(10, 80).await?;
		wait_for("shown by double press", 5000, async || Ok(json!(obs.shown("Szene", &id).await?))).await?;
		sleep(500).await;
		deck.press(10, 80).await?;
		wait_for("toggled by press (after the double press wait)", 5000, async || Ok(json!(!obs.shown("Szene", &id).await?))).await?;
		obs.set_shown("Szene", &id, true).await
	})
	.await;

	s.test("press, hold and double press on a media key", async || {
		let id = obs.item("Szene", "Clip Player").await?;
		obs.set_shown("Szene", &id, true).await?;
		sleep(800).await;
		deck.press(9, 1000).await?;
		wait_for("stopped by hold", 5000, async || {
			Ok(json!(["OBS_MEDIA_STATE_STOPPED", "OBS_MEDIA_STATE_ENDED"].contains(&obs.media_state().await?.as_str())))
		})
		.await?;
		sleep(500).await;
		deck.press(9, 80).await?;
		deck.press(9, 80).await?;
		wait_for("playing again by double press", 5000, async || Ok(json!(obs.media_state().await? == "OBS_MEDIA_STATE_PLAYING"))).await?;
		obs.set_shown("Szene", &id, false).await
	})
	.await;
	Ok(())
}
