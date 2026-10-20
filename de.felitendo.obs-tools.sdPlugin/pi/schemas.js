// What each settings page shows. Field types: select, text, number, checkbox,
// color, textarea, file, segmented, tiles, help, and the custom "phases" and "twitch".
// section: where a field goes. Without one it is at the top: what the key is about (source, scene).
// CONTROLS (what press, hold and double press do) and LOOK ("On the key", dials: lookTitle) are open.
// Any other name is a section of its own that is closed until opened, or until openIf(settings) of a
// field in it is true.
// tiles: for choices that change how the key looks. Like segmented, with a small picture of the
// key above each choice: preview(value, settings) is its SVG. value(settings) and apply(value)
// can map the choice onto more than one setting.
// The texts are English. src/i18n/de.rs has the German ones.
import { num, t } from "./lib/i18n.js";
import { COLORS, MEDIA_GLYPHS, VOLUME_GLYPHS } from "./lib/icons.js";
import { glyphKeySvg, sceneKeySvg, statKeySvg } from "./lib/render.js";
import { TEXT_ACTIONS, textUses } from "./lib/text-gestures.js";
import { formatVolume } from "./lib/volume.js";

export const CONTROLS = "Controls";
export const LOOK = "look";
const MORE = "More options";
// Dials have no key: their look is on the touch strip.
const DIAL = { lookTitle: "On the display" };

// Press, hold and double press offer the same choices (the plugin side: Action.gesture).
// pressOnly: choices only for press, they use the key going down and up (push to talk).
// repeat: hold can also repeat the press while the key is held.
const gestures = (key, options, { pressOnly = [], repeat = false } = {}) => {
	const more = [["none", "Nothing"], ...(repeat ? [["repeat", "Keep going while held"]] : []), ...options.filter(([v]) => v !== "none" && !pressOnly.includes(v))];
	const free = (s) => !pressOnly.includes(s[key] ?? options[0][0]);
	return [
		{ key, label: "Press does", type: "select", options, section: CONTROLS },
		{ key: "hold", label: "Hold does", type: "select", options: more, showIf: free, section: CONTROLS },
		{ key: "double", label: "Double press", type: "select", options: more.filter(([v]) => v !== "repeat"), showIf: free, section: CONTROLS },
		{ type: "note", text: "With a double press set, a single press waits a moment for the second one.", showIf: (s) => free(s) && (s.double ?? "none") !== "none", section: CONTROLS },
	];
};

// Does press, hold or double press use one of these choices? first: the press default.
const uses = (s, key, choices, first) => [s[key] ?? first, s.hold, s.double].some((v) => choices.includes(v));

const TOGGLE = [
	["toggle", "Toggle"],
	["enable", "Turn on"],
	["disable", "Turn off"],
];
const START_STOP = [
	["toggle", "Toggle"],
	["start", "Start"],
	["stop", "Stop"],
];

// Where a key shows a time or value: not at all, or below or above the glyph. on and at are the
// settings behind it. key(settings) gives the glyph, its color, the text as the key looks while it
// runs, and more options for glyphKeySvg (a meter).
const titleTiles = (label, on, at, key) => ({
	key: at,
	label,
	type: "tiles",
	section: LOOK,
	options: [
		["off", "Off"],
		["below", "Below"],
		["above", "Above"],
	],
	value: (s) => (s[on] === false ? "off" : (s[at] ?? "below")),
	apply: (v) => (v === "off" ? { [on]: false } : { [on]: true, [at]: v }),
	preview: (v, s) => {
		const [glyph, color, text, more] = key(s);
		return glyphKeySvg(glyph, { ...more, color, title: v === "off" ? "" : text, titleAt: v });
	},
});

// A made-up scene for the pictures of the scene key: sky, sun and two hills.
const SAMPLE_SCENE =
	"data:image/svg+xml," +
	encodeURIComponent(
		`<svg xmlns="http://www.w3.org/2000/svg" width="160" height="90" viewBox="0 0 160 90"><defs><linearGradient id="sky" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#3f6df2"/><stop offset="1" stop-color="#a97bf0"/></linearGradient></defs>` +
			`<rect width="160" height="90" fill="url(#sky)"/><circle cx="116" cy="30" r="13" fill="#ffd166"/>` +
			`<path d="M0 68 Q40 40 82 64 T160 56 V90 H0 Z" fill="#2f9e6e"/><path d="M0 80 Q52 60 102 77 T160 74 V90 H0 Z" fill="#1f7a52"/></svg>`,
	);

const ID = "de.felitendo.obs-tools";

// Every dial: a touch does what a press does, unless touches are off.
const TOUCH = { key: "disableTouch", label: "Touch screen", type: "checkbox", text: "Ignore touches", section: MORE };

