// Entry point. OpenDeck starts this with the usual Stream Deck arguments.
//
// openaction talks to OpenDeck. Its handlers only pass the events on to the plugin, which runs on
// this one thread: every event becomes a task of its own.
use std::sync::OnceLock;
use std::sync::atomic::Ordering::Relaxed;

use obs_tools::actions::Controller;
use obs_tools::i18n;
use obs_tools::plugin::{Event, EventKind, Plugin, dispatch};
use obs_tools::sd::{Sd, log};
use openaction::global_events::{DidReceiveGlobalSettingsEvent, GlobalEventHandler, SystemDidWakeUpEvent, set_global_event_handler};
use openaction::{Action, Instance, OpenActionResult, async_trait, register_action};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::task::{LocalSet, spawn_local};

const VERSION: &str = include_str!("../VERSION");

enum Inbound {
	Ready,
	GlobalSettings(Value),
	WakeUp,
	Event(Box<Event>),
}

static INBOUND: OnceLock<mpsc::UnboundedSender<Inbound>> = OnceLock::new();

fn send(message: Inbound) {
	if let Some(tx) = INBOUND.get() {
		let _ = tx.send(message);
	}
}

fn forward(kind: EventKind, instance: &Instance, settings: Option<&Value>) -> OpenActionResult<()> {
	send(Inbound::Event(Box::new(Event {
		kind,
		action: instance.action_uuid.clone(),
		context: instance.instance_id.clone(),
		controller: Some(Controller::from_name(&instance.controller)),
		is_in_multi_action: instance.is_in_multi_action,
		state: Some(instance.current_state_index.load(Relaxed)),
		settings: settings.cloned(),
	})));
	Ok(())
}

// One openaction action per action of the plugin. They all just pass the events on.
macro_rules! actions {
	($($name:ident => $uuid:literal),* $(,)?) => {
		$(
			struct $name;

			#[async_trait]
			impl Action for $name {
				const UUID: &'static str = $uuid;
				type Settings = Value;

				async fn will_appear(&self, i: &Instance, s: &Value) -> OpenActionResult<()> {
					forward(EventKind::WillAppear, i, Some(s))
				}

				async fn will_disappear(&self, i: &Instance, s: &Value) -> OpenActionResult<()> {
					forward(EventKind::WillDisappear, i, Some(s))
				}

				async fn key_down(&self, i: &Instance, s: &Value) -> OpenActionResult<()> {
					forward(EventKind::KeyDown, i, Some(s))
				}

				async fn key_up(&self, i: &Instance, s: &Value) -> OpenActionResult<()> {
					forward(EventKind::KeyUp, i, Some(s))
				}

				async fn dial_rotate(&self, i: &Instance, s: &Value, ticks: i16, pressed: bool) -> OpenActionResult<()> {
					forward(EventKind::DialRotate { ticks: f64::from(ticks), pressed }, i, Some(s))
				}

				async fn dial_down(&self, i: &Instance, s: &Value) -> OpenActionResult<()> {
					forward(EventKind::DialDown, i, Some(s))
				}

				async fn dial_up(&self, i: &Instance, s: &Value) -> OpenActionResult<()> {
					forward(EventKind::DialUp, i, Some(s))
				}

				async fn touch_tap(&self, i: &Instance, s: &Value, _position: (u16, u16), _hold: bool) -> OpenActionResult<()> {
					forward(EventKind::TouchTap, i, Some(s))
				}

				async fn did_receive_settings(&self, i: &Instance, s: &Value) -> OpenActionResult<()> {
					forward(EventKind::DidReceiveSettings, i, Some(s))
				}

				async fn property_inspector_did_appear(&self, i: &Instance, _s: &Value) -> OpenActionResult<()> {
					forward(EventKind::InspectorDidAppear, i, None)
				}

				async fn property_inspector_did_disappear(&self, i: &Instance, _s: &Value) -> OpenActionResult<()> {
					forward(EventKind::InspectorDidDisappear, i, None)
				}

				async fn send_to_plugin(&self, i: &Instance, _s: &Value, payload: &Value) -> OpenActionResult<()> {
					forward(EventKind::SendToPlugin(payload.clone()), i, None)
				}
			}
		)*

		async fn register_all() {
			$(register_action($name).await;)*
		}
	};
}

