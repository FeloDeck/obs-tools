// Plays a file in an OBS media source, used by Video Player and Instant Replay.
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use serde_json::{Map, Value};
use tokio::sync::{oneshot, watch};
use tokio::task::spawn_local;

use crate::js::{self, truthy};
use crate::obs::ObsClient;
use crate::obs_state::ObsState;
use crate::timers::{Timer, set_interval};
use crate::{Error, Res};

const POLL_MS: f64 = 500.0;
const START_GRACE_MS: i64 = 1200;
const ENDED_STATES: &[&str] = &["OBS_MEDIA_STATE_ENDED", "OBS_MEDIA_STATE_STOPPED", "OBS_MEDIA_STATE_NONE", "OBS_MEDIA_STATE_ERROR"];

// Removes the listener when the wait is over or no longer wanted.
struct Listening {
	client: Rc<ObsClient>,
	id: u64,
}

impl Drop for Listening {
	fn drop(&mut self) {
		self.client.off_type(self.id);
	}
}

/// Waits for an event of this type. Listens from the moment of the call, so the event may come
/// while the caller still sends the request that causes it. The time counts from the call too.
pub fn wait_for_event(client: &Rc<ObsClient>, kind: &str, timeout_ms: f64, filter: impl Fn(&Value) -> bool + 'static) -> crate::obs::Reply {
	let (tx, rx) = oneshot::channel();
	let tx = RefCell::new(Some(tx));
	let id = client.on_type(kind, move |data| {
		if !filter(data) {
			return;
		}
		if let Some(tx) = tx.borrow_mut().take() {
			let _ = tx.send(data.clone());
		}
	});
	let listening = Listening { client: client.clone(), id };
	let deadline = tokio::time::Instant::now() + Duration::from_secs_f64(timeout_ms / 1000.0);
	let kind = kind.to_string();
	Box::pin(async move {
		let result = tokio::time::timeout_at(deadline, rx).await;
		drop(listening);
		match result {
			Ok(Ok(data)) => Ok(data),
			_ => Err(Error::from(format!("OBS did not send {kind} in time"))),
		}
	})
}

async fn find_item(client: &ObsClient, scene_name: &Value, source_name: &Value) -> Option<Value> {
	if !truthy(Some(scene_name)) {
		return None;
	}
	client
		.call("GetSceneItemId", crate::obj! { "sceneName" => scene_name.clone(), "sourceName" => source_name.clone() })
		.await
		.ok()
		.and_then(|r| r.get("sceneItemId").cloned())
}

pub struct PlayOptions {
	pub input: Value,
	pub file: Value,
	pub speed: Option<Value>,
	pub scene: Value,
	pub show_source: bool,
	pub hide_when_done: bool,
	pub return_to_scene: bool,
}

struct Running {
	client: Rc<ObsClient>,
	state: Rc<ObsState>,
	input: Value,
	scene: Value,
	target_scene: Value,
	previous_scene: Option<String>,
	item_id: Option<Value>,
	hide_when_done: bool,
	return_to_scene: bool,
	started_at: i64,
	finished: Cell<bool>,
	poll: RefCell<Option<Timer>>,
	listeners: RefCell<Option<(u64, u64)>>,
	done: watch::Sender<bool>,
}

/// A file that plays in a media source.
#[derive(Clone)]
pub struct Playback(Rc<Running>);

impl PartialEq for Playback {
	fn eq(&self, other: &Self) -> bool {
		Rc::ptr_eq(&self.0, &other.0)
	}
}

impl Playback {
	/// When the media ended (or was stopped) and the cleanup ran.
	pub async fn done(&self) {
		let mut rx = self.0.done.subscribe();
		let _ = rx.wait_for(|done| *done).await;
	}

	pub async fn stop(&self) {
		let r = &self.0;
		if r.finished.get() {
			return;
		}
		let _ = r
			.client
			.call(
				"TriggerMediaInputAction",
				crate::obj! { "inputName" => r.input.clone(), "mediaAction" => "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_STOP" },
			)
			.await;
		finish(self.0.clone()).await;
	}
}

