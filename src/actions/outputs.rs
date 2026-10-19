// Outputs: recording, streaming, the virtual camera and the replay buffer.
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Behavior, KeyRc, Kind, Press, Stop, defaults};
use crate::i18n::t;
use crate::icons::colors;
use crate::js::{self, truthy};
use crate::media::{PlayOptions, Playback, play_media, wait_for_event};
use crate::obs_state::Output;
use crate::render::{GlyphKey, glyph_key};
use crate::settings::or;
use crate::time::{format_duration, timecode_seconds};
use crate::twitch::{ChatMessage, is_allowed};
use crate::{Error, Res};

// ----- Start, stop or toggle an output. Can point at another OBS on the network. -----

struct OutputConfig {
	output: fn(&crate::obs_state::Data) -> Output,
	glyph: &'static str,
	status_request: &'static str,
	start: &'static str,
	stop: &'static str,
	toggle: &'static str,
}

static RECORD: OutputConfig = OutputConfig {
	output: |d| d.record,
	glyph: "record",
	status_request: "GetRecordStatus",
	start: "StartRecord",
	stop: "StopRecord",
	toggle: "ToggleRecord",
};
static STREAM: OutputConfig = OutputConfig {
	output: |d| d.stream,
	glyph: "stream",
	status_request: "GetStreamStatus",
	start: "StartStream",
	stop: "StopStream",
	toggle: "ToggleStream",
};

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct OutputSettings {
	show_timer: bool,
	timer_at: String,
}

#[derive(Default)]
struct OutputState {
	time: String,
	stop_timer: Option<Stop>,
}

struct OutputToggle(&'static OutputConfig);

fn out(k: &KeyRc) -> std::cell::RefMut<'_, OutputState> {
	k.own::<OutputState>()
}

impl OutputToggle {
	fn active(&self, k: &KeyRc) -> bool {
		(self.0.output)(&k.data()).active
	}

	async fn tick(&self, k: &KeyRc) -> Res<()> {
		if !k.visible() || !k.settings::<OutputSettings>().show_timer || !self.active(k) {
			return Ok(());
		}
		let status = k.call(self.0.status_request, json!({})).await?;
		out(k).time = if truthy(status.get("outputPaused")) {
			t("Paused")
		} else {
			format_duration(timecode_seconds(&js::str_or_empty(status.get("outputTimecode"))))
		};
		if self.active(k) {
			self.render(k);
		}
		Ok(())
	}

	async fn toggle_press(&self, k: &KeyRc, mode: &str) -> Res<()> {
		let c = self.0;
		if mode == "start" {
			if !self.active(k) {
				k.call(c.start, json!({})).await?;
			}
		} else if mode == "stop" {
			if self.active(k) {
				k.call(c.stop, json!({})).await?;
			}
		} else {
			k.call(c.toggle, json!({})).await?;
		}
		Ok(())
	}
}

#[async_trait(?Send)]
impl Behavior for OutputToggle {
	async fn on_connected(&self, k: &KeyRc) -> Res<()> {
		let old = out(k).stop_timer.take();
		if let Some(stop) = old {
			stop();
		}
		let config = self.0;
		let stop = k.interval(1000.0, move |k| Box::pin(async move { OutputToggle(config).tick(&k).await }));
		out(k).stop_timer = Some(stop);
		Ok(())
	}

	fn on_disconnected(&self, k: &KeyRc) {
		let stop = out(k).stop_timer.clone();
		if let Some(stop) = stop {
			stop();
		}
		out(k).time = String::new();
	}

	// The time goes above or below the glyph, so the key draws it into the image.
	fn render(&self, k: &KeyRc) {
		let s: OutputSettings = k.settings();
		let active = self.active(k);
		k.set_state(if active { 1 } else { 0 });
		if !active {
			out(k).time = String::new();
		}
		if active && s.show_timer {
			let time = out(k).time.clone();
			k.own_title(&time);
			k.set_image(&glyph_key(
				self.0.glyph,
				&GlyphKey { color: colors::RED, title: &time, title_at: &s.timer_at, ..Default::default() },
			));
		} else {
			k.reset_image(false);
		}
	}

	async fn on_press(&self, k: &KeyRc, press: Press) -> Res<()> {
		if std::ptr::eq(self.0, &RECORD) && press.op == "pause" {
			if self.active(k) {
				k.call("ToggleRecordPause", json!({})).await?;
				return Ok(());
			}
			return Err("not recording".into());
		}
		self.toggle_press(k, &press.op).await
	}
}

fn output_defaults() -> serde_json::Map<String, Value> {
	defaults(
		json!({ "mode": "toggle", "hold": "none", "double": "none", "showTimer": true, "timerAt": "below", "customConnection": false, "host": "", "port": 4455, "password": "" }),
	)
}

