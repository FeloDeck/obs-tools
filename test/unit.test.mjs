import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";

import { buildFrames, buildRequests, diffTransforms, normalizePhase, totalDuration } from "../de.felitendo.obs-tools.sdPlugin/plugin/lib/animation.js";
import { readLocalObsConfig, resolveConnection } from "../de.felitendo.obs-tools.sdPlugin/plugin/lib/connections.js";
import { xcompositeWindowValue } from "../de.felitendo.obs-tools.sdPlugin/plugin/lib/desktop.js";
import { LANGUAGES, num, setLanguage, t, translateMessage } from "../de.felitendo.obs-tools.sdPlugin/plugin/lib/i18n.js";
import { authResponse } from "../de.felitendo.obs-tools.sdPlugin/plugin/lib/obs.js";
import { fitSize, glyphKey, statKey, svgUrl, titleLine } from "../de.felitendo.obs-tools.sdPlugin/plugin/lib/render.js";
import { chatUser, isAllowed, normalizeChannel, parseIrc } from "../de.felitendo.obs-tools.sdPlugin/plugin/lib/twitch.js";
import { dbToFader, faderToDb, formatVolume, readDb, stepVolume, volumeRequest } from "../de.felitendo.obs-tools.sdPlugin/plugin/lib/volume.js";
import { Action, DOUBLE_PRESS_MS, LONG_PRESS_MS } from "../de.felitendo.obs-tools.sdPlugin/plugin/actions/base.js";
import { SetText } from "../de.felitendo.obs-tools.sdPlugin/plugin/actions/basics.js";
import { MuteToggle } from "../de.felitendo.obs-tools.sdPlugin/plugin/actions/audio.js";
import { formatDuration, parseDuration, timecodeSeconds } from "../de.felitendo.obs-tools.sdPlugin/plugin/lib/time.js";
import { SCHEMAS } from "../de.felitendo.obs-tools.sdPlugin/pi/schemas.js";

const near = (a, b, eps = 1e-6) => assert.ok(Math.abs(a - b) < eps, `${a} is not close to ${b}`);

test("obs-websocket auth matches the example from the protocol docs", () => {
	assert.equal(
		authResponse("supersecretpassword", "lM1GncleQOaCu9lT1yeUZhFYnqhsLLP1G5lAGo3ixaI=", "+IxH4CnCiqpX1rM9scsNynZzbOe4KhDeYcTNS3PDaeY="),
		"1Ct943GAT+6YQUUX47Ia/ncufilbe6+oD6lY+5kaCu4=",
	);
});

test("fader math round trips like the OBS mixer", () => {
	assert.equal(dbToFader(0), 1);
	assert.equal(dbToFader(-Infinity), 0);
	assert.equal(dbToFader(-100), 0);
	for (const db of [-60, -30, -12, -6, -1]) near(faderToDb(dbToFader(db)), db, 1e-6);
	assert.ok(dbToFader(-6) > dbToFader(-12));
});

test("volume steps", () => {
	assert.equal(stepVolume(-10, { unit: "db", step: 2, direction: 1 }), -8);
	assert.equal(stepVolume(-1, { unit: "db", step: 2, direction: 1 }), 0, "stops at 0 dB");
	assert.equal(stepVolume(-1, { unit: "db", step: 2, direction: 1, allowBoost: true }), 1);
	assert.equal(stepVolume(-Infinity, { unit: "db", step: 2, direction: 1 }), -60, "up from silence starts at -60 dB");
	assert.equal(stepVolume(-99, { unit: "db", step: 2, direction: -1 }), -Infinity);
	near(dbToFader(stepVolume(-12, { unit: "percent", step: 10, direction: 1 })), dbToFader(-12) + 0.1, 1e-9);
	assert.deepEqual(volumeRequest("Mic", -Infinity), { inputName: "Mic", inputVolumeMul: 0 });
	assert.deepEqual(volumeRequest("Mic", -6.123), { inputName: "Mic", inputVolumeDb: -6.12 });
	assert.equal(readDb({ inputVolumeDb: null, inputVolumeMul: 0 }), -Infinity);
	assert.equal(formatVolume(-6), "-6.0 dB");
	assert.equal(formatVolume(-Infinity), "-inf dB");
	assert.equal(formatVolume(0, "percent"), "100%");
});

