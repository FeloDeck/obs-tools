// End-to-end tests: press keys on the virtual deck (through OpenDeck) and check OBS.
// Needs OBS with the test scenes (cargo xtask setup-obs), OpenDeck with both plugins and the profile
// of the suite (cargo xtask profile test/e2e/layout-<suite>.json, OpenDeck stopped meanwhile).
//
//   cargo xtask e2e main|new|more|extra [test name filter]
use std::future::Future;
use std::rc::Rc;
use std::time::{Duration, Instant};

use obs_tools::connections::{local_config_paths, read_local_obs_config};
use obs_tools::obs::ObsClient;
use obs_tools::volume::fader_to_db;
use serde_json::{Value, json};
use tokio::task::JoinHandle;

use crate::Result;

/// Runs async code on one thread, like the plugin.
pub fn local<F: Future>(future: F) -> F::Output {
	let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio runtime");
	tokio::task::LocalSet::new().block_on(&runtime, future)
}

pub async fn sleep(ms: u64) {
	tokio::time::sleep(Duration::from_millis(ms)).await;
}

/// The OBS on this PC, with the port and password from its config.
pub struct Obs {
	pub client: Rc<ObsClient>,
}

impl Obs {
	pub async fn connect() -> Result<Obs> {
		let config = read_local_obs_config(&local_config_paths()).ok_or("no obs-websocket config on this PC")?;
		let client = ObsClient::new("127.0.0.1", config.port, &config.password, "e2e");
		let (tx, rx) = tokio::sync::oneshot::channel();
		let tx = std::cell::RefCell::new(Some(tx));
		client.on_connected.on(move |_| {
			if let Some(tx) = tx.borrow_mut().take() {
				let _ = tx.send(());
			}
		});
		client.start();
		tokio::time::timeout(Duration::from_secs(20), rx)
			.await
			.map_err(|_| format!("OBS does not answer: {}", client.last_error()))?
			.map_err(|e| e.to_string())?;
		Ok(Obs { client })
	}

	pub async fn call(&self, kind: &str, data: Value) -> Result<Value> {
		self.client.call(kind, data).await.map_err(|e| format!("{kind}: {e}"))
	}

	async fn program(&self) -> Result<String> {
		Ok(self.call("GetCurrentProgramScene", json!({})).await?["sceneName"].as_str().unwrap_or("").to_string())
	}

	async fn scene_order(&self) -> Result<Vec<String>> {
		Ok(obs_tools::obs_state::sorted_scene_names(&self.call("GetSceneList", json!({})).await?))
	}

	async fn item(&self, scene: &str, source: &str) -> Result<Value> {
		Ok(self.call("GetSceneItemId", json!({ "sceneName": scene, "sourceName": source })).await?["sceneItemId"].clone())
	}

	async fn shown(&self, scene: &str, id: &Value) -> Result<bool> {
		Ok(self.call("GetSceneItemEnabled", json!({ "sceneName": scene, "sceneItemId": id })).await?["sceneItemEnabled"]
			.as_bool()
			.unwrap_or(false))
	}

	async fn set_shown(&self, scene: &str, id: &Value, shown: bool) -> Result {
		self.call("SetSceneItemEnabled", json!({ "sceneName": scene, "sceneItemId": id, "sceneItemEnabled": shown })).await.map(|_| ())
	}

	async fn muted(&self, input: &str) -> Result<bool> {
		Ok(self.call("GetInputMute", json!({ "inputName": input })).await?["inputMuted"].as_bool().unwrap_or(false))
	}

	async fn db(&self, input: &str) -> Result<f64> {
		Ok(self.call("GetInputVolume", json!({ "inputName": input })).await?["inputVolumeDb"].as_f64().unwrap_or(f64::NAN))
	}

	async fn text(&self) -> Result<String> {
		Ok(self.call("GetInputSettings", json!({ "inputName": "Title Text" })).await?["inputSettings"]["text"]
			.as_str()
			.unwrap_or("")
			.to_string())
	}

	async fn media_state(&self) -> Result<String> {
		Ok(self.call("GetMediaInputStatus", json!({ "inputName": "Clip Player" })).await?["mediaState"].as_str().unwrap_or("").to_string())
	}
}

/// The virtual deck, through its HTTP API.
#[derive(Clone)]
struct Deck {
	url: String,
}

impl Deck {
	fn new() -> Deck {
		Deck { url: std::env::var("VIRTUAL_DECK_URL").unwrap_or_else(|_| "http://127.0.0.1:57190".into()) }
	}

	/// The request runs on its own thread, so a long press can go on while the test looks at OBS.
	fn post_in_background(&self, path: &str) -> JoinHandle<Result> {
		let url = format!("{}{path}", self.url);
		tokio::task::spawn_blocking(move || ureq::post(&url).send_empty().map(|_| ()).map_err(|e| format!("{url}: {e}")))
	}

	async fn post(&self, path: &str) -> Result {
		self.post_in_background(path).await.map_err(|e| e.to_string())?
	}

