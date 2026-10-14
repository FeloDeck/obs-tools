// End-to-end test: presses keys on the virtual deck (through OpenDeck) and checks OBS.
// Needs: OBS with the test scenes (test/e2e/setup-obs.mjs), OpenDeck with both plugins
// and the profile from layout-main.json.
//
//   node test/e2e/run.mjs [test name filter]
import fs from "node:fs";
import { ObsClient } from "../../de.felitendo.obs-tools.sdPlugin/plugin/lib/obs.js";
import { readLocalObsConfig } from "../../de.felitendo.obs-tools.sdPlugin/plugin/lib/connections.js";

const DECK = process.env.VIRTUAL_DECK_URL || "http://127.0.0.1:57190";
const SHOTS = process.env.SHOTS_DIR || "";
const filter = process.argv[2] ?? "";

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const post = (path) => fetch(DECK + path, { method: "POST" });
const press = (key, ms = 80) => post(`/api/key/${key}/press?ms=${ms}`);
const hold = (key, ms) => press(key, ms);
const deckState = async () => (await fetch(`${DECK}/api/state`)).json();

const cfg = readLocalObsConfig();
const obs = new ObsClient({ port: cfg.port, password: cfg.password });
await new Promise((resolve) => {
	obs.once("connected", resolve);
	obs.start();
});
const call = (type, data) => obs.call(type, data);
const program = async () => (await call("GetCurrentProgramScene")).sceneName;

async function waitFor(what, fn, timeout = 5000) {
	const until = Date.now() + timeout;
	let last;
	while (Date.now() < until) {
		last = await fn();
		if (last) return last;
		await sleep(100);
	}
	throw new Error(`timeout waiting for ${what} (last: ${JSON.stringify(last)})`);
}

async function keyImage(key) {
	const state = await deckState();
	return state.keys[key];
}

async function shot(name) {
	if (!SHOTS) return;
	fs.mkdirSync(SHOTS, { recursive: true });
	const state = await deckState();
	fs.writeFileSync(`${SHOTS}/${name}.json`, JSON.stringify(state));
}

const results = [];
async function test(name, fn) {
	if (filter && !name.includes(filter)) return;
	const start = Date.now();
	try {
		await fn();
		results.push({ name, ok: true });
		console.log(`ok   ${name} (${Date.now() - start} ms)`);
	} catch (error) {
		results.push({ name, ok: false, error: error.message });
		console.log(`FAIL ${name}: ${error.message}`);
	}
}

function assert(condition, message) {
	if (!condition) throw new Error(message);
}

// ---------- tests ----------

await call("SetStudioModeEnabled", { studioModeEnabled: false });
await call("SetCurrentProgramScene", { sceneName: "Szene" });
await sleep(800);

await test("smart scene switches the live scene", async () => {
	const before = await keyImage(1);
	await press(1);
	await waitFor("program = Scene 2", async () => (await program()) === "Scene 2");
	await waitFor("key 1 image changes", async () => (await keyImage(1)) !== before);
	await shot("smartscene-live");
});

await test("previous scene goes back", async () => {
	await sleep(300);
	await press(3);
	await waitFor("program = Szene", async () => (await program()) === "Szene");
});

await test("next scene follows the scene list", async () => {
	const { scenes } = await call("GetSceneList");
	const order = [...scenes].sort((a, b) => b.sceneIndex - a.sceneIndex).map((s) => s.sceneName);
	const expected = order[(order.indexOf("Szene") + 1) % order.length];
	await press(4);
	await waitFor(`program = ${expected}`, async () => (await program()) === expected);
	await call("SetCurrentProgramScene", { sceneName: "Szene" });
	await sleep(300);
});

await test("studio mode: preview first, then live", async () => {
	await press(2);
	await waitFor("studio mode on", async () => (await call("GetStudioModeEnabled")).studioModeEnabled);
	await sleep(400);
	await press(1);
	await waitFor("preview = Scene 2", async () => (await call("GetCurrentPreviewScene")).sceneName === "Scene 2");
	await sleep(400);
	await shot("smartscene-preview");
	assert((await program()) === "Szene", "program changed too early");
	await press(1);
	await waitFor("program = Scene 2", async () => (await program()) === "Scene 2", 6000);
	await press(2);
	await waitFor("studio mode off", async () => !(await call("GetStudioModeEnabled")).studioModeEnabled);
	await call("SetCurrentProgramScene", { sceneName: "Szene" });
	await sleep(300);
});

