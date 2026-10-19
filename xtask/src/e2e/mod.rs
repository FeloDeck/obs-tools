// End-to-end tests: press keys on the virtual deck (through OpenDeck) and check OBS.
// Needs OBS with the test scenes (cargo xtask setup-obs), OpenDeck with both plugins and the profile
// of the suite (cargo xtask profile test/e2e/layout-<suite>.json, OpenDeck stopped meanwhile).
//
//   cargo xtask e2e main|new|more|extra [test name filter]
mod extra_suite;
mod main_suite;
mod more_suite;
mod new_suite;

use std::future::Future;
use std::rc::Rc;
use std::time::{Duration, Instant};

use obs_tools::connections::{local_config_paths, read_local_obs_config};
use obs_tools::obs::ObsClient;
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

/// Polls until the check gives something truthy.
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
			"main" => main_suite::run(&mut s, &obs, &deck).await?,
			"new" => new_suite::run(&mut s, &obs, &deck).await?,
			"more" => more_suite::run(&mut s, &obs, &deck).await?,
			"extra" => extra_suite::run(&mut s, &obs, &deck).await?,
			_ => return Err("usage: cargo xtask e2e main|new|more|extra [filter]".into()),
		}
		obs.client.stop();
		println!("\n{}/{} passed", s.count - s.failed, s.count);
		if s.failed > 0 { Err(format!("{} failed", s.failed)) } else { Ok(()) }
	})
}