	async fn press(&self, key: usize, ms: u64) -> Result {
		self.post(&format!("/api/key/{key}/press?ms={ms}")).await
	}

	async fn get(&self, path: &str) -> Result<Vec<u8>> {
		let url = format!("{}{path}", self.url);
		tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
			let mut response = ureq::get(&url).call().map_err(|e| format!("{url}: {e}"))?;
			response.body_mut().read_to_vec().map_err(|e| e.to_string())
		})
		.await
		.map_err(|e| e.to_string())?
	}

	async fn state(&self) -> Result<Value> {
		serde_json::from_slice(&self.get("/api/state").await?).map_err(|e| e.to_string())
	}

	async fn key_image(&self, key: usize) -> Result<Value> {
		Ok(self.state().await?["keys"][key].clone())
	}

	async fn dial_image(&self, dial: usize) -> Result<Value> {
		Ok(self.state().await?["dials"][dial].clone())
	}

	/// Color of one pixel of a key, as OpenDeck drew it, like "srgb(245,165,36)".
	async fn pixel(&self, key: usize, x: u32, y: u32) -> Result<String> {
		let png = self.get(&format!("/api/key/{key}.png")).await?;
		let mut child = std::process::Command::new("magick")
			.args(["-", "-format", &format!("%[pixel:p{{{x},{y}}}]"), "info:"])
			.stdin(std::process::Stdio::piped())
			.stdout(std::process::Stdio::piped())
			.spawn()
			.map_err(|e| format!("magick: {e}"))?;
		use std::io::Write;
		child.stdin.take().ok_or("no stdin")?.write_all(&png).map_err(|e| e.to_string())?;
		let output = child.wait_with_output().map_err(|e| e.to_string())?;
		Ok(String::from_utf8_lossy(&output.stdout).into_owned())
	}

	async fn shot(&self, name: &str) -> Result {
		let Ok(dir) = std::env::var("SHOTS_DIR") else { return Ok(()) };
		std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
		std::fs::write(format!("{dir}/{name}.json"), self.state().await?.to_string()).map_err(|e| e.to_string())
	}
}

/// Polls until the check gives something truthy, like waitFor of the JavaScript tests.
async fn wait_for(what: &str, timeout: u64, check: impl AsyncFn() -> Result<Value>) -> Result<Value> {
	let until = Instant::now() + Duration::from_millis(timeout);
	let mut last = Value::Null;
	while Instant::now() < until {
		last = check().await?;
		if obs_tools::js::truthy(Some(&last)) {
			return Ok(last);
		}
		sleep(100).await;
	}
	Err(format!("timeout waiting for {what} (last: {last})"))
}

fn check(condition: bool, message: impl Into<String>) -> Result {
	if condition { Ok(()) } else { Err(message.into()) }
}

struct Suite {
	filter: String,
	count: usize,
	failed: usize,
}

impl Suite {
	async fn test(&mut self, name: &str, test: impl AsyncFnOnce() -> Result) {
		if !self.filter.is_empty() && !name.contains(&self.filter) {
			return;
		}
		self.count += 1;
		let start = Instant::now();
		match test().await {
			Ok(()) => println!("ok   {name} ({} ms)", start.elapsed().as_millis()),
			Err(error) => {
				self.failed += 1;
				println!("FAIL {name}: {error}");
			}
		}
	}
}

pub fn run(args: &[String]) -> Result {
	let suite = args.first().map(String::as_str).unwrap_or("");
	let filter = args.get(1).cloned().unwrap_or_default();
	local(async move {
		let obs = Obs::connect().await?;
		let deck = Deck::new();
		let mut s = Suite { filter, count: 0, failed: 0 };
		match suite {
			"main" => main_suite(&mut s, &obs, &deck).await?,
			"new" => new_suite(&mut s, &obs, &deck).await?,
			"more" => more_suite(&mut s, &obs, &deck).await?,
			"extra" => extra_suite(&mut s, &obs, &deck).await?,
			_ => return Err("usage: cargo xtask e2e main|new|more|extra [filter]".into()),
		}
		obs.client.stop();
		println!("\n{}/{} passed", s.count - s.failed, s.count);
		if s.failed > 0 { Err(format!("{} failed", s.failed)) } else { Ok(()) }
	})
}

// ----- main: layout-main.json -----

async fn main_suite(s: &mut Suite, obs: &Obs, deck: &Deck) -> Result {
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

// ----- new: layout-new.json. The everyday actions, stream health, meters and dials. -----

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

async fn new_suite(s: &mut Suite, obs: &Obs, deck: &Deck) -> Result {
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

// ----- more: layout-more.json. Push to talk, take preview live, the text timer, setup images,
// names as titles, and press, hold and double press on other keys. -----

async fn more_suite(s: &mut Suite, obs: &Obs, deck: &Deck) -> Result {
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

// ----- extra: layout-extra.json. Monitoring, set volume, scene above, an OpenDeck multi action,
// image source and a counter. -----

async fn extra_suite(s: &mut Suite, obs: &Obs, deck: &Deck) -> Result {
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