await test("source volume: press and hold", async () => {
	await call("SetInputVolume", { inputName: "Desktop-Audio", inputVolumeDb: 0 });
	await sleep(300);
	await press(7);
	await waitFor("-3 dB", async () => Math.abs((await call("GetInputVolume", { inputName: "Desktop-Audio" })).inputVolumeDb + 3) < 0.05);
	await hold(7, 1100);
	const db = (await call("GetInputVolume", { inputName: "Desktop-Audio" })).inputVolumeDb;
	assert(db <= -12, `holding should repeat, volume is ${db} dB`);
});

await test("dial turns volume and mutes", async () => {
	await call("SetInputVolume", { inputName: "Desktop-Audio", inputVolumeDb: -10 });
	await sleep(300);
	await post("/api/dial/0/rotate?ticks=3");
	await waitFor("-7 dB", async () => Math.abs((await call("GetInputVolume", { inputName: "Desktop-Audio" })).inputVolumeDb + 7) < 0.05);
	await post("/api/dial/0/press");
	await waitFor("muted", async () => (await call("GetInputMute", { inputName: "Desktop-Audio" })).inputMuted);
	await sleep(500);
	await shot("dial-muted");
	await post("/api/touch/0");
	await waitFor("unmuted", async () => !(await call("GetInputMute", { inputName: "Desktop-Audio" })).inputMuted);
	await call("SetInputVolume", { inputName: "Desktop-Audio", inputVolumeDb: 0 });
});

await test("mute toggle", async () => {
	await press(8);
	await waitFor("muted", async () => (await call("GetInputMute", { inputName: "Desktop-Audio" })).inputMuted);
	await sleep(400);
	await shot("muted");
	await press(8);
	await waitFor("unmuted", async () => !(await call("GetInputMute", { inputName: "Desktop-Audio" })).inputMuted);
});

await test("set transition", async () => {
	await call("SetCurrentSceneTransition", { transitionName: "Überblende" });
	await sleep(300);
	await press(9);
	await waitFor("transition = Schnitt", async () => (await call("GetCurrentSceneTransition")).transitionName === "Schnitt");
	await call("SetCurrentSceneTransition", { transitionName: "Überblende" });
});

await test("filter toggle", async () => {
	await call("SetSourceFilterEnabled", { sourceName: "Box", filterName: "Tint", filterEnabled: true });
	await sleep(300);
	await press(14);
	await waitFor("filter off", async () => !(await call("GetSourceFilter", { sourceName: "Box", filterName: "Tint" })).filterEnabled);
	await press(14);
	await waitFor("filter on", async () => (await call("GetSourceFilter", { sourceName: "Box", filterName: "Tint" })).filterEnabled);
});

await test("source animation plays and resets", async () => {
	const { sceneItemId } = await call("GetSceneItemId", { sceneName: "Szene", sourceName: "Box" });
	const start = (await call("GetSceneItemTransform", { sceneName: "Szene", sceneItemId })).sceneItemTransform;
	await press(13);
	await sleep(700);
	const mid = (await call("GetSceneItemTransform", { sceneName: "Szene", sceneItemId })).sceneItemTransform;
	assert(mid.positionX > start.positionX + 50, `should be moving (x ${start.positionX} -> ${mid.positionX})`);
	assert(mid.positionX < start.positionX + 600, `should not be done yet (x ${mid.positionX})`);
	await sleep(1300);
	const peak = (await call("GetSceneItemTransform", { sceneName: "Szene", sceneItemId })).sceneItemTransform;
	await waitFor("back at start", async () => {
		const t = (await call("GetSceneItemTransform", { sceneName: "Szene", sceneItemId })).sceneItemTransform;
		return Math.abs(t.positionX - start.positionX) < 0.5 && Math.abs(t.rotation - start.rotation) < 0.5 && Math.abs(t.scaleX - start.scaleX) < 0.01;
	}, 4000);
	console.log(`     x: ${start.positionX} -> ${mid.positionX.toFixed(1)} (0.7 s) -> back; peak sample scale ${peak.scaleX.toFixed(2)}`);
});