test("animation: diff only keeps fields that moved", () => {
	const a = { positionX: 100, positionY: 50, rotation: 0, scaleX: 1, scaleY: 1, cropLeft: 0, boundsType: "OBS_BOUNDS_NONE", boundsWidth: 0 };
	const b = { ...a, positionX: 400, scaleX: 1.5, cropLeft: 20, boundsWidth: 300 };
	assert.deepEqual(diffTransforms(a, b), { positionX: 300, scaleX: 0.5, cropLeft: 20 });
});

test("animation: frames per phase, final values and easing ends", () => {
	const start = { positionX: 0, positionY: 0, rotation: 0, scaleX: 1, scaleY: 1, cropLeft: 0 };
	const phases = [
		{ duration: 1000, easing: "easeInOut", changes: { positionX: 300 } },
		{ duration: 500, delay: 200, easing: "easeOut", changes: { cropLeft: 10.6, scaleX: -1 } },
	];
	const { fields, steps } = buildFrames(start, phases, 30);
	assert.deepEqual(fields.sort(), ["cropLeft", "positionX", "scaleX"]);
	const frames = steps.filter((s) => s.transform);
	assert.equal(frames.length, 30 + 15);
	assert.deepEqual(steps.filter((s) => s.wait), [{ wait: 6 }]);
	assert.equal(frames[29].transform.positionX, 300);
	assert.equal(frames.at(-1).transform.cropLeft, 11, "crop is a whole number");
	assert.equal(frames.at(-1).transform.scaleX, 0.0001, "scale never hits 0");
	assert.ok(frames[0].transform.positionX < 10, "ease in starts slow");
	assert.equal(totalDuration(phases), 1700);
});

test("animation: requests include start and end options", () => {
	const { requests, frames } = buildRequests({
		sceneName: "S",
		sceneItemId: 3,
		sourceName: "Box",
		start: { positionX: 5 },
		phases: [{ duration: 100, changes: { positionX: 10 } }],
		fps: 30,
		options: { showAtStart: true, hideAtEnd: true, resetAtEnd: true, filter: "Blur", filterStart: "enable", filterEnd: "disable" },
	});
	assert.equal(frames, 3);
	const types = requests.map((r) => r.requestType);
	assert.equal(types[0], "SetSourceFilterEnabled");
	assert.equal(types[1], "SetSceneItemEnabled");
	assert.equal(types.at(-1), "SetSourceFilterEnabled");
	assert.equal(requests.at(-1).requestData.filterEnabled, false);
	assert.deepEqual(requests.at(-2).requestData.sceneItemTransform, { positionX: 5 }, "reset to the start");
	assert.equal(normalizePhase({ easing: "nope" }).easing, "linear");
});

test("twitch: IRC lines with tags", () => {
	const line = "@badge-info=subscriber/12;badges=moderator/1,subscriber/12;display-name=SomeMod;mod=1;subscriber=1 :somemod!somemod@somemod.tmi.twitch.tv PRIVMSG #chan :!replay please";
	const message = parseIrc(line);
	assert.equal(message.command, "PRIVMSG");
	assert.deepEqual(message.params, ["#chan", "!replay please"]);
	const user = chatUser(message);
	assert.equal(user.login, "somemod");
	assert.equal(user.name, "SomeMod");
	assert.ok(user.mod && user.sub && !user.vip && !user.broadcaster);
	assert.equal(parseIrc("PING :tmi.twitch.tv").params[0], "tmi.twitch.tv");
	assert.ok(isAllowed(user, "mods"));
	assert.ok(!isAllowed({ sub: true }, "vips"));
	assert.ok(isAllowed({ broadcaster: true }, "mods"));
	assert.equal(normalizeChannel("https://www.twitch.tv/SomeOne/videos"), "someone");
	assert.equal(normalizeChannel("#Chan "), "chan");
});

