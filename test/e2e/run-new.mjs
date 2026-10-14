// End-to-end test for the everyday actions, stream health, meters and dials.
// Needs the profile from layout-new.json and the scenes from setup-obs.mjs.
// The stream test starts a local RTMP server with ffmpeg and streams to it for a few seconds.
import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { ObsClient } from "../../de.felitendo.obs-tools.sdPlugin/plugin/lib/obs.js";
import { readLocalObsConfig } from "../../de.felitendo.obs-tools.sdPlugin/plugin/lib/connections.js";

const DECK = process.env.VIRTUAL_DECK_URL || "http://127.0.0.1:57190";
const filter = process.argv[2] ?? "";
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const post = (p) => fetch(DECK + p, { method: "POST" });
const press = (key, ms = 80) => post(`/api/key/${key}/press?ms=${ms}`);
const keyImage = async (key) => (await (await fetch(`${DECK}/api/state`)).json()).keys[key];
const dialImage = async (dial) => (await (await fetch(`${DECK}/api/state`)).json()).dials[dial];

const cfg = readLocalObsConfig();
const obs = new ObsClient({ port: cfg.port, password: cfg.password });
await new Promise((resolve) => (obs.once("connected", resolve), obs.start()));
const call = (t, d) => obs.call(t, d);
const pluginLog = path.join(os.homedir(), ".local/share/opendeck/logs/plugins/de.felitendo.obs-tools.sdPlugin.log");
const logSize = () => fs.statSync(pluginLog).size;
const logSince = (size) => fs.readFileSync(pluginLog, "utf8").slice(size);

async function waitFor(what, fn, timeout = 5000) {
	const until = Date.now() + timeout;
	let last;
	while (Date.now() < until) {
		if ((last = await fn())) return last;
		await sleep(100);
	}
	throw new Error(`timeout waiting for ${what} (last: ${JSON.stringify(last)})`);
}
let failed = 0;
let count = 0;
async function test(name, fn) {
	if (filter && !name.includes(filter)) return;
	count++;
	try {
		await fn();
		console.log(`ok   ${name}`);
	} catch (error) {
		failed++;
		console.log(`FAIL ${name}: ${error.message}`);
	}
}
const text = async () => (await call("GetInputSettings", { inputName: "Title Text" })).inputSettings.text;
const item = async (scene, source) => (await call("GetSceneItemId", { sceneName: scene, sourceName: source })).sceneItemId;

await call("SetCurrentProgramScene", { sceneName: "Szene" });
await sleep(500);

await test("source visibility toggles", async () => {
	const id = await item("Szene", "Title Text");
	await call("SetSceneItemEnabled", { sceneName: "Szene", sceneItemId: id, sceneItemEnabled: true });
	await sleep(300);
	await press(0);
	await waitFor("hidden", async () => !(await call("GetSceneItemEnabled", { sceneName: "Szene", sceneItemId: id })).sceneItemEnabled);
	await press(0);
	await waitFor("visible", async () => (await call("GetSceneItemEnabled", { sceneName: "Szene", sceneItemId: id })).sceneItemEnabled);
});

await test("hotkey by name with a source", async () => {
	await call("SetInputMute", { inputName: "Desktop-Audio", inputMuted: false });
	await press(1);
	await waitFor("muted by hotkey", async () => (await call("GetInputMute", { inputName: "Desktop-Audio" })).inputMuted);
	await call("SetInputMute", { inputName: "Desktop-Audio", inputMuted: false });
});

await test("screenshot saves a file", async () => {
	const dir = path.join(os.homedir(), "Pictures/obstools-test");
	fs.mkdirSync(dir, { recursive: true });
	const before = new Set(fs.readdirSync(dir));
	await press(2);
	const file = await waitFor("new file", async () => fs.readdirSync(dir).find((f) => !before.has(f)));
	console.log(`     ${file}`);
});