await test("video player plays, hides when done", async () => {
	const { sceneItemId } = await call("GetSceneItemId", { sceneName: "Szene", sourceName: "Clip Player" });
	await call("SetSceneItemEnabled", { sceneName: "Szene", sceneItemId, sceneItemEnabled: false });
	await press(12);
	await waitFor("visible", async () => (await call("GetSceneItemEnabled", { sceneName: "Szene", sceneItemId })).sceneItemEnabled);
	await waitFor("playing", async () => (await call("GetMediaInputStatus", { inputName: "Clip Player" })).mediaState === "OBS_MEDIA_STATE_PLAYING");
	await sleep(600);
	await shot("video-playing");
	await waitFor("hidden after the end", async () => !(await call("GetSceneItemEnabled", { sceneName: "Szene", sceneItemId })).sceneItemEnabled, 9000);
});

await test("video player: second press stops", async () => {
	const { sceneItemId } = await call("GetSceneItemId", { sceneName: "Szene", sourceName: "Clip Player" });
	await press(12);
	await waitFor("visible", async () => (await call("GetSceneItemEnabled", { sceneName: "Szene", sceneItemId })).sceneItemEnabled);
	await sleep(800);
	await press(12);
	await waitFor("hidden", async () => !(await call("GetSceneItemEnabled", { sceneName: "Szene", sceneItemId })).sceneItemEnabled, 3000);
});

await test("instant replay: hold starts buffer, press replays in slow motion", async () => {
	if ((await call("GetReplayBufferStatus")).outputActive) await call("StopReplayBuffer");
	await sleep(500);
	await hold(11, 1000);
	await waitFor("replay buffer on", async () => (await call("GetReplayBufferStatus")).outputActive, 6000);
	await sleep(500);
	await shot("replay-armed");
	await sleep(3000);
	await call("SetCurrentProgramScene", { sceneName: "Szene" });
	await sleep(300);
	await press(11);
	await waitFor("switched to Replay", async () => (await program()) === "Replay", 10000);
	const settings = await waitFor("replay file loaded", async () => {
		const s = (await call("GetInputSettings", { inputName: "Replay Player" })).inputSettings;
		return s.speed_percent === 50 && s;
	});
	assert(settings.speed_percent === 50, `speed should be 50, is ${settings.speed_percent}`);
	assert(/Videos\/Replay/.test(settings.local_file ?? ""), `replay file not set: ${settings.local_file}`);
	console.log(`     playing ${settings.local_file} at ${settings.speed_percent}%`);
	await waitFor("back to Szene after the replay", async () => (await program()) === "Szene", 20000);
	await hold(11, 1000);
	await waitFor("replay buffer off", async () => !(await call("GetReplayBufferStatus")).outputActive, 6000);
});

await test("record toggle shows a timer", async () => {
	await press(10);
	await waitFor("recording", async () => (await call("GetRecordStatus")).outputActive, 6000);
	await sleep(2500);
	await shot("recording");
	await press(10);
	const stopped = await waitFor("stopped", async () => {
		const s = await call("GetRecordStatus");
		return !s.outputActive && s;
	}, 8000);
	assert(stopped, "still recording");
});

await test("scene rename follows into the key settings", async () => {
	await call("SetSceneName", { sceneName: "Scene 2", newSceneName: "Scene Two" });
	await sleep(800);
	await press(1);
	try {
		await waitFor("program = Scene Two", async () => (await program()) === "Scene Two");
	} finally {
		await call("SetSceneName", { sceneName: "Scene Two", newSceneName: "Scene 2" });
		await sleep(500);
		await call("SetCurrentProgramScene", { sceneName: "Szene" });
	}
});

obs.stop();
const failed = results.filter((r) => !r.ok);
console.log(`\n${results.length - failed.length}/${results.length} passed`);
process.exit(failed.length ? 1 : 0);