pub static RECORD_TOGGLE: Kind = Kind {
	uuid: "de.felitendo.obs-tools.recordtoggle",
	name: "RecordToggle",
	defaults: output_defaults,
	press_key: "mode",
	ops: &["toggle", "start", "stop", "pause"],
	behavior: &OutputToggle(&RECORD),
	new_state: || Box::new(OutputState::default()),
	..Kind::BASE
};

pub static STREAM_TOGGLE: Kind = Kind {
	uuid: "de.felitendo.obs-tools.streamtoggle",
	name: "StreamToggle",
	defaults: output_defaults,
	press_key: "mode",
	ops: &["toggle", "start", "stop"],
	behavior: &OutputToggle(&STREAM),
	new_state: || Box::new(OutputState::default()),
	..Kind::BASE
};

// ----- Virtual Camera -----

struct VirtualCam;

#[async_trait(?Send)]
impl Behavior for VirtualCam {
	fn render(&self, k: &KeyRc) {
		k.set_state(if k.data().virtualcam.active { 1 } else { 0 });
	}

	async fn on_press(&self, k: &KeyRc, press: Press) -> Res<()> {
		let active = k.data().virtualcam.active;
		if press.op == "start" || (press.op == "toggle" && !active) {
			if active {
				return Ok(());
			}
			// Without v4l2loopback, OBS asks for an admin password to load it. Give the user time.
			let loaded = std::path::Path::new("/sys/module/v4l2loopback").exists();
			if !loaded {
				k.log("v4l2loopback is not loaded, OBS will ask for a password to load it");
			}
			let Some(client) = k.obs() else { return Err("no connection".into()) };
			let started =
				wait_for_event(&client, "VirtualcamStateChanged", if loaded { 5000.0 } else { 60000.0 }, |d| truthy(d.get("outputActive")));
			k.call("StartVirtualCam", json!({})).await?;
			if started.await.is_err() {
				return Err(if loaded {
					"the virtual camera did not start"
				} else {
					"the virtual camera did not start: the v4l2loopback kernel module is not loaded"
				}
				.into());
			}
		} else if active {
			k.call("StopVirtualCam", json!({})).await?;
		}
		Ok(())
	}
}

pub static VIRTUAL_CAM: Kind = Kind {
	uuid: "de.felitendo.obs-tools.virtualcam",
	name: "VirtualCam",
	defaults: || defaults(json!({ "mode": "toggle", "hold": "none", "double": "none" })),
	press_key: "mode",
	ops: &["toggle", "start", "stop"],
	behavior: &VirtualCam,
	..Kind::BASE
};

// ----- Instant Replay: save the replay buffer (and play it, clip it), or turn it on or off. -----

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct ReplaySettings {
	/// Play the replay right away, in media_input.
	auto_replay: bool,
	media_input: String,
	/// Empty: the live scene, and no scene switch.
	replay_scene: String,
	/// In percent.
	speed: f64,
	hide_when_done: bool,
	return_to_scene: bool,
	create_clip: bool,
	post_clip: bool,
	/// "{url}" becomes the link to the clip.
	clip_message: String,
	chat_command: bool,
	command: String,
	/// Seconds between two chat replays.
	cooldown: f64,
	/// Who may use the chat command.
	permission: String,
}

#[derive(Default)]
struct ReplayState {
	busy: bool,
	last_chat: i64,
	playback: Option<Playback>,
}

struct InstantReplay;

fn replay_state(k: &KeyRc) -> std::cell::RefMut<'_, ReplayState> {
	k.own::<ReplayState>()
}

async fn replay(k: &KeyRc) -> Res<()> {
	if replay_state(k).busy {
		return Ok(());
	}
	replay_state(k).busy = true;
	let result = replay_inner(k).await;
	replay_state(k).busy = false;
	result
}

async fn replay_inner(k: &KeyRc) -> Res<()> {
	let s: ReplaySettings = k.settings();
	let buffer_on = k.data().replay.active;
	if !buffer_on && !s.create_clip {
		return Err("the replay buffer is off".into());
	}
	// The clip is made while the replay plays. Its error comes below, where it is awaited.
	let clip = if s.create_clip {
		let k = k.clone();
		Some(tokio::task::spawn_local(async move { clip(&k).await }))
	} else {
		None
	};
	if buffer_on {
		let Some(client) = k.obs() else { return Err("no connection".into()) };
		let saved = wait_for_event(&client, "ReplayBufferSaved", 20000.0, |_| true);
		k.call("SaveReplayBuffer", json!({})).await?;
		let path = match saved.await {
			Ok(data) => data.get("savedReplayPath").cloned(),
			Err(_) => k.call("GetLastReplayBufferReplay", json!({})).await?.get("savedReplayPath").cloned(),
		};
		k.log(&format!("replay saved: {}", js::to_string(path.as_ref())));
		if let Some(path) = path.as_ref().and_then(Value::as_str).filter(|p| !p.is_empty())
			&& s.auto_replay
			&& !s.media_input.is_empty()
		{
			play(k, &s, path).await?;
		}
	}
	if let Some(clip) = clip {
		clip.await.map_err(|e| Error::from(e.to_string()))??;
	}
	k.show_ok();
	Ok(())
}

