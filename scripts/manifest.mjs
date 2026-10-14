// Builds manifest.json and the translated names (de.json). Edit the tables below, then run: node scripts/manifest.mjs
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), "..");
const version = fs.readFileSync(path.join(root, "VERSION"), "utf8").trim();
const ID = "de.felitendo.obs-tools";

const state = (image, extra = {}) => ({ Image: `images/keys/${image}`, TitleAlignment: "bottom", FontSize: 11, ...extra });

const dial = (icon, layout, trigger) => ({
	Controllers: ["Encoder"],
	Encoder: { layout, Icon: `images/encoder/${icon}`, TriggerDescription: trigger },
});

// [uuid, name, icon, tooltip, states, extra]. OpenDeck lists the actions in this order:
// the ones people use most come first.
const actions = [
	["recordtoggle", "Recording Toggle", "record", "Start, stop or pause recording, on this PC or on another OBS in your network.", [state("record"), state("record-on")]],
	["streamtoggle", "Streaming Toggle", "stream", "Start or stop streaming, on this PC or on another OBS in your network.", [state("stream"), state("stream-on")]],
	["smartscene", "Smart Scene Switcher", "scene", "Switch scenes. In studio mode: first press to preview, second press to go live. Shows a live picture with a red (live) or green (preview) frame.", [state("scene")]],
	["mutetoggle", "Source Mute Toggle", "mute", "Mute or unmute an audio source.", [state("unmuted"), state("muted")]],
	["sourcevisibility", "Source Visibility", "eye", "Show or hide a source in a scene.", [state("visibility"), state("visibility-on")]],
	["instantreplay", "Instant Replay", "replay", "Press to save the replay buffer, play it on stream and clip it on Twitch. Hold to turn the replay buffer on or off.", [state("replay"), state("replay-on")]],
	["sourcevolume", "Source Volume", "volume", "Turn the volume of an audio source up, down or to a set value, with a live level meter. Hold the key to keep going.", [state("volume")]],
	["inputvolume", "Volume (Dial)", "volume", "Turn the dial to change the volume of an audio source, with a live level meter. Press or touch to mute.", [{ Image: "images/encoder/volume-key" }], dial("volume", "layouts/volume-meter.json", { Rotate: "Volume", Push: "Mute", Touch: "Mute" })],
	["scenedial", "Scene (Dial)", "scene", "Turn the dial to pick a scene (with a picture of it), press to switch.", [{ Image: "images/encoder/scene-key" }], dial("scene", "layouts/scene-dial.json", { Rotate: "Pick a scene", Push: "Switch", Touch: "Switch" })],
	["studiomode", "Studio Mode Toggle", "studio", "Turn studio mode on or off.", [state("studio"), state("studio-on")]],
	["previousscene", "Last Scene", "back", "Go back to the scene that was live before. Shows its name.", [state("back")]],
	["nextscene", "Next Scene", "next", "Go to the next scene in the scene list, or the one above.", [state("next")]],
	["mediacontrol", "Media Control", "playPause", "Play, pause, stop, restart or skip in a media source. Shows the time left or played.", [state("media")]],
	["videoplayer", "Video Player", "video", "Play a file in a media source. Many keys can share one source, each with its own file and speed.", [state("video"), state("video-on")]],
	["imagesource", "Image Source", "image", "Show a picture in an image source. Many keys can share one source, each with its own picture. Can hide it again after some seconds.", [state("image"), state("image-on")]],
	["settext", "Set Text", "text", "Change the text of a text source: a fixed text, the next line of a list, or a counter that counts up or down.", [state("text")]],
	["filtertoggle", "Filter Toggle", "filter", "Turn a filter on a source on or off.", [state("filter"), state("filter-on")]],
	["streamhealth", "Stream Health", "health", "Bitrate, dropped frames and a green, yellow or red light for your stream.", [state("health")]],
	["virtualcam", "Virtual Camera Toggle", "virtualcam", "Start or stop the OBS virtual camera.", [state("virtualcam"), state("virtualcam-on")]],
	["recordtools", "Recording Marker", "chapter", "While recording: add a chapter marker or start a new file.", [state("chapter")]],
	["screenshot", "Screenshot", "camera", "Save a picture of the live scene, or of any scene or source.", [state("screenshot")]],
	["hotkey", "OBS Hotkey", "keyboard", "Press any OBS hotkey, by its name or by the keys it is set to in OBS.", [state("hotkey")]],
	["settransition", "Set Transition", "transition", "Set the scene transition and its duration.", [state("transition"), state("transition-on")]],
	["transitiondial", "Transition (Dial)", "transition", "Turn the dial to change the transition time, press for the next transition.", [{ Image: "images/encoder/transition-key" }], dial("transition", "layouts/dial.json", { Rotate: "Transition time", Push: "Next transition", Touch: "Next transition" })],
	["filterdial", "Filter (Dial)", "filter", "Turn the dial to change a number in a filter, like gain, opacity or blur. Press to turn the filter on or off.", [{ Image: "images/encoder/filter-key" }], dial("filter", "layouts/dial.json", { Rotate: "Change value", Push: "Filter on or off", Touch: "Filter on or off" })],
	["sourceanimation", "Source Animation", "animation", "Move, scale, rotate or crop a source in phases. Record the change in OBS and the plugin fills in the numbers.", [state("animation"), state("animation-on")]],
	["browsersource", "Browser Source", "browser", "Refresh a browser source, set its URL, cycle through URLs or show and hide it.", [state("browser")]],
	["droppedframes", "Dropped Frames Alarm", "dropped", "Shows dropped, skipped or missed frames and flashes when the number goes up. Press to stop the alarm.", [state("dropped")]],
	["cpuusage", "OBS CPU Usage", "cpu", "Shows how much CPU OBS uses.", [state("cpu")]],
	["monitorset", "Source Monitor Set", "monitor", "Set how an audio source is monitored: off, monitor only, or monitor and output.", [state("monitor"), state("monitor-on")]],
	["profile", "Profile / Scene Collection", "collection", "Switch to another OBS profile or scene collection.", [state("profile")]],
	["focusedwindow", "Focused Window Capture", "window", "Point a Window Capture (Xcomposite) source at the window that has focus. Needs OBS on X11.", [state("window")]],
];

