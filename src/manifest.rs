// The actions as OpenDeck lists them. `cargo xtask manifest` writes manifest.json and de.json from
// this table, and the plugin takes the icons of its keys from it. OpenDeck lists the actions in this
// order: the ones people use most come first.
pub const ID: &str = "de.felitendo.obs-tools";

pub struct Dial {
	pub icon: &'static str,
	pub layout: &'static str,
	pub rotate: &'static str,
	pub push: &'static str,
	pub touch: &'static str,
}

pub struct ActionDef {
	/// After "de.felitendo.obs-tools."
	pub id: &'static str,
	pub name: &'static str,
	/// Glyph of the icon in the action list (images/actions/<icon>.svg).
	pub icon: &'static str,
	pub tooltip: &'static str,
	/// Key images, images/keys/<name>.svg. Dials have one image, images/encoder/<icon>-key.svg.
	pub states: &'static [&'static str],
	pub dial: Option<Dial>,
	/// What OpenDeck shows when its language is German.
	pub de_name: &'static str,
	pub de_tooltip: &'static str,
}

const fn key(
	id: &'static str,
	name: &'static str,
	icon: &'static str,
	tooltip: &'static str,
	states: &'static [&'static str],
	de: [&'static str; 2],
) -> ActionDef {
	ActionDef { id, name, icon, tooltip, states, dial: None, de_name: de[0], de_tooltip: de[1] }
}