test("connection: saved settings, never read from OBS by itself", () => {
	assert.deepEqual(resolveConnection({ host: " localhost ", password: "typed" }), { host: "localhost", port: 4455, password: "typed" });
	assert.deepEqual(resolveConnection({ host: "192.168.1.5", port: "4460" }), { host: "192.168.1.5", port: 4460, password: "" });
	assert.deepEqual(resolveConnection({}), { host: "127.0.0.1", port: 4455, password: "" });
});

test("connection: newest obs-websocket config file is used", () => {
	const dir = fs.mkdtempSync(path.join(os.tmpdir(), "obstools-"));
	const a = path.join(dir, "a.json");
	const b = path.join(dir, "b.json");
	fs.writeFileSync(a, JSON.stringify({ server_port: 1111, server_password: "old" }));
	fs.writeFileSync(b, JSON.stringify({ server_port: 2222, server_password: "new", server_enabled: false }));
	fs.utimesSync(a, new Date(2000, 1, 1), new Date(2000, 1, 1));
	const found = readLocalObsConfig([a, b, path.join(dir, "missing.json")]);
	assert.equal(found.port, 2222);
	assert.equal(found.password, "new");
	assert.equal(found.enabled, false);
	fs.rmSync(dir, { recursive: true });
});

test("render: SVG data URLs with escaped text", () => {
	const url = statKey({ label: "A<B", value: "1&2" });
	assert.ok(url.startsWith("data:image/svg+xml;base64,"));
	const svg = Buffer.from(url.split(",")[1], "base64").toString();
	assert.ok(svg.includes("A&lt;B") && svg.includes("1&amp;2"));
	assert.ok(fitSize("a very long scene name", 40) < 40);
	const key = Buffer.from(glyphKey("record").split(",")[1], "base64").toString();
	assert.ok(!key.includes("<rect width=\"144\""), "keys have no background");
	assert.ok(key.includes("translate(28.8 28.8)"), "glyph sits in the middle");
	assert.ok(svgUrl("<svg/>").length > 20);
});

test("xcomposite window value uses the OBS format", () => {
	assert.equal(xcompositeWindowValue({ id: 123, name: "Firefox", cls: "Navigator" }), "123\r\nFirefox\r\nNavigator");
});

import { filterRange, filterValueLabel } from "../de.felitendo.obs-tools.sdPlugin/plugin/actions/dials.js";
import { hotkeyLabel } from "../de.felitendo.obs-tools.sdPlugin/plugin/lib/plugin.js";
import { ObsState, levelFromMeter } from "../de.felitendo.obs-tools.sdPlugin/plugin/lib/obs-state.js";
import { meterLevel } from "../de.felitendo.obs-tools.sdPlugin/plugin/lib/render.js";

test("hotkey names become readable", () => {
	assert.equal(hotkeyLabel("OBSBasic.StartRecording"), "Start recording");
	assert.equal(hotkeyLabel("libobs.mute"), "Mute (source)");
	assert.equal(hotkeyLabel("MediaSource.Play"), "Play (media source)");
	assert.equal(hotkeyLabel("libobs.show_scene_item.1"), "Show scene item 1 (source)");
});

test("level meter: loudest peak in dB, -60..0 dB on the bar", () => {
	const level = levelFromMeter([
		[0.1, 0.5, 0.5],
		[0.2, 0.25, 0.25],
	]);
	near(level.peak, 20 * Math.log10(0.5), 1e-9);
	assert.equal(levelFromMeter([]).peak, -Infinity);
	assert.equal(meterLevel(-Infinity), 0);
	assert.equal(meterLevel(0), 1);
	assert.equal(meterLevel(-30), 0.5);
	assert.equal(meterLevel(-90), 0);
});

test("filter ranges know versioned filter kinds", () => {
	assert.deepEqual(filterRange("color_filter_v2", "opacity"), [0, 1, 0.01]);
	assert.deepEqual(filterRange("gain_filter", "db"), [-30, 30, 0.5]);
	assert.equal(filterRange("unknown_filter", "x"), null);
});

test("stream bitrate from bytes between two polls", () => {
	const state = Object.create(ObsState.prototype);
	state.lastBytes = null;
	assert.equal(state.bitrate({ outputActive: true, outputBytes: 0 }), null);
	state.lastBytes.now -= 1000;
	near(state.bitrate({ outputActive: true, outputBytes: 750000 }), 6000, 1);
	assert.equal(state.bitrate({ outputActive: false, outputBytes: 0 }), null);
	assert.equal(state.lastBytes, null);
});