async fn finish(r: Rc<Running>) {
	if r.finished.replace(true) {
		return;
	}
	if let Some(timer) = r.poll.borrow_mut().take() {
		timer.clear();
	}
	if let Some((ended, disconnected)) = r.listeners.borrow_mut().take() {
		r.client.off_type(ended);
		r.client.on_disconnected.off(disconnected);
	}
	let result: Res<()> = async {
		if r.hide_when_done
			&& let Some(item_id) = &r.item_id
		{
			r.client
				.call(
					"SetSceneItemEnabled",
					crate::obj! { "sceneName" => r.target_scene.clone(), "sceneItemId" => item_id.clone(), "sceneItemEnabled" => false },
				)
				.await?;
		}
		let scene = js::str_or_empty(Some(&r.scene));
		if r.return_to_scene
			&& truthy(Some(&r.scene))
			&& let Some(previous) = r.previous_scene.clone().filter(|p| !p.is_empty() && *p != scene)
			&& r.state.with(|d| d.program_scene.as_deref() == Some(scene.as_str()))
		{
			r.client.call("SetCurrentProgramScene", crate::obj! { "sceneName" => previous }).await?;
		}
		Ok(())
	}
	.await;
	let _ = result;
	let _ = r.done.send(true);
}

/// Starts playback. The returned Playback says when it is done and can stop it.
pub async fn play_media(client: &Rc<ObsClient>, state: &Rc<ObsState>, o: PlayOptions) -> Res<Playback> {
	let previous_scene = state.with(|d| d.program_scene.clone());
	let target_scene = if truthy(Some(&o.scene)) { o.scene.clone() } else { previous_scene.clone().map_or(Value::Null, Value::from) };

	// Load the file first, so the scene never shows the old one.
	let speed = js::min(200.0, js::max(1.0, js::round(js::number_or(o.speed.as_ref(), 100.0))));
	let mut settings = Map::new();
	settings.insert("speed_percent".into(), js::num(speed));
	settings.insert("looping".into(), false.into());
	settings.insert("restart_on_activate".into(), true.into());
	if truthy(Some(&o.file)) {
		settings.insert("is_local_file".into(), true.into());
		settings.insert("local_file".into(), o.file.clone());
	}
	client
		.call(
			"SetInputSettings",
			crate::obj! { "inputName" => o.input.clone(), "inputSettings" => Value::Object(settings), "overlay" => true },
		)
		.await?;

	let item_id = if o.show_source || o.hide_when_done { find_item(client, &target_scene, &o.input).await } else { None };
	if o.show_source
		&& let Some(item_id) = &item_id
	{
		client
			.call(
				"SetSceneItemEnabled",
				crate::obj! { "sceneName" => target_scene.clone(), "sceneItemId" => item_id.clone(), "sceneItemEnabled" => true },
			)
			.await?;
	}
	client
		.call(
			"TriggerMediaInputAction",
			crate::obj! { "inputName" => o.input.clone(), "mediaAction" => "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_RESTART" },
		)
		.await?;
	let scene = js::str_or_empty(Some(&o.scene));
	if truthy(Some(&o.scene)) && state.with(|d| d.program_scene.as_deref() != Some(scene.as_str())) {
		client.call("SetCurrentProgramScene", crate::obj! { "sceneName" => o.scene.clone() }).await?;
	}

	let (done, _) = watch::channel(false);
	let running = Rc::new(Running {
		client: client.clone(),
		state: state.clone(),
		input: o.input.clone(),
		scene: o.scene.clone(),
		target_scene,
		previous_scene,
		item_id,
		hide_when_done: o.hide_when_done,
		return_to_scene: o.return_to_scene,
		started_at: js::now(),
		finished: Cell::new(false),
		poll: RefCell::new(None),
		listeners: RefCell::new(None),
		done,
	});

	// The listeners and the poll keep the playback alive until it finished, then they go away.
	let r = running.clone();
	let input = o.input.clone();
	let ended = client.on_type("MediaInputPlaybackEnded", move |data| {
		if data.get("inputName") == Some(&input) && js::now() - r.started_at > START_GRACE_MS / 2 {
			spawn_local(finish(r.clone()));
		}
	});
	let r = running.clone();
	let disconnected = client.on_disconnected.on(move |_| {
		spawn_local(finish(r.clone()));
	});
	*running.listeners.borrow_mut() = Some((ended, disconnected));

	// The ended event is not sent in every case, so the status is checked too.
	let r = running.clone();
	let poll = set_interval(POLL_MS, move || {
		let r = r.clone();
		if r.finished.get() || js::now() - r.started_at < START_GRACE_MS {
			return;
		}
		let reply = r.client.call("GetMediaInputStatus", crate::obj! { "inputName" => r.input.clone() });
		spawn_local(async move {
			match reply.await {
				Ok(status) => {
					if ENDED_STATES.contains(&status.get("mediaState").and_then(Value::as_str).unwrap_or("")) {
						finish(r).await;
					}
				}
				Err(_) => finish(r).await,
			}
		});
	});
	*running.poll.borrow_mut() = Some(poll);
	Ok(Playback(running))
}