await test("set text: fixed, counter, double press, hold to reset, list", async () => {
	await press(9);
	await waitFor("Live in 5", async () => (await text()) === "Live in 5");
	// This counter counts down on a double press, so single presses need a pause between them.
	await press(10, 1000);
	await waitFor("Deaths: 0", async () => (await text()) === "Deaths: 0");
	await press(10);
	await sleep(500);
	await press(10);
	await waitFor("Deaths: 2", async () => (await text()) === "Deaths: 2");
	await sleep(500);
	await press(10);
	await press(10);
	await waitFor("Deaths: 1 (double press)", async () => (await text()) === "Deaths: 1");
	await press(10, 1000);
	await waitFor("Deaths: 0", async () => (await text()) === "Deaths: 0");
	// The list key goes One, Two, Three, One... from wherever it is now.
	const lines = ["One", "Two", "Three"];
	await press(11);
	const first = await waitFor("a line of the list", async () => {
		const now = await text();
		return lines.includes(now) && now;
	});
	await press(11);
	const next = lines[(lines.indexOf(first) + 1) % lines.length];
	await waitFor(next, async () => (await text()) === next);
});

await test("media control: play, pause, stop", async () => {
	const id = await item("Szene", "Clip Player");
	await call("SetSceneItemEnabled", { sceneName: "Szene", sceneItemId: id, sceneItemEnabled: true });
	// Showing the source restarts it in OBS, so stop it only after that.
	await sleep(800);
	await call("TriggerMediaInputAction", { inputName: "Clip Player", mediaAction: "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_STOP" });
	await sleep(500);
	const state = async () => (await call("GetMediaInputStatus", { inputName: "Clip Player" })).mediaState;
	await press(5);
	await waitFor("playing", async () => (await state()) === "OBS_MEDIA_STATE_PLAYING");
	await sleep(700);
	await press(5);
	await waitFor("paused", async () => (await state()) === "OBS_MEDIA_STATE_PAUSED");
	await press(6);
	await waitFor("stopped", async () => ["OBS_MEDIA_STATE_STOPPED", "OBS_MEDIA_STATE_ENDED"].includes(await state()));
	await call("SetSceneItemEnabled", { sceneName: "Szene", sceneItemId: id, sceneItemEnabled: false });
});

await test("profile switch and back", async () => {
	await press(7);
	await waitFor("profile Test 2", async () => (await call("GetProfileList")).currentProfileName === "Test 2", 8000);
	await sleep(1500);
	await press(8);
	await waitFor("profile Unbenannt", async () => (await call("GetProfileList")).currentProfileName === "Unbenannt", 8000);
	await sleep(1500);
});

await test("recording marker: chapter and split while recording", async () => {
	const size = logSize();
	await press(3);
	await sleep(500);
	if (!/RecordTools: error: not recording/.test(logSince(size))) throw new Error("chapter while not recording should fail");
	await press(14);
	await waitFor("recording", async () => (await call("GetRecordStatus")).outputActive, 6000);
	await sleep(1500);
	const before = logSize();
	await press(3);
	await sleep(800);
	await press(4);
	await sleep(1500);
	const log = logSince(before);
	// The chapter must work. Split only works with file splitting on in OBS, else OBS says so.
	if (/chapter|hybrid/i.test(log)) throw new Error(`chapter failed: ${log.trim()}`);
	console.log(`     chapter ok; split: ${/file splitting/.test(log) ? "OBS says file splitting is off (expected here)" : "ok"}`);
	await press(14);
	await waitFor("stopped", async () => !(await call("GetRecordStatus")).outputActive, 8000);
});

await test("source volume key and dial show a live meter", async () => {
	const id = await item("Szene", "Clip Player");
	const quietKey = await keyImage(13);
	const quietDial = await dialImage(0);
	await call("SetInputSettings", { inputName: "Clip Player", inputSettings: { looping: true }, overlay: true });
	await call("SetSceneItemEnabled", { sceneName: "Szene", sceneItemId: id, sceneItemEnabled: true });
	await sleep(800);
	await call("TriggerMediaInputAction", { inputName: "Clip Player", mediaAction: "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_RESTART" });
	try {
		await waitFor("key meter moves", async () => (await keyImage(13)) !== quietKey, 4000);
		await waitFor("dial meter moves", async () => (await dialImage(0)) !== quietDial, 4000);
	} finally {
		await call("TriggerMediaInputAction", { inputName: "Clip Player", mediaAction: "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_STOP" });
		await call("SetInputSettings", { inputName: "Clip Player", inputSettings: { looping: false }, overlay: true });
		await call("SetSceneItemEnabled", { sceneName: "Szene", sceneItemId: id, sceneItemEnabled: false });
	}
});