const TEXT_ACTION_LABELS = {
	none: "Nothing",
	set: "Set the text",
	toggle: "Text on or off (set, then clear)",
	next: "Next line",
	previous: "Previous line",
	first: "First line",
	up: "Count up",
	down: "Count down",
	reset: "Reset the counter",
	timer: "Start or stop the timer",
	clear: "Clear the text",
};


export const SCHEMAS = {
	[`${ID}.smartscene`]: {
		help: "Not in studio mode: press to switch to the scene. In studio mode: the first press puts the scene in preview, the second press takes it live. In a multi action it always goes live.",
		fields: [
			{ key: "scene", label: "Scene", type: "select", list: "scenes" },
			...gestures("press", [
				["switch", "Switch (studio mode: preview first)"],
				["live", "Go live right away"],
				["preview", "Put in preview"],
			]),
			{ key: "showTitle", section: LOOK, label: "Title", type: "checkbox", text: "Show a title on the key" },
			{ key: "title", section: LOOK, label: "Custom title", type: "text", placeholder: "Scene name", showIf: (s) => s.showTitle },
			{
				key: "previewFit",
				section: LOOK,
				label: "Picture",
				type: "tiles",
				options: [
					["icon", "Icon"],
					["fit", "Whole scene"],
					["fill", "Fill the key"],
				],
				value: (s) => (s.showPreview === false ? "icon" : (s.previewFit ?? "fit")),
				apply: (v) => (v === "icon" ? { showPreview: false } : { showPreview: true, previewFit: v }),
				preview: (v, s) => {
					const title = s.showTitle === false ? "" : s.title || s.scene || t("Scene");
					return sceneKeySvg({ image: v === "icon" ? null : SAMPLE_SCENE, fit: v, title });
				},
			},
			{ key: "previewInterval", section: MORE, label: "Refresh every", type: "number", min: 0, max: 60, step: 0.5, unit: "seconds", hint: "0 = only when the live scene changes", showIf: (s) => s.showPreview },
			{ key: "liveColor", section: MORE, label: "Live frame", type: "color" },
			{ key: "previewColor", section: MORE, label: "Preview frame", type: "color" },
		],
	},
	[`${ID}.sourceanimation`]: {
		help: "Build an animation from phases. Each phase changes the source by the numbers you enter, over its duration. Easiest way: press Record, move or resize the source in OBS, then press Stop recording.",
		fields: [
			{ key: "scene", label: "Scene", type: "select", list: "scenes", empty: "Current live scene" },
			{ key: "source", label: "Source", type: "select", list: "sceneItems" },
			{ type: "phases" },
			{ key: "showAtStart", section: "Before and after", openIf: (s) => !!s.showAtStart, label: "At the start", type: "checkbox", text: "Show the source" },
			{ key: "hideAtEnd", section: "Before and after", openIf: (s) => !!s.hideAtEnd, label: "At the end", type: "checkbox", text: "Hide the source" },
			{ key: "resetAtEnd", section: "Before and after", openIf: (s) => !!s.resetAtEnd, label: "", type: "checkbox", text: "Put the source back where it started" },
			{ key: "filter", section: "Before and after", openIf: (s) => !!s.filter, label: "Filter", type: "select", list: "filters", empty: "No filter" },
			{
				key: "filterStart",
				section: "Before and after",
				label: "Filter at start",
				type: "segmented",
				options: [
					["none", "Keep as is"],
					["enable", "On"],
					["disable", "Off"],
				],
				showIf: (s) => s.filter,
			},
			{
				key: "filterEnd",
				section: "Before and after",
				label: "Filter at end",
				type: "segmented",
				options: [
					["none", "Keep as is"],
					["enable", "On"],
					["disable", "Off"],
				],
				showIf: (s) => s.filter,
			},
		],
	},
	[`${ID}.studiomode`]: {
		fields: [
			...gestures("mode", [...TOGGLE, ["take", "Take preview live"]]),
			{ type: "note", text: "Take preview live works like the Transition button in OBS: the scene in preview goes live.", showIf: (s) => uses(s, "mode", ["take"], "toggle"), section: CONTROLS },
		],
	},

	[`${ID}.videoplayer`]: {
		help: "Many keys can use the same media source. Each key plays its own file.",
		fields: [
			{ key: "input", label: "Media source", type: "select", list: "mediaInputs" },
			{ key: "file", label: "File", type: "file", placeholder: "/home/you/Videos/clip.mp4", hint: "Empty = play the file the source already has" },
			...gestures("press", [
				["play", "Play"],
				["stop", "Stop"],
				["restart", "Play from the start"],
			]),
			{ key: "speed", section: MORE, label: "Speed", type: "number", min: 1, max: 200, step: 5, unit: "%", hint: "100 = normal" },
			{ key: "scene", section: MORE, label: "Switch to scene", type: "select", list: "scenes", empty: "Stay on the current scene" },
			{ key: "showSource", section: MORE, label: "Visibility", type: "checkbox", text: "Show the source when it starts" },
			{ key: "hideWhenDone", section: MORE, label: "", type: "checkbox", text: "Hide the source when it ends" },
			{ key: "returnToScene", section: MORE, label: "", type: "checkbox", text: "Go back to the scene before", showIf: (s) => s.scene },
			{
				key: "pressAgain",
				section: CONTROLS,
				label: "Press again",
				type: "segmented",
				options: [
					["stop", "Stops the video"],
					["restart", "Starts it over"],
				],
				showIf: (s) => uses(s, "press", ["play"], "play"),
			},
			{ key: "showName", section: LOOK, label: "Title", type: "checkbox", text: "Show the file name" },
		],
	},
	[`${ID}.imagesource`]: {
		help: "Many keys can use the same image source. Each key shows its own picture.",
		fields: [
			{ key: "scene", label: "Scene", type: "select", list: "scenes", empty: "Current live scene" },
			{ key: "input", label: "Image source", type: "select", list: "imageInputs" },
			{ key: "file", label: "Picture", type: "file", images: true, placeholder: "/home/you/Pictures/brb.png", hint: "Empty = show the picture the source already has" },
			...gestures("mode", [
				["toggle", "Toggle"],
				["show", "Show"],
				["hide", "Hide"],
			]),
			{ key: "autoHide", section: MORE, label: "Hide after", type: "number", min: 0, max: 3600, step: 1, unit: "seconds", hint: "0 = stays visible" },
			{ key: "showName", section: LOOK, label: "Title", type: "checkbox", text: "Show the file name" },
		],
	},
	[`${ID}.sourcevolume`]: {
		fields: [
			{ key: "input", label: "Audio source", type: "select", list: "audioInputs" },
			...gestures(
				"mode",
				[
					["up", "Volume up"],
					["down", "Volume down"],
					["set", "Set volume"],
					["mute", "Mute or unmute"],
				],
				{ repeat: true },
			),
			{
				key: "unit",
				section: MORE,
				label: "Unit",
				type: "segmented",
				options: [
					["db", "dB"],
					["percent", "% of the fader"],
				],
			},
			{ key: "step", section: CONTROLS, label: "Step", type: "number", min: 0.1, max: 50, step: 0.5, unit: (s) => (s.unit === "percent" ? "%" : "dB"), showIf: (s) => uses(s, "mode", ["up", "down"], "up") },
			{ key: "value", section: CONTROLS, label: "Volume", type: "number", min: -100, max: 100, step: 1, unit: (s) => (s.unit === "percent" ? "%" : "dB"), showIf: (s) => uses(s, "mode", ["set"], "up") },
			{ key: "allowBoost", section: CONTROLS, label: "Above 0 dB", type: "checkbox", text: "Allow up to +26 dB", showIf: (s) => s.unit === "db" },
			titleTiles("Volume", "showValue", "valueAt", (s) => [VOLUME_GLYPHS[s.mode ?? "up"] ?? "volume", COLORS.white, formatVolume(-6, s.unit), { meter: s.showMeter === false ? null : 0.7 }]),
			{ key: "showMeter", section: LOOK, label: "Level meter", type: "checkbox", text: "Show the live level" },
		],
	},
	[`${ID}.inputvolume`]: {
		...DIAL,
		fields: [
			{ key: "input", label: "Audio source", type: "select", list: "audioInputs" },
			{
				key: "unit",
				section: MORE,
				label: "Unit",
				type: "segmented",
				options: [
					["db", "dB"],
					["percent", "% of the fader"],
				],
			},
			{ key: "step", section: MORE, label: "Step per tick", type: "number", min: 0.1, max: 20, step: 0.5, unit: (s) => (s.unit === "percent" ? "%" : "dB") },
			{
				key: "pressAction",
				section: CONTROLS,
				label: "Press does",
				type: "segmented",
				options: [
					["mute", "Mute"],
					["reset", "Back to 0 dB"],
					["nothing", "Nothing"],
				],
			},
			{ key: "allowBoost", section: CONTROLS, label: "Above 0 dB", type: "checkbox", text: "Allow up to +26 dB", showIf: (s) => s.unit === "db" },
			TOUCH,
			{ key: "showMeter", section: LOOK, label: "Level meter", type: "checkbox", text: "Show the live level" },
		],
	},
	[`${ID}.mutetoggle`]: {
		fields: [
			{ key: "input", label: "Audio source", type: "select", list: "audioInputs" },
			...gestures(
				"mode",
				[
					["toggle", "Toggle"],
					["mute", "Mute"],
					["unmute", "Unmute"],
					["talk", "Hold to talk (push to talk)"],
					["holdMute", "Hold to mute"],
				],
				{ pressOnly: ["talk", "holdMute"] },
			),
			{ type: "note", text: "Push to talk uses the key going down and up, so there is no hold or double press.", showIf: (s) => ["talk", "holdMute"].includes(s.mode), section: CONTROLS },
			{ key: "showName", section: LOOK, label: "Title", type: "checkbox", text: "Show the source name" },
		],
	},
	[`${ID}.monitorset`]: {
		fields: [
			{ key: "input", label: "Audio source", type: "select", list: "audioInputs" },
			...gestures("press", [
				["set", "Set monitoring"],
				["off", "Monitoring off"],
			]),
			{
				key: "monitorType",
				section: CONTROLS,
				label: "Monitoring",
				type: "select",
				showIf: (s) => uses(s, "press", ["set"], "set"),
				options: [
					["none", "Monitoring off"],
					["monitorOnly", "Monitor only (mute output)"],
					["monitorAndOutput", "Monitor and output"],
				],
			},
			{ key: "pressAgainOff", section: CONTROLS, label: "", type: "checkbox", text: "Press again to turn monitoring off", showIf: (s) => uses(s, "press", ["set"], "set") },
			{ key: "showName", section: LOOK, label: "Title", type: "checkbox", text: "Show the source name" },
		],
	},
	[`${ID}.droppedframes`]: {
		help: "The key flashes when the number goes up. Press it to stop the alarm.",
		fields: [
			{
				key: "frameType",
				label: "Count",
				type: "select",
				options: [
					["dropped", "Dropped frames (network, while live)"],
					["skipped", "Skipped frames (encoder too slow)"],
					["missed", "Missed frames (rendering too slow)"],
				],
			},
			{ key: "threshold", label: "Alarm after", type: "number", min: 1, max: 10000, step: 1, unit: "new frames" },
			{ key: "alertSeconds", section: LOOK, label: "Flash for", type: "number", min: 1, max: 600, step: 1, unit: "seconds" },
			{ key: "alertColor", section: LOOK, label: "Alarm color", type: "color" },
		],
	},
	[`${ID}.cpuusage`]: {
		fields: [
			{
				key: "showMemory",
				section: LOOK,
				label: "Display",
				type: "tiles",
				options: [
					["cpu", "CPU"],
					["memory", "CPU and memory"],
				],
				value: (s) => (s.showMemory ? "memory" : "cpu"),
				apply: (v) => ({ showMemory: v === "memory" }),
				preview: (v) => statKeySvg({ label: t("OBS CPU"), value: `${num(12.5, 1)}%`, sub: v === "memory" ? "820 MB" : "", bar: 0.125 }),
			},
			{ key: "warnAt", section: LOOK, label: "Turn red at", type: "number", min: 0, max: 100, step: 5, unit: "%", hint: "0 = never" },
		],
	},
	[`${ID}.previousscene`]: {
		help: "Goes back to the scene that was live before the current one. Pressing again goes back and forth.",
		fields: [{ key: "showName", section: LOOK, label: "Title", type: "checkbox", text: "Show the scene name" }],
	},
	[`${ID}.nextscene`]: {
		fields: [
			...gestures("direction", [
				["next", "Next scene"],
				["previous", "Scene above"],
			]),
			{ key: "wrap", section: CONTROLS, label: "At the end", type: "checkbox", text: "Start over at the other end" },
			{ key: "showName", section: LOOK, label: "Title", type: "checkbox", text: "Show the scene name" },
		],
	},
	[`${ID}.instantreplay`]: {
		help: "Saves the replay buffer, and can play it on stream or clip it on Twitch. Turn on the replay buffer in OBS first: Settings → Output → Replay Buffer.",
		fields: [
			...gestures("press", [
				["save", "Save the replay"],
				["buffer", "Turn the replay buffer on or off"],
				["bufferStart", "Start the replay buffer"],
				["bufferStop", "Stop the replay buffer"],
			]),
			{ key: "autoReplay", section: "Show the replay on stream", openIf: (s) => !!s.autoReplay, label: "Auto replay", type: "checkbox", text: "Play the saved replay in a media source" },
			{ key: "mediaInput", section: "Show the replay on stream", label: "Media source", type: "select", list: "mediaInputs", showIf: (s) => s.autoReplay },
			{ key: "replayScene", section: "Show the replay on stream", label: "Switch to scene", type: "select", list: "scenes", empty: "Stay on the current scene", showIf: (s) => s.autoReplay },
			{ key: "speed", section: "Show the replay on stream", label: "Speed", type: "number", min: 1, max: 200, step: 5, unit: "%", hint: "50 = slow motion", showIf: (s) => s.autoReplay },
			{ key: "hideWhenDone", section: "Show the replay on stream", label: "", type: "checkbox", text: "Hide the media source when the replay ends", showIf: (s) => s.autoReplay },
			{ key: "returnToScene", section: "Show the replay on stream", label: "", type: "checkbox", text: "Go back to the scene before", showIf: (s) => s.autoReplay && s.replayScene },
			{ type: "twitch", section: "Twitch chat and clips" },
			{ key: "chatCommand", section: "Twitch chat and clips", openIf: (s) => !!s.chatCommand, label: "Chat command", type: "checkbox", text: "Let chat start a replay" },
			{ key: "command", section: "Twitch chat and clips", label: "Command", type: "text", placeholder: "!replay", showIf: (s) => s.chatCommand },
			{
				key: "permission",
				section: "Twitch chat and clips",
				label: "Who may use it",
				type: "select",
				options: [
					["everyone", "Everyone"],
					["subs", "Subscribers, VIPs and mods"],
					["vips", "VIPs and mods"],
					["mods", "Mods only"],
				],
				showIf: (s) => s.chatCommand,
			},
			{ key: "cooldown", section: "Twitch chat and clips", label: "Cooldown", type: "number", min: 0, max: 3600, step: 5, unit: "seconds", showIf: (s) => s.chatCommand },
			{ key: "createClip", section: "Twitch chat and clips", openIf: (s) => !!s.createClip, label: "Clip", type: "checkbox", text: "Also create a Twitch clip (needs login, only while live)" },
			{ key: "postClip", section: "Twitch chat and clips", label: "", type: "checkbox", text: "Post the clip link in chat", showIf: (s) => s.createClip },
			{ key: "clipMessage", section: "Twitch chat and clips", label: "Message", type: "text", placeholder: "Instant replay: {url}", showIf: (s) => s.createClip && s.postClip },
		],
	},
	[`${ID}.browsersource`]: {
		fields: [
			{ key: "input", label: "Browser source", type: "select", list: "browserInputs" },
			...gestures("operation", [
				["refresh", "Refresh the page"],
				["url", "Open a URL"],
				["cycle", "Go to the next URL in a list"],
				["visibility", "Show or hide it (current scene)"],
			]),
			{ key: "url", section: CONTROLS, label: "URL", type: "text", placeholder: "https://…", showIf: (s) => uses(s, "operation", ["url"], "refresh") },
			{ key: "urls", section: CONTROLS, label: "URLs", type: "textarea", placeholder: "One URL per line", showIf: (s) => uses(s, "operation", ["cycle"], "refresh") },
			{ key: "showName", section: LOOK, label: "Title", type: "checkbox", text: "Show the source name" },
		],
	},
	[`${ID}.recordtoggle`]: {
		remote: true,
		fields: [
			...gestures("mode", [...START_STOP, ["pause", "Pause or resume"]]),
			titleTiles("Recording time", "showTimer", "timerAt", () => ["record", COLORS.red, "12:34"]),
		],
	},
	[`${ID}.streamtoggle`]: {
		remote: true,
		fields: [
			...gestures("mode", START_STOP),
			titleTiles("Stream time", "showTimer", "timerAt", () => ["stream", COLORS.red, "1:23:45"]),
		],
	},
	[`${ID}.settransition`]: {
		fields: [
			{ key: "transition", label: "Transition", type: "select", list: "transitions" },
			{ key: "duration", label: "Duration", type: "number", min: 50, max: 20000, step: 50, unit: "ms", hint: "Empty = keep", allowEmpty: true },
			{ key: "showName", section: LOOK, label: "Title", type: "checkbox", text: "Show the transition name" },
		],
	},
	[`${ID}.filtertoggle`]: {
		fields: [
			{ key: "source", label: "Source", type: "select", list: "sources" },
			{ key: "filter", label: "Filter", type: "select", list: "filters" },
			...gestures("mode", TOGGLE),
			{ key: "showName", section: LOOK, label: "Title", type: "checkbox", text: "Show the filter name" },
		],
	},
	[`${ID}.virtualcam`]: {
		help: "On Linux the virtual camera needs the v4l2loopback kernel module. If it is not loaded, OBS asks for your password to load it.",
		fields: [...gestures("mode", START_STOP)],
	},
	[`${ID}.focusedwindow`]: {
		help: "Linux only: points a Window Capture (Xcomposite) source at the window that has focus when you press the key. This only works when OBS runs on X11. On Wayland, OBS has no way to pick a window without asking you.",
		fields: [
			{ key: "input", label: "Window capture", type: "select", list: "windowInputs" },
			{ key: "delay", label: "Wait first", type: "number", min: 0, max: 10, step: 0.5, unit: "seconds" },
		],
	},
	[`${ID}.sourcevisibility`]: {
		fields: [
			{ key: "scene", label: "Scene", type: "select", list: "scenes", empty: "Current live scene" },
			{ key: "source", label: "Source", type: "select", list: "sceneItems" },
			...gestures("mode", [
				["toggle", "Toggle"],
				["show", "Show"],
				["hide", "Hide"],
			]),
			{ key: "showName", section: LOOK, label: "Title", type: "checkbox", text: "Show the source name" },
		],
	},
	[`${ID}.hotkey`]: {
		fields: [
			{
				key: "mode",
				label: "Sends",
				type: "segmented",
				options: [
					["name", "OBS action"],
					["keys", "Key combination"],
				],
			},
			{ key: "hotkey", label: "Action", type: "select", list: "hotkeys", translate: true, showIf: (s) => s.mode !== "keys" },
			{ key: "context", label: "For source", type: "select", list: "sources", empty: "No source", hint: "Only for hotkeys that belong to a source, like mute or push to talk.", showIf: (s) => s.mode !== "keys" && /^(libobs|MediaSource)\./.test(String(s.hotkey)) },
			{
				key: "key",
				label: "Key",
				type: "select",
				options: [
					["OBS_KEY_F13", "F13"],
					["OBS_KEY_F14", "F14"],
					["OBS_KEY_F15", "F15"],
					["OBS_KEY_F16", "F16"],
					["OBS_KEY_F17", "F17"],
					["OBS_KEY_F18", "F18"],
					["OBS_KEY_F19", "F19"],
					["OBS_KEY_F20", "F20"],
					["OBS_KEY_F21", "F21"],
					["OBS_KEY_F22", "F22"],
					["OBS_KEY_F23", "F23"],
					["OBS_KEY_F24", "F24"],
					["OBS_KEY_F1", "F1"],
					["OBS_KEY_F2", "F2"],
					["OBS_KEY_F3", "F3"],
					["OBS_KEY_F4", "F4"],
					["OBS_KEY_F5", "F5"],
					["OBS_KEY_F6", "F6"],
					["OBS_KEY_F7", "F7"],
					["OBS_KEY_F8", "F8"],
					["OBS_KEY_F9", "F9"],
					["OBS_KEY_F10", "F10"],
					["OBS_KEY_F11", "F11"],
					["OBS_KEY_F12", "F12"],
					["OBS_KEY_A", "A"],
					["OBS_KEY_B", "B"],
					["OBS_KEY_C", "C"],
					["OBS_KEY_D", "D"],
					["OBS_KEY_E", "E"],
					["OBS_KEY_F", "F"],
					["OBS_KEY_G", "G"],
					["OBS_KEY_H", "H"],
					["OBS_KEY_I", "I"],
					["OBS_KEY_J", "J"],
					["OBS_KEY_K", "K"],
					["OBS_KEY_L", "L"],
					["OBS_KEY_M", "M"],
					["OBS_KEY_N", "N"],
					["OBS_KEY_O", "O"],
					["OBS_KEY_P", "P"],
					["OBS_KEY_Q", "Q"],
					["OBS_KEY_R", "R"],
					["OBS_KEY_S", "S"],
					["OBS_KEY_T", "T"],
					["OBS_KEY_U", "U"],
					["OBS_KEY_V", "V"],
					["OBS_KEY_W", "W"],
					["OBS_KEY_X", "X"],
					["OBS_KEY_Y", "Y"],
					["OBS_KEY_Z", "Z"],
					["OBS_KEY_0", "0"],
					["OBS_KEY_1", "1"],
					["OBS_KEY_2", "2"],
					["OBS_KEY_3", "3"],
					["OBS_KEY_4", "4"],
					["OBS_KEY_5", "5"],
					["OBS_KEY_6", "6"],
					["OBS_KEY_7", "7"],
					["OBS_KEY_8", "8"],
					["OBS_KEY_9", "9"],
					["OBS_KEY_NUM0", "Num 0"],
					["OBS_KEY_NUM1", "Num 1"],
					["OBS_KEY_NUM2", "Num 2"],
					["OBS_KEY_NUM3", "Num 3"],
					["OBS_KEY_NUM4", "Num 4"],
					["OBS_KEY_NUM5", "Num 5"],
					["OBS_KEY_NUM6", "Num 6"],
					["OBS_KEY_NUM7", "Num 7"],
					["OBS_KEY_NUM8", "Num 8"],
					["OBS_KEY_NUM9", "Num 9"],
					["OBS_KEY_HOME", "Home"],
					["OBS_KEY_END", "End"],
					["OBS_KEY_PAGEUP", "Page Up"],
					["OBS_KEY_PAGEDOWN", "Page Down"],
					["OBS_KEY_INSERT", "Insert"],
					["OBS_KEY_DELETE", "Delete"],
					["OBS_KEY_PAUSE", "Pause/Break"],
					["OBS_KEY_SCROLLLOCK", "Scroll Lock"],
					["OBS_KEY_PRINT", "Print"]
				],
				showIf: (s) => s.mode === "keys",
			},
			{ key: "control", label: "With", type: "checkbox", text: "Ctrl", showIf: (s) => s.mode === "keys" },
			{ key: "shift", label: "", type: "checkbox", text: "Shift", showIf: (s) => s.mode === "keys" },
			{ key: "alt", label: "", type: "checkbox", text: "Alt", showIf: (s) => s.mode === "keys" },
			{ key: "command", label: "", type: "checkbox", text: "Super", showIf: (s) => s.mode === "keys" },
			{ type: "note", text: "Set the same keys in OBS under Settings → Hotkeys. F13 to F24 work well, because no keyboard uses them.", showIf: (s) => s.mode === "keys" },
		],
	},
	[`${ID}.screenshot`]: {
		fields: [
			{ key: "source", label: "Picture of", type: "select", list: "sources", empty: "Current live scene" },
			{ key: "folder", label: "Folder", type: "folder", placeholder: "Recording folder of OBS" },
			{
				key: "format",
				section: MORE,
				label: "Format",
				type: "segmented",
				options: [
					["png", "PNG"],
					["jpg", "JPG"],
				],
			},
			{ key: "width", section: MORE, label: "Width", type: "number", min: 0, max: 7680, step: 10, unit: "px", hint: "0 = full size" },
		],
	},
	[`${ID}.recordtools`]: {
		help: "Works while recording. Chapter markers need the recording format Hybrid MP4. A new file needs Automatic file splitting in OBS (Settings → Output → Recording).",
		fields: [
			...gestures("operation", [
				["chapter", "Chapter marker"],
				["split", "Start a new file"],
			]),
			{ key: "chapterName", section: CONTROLS, label: "Chapter name", type: "text", placeholder: "Optional", showIf: (s) => uses(s, "operation", ["chapter"], "chapter") },
		],
	},
	[`${ID}.mediacontrol`]: {
		fields: [
			{ key: "input", label: "Media source", type: "select", list: "mediaInputs" },
			...gestures("operation", [
				["playpause", "Play / pause"],
				["play", "Play"],
				["pause", "Pause"],
				["stop", "Stop"],
				["restart", "Play from the start"],
				["forward", "Jump forward"],
				["back", "Jump back"],
				["next", "Next file (playlist)"],
				["previous", "Previous file (playlist)"],
			]),
			{ key: "seconds", section: CONTROLS, label: "Jump by", type: "number", min: 1, max: 600, step: 1, unit: "seconds", showIf: (s) => uses(s, "operation", ["forward", "back"], "playpause") },
			titleTiles("Time", "showTime", "timeAt", (s) => [!s.operation || s.operation === "playpause" ? "pauseMedia" : (MEDIA_GLYPHS[s.operation] ?? "playPause"), COLORS.white, s.timeShows === "played" ? "1:05" : "-2:31"]),
			{
				key: "timeShows",
				section: LOOK,
				label: "Shows",
				type: "segmented",
				options: [
					["left", "Time left"],
					["played", "Time played"],
				],
				showIf: (s) => s.showTime !== false,
			},
		],
	},
	[`${ID}.profile`]: {
		fields: [
			{
				key: "kind",
				label: "Switch",
				type: "segmented",
				options: [
					["profile", "Profile"],
					["collection", "Scene collection"],
				],
			},
			{ key: "name", label: "Profile", type: "select", list: "profiles", showIf: (s) => s.kind !== "collection" },
			{ key: "name", label: "Collection", type: "select", list: "collections", showIf: (s) => s.kind === "collection" },
			{ key: "showName", section: LOOK, label: "Title", type: "checkbox", text: "Show the name" },
		],
	},
	[`${ID}.settext`]: {
		fields: [
			{ key: "input", label: "Text source", type: "select", list: "textInputs" },
			...gestures(
				"press",
				TEXT_ACTIONS.map((a) => [a, TEXT_ACTION_LABELS[a]]),
			),
			{ key: "text", section: CONTROLS, label: "Text", type: "textarea", placeholder: "Be right back", showIf: (s) => textUses(s, "text") },
			{ key: "lines", section: CONTROLS, label: "Lines", type: "textarea", placeholder: "One text per line", showIf: (s) => textUses(s, "line") },
			{ key: "template", section: CONTROLS, label: "Counter", type: "text", placeholder: "Deaths: {n}", hint: "{n} becomes the number.", showIf: (s) => textUses(s, "count") },
			{ key: "start", section: CONTROLS, label: "Start at", type: "number", min: 0, step: 1, showIf: (s) => textUses(s, "count") },
			{ key: "count", section: CONTROLS, label: "Now at", type: "number", min: 0, step: 1, showIf: (s) => textUses(s, "count") },
			{ key: "step", section: CONTROLS, label: "Step", type: "number", min: 1, step: 1, showIf: (s) => textUses(s, "count") },
			{ type: "note", text: "The counter never goes below 0. Reset puts it back to the start number.", showIf: (s) => textUses(s, "count"), section: CONTROLS },
			{ key: "timerText", section: CONTROLS, label: "Timer", type: "text", placeholder: "Live in {t}", hint: "{t} becomes the time left.", showIf: (s) => textUses(s, "timer") },
			{ key: "timerLength", section: CONTROLS, label: "Length", type: "text", placeholder: "5:00", hint: "Minutes:seconds, or hours:minutes:seconds.", showIf: (s) => textUses(s, "timer") },
			{ key: "timerEnd", section: CONTROLS, label: "At 0", type: "text", placeholder: "Live now!", hint: "Empty = shows 0:00.", showIf: (s) => textUses(s, "timer") },
			{ type: "note", text: "The timer counts down by itself. Start it again to stop it.", showIf: (s) => textUses(s, "timer"), section: CONTROLS },
			{ key: "showText", section: LOOK, label: "Title", type: "checkbox", text: "Show the text on the key" },
		],
	},
	[`${ID}.streamhealth`]: {
		help: "Green: all good. Yellow: some frames dropped or the connection is slow. Red: many drops or OBS is reconnecting. Counts the last 10 seconds.",
		fields: [
			{
				key: "show",
				section: LOOK,
				label: "Big number",
				type: "tiles",
				options: [
					["bitrate", "Bitrate"],
					["time", "Stream time"],
				],
				preview: (v) => {
					const bitrate = `${num(6, 1)} Mb/s`;
					const sub = v === "time" ? bitrate : t("drops {pct}%", { pct: num(0, 1) });
					return statKeySvg({ label: t("LIVE"), value: v === "time" ? "1:23:45" : bitrate, sub, dot: COLORS.green });
				},
			},
			{ key: "warnDrops", section: LOOK, label: "Yellow from", type: "number", min: 0.1, max: 100, step: 0.5, unit: "% dropped" },
			{ key: "badDrops", section: LOOK, label: "Red from", type: "number", min: 0.1, max: 100, step: 0.5, unit: "% dropped" },
		],
	},
	[`${ID}.transitiondial`]: {
		...DIAL,
		fields: [
			{ key: "step", section: MORE, label: "Step per tick", type: "number", min: 10, max: 2000, step: 10, unit: "ms" },
			{
				key: "pressAction",
				section: CONTROLS,
				label: "Press does",
				type: "segmented",
				options: [
					["next", "Next transition"],
					["studio", "Take preview live"],
					["nothing", "Nothing"],
				],
			},
			TOUCH,
		],
	},
	[`${ID}.scenedial`]: {
		...DIAL,
		fields: [
			{
				key: "mode",
				section: CONTROLS,
				label: "Turning",
				type: "segmented",
				options: [
					["pick", "Picks, press switches"],
					["direct", "Switches right away"],
				],
			},
			{ type: "note", text: "In studio mode the press puts the scene in preview, a second press takes it live.", section: CONTROLS },
			TOUCH,
		],
	},
	[`${ID}.filterdial`]: {
		...DIAL,
		fields: [
			{ key: "source", label: "Source", type: "select", list: "sources" },
			{ key: "filter", label: "Filter", type: "select", list: "filters" },
			{ key: "setting", label: "Value", type: "select", list: "filterSettings", translate: true },
			{ key: "step", section: MORE, label: "Step per tick", type: "number", step: "any", placeholder: "auto", allowEmpty: true },
			{ key: "min", section: MORE, label: "From", type: "number", step: "any", placeholder: "auto", allowEmpty: true },
			{ key: "max", section: MORE, label: "To", type: "number", step: "any", placeholder: "auto", allowEmpty: true },
			{
				key: "pressAction",
				section: CONTROLS,
				label: "Press does",
				type: "segmented",
				options: [
					["toggle", "Filter on/off"],
					["reset", "Back to default"],
					["nothing", "Nothing"],
				],
			},
			TOUCH,
		],
	},
};