// Every text a user can see needs a German version, with the same {placeholders}.
test("every text has a German translation", () => {
	const texts = new Set();
	const add = (value) => typeof value === "string" && value && texts.add(value);
	for (const schema of Object.values(SCHEMAS)) {
		add(schema.help);
		add(schema.hint);
		add(schema.lookTitle);
		for (const field of schema.fields ?? []) {
			for (const key of ["label", "text", "hint", "placeholder", "empty", "unit"]) add(field[key]);
			// Section titles. "look" is a name in the code, the page shows "On the key" or lookTitle.
			if (field.section !== "look") add(field.section);
			if (typeof field.unit === "function") for (const unit of ["db", "percent"]) add(field.unit({ unit }));
			for (const [, label] of field.options ?? []) add(label);
		}
	}
	// t("...") in the code, and the errors it throws: the settings page shows them translated.
	const sdPlugin = path.join(import.meta.dirname, "../de.felitendo.obs-tools.sdPlugin");
	const files = ["pi/pi.js", ...["plugin/actions", "plugin/lib"].flatMap((dir) => fs.readdirSync(path.join(sdPlugin, dir)).map((f) => `${dir}/${f}`))];
	const errors = new Set();
	for (const file of files) {
		const code = fs.readFileSync(path.join(sdPlugin, file), "utf8");
		for (const [, text] of code.matchAll(/\bt\("((?:[^"\\]|\\.)+)"/g)) add(text);
		// Messages are the texts with a space. ${...} can be any value, like {name} in the table.
		for (const [, args] of code.matchAll(/new Error\((.*)\)/g)) {
			for (const [, plain, template] of args.matchAll(/"((?:[^"\\]|\\.)+)"|`([^`]+)`/g)) {
				const text = plain ?? template.replace(/\$\{[^}]*\}/g, "{x}");
				if (text.includes(" ")) errors.add(text);
			}
		}
	}
	for (const [, , label] of fs.readFileSync(path.join(sdPlugin, "plugin/actions/stats.js"), "utf8").matchAll(/(\w+): \{ label: "([^"]+)"/g)) add(label);
	// Names, units, keys and examples that stay as they are.
	const keep = /^(?:%|\d+:\d\d|[A-Z0-9]{1,3}|F\d+|Num \d|PNG|JPG|dB|ms|px|auto|Alt|Super|URLs?|!replay|https:\/\/…|\d+)$/;
	const de = LANGUAGES.de;
	const missing = [...texts].filter((text) => !(text in de) && !keep.test(text));
	assert.deepEqual(missing, [], "missing German texts");
	const known = new Set(Object.keys(de).map((text) => text.replace(/\{\w+\}/g, "{x}")));
	assert.deepEqual([...errors].filter((text) => !known.has(text)), [], "errors without a German text");
	// A text listed twice would quietly lose its first translation.
	const table = fs.readFileSync(path.join(sdPlugin, "plugin/lib/i18n.js"), "utf8");
	const keys = [...table.matchAll(/^\t("(?:[^"\\]|\\.)*"|'[^']*'|\w+):/gm)].map((m) => m[1].replace(/^["']|["']$/g, ""));
	assert.deepEqual(keys.filter((k, i) => keys.indexOf(k) !== i), [], "texts listed twice");
	const holes = (text) => [...text.matchAll(/\{\w+\}/g)].map((m) => m[0]).sort().join();
	for (const [en, translated] of Object.entries(de)) assert.equal(holes(translated), holes(en), `placeholders differ: ${en}`);
});

test("plugin messages are translated with their values", () => {
	setLanguage("de");
	try {
		assert.equal(translateMessage("Nothing answers on 127.0.0.1:4455"), "Unter 127.0.0.1:4455 antwortet nichts");
		assert.equal(translateMessage("Recorded 3 changes into phase 2."), "3 Änderungen in Phase 2 aufgenommen.");
		assert.equal(translateMessage("something we do not know"), "something we do not know");
		assert.equal(t("Phase {n}", { n: 4 }), "Phase 4");
		assert.equal(num(6.43, 1), "6,4");
		assert.equal(formatVolume(-2), "-2,0 dB");
	} finally {
		setLanguage("en");
	}
	assert.equal(num(6.43, 1), "6.4");
	assert.equal(t("Wrong password"), "Wrong password");
});