// OpenDeck shows these names and tooltips when its language is German.
const de = {
	recordtoggle: ["Aufnahme an/aus", "Aufnahme starten, stoppen oder pausieren, auf diesem PC oder in einem anderen OBS im Netzwerk."],
	streamtoggle: ["Stream an/aus", "Stream starten oder stoppen, auf diesem PC oder in einem anderen OBS im Netzwerk."],
	smartscene: ["Smarter Szenenwechsel", "Szene wechseln. Im Studiomodus: erster Druck in die Vorschau, zweiter live. Zeigt ein Live-Bild mit rotem (live) oder grünem (Vorschau) Rahmen."],
	mutetoggle: ["Quelle stummschalten", "Eine Audioquelle stumm- oder lautschalten."],
	sourcevisibility: ["Quelle ein-/ausblenden", "Eine Quelle in einer Szene ein- oder ausblenden."],
	instantreplay: ["Instant Replay", "Drücken speichert den Wiederholungspuffer, spielt ihn im Stream ab und erstellt einen Twitch-Clip. Halten schaltet den Puffer an oder aus."],
	sourcevolume: ["Quellen-Lautstärke", "Lautstärke einer Audioquelle lauter, leiser oder auf einen Wert stellen, mit Live-Pegelanzeige. Halten macht weiter."],
	inputvolume: ["Lautstärke (Drehregler)", "Drehen ändert die Lautstärke einer Audioquelle, mit Live-Pegelanzeige. Drücken oder Antippen schaltet stumm."],
	scenedial: ["Szene (Drehregler)", "Drehen wählt eine Szene (mit Bild), Drücken wechselt."],
	studiomode: ["Studiomodus an/aus", "Studiomodus ein- oder ausschalten."],
	previousscene: ["Letzte Szene", "Zurück zur Szene, die davor live war. Zeigt ihren Namen."],
	nextscene: ["Nächste Szene", "Zur nächsten Szene in der Szenenliste oder zur Szene darüber."],
	mediacontrol: ["Mediensteuerung", "Abspielen, Pause, Stopp, Neustart oder Springen in einer Medienquelle. Zeigt die Rest- oder die gespielte Zeit."],
	videoplayer: ["Videoplayer", "Eine Datei in einer Medienquelle abspielen. Mehrere Tasten können sich eine Quelle teilen, jede mit eigener Datei und eigenem Tempo."],
	imagesource: ["Bildquelle", "Ein Bild in einer Bildquelle zeigen. Mehrere Tasten können sich eine Quelle teilen, jede mit eigenem Bild. Kann es nach ein paar Sekunden wieder ausblenden."],
	settext: ["Text setzen", "Text einer Textquelle ändern: fester Text, nächste Zeile einer Liste oder ein Zähler, der hoch- oder runterzählt."],
	filtertoggle: ["Filter an/aus", "Einen Filter einer Quelle ein- oder ausschalten."],
	streamhealth: ["Stream-Zustand", "Bitrate, ausgelassene Frames und ein grünes, gelbes oder rotes Licht für deinen Stream."],
	virtualcam: ["Virtuelle Kamera an/aus", "Die virtuelle Kamera von OBS starten oder stoppen."],
	recordtools: ["Aufnahme-Marker", "Während der Aufnahme: Kapitelmarke setzen oder eine neue Datei beginnen."],
	screenshot: ["Screenshot", "Ein Bild der Live-Szene speichern, oder von einer beliebigen Szene oder Quelle."],
	hotkey: ["OBS-Hotkey", "Einen OBS-Hotkey auslösen, über seinen Namen oder die Tasten, die in OBS dafür eingestellt sind."],
	settransition: ["Übergang festlegen", "Szenenübergang und seine Dauer festlegen."],
	transitiondial: ["Übergang (Drehregler)", "Drehen ändert die Übergangsdauer, Drücken nimmt den nächsten Übergang."],
	filterdial: ["Filter (Drehregler)", "Drehen ändert eine Zahl in einem Filter, etwa Verstärkung, Deckkraft oder Unschärfe. Drücken schaltet den Filter an oder aus."],
	sourceanimation: ["Quellen-Animation", "Eine Quelle in Phasen verschieben, skalieren, drehen oder zuschneiden. Die Änderung in OBS aufnehmen, das Plugin trägt die Zahlen ein."],
	browsersource: ["Browserquelle", "Eine Browserquelle neu laden, ihre URL setzen, URLs durchschalten oder sie ein- und ausblenden."],
	droppedframes: ["Frame-Verlust-Alarm", "Zeigt ausgelassene, übersprungene oder verpasste Frames und blinkt, wenn die Zahl steigt. Drücken beendet den Alarm."],
	cpuusage: ["OBS-CPU-Last", "Zeigt, wie viel CPU OBS braucht."],
	monitorset: ["Audioüberwachung festlegen", "Festlegen, wie eine Audioquelle überwacht wird: aus, nur überwachen oder überwachen und ausgeben."],
	profile: ["Profil / Szenensammlung", "Zu einem anderen OBS-Profil oder einer anderen Szenensammlung wechseln."],
	focusedwindow: ["Aktives Fenster aufnehmen", "Eine Fensteraufnahme (Xcomposite) auf das Fenster mit dem Fokus richten. Braucht OBS unter X11."],
};
const missing = actions.map(([uuid]) => uuid).filter((uuid) => !de[uuid]);
if (missing.length) throw new Error(`no German name for: ${missing.join(", ")}`);