const fn dial(
	id: &'static str,
	name: &'static str,
	icon: &'static str,
	tooltip: &'static str,
	layout: &'static str,
	trigger: [&'static str; 3],
	de: [&'static str; 2],
) -> ActionDef {
	ActionDef {
		id,
		name,
		icon,
		tooltip,
		states: &[],
		dial: Some(Dial { icon, layout, rotate: trigger[0], push: trigger[1], touch: trigger[2] }),
		de_name: de[0],
		de_tooltip: de[1],
	}
}

pub static ACTIONS: &[ActionDef] = &[
	key(
		"recordtoggle",
		"Recording Toggle",
		"record",
		"Start, stop or pause recording, on this PC or on another OBS in your network.",
		&["record", "record-on"],
		["Aufnahme an/aus", "Aufnahme starten, stoppen oder pausieren, auf diesem PC oder in einem anderen OBS im Netzwerk."],
	),
	key(
		"streamtoggle",
		"Streaming Toggle",
		"stream",
		"Start or stop streaming, on this PC or on another OBS in your network.",
		&["stream", "stream-on"],
		["Stream an/aus", "Stream starten oder stoppen, auf diesem PC oder in einem anderen OBS im Netzwerk."],
	),
	key(
		"smartscene",
		"Smart Scene Switcher",
		"scene",
		"Switch scenes. In studio mode: first press to preview, second press to go live. Shows a live picture with a red (live) or green (preview) frame.",
		&["scene"],
		[
			"Smarter Szenenwechsel",
			"Szene wechseln. Im Studiomodus: erster Druck in die Vorschau, zweiter live. Zeigt ein Live-Bild mit rotem (live) oder grünem (Vorschau) Rahmen.",
		],
	),
	key(
		"mutetoggle",
		"Source Mute Toggle",
		"mute",
		"Mute or unmute an audio source.",
		&["unmuted", "muted"],
		["Quelle stummschalten", "Eine Audioquelle stumm- oder lautschalten."],
	),
	key(
		"sourcevisibility",
		"Source Visibility",
		"eye",
		"Show or hide a source in a scene.",
		&["visibility", "visibility-on"],
		["Quelle ein-/ausblenden", "Eine Quelle in einer Szene ein- oder ausblenden."],
	),
	key(
		"instantreplay",
		"Instant Replay",
		"replay",
		"Press to save the replay buffer, play it on stream and clip it on Twitch. Hold to turn the replay buffer on or off.",
		&["replay", "replay-on"],
		[
			"Instant Replay",
			"Drücken speichert den Wiederholungspuffer, spielt ihn im Stream ab und erstellt einen Twitch-Clip. Halten schaltet den Puffer an oder aus.",
		],
	),
	key(
		"sourcevolume",
		"Source Volume",
		"volume",
		"Turn the volume of an audio source up, down or to a set value, with a live level meter. Hold the key to keep going.",
		&["volume"],
		[
			"Quellen-Lautstärke",
			"Lautstärke einer Audioquelle lauter, leiser oder auf einen Wert stellen, mit Live-Pegelanzeige. Halten macht weiter.",
		],
	),
	dial(
		"inputvolume",
		"Volume (Dial)",
		"volume",
		"Turn the dial to change the volume of an audio source, with a live level meter. Press or touch to mute.",
		"layouts/volume-meter.json",
		["Volume", "Mute", "Mute"],
		[
			"Lautstärke (Drehregler)",
			"Drehen ändert die Lautstärke einer Audioquelle, mit Live-Pegelanzeige. Drücken oder Antippen schaltet stumm.",
		],
	),
	dial(
		"scenedial",
		"Scene (Dial)",
		"scene",
		"Turn the dial to pick a scene (with a picture of it), press to switch.",
		"layouts/scene-dial.json",
		["Pick a scene", "Switch", "Switch"],
		["Szene (Drehregler)", "Drehen wählt eine Szene (mit Bild), Drücken wechselt."],
	),
	key(
		"studiomode",
		"Studio Mode Toggle",
		"studio",
		"Turn studio mode on or off.",
		&["studio", "studio-on"],
		["Studiomodus an/aus", "Studiomodus ein- oder ausschalten."],
	),
	key(
		"previousscene",
		"Last Scene",
		"back",
		"Go back to the scene that was live before. Shows its name.",
		&["back"],
		["Letzte Szene", "Zurück zur Szene, die davor live war. Zeigt ihren Namen."],
	),
	key(
		"nextscene",
		"Next Scene",
		"next",
		"Go to the next scene in the scene list, or the one above.",
		&["next"],
		["Nächste Szene", "Zur nächsten Szene in der Szenenliste oder zur Szene darüber."],
	),
	key(
		"mediacontrol",
		"Media Control",
		"playPause",
		"Play, pause, stop, restart or skip in a media source. Shows the time left or played.",
		&["media"],
		[
			"Mediensteuerung",
			"Abspielen, Pause, Stopp, Neustart oder Springen in einer Medienquelle. Zeigt die Rest- oder die gespielte Zeit.",
		],
	),
	key(
		"videoplayer",
		"Video Player",
		"video",
		"Play a file in a media source. Many keys can share one source, each with its own file and speed.",
		&["video", "video-on"],
		[
			"Videoplayer",
			"Eine Datei in einer Medienquelle abspielen. Mehrere Tasten können sich eine Quelle teilen, jede mit eigener Datei und eigenem Tempo.",
		],
	),
	key(
		"imagesource",
		"Image Source",
		"image",
		"Show a picture in an image source. Many keys can share one source, each with its own picture. Can hide it again after some seconds.",
		&["image", "image-on"],
		[
			"Bildquelle",
			"Ein Bild in einer Bildquelle zeigen. Mehrere Tasten können sich eine Quelle teilen, jede mit eigenem Bild. Kann es nach ein paar Sekunden wieder ausblenden.",
		],
	),
	key(
		"settext",
		"Set Text",
		"text",
		"Change the text of a text source: a fixed text, the next line of a list, or a counter that counts up or down.",
		&["text"],
		[
			"Text setzen",
			"Text einer Textquelle ändern: fester Text, nächste Zeile einer Liste oder ein Zähler, der hoch- oder runterzählt.",
		],
	),
	key(
		"filtertoggle",
		"Filter Toggle",
		"filter",
		"Turn a filter on a source on or off.",
		&["filter", "filter-on"],
		["Filter an/aus", "Einen Filter einer Quelle ein- oder ausschalten."],
	),
	key(
		"streamhealth",
		"Stream Health",
		"health",
		"Bitrate, dropped frames and a green, yellow or red light for your stream.",
		&["health"],
		["Stream-Zustand", "Bitrate, ausgelassene Frames und ein grünes, gelbes oder rotes Licht für deinen Stream."],
	),
	key(
		"virtualcam",
		"Virtual Camera Toggle",
		"virtualcam",
		"Start or stop the OBS virtual camera.",
		&["virtualcam", "virtualcam-on"],
		["Virtuelle Kamera an/aus", "Die virtuelle Kamera von OBS starten oder stoppen."],
	),
	key(
		"recordtools",
		"Recording Marker",
		"chapter",
		"While recording: add a chapter marker or start a new file.",
		&["chapter"],
		["Aufnahme-Marker", "Während der Aufnahme: Kapitelmarke setzen oder eine neue Datei beginnen."],
	),
	key(
		"screenshot",
		"Screenshot",
		"camera",
		"Save a picture of the live scene, or of any scene or source.",
		&["screenshot"],
		["Screenshot", "Ein Bild der Live-Szene speichern, oder von einer beliebigen Szene oder Quelle."],
	),
	key(
		"hotkey",
		"OBS Hotkey",
		"keyboard",
		"Press any OBS hotkey, by its name or by the keys it is set to in OBS.",
		&["hotkey"],
		["OBS-Hotkey", "Einen OBS-Hotkey auslösen, über seinen Namen oder die Tasten, die in OBS dafür eingestellt sind."],
	),
	key(
		"settransition",
		"Set Transition",
		"transition",
		"Set the scene transition and its duration.",
		&["transition", "transition-on"],
		["Übergang festlegen", "Szenenübergang und seine Dauer festlegen."],
	),
	dial(
		"transitiondial",
		"Transition (Dial)",
		"transition",
		"Turn the dial to change the transition time, press for the next transition.",
		"layouts/dial.json",
		["Transition time", "Next transition", "Next transition"],
		["Übergang (Drehregler)", "Drehen ändert die Übergangsdauer, Drücken nimmt den nächsten Übergang."],
	),
	dial(
		"filterdial",
		"Filter (Dial)",
		"filter",
		"Turn the dial to change a number in a filter, like gain, opacity or blur. Press to turn the filter on or off.",
		"layouts/dial.json",
		["Change value", "Filter on or off", "Filter on or off"],
		[
			"Filter (Drehregler)",
			"Drehen ändert eine Zahl in einem Filter, etwa Verstärkung, Deckkraft oder Unschärfe. Drücken schaltet den Filter an oder aus.",
		],
	),
	key(
		"sourceanimation",
		"Source Animation",
		"animation",
		"Move, scale, rotate or crop a source in phases. Record the change in OBS and the plugin fills in the numbers.",
		&["animation", "animation-on"],
		[
			"Quellen-Animation",
			"Eine Quelle in Phasen verschieben, skalieren, drehen oder zuschneiden. Die Änderung in OBS aufnehmen, das Plugin trägt die Zahlen ein.",
		],
	),
	key(
		"browsersource",
		"Browser Source",
		"browser",
		"Refresh a browser source, set its URL, cycle through URLs or show and hide it.",
		&["browser"],
		["Browserquelle", "Eine Browserquelle neu laden, ihre URL setzen, URLs durchschalten oder sie ein- und ausblenden."],
	),
	key(
		"droppedframes",
		"Dropped Frames Alarm",
		"dropped",
		"Shows dropped, skipped or missed frames and flashes when the number goes up. Press to stop the alarm.",
		&["dropped"],
		[
			"Frame-Verlust-Alarm",
			"Zeigt ausgelassene, übersprungene oder verpasste Frames und blinkt, wenn die Zahl steigt. Drücken beendet den Alarm.",
		],
	),
	key("cpuusage", "OBS CPU Usage", "cpu", "Shows how much CPU OBS uses.", &["cpu"], ["OBS-CPU-Last", "Zeigt, wie viel CPU OBS braucht."]),
	key(
		"monitorset",
		"Source Monitor Set",
		"monitor",
		"Set how an audio source is monitored: off, monitor only, or monitor and output.",
		&["monitor", "monitor-on"],
		["Audioüberwachung festlegen", "Festlegen, wie eine Audioquelle überwacht wird: aus, nur überwachen oder überwachen und ausgeben."],
	),
	key(
		"profile",
		"Profile / Scene Collection",
		"collection",
		"Switch to another OBS profile or scene collection.",
		&["profile"],
		["Profil / Szenensammlung", "Zu einem anderen OBS-Profil oder einer anderen Szenensammlung wechseln."],
	),
	key(
		"focusedwindow",
		"Focused Window Capture",
		"window",
		"Point a Window Capture (Xcomposite) source at the window that has focus. Needs OBS on X11.",
		&["window"],
		["Aktives Fenster aufnehmen", "Eine Fensteraufnahme (Xcomposite) auf das Fenster mit dem Fokus richten. Braucht OBS unter X11."],
	),
];

/// Icon and state images of an action, by its full UUID.
pub fn action_def(uuid: &str) -> Option<&'static ActionDef> {
	let id = uuid.strip_prefix(ID)?.strip_prefix('.')?;
	ACTIONS.iter().find(|a| a.id == id)
}

impl ActionDef {
	/// The names of the state images, like the plugin read them from manifest.json.
	pub fn state_images(&self) -> Vec<String> {
		match &self.dial {
			Some(dial) => vec![format!("{}-key", dial.icon)],
			None => self.states.iter().map(|s| s.to_string()).collect(),
		}
	}
}