function textKey(settings) {
	const key = Object.create(SetText.prototype);
	key.settings = { ...SetText.defaults, ...settings };
	key.written = [];
	key.setText = async (text) => key.written.push(text);
	key.saveSettings = (patch) => Object.assign(key.settings, patch);
	key.render = () => {};
	key.showOk = () => {};
	return key;
}

test("set text: press, hold and double press can each do anything", async () => {
	// A countdown: press counts down and stops at 0, hold resets to the start number.
	const key = textKey({ input: "T", press: "down", hold: "reset", template: "Lives: {n}", start: 2, count: 2 });
	await key.onPress();
	await key.onPress();
	await key.onPress();
	assert.deepEqual(key.written, ["Lives: 1", "Lives: 0", "Lives: 0"]);
	await key.press("hold");
	assert.equal(key.written.at(-1), "Lives: 2");
	// Any mix: press sets a text, hold goes to the next line, double press counts up.
	const mixed = textKey({ input: "T", press: "set", hold: "next", double: "up", text: "BRB", lines: "One\nTwo", template: "{n}" });
	assert.deepEqual([mixed.gesture("hold"), mixed.gesture("double")], ["next", "up"]);
	await mixed.press("press");
	await mixed.press("hold");
	await mixed.press("double");
	assert.deepEqual(mixed.written, ["BRB", "One", "1"]);
	// The key shows what was set last.
	assert.equal(mixed.preview(), "1");
	await mixed.run("clear");
	assert.equal(mixed.preview(), "");
	// Unknown values fall back: press sets the text, the others do nothing.
	assert.equal(textKey({ press: "bogus", hold: "bogus" }).gesture("press"), "set");
	assert.equal(textKey({ hold: "bogus" }).gesture("hold"), "none");
});

// A key with two choices, for the gestures of the base class.
class Probe extends Action {
	static pressKey = "mode";
	static ops = ["a", "b"];
	onPress(args) {
		this.seen.push(this.op(args));
	}
}

function probe(settings) {
	const sd = { setSettings() {}, showAlert() {}, showOk() {}, setImage() {}, setTitle() {} };
	const key = new Probe({ sd }, { context: "c", action: "a", payload: { settings } });
	key.online = true;
	key.visible = true;
	key.seen = [];
	return key;
}

test("a second press within a moment is a double press", async () => {
	const key = probe({ mode: "a", double: "b" });
	const tap = async () => (await key.keyDown({ payload: {} }), await key.keyUp());
	await tap();
	await new Promise((r) => setTimeout(r, DOUBLE_PRESS_MS + 100));
	assert.deepEqual(key.seen, ["a"]);
	await tap();
	await tap();
	await new Promise((r) => setTimeout(r, DOUBLE_PRESS_MS + 100));
	assert.deepEqual(key.seen, ["a", "b"]);
});

test("holding a key runs the hold choice, a short press the press choice", async () => {
	const key = probe({ mode: "a", hold: "b" });
	await key.keyDown({ payload: {} });
	await new Promise((r) => setTimeout(r, LONG_PRESS_MS + 100));
	await key.keyUp();
	await key.keyDown({ payload: {} });
	await key.keyUp();
	assert.deepEqual(key.seen, ["b", "a"]);
	// Without hold and double press the press runs right on key down.
	const plain = probe({ mode: "b" });
	await plain.keyDown({ payload: {} });
	assert.deepEqual(plain.seen, ["b"]);
	// In a multi action there are no gestures.
	const multi = probe({ mode: "a", hold: "b" });
	await multi.keyDown({ payload: { isInMultiAction: true } });
	assert.deepEqual(multi.seen, ["a"]);
});