const manifest = {
	$schema: "https://schemas.elgato.com/streamdeck/plugins/manifest.json",
	UUID: ID,
	Name: "OBS Tools",
	Version: version,
	Author: "Felitendo",
	Description: "Advanced OBS tools for OpenDeck on Linux: smart scene switching with previews, source animations, instant replay, video player, dropped frame alarm and more.",
	URL: "https://github.com/FeloDeck/obs-tools",
	Icon: "images/plugin",
	Category: "OBS Tools",
	CategoryIcon: "images/category",
	CodePath: "plugin/main.js",
	Nodejs: { Version: "22" },
	PropertyInspectorPath: "pi/index.html",
	SDKVersion: 2,
	OS: [{ Platform: "linux" }],
	Actions: actions.map(([uuid, name, icon, tooltip, states, extra = {}]) => ({
		UUID: `${ID}.${uuid}`,
		Name: name,
		Icon: `images/actions/${icon}`,
		Tooltip: tooltip,
		Controllers: ["Keypad"],
		SupportedInMultiActions: true,
		DisableAutomaticStates: true,
		States: states,
		...extra,
	})),
};

const localized = {
	Name: "OBS Tools",
	Description: "Erweiterte OBS-Werkzeuge für OpenDeck unter Linux: smarter Szenenwechsel mit Vorschau, Quellen-Animationen, Instant Replay, Videoplayer, Alarm bei Frame-Verlust und mehr.",
	...Object.fromEntries(actions.map(([uuid]) => [`${ID}.${uuid}`, { Name: de[uuid][0], Tooltip: de[uuid][1] }])),
};

const dir = path.join(root, "de.felitendo.obs-tools.sdPlugin");
fs.writeFileSync(path.join(dir, "manifest.json"), JSON.stringify(manifest, null, "\t") + "\n");
fs.writeFileSync(path.join(dir, "de.json"), JSON.stringify(localized, null, "\t") + "\n");
console.log(`manifest.json and de.json: ${manifest.Actions.length} actions, version ${version}`);