await test("transition dial: time and next transition", async () => {
	await call("SetCurrentSceneTransition", { transitionName: "Überblende" });
	await call("SetCurrentSceneTransitionDuration", { transitionDuration: 300 });
	await sleep(400);
	await post("/api/dial/1/rotate?ticks=2");
	await waitFor("400 ms", async () => (await call("GetCurrentSceneTransition")).transitionDuration === 400);
	await post("/api/dial/1/press");
	await waitFor("next transition", async () => (await call("GetCurrentSceneTransition")).transitionName !== "Überblende");
	await call("SetCurrentSceneTransition", { transitionName: "Überblende" });
	await call("SetCurrentSceneTransitionDuration", { transitionDuration: 300 });
});

await test("scene dial: turn picks, press switches", async () => {
	await call("SetCurrentProgramScene", { sceneName: "Szene" });
	await sleep(500);
	const { scenes } = await call("GetSceneList");
	const order = [...scenes].sort((a, b) => b.sceneIndex - a.sceneIndex).map((s) => s.sceneName);
	const target = order[(order.indexOf("Szene") + 2) % order.length];
	await post("/api/dial/2/rotate?ticks=2");
	await sleep(600);
	if ((await call("GetCurrentProgramScene")).sceneName !== "Szene") throw new Error("turning should not switch in pick mode");
	await post("/api/dial/2/press");
	await waitFor(`program = ${target}`, async () => (await call("GetCurrentProgramScene")).sceneName === target);
	await call("SetCurrentProgramScene", { sceneName: "Szene" });
});

await test("filter dial: value, on and off", async () => {
	const get = async () => await call("GetSourceFilter", { sourceName: "Box", filterName: "Look" });
	await call("SetSourceFilterSettings", { sourceName: "Box", filterName: "Look", filterSettings: { opacity: 1 }, overlay: true });
	await sleep(400);
	await post("/api/dial/3/rotate?ticks=-5");
	await waitFor("opacity 0.95", async () => Math.abs(((await get()).filterSettings.opacity ?? 1) - 0.95) < 0.001);
	await post("/api/dial/3/press");
	await waitFor("filter off", async () => !(await get()).filterEnabled);
	await post("/api/dial/3/press");
	await waitFor("filter on", async () => (await get()).filterEnabled);
	await call("SetSourceFilterSettings", { sourceName: "Box", filterName: "Look", filterSettings: { opacity: 1 }, overlay: true });
});

await test("stream health while streaming to a local server", async () => {
	const old = await call("GetStreamServiceSettings");
	const server = spawn("ffmpeg", ["-hide_banner", "-loglevel", "error", "-listen", "1", "-i", "rtmp://127.0.0.1:19350/live/test", "-f", "null", "-"], { stdio: "ignore" });
	try {
		await sleep(800);
		await call("SetStreamServiceSettings", { streamServiceType: "rtmp_custom", streamServiceSettings: { server: "rtmp://127.0.0.1:19350/live", key: "test" } });
		const before = await keyImage(12);
		await call("StartStream");
		await waitFor("streaming", async () => (await call("GetStreamStatus")).outputActive, 10000);
		await sleep(4000);
		const live = await keyImage(12);
		if (live === before) throw new Error("key did not change while live");
		if (process.env.SHOTS_DIR) fs.writeFileSync(path.join(process.env.SHOTS_DIR, "stream-health.png"), Buffer.from(live.split(",")[1], "base64"));
		const status = await call("GetStreamStatus");
		console.log(`     ${status.outputTimecode}, ${status.outputBytes} bytes, congestion ${status.outputCongestion}`);
		await call("StopStream");
		await waitFor("stopped", async () => !(await call("GetStreamStatus")).outputActive, 10000);
	} finally {
		server.kill();
		await call("SetStreamServiceSettings", { streamServiceType: old.streamServiceType, streamServiceSettings: old.streamServiceSettings }).catch(() => {});
	}
});

obs.stop();
console.log(failed ? `\n${failed} of ${count} failed` : `\nall ${count} passed`);
process.exit(failed ? 1 : 0);