test("timer texts: lengths and the time left", () => {
	assert.equal(parseDuration("5:00"), 300);
	assert.equal(parseDuration("90"), 90);
	assert.equal(parseDuration("1:02:03"), 3723);
	assert.equal(parseDuration("five"), 0);
	assert.equal(formatDuration(300), "5:00");
	assert.equal(formatDuration(3723), "1:02:03");
	assert.equal(timecodeSeconds("00:04:05.900"), 245);
	assert.equal(formatDuration(timecodeSeconds("01:02:03.456")), "1:02:03");
	const key = textKey({ timerText: "Live in {t}", timerEnd: "Live!" });
	assert.equal(key.timerText(65), "Live in 1:05");
	assert.equal(key.timerText(0), "Live!");
});

// A fake key with the parts of the plugin that Action uses.
function fakeKey(Class, settings, extra = {}) {
	const calls = { images: [], titles: [], states: [], obs: [], stateChanged: 0 };
	const sd = {
		setImage: (c, image) => calls.images.push(image),
		setTitle: (c, title) => calls.titles.push(title),
		setState: (c, state) => calls.states.push(state),
		setSettings() {},
		showAlert() {},
		log() {},
	};
	const plugin = {
		sd,
		actionInfo: new Map([[Class.uuid, { icon: "unmuted", states: ["unmuted", "muted"] }]]),
		stateChanged: () => calls.stateChanged++,
	};
	const key = new Class(plugin, { context: "c", action: Class.uuid, payload: { settings } });
	key.visible = true;
	key.online = true;
	key.call = async (type, data) => calls.obs.push([type, data]);
	Object.assign(key, extra);
	return { key, calls };
}

test("a failed press is kept for the settings page until a press works", async () => {
	const { key, calls } = fakeKey(MuteToggle, { input: "" });
	await key.act(() => key.onPress());
	assert.equal(key.lastError, "no audio source selected");
	assert.equal(calls.stateChanged, 1);
	key.settings.input = "Mic";
	await key.act(() => key.onPress());
	assert.equal(key.lastError, null);
	assert.equal(calls.stateChanged, 2);
});

test("a key that is not set up shows a gray glyph with a yellow dot", () => {
	const { key, calls } = fakeKey(MuteToggle, { input: "" });
	assert.equal(key.isSetUp(), false);
	key.draw();
	assert.ok(Buffer.from(calls.images.at(-1).split(",")[1], "base64").toString().includes("#F5A524"), "yellow dot");
	// Normal images wait until it is set up.
	key.setImage("data:other");
	assert.notEqual(calls.images.at(-1), "data:other");
	key.settings.input = "Mic";
	key.draw();
	assert.equal(key.setupShown, false);
});

test("push to talk: on while held, off again on release", async () => {
	const { key, calls } = fakeKey(MuteToggle, { input: "Mic", mode: "talk" });
	await key.onPress();
	await key.onRelease();
	assert.deepEqual(
		calls.obs.map(([, d]) => d.inputMuted),
		[false, true],
	);
	const hold = fakeKey(MuteToggle, { input: "Mic", mode: "holdMute" });
	await hold.key.onPress();
	await hold.key.onRelease();
	assert.deepEqual(
		hold.calls.obs.map(([, d]) => d.inputMuted),
		[true, false],
	);
});

test("the source name is drawn under the glyph, not as an OpenDeck title", () => {
	const { key, calls } = fakeKey(MuteToggle, { input: "Mic", showName: true });
	key.render();
	assert.equal(calls.titles.at(-1), "", "no second title from OpenDeck");
	const svg = Buffer.from(calls.images.at(-1).split(",")[1], "base64").toString();
	assert.match(svg, />Mic<\/text>/);
});

test("long titles get smaller, then cut", () => {
	assert.equal(titleLine("Mic").size, 22);
	const long = titleLine("Bildschirmaufnahme (PipeWire)");
	assert.ok(long.size < 22);
	assert.ok(long.line.endsWith("…"));
});

test("readable names for filter values and hotkeys", () => {
	assert.equal(filterValueLabel("opacity"), "Opacity");
	assert.equal(filterValueLabel("some_new_value"), "Some new value");
	assert.equal(hotkeyLabel("OBSBasic.StartVirtualCam"), "Start virtual camera");
});