actions! {
	SmartScene => "de.felitendo.obs-tools.smartscene",
	SourceAnimation => "de.felitendo.obs-tools.sourceanimation",
	StudioMode => "de.felitendo.obs-tools.studiomode",
	VideoPlayer => "de.felitendo.obs-tools.videoplayer",
	ImageSource => "de.felitendo.obs-tools.imagesource",
	SourceVolume => "de.felitendo.obs-tools.sourcevolume",
	InputVolume => "de.felitendo.obs-tools.inputvolume",
	MuteToggle => "de.felitendo.obs-tools.mutetoggle",
	MonitorSet => "de.felitendo.obs-tools.monitorset",
	DroppedFrames => "de.felitendo.obs-tools.droppedframes",
	CpuUsage => "de.felitendo.obs-tools.cpuusage",
	PreviousScene => "de.felitendo.obs-tools.previousscene",
	NextScene => "de.felitendo.obs-tools.nextscene",
	InstantReplay => "de.felitendo.obs-tools.instantreplay",
	BrowserSource => "de.felitendo.obs-tools.browsersource",
	RecordToggle => "de.felitendo.obs-tools.recordtoggle",
	StreamToggle => "de.felitendo.obs-tools.streamtoggle",
	SetTransition => "de.felitendo.obs-tools.settransition",
	FilterToggle => "de.felitendo.obs-tools.filtertoggle",
	VirtualCam => "de.felitendo.obs-tools.virtualcam",
	FocusedWindow => "de.felitendo.obs-tools.focusedwindow",
	SourceVisibility => "de.felitendo.obs-tools.sourcevisibility",
	Hotkey => "de.felitendo.obs-tools.hotkey",
	Screenshot => "de.felitendo.obs-tools.screenshot",
	RecordTools => "de.felitendo.obs-tools.recordtools",
	MediaControl => "de.felitendo.obs-tools.mediacontrol",
	ProfileSwitch => "de.felitendo.obs-tools.profile",
	SetText => "de.felitendo.obs-tools.settext",
	StreamHealth => "de.felitendo.obs-tools.streamhealth",
	TransitionDial => "de.felitendo.obs-tools.transitiondial",
	SceneDial => "de.felitendo.obs-tools.scenedial",
	FilterDial => "de.felitendo.obs-tools.filterdial",
}

struct Globals;

#[async_trait]
impl GlobalEventHandler for Globals {
	async fn plugin_ready(&self) -> OpenActionResult<()> {
		send(Inbound::Ready);
		Ok(())
	}

	async fn did_receive_global_settings(&self, event: DidReceiveGlobalSettingsEvent) -> OpenActionResult<()> {
		send(Inbound::GlobalSettings(event.payload.settings));
		Ok(())
	}

	async fn system_did_wake_up(&self, _event: SystemDidWakeUpEvent) -> OpenActionResult<()> {
		send(Inbound::WakeUp);
		Ok(())
	}
}

static GLOBALS: Globals = Globals;

// openaction logs what it could not handle. Those go to the plugin log too.
struct Logger;

impl log::Log for Logger {
	fn enabled(&self, metadata: &log::Metadata) -> bool {
		metadata.level() <= log::Level::Warn
	}

	fn log(&self, record: &log::Record) {
		if self.enabled(record.metadata()) {
			log(&format!("{}: {}", record.level(), record.args()));
		}
	}

	fn flush(&self) {}
}

static LOGGER: Logger = Logger;

fn main() {
	let args: Vec<String> = std::env::args().collect();
	let flag = |name: &str| args.iter().position(|a| a.trim().to_lowercase() == name).and_then(|i| args.get(i + 1)).cloned();
	let info: Value = flag("-info").and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null);
	let uuid = flag("-pluginuuid").unwrap_or_default();
	let language = i18n::set_language(info.pointer("/application/language").and_then(Value::as_str).unwrap_or(""));
	let _ = log::set_logger(&LOGGER).map(|()| log::set_max_level(log::LevelFilter::Warn));
	std::panic::set_hook(Box::new(|panic| log(&format!("uncaught: {panic}"))));

	let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
		Ok(runtime) => runtime,
		Err(error) => {
			log(&format!("OBS Tools could not start: {error}"));
			std::process::exit(1);
		}
	};
	LocalSet::new().block_on(&runtime, async move {
		let (tx, mut rx) = mpsc::unbounded_channel();
		let _ = INBOUND.set(tx);
		let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
		let sd = Sd::new(out_tx, &uuid);
		let plugin = Plugin::new(sd.clone(), obs_tools::actions::all());

		let core_sd = sd.clone();
		spawn_local(async move {
			while let Some(message) = rx.recv().await {
				match message {
					Inbound::Ready => {
						// Messages sent before the connection was up wait in the queue, in order.
						let mut queue = std::mem::replace(&mut out_rx, mpsc::unbounded_channel().1);
						spawn_local(async move {
							while let Some(text) = queue.recv().await {
								let Ok(raw) = serde_json::value::RawValue::from_string(text) else { continue };
								if let Err(error) = openaction::send_arbitrary_json(raw).await {
									log(&format!("OpenDeck socket error: {error}"));
								}
							}
						});
						core_sd.log(&format!("OBS Tools started (version {}, language {language})", VERSION.trim()));
						core_sd.get_global_settings();
					}
					Inbound::GlobalSettings(settings) => plugin.on_global_settings(Some(&settings)),
					Inbound::WakeUp => plugin.on_wake_up(),
					Inbound::Event(event) => dispatch(&plugin, *event),
				}
			}
		});

		register_all().await;
		set_global_event_handler(&GLOBALS);
		match openaction::run(args).await {
			Ok(()) => sd.log("OpenDeck closed the connection, exiting"),
			Err(error) => sd.log(&format!("OpenDeck socket error: {error}")),
		}
		std::process::exit(0);
	});
}