async fn play(k: &KeyRc, s: &ReplaySettings, file: &str) -> Res<()> {
	let old = replay_state(k).playback.clone();
	if let Some(old) = old {
		old.stop().await;
	}
	let (Some(client), Some(state)) = (k.obs(), k.obs_state()) else { return Err("no connection".into()) };
	let playback = play_media(
		&client,
		&state,
		PlayOptions {
			input: s.media_input.clone(),
			file: file.to_string(),
			speed: s.speed,
			scene: s.replay_scene.clone(),
			show_source: true,
			hide_when_done: s.hide_when_done,
			return_to_scene: s.return_to_scene,
		},
	)
	.await?;
	replay_state(k).playback = Some(playback.clone());
	let k = k.clone();
	tokio::task::spawn_local(async move {
		playback.done().await;
		let mut s = replay_state(&k);
		if s.playback.as_ref() == Some(&playback) {
			s.playback = None;
		}
	});
	Ok(())
}

async fn clip(k: &KeyRc) -> Res<()> {
	let Some(plugin) = k.plugin() else { return Ok(()) };
	let twitch = plugin.twitch.clone();
	let url = twitch.create_clip().await?;
	k.log(&format!("clip created: {url}"));
	let s: ReplaySettings = k.settings();
	if s.post_clip {
		let template = if s.clip_message.is_empty() { "{url}" } else { &s.clip_message };
		let message = template.replace("{url}", &url);
		let message = if message.contains(&url) { message } else { format!("{message} {url}") };
		twitch.say(&message).await?;
	}
	Ok(())
}

#[async_trait(?Send)]
impl Behavior for InstantReplay {
	// Auto replay needs its media source. Without auto replay the key just saves.
	fn is_set_up(&self, k: &KeyRc) -> bool {
		let s: ReplaySettings = k.settings();
		!s.auto_replay || !s.media_input.is_empty()
	}

	fn render(&self, k: &KeyRc) {
		k.set_state(if k.data().replay.active { 1 } else { 0 });
	}

	fn wants_chat(&self, k: &KeyRc) -> bool {
		k.visible() && k.settings::<ReplaySettings>().chat_command
	}

	fn on_chat_message(&self, k: &KeyRc, message: &ChatMessage) {
		let s: ReplaySettings = k.settings();
		if !s.chat_command || !k.online() {
			return;
		}
		let text = js::trim(&message.text);
		let word = text.split(js::is_js_space).next().unwrap_or("").to_lowercase();
		let command = if s.command.is_empty() { "!replay" } else { &s.command };
		if word != js::trim(command).to_lowercase() {
			return;
		}
		if !is_allowed(&message.user, &s.permission) {
			return;
		}
		let now = js::now();
		let last = replay_state(k).last_chat;
		if last != 0 && ((now - last) as f64) < or(s.cooldown, 0.0) * 1000.0 {
			return;
		}
		replay_state(k).last_chat = now;
		k.log(&format!("chat replay by {}", message.user.login));
		let k = k.clone();
		tokio::task::spawn_local(async move {
			k.safe(replay(&k)).await;
		});
	}

	async fn on_press(&self, k: &KeyRc, press: Press) -> Res<()> {
		let on = k.data().replay.active;
		match press.op.as_str() {
			"buffer" => {
				k.call("ToggleReplayBuffer", json!({})).await?;
			}
			"bufferStart" => {
				if !on {
					k.call("StartReplayBuffer", json!({})).await?;
				}
			}
			"bufferStop" => {
				if on {
					k.call("StopReplayBuffer", json!({})).await?;
				}
			}
			_ => replay(k).await?,
		}
		Ok(())
	}
}

pub static INSTANT_REPLAY: Kind = Kind {
	uuid: "de.felitendo.obs-tools.instantreplay",
	name: "InstantReplay",
	defaults: || {
		defaults(json!({
			"press": "save",
			"hold": "buffer",
			"double": "none",
			"autoReplay": false,
			"mediaInput": "",
			"replayScene": "",
			"speed": 100,
			"hideWhenDone": true,
			"returnToScene": true,
			"createClip": false,
			"postClip": false,
			"clipMessage": "Instant replay: {url}",
			"chatCommand": false,
			"command": "!replay",
			"cooldown": 30,
			"permission": "everyone",
		}))
	},
	press_key: "press",
	ops: &["save", "buffer", "bufferStart", "bufferStop"],
	behavior: &InstantReplay,
	new_state: || Box::new(ReplayState::default()),
	..Kind::BASE
};

#[cfg(test)]
mod tests {
	use super::*;
	use crate::settings::testing::assert_defaults;

	#[test]
	fn every_setting_has_a_default() {
		assert_defaults::<OutputSettings>(&[&RECORD_TOGGLE]);
		assert_defaults::<OutputSettings>(&[&STREAM_TOGGLE]);
		assert_defaults::<ReplaySettings>(&[&INSTANT_REPLAY]);
	}
}
