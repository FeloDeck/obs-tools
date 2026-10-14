// Push to talk, take preview live, the text timer, setup images, names as titles,
// and press, hold and double press on other keys.
// Needs the profile from layout-more.json and the scenes from setup-obs.mjs.
import { execFileSync } from "node:child_process";

import { ObsClient } from "../../de.felitendo.obs-tools.sdPlugin/plugin/lib/obs.js";
import { readLocalObsConfig } from "../../de.felitendo.obs-tools.sdPlugin/plugin/lib/connections.js";

const DECK = process.env.VIRTUAL_DECK_URL || "http://127.0.0.1:57190";
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const post = (p) => fetch(DECK + p, { method: "POST" });
const press = (key, ms = 80) => post(`/api/key/${key}/press?ms=${ms}`);
const cfg = readLocalObsConfig();
const obs = new ObsClient({ port: cfg.port, password: cfg.password });
await new Promise((resolve) => (obs.once("connected", resolve), obs.start()));
const call = (t, d) => obs.call(t, d);
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
async function test(name, fn) {
	try {
		await fn();
		console.log(`ok   ${name}`);
	} catch (error) {
		failed++;
		console.log(`FAIL ${name}: ${error.message}`);
	}
}
const muted = async () => (await call("GetInputMute", { inputName: "Desktop-Audio" })).inputMuted;
const text = async () => (await call("GetInputSettings", { inputName: "Title Text" })).inputSettings.text;
// Color of one pixel of a key, as OpenDeck drew it (the deck sends JPEG).
async function pixel(key, x, y) {
	const png = Buffer.from(await (await fetch(`${DECK}/api/key/${key}.png`)).arrayBuffer());
	return execFileSync("magick", ["-", "-format", `%[pixel:p{${x},${y}}]`, "info:"], { input: png }).toString();
}

await test("push to talk: on while held, muted again after", async () => {
	await call("SetInputMute", { inputName: "Desktop-Audio", inputMuted: true });
	await sleep(300);
	const holding = post("/api/key/0/press?ms=1200");
	await waitFor("on while held", async () => !(await muted()));
	await holding;
	await waitFor("muted after release", muted);
});

await test("hold to mute: muted while held, on again after", async () => {
	await call("SetInputMute", { inputName: "Desktop-Audio", inputMuted: false });
	await sleep(300);
	const holding = post("/api/key/1/press?ms=1200");
	await waitFor("muted while held", muted);
	await holding;
	await waitFor("on after release", async () => !(await muted()));
});

await test("take preview live", async () => {
	await call("SetCurrentProgramScene", { sceneName: "Szene" });
	await call("SetStudioModeEnabled", { studioModeEnabled: true });
	await sleep(500);
	await call("SetCurrentPreviewScene", { sceneName: "Scene 2" });
	await sleep(300);
	await press(2);
	await waitFor("Scene 2 live", async () => (await call("GetCurrentProgramScene")).sceneName === "Scene 2", 6000);
	await call("SetStudioModeEnabled", { studioModeEnabled: false });
	await call("SetCurrentProgramScene", { sceneName: "Szene" });
	await sleep(500);
});

await test("text timer counts down to the end text, a second press stops it", async () => {
	await press(3);
	await waitFor("Live in 0:02", async () => (await text()) === "Live in 0:02");
	await waitFor("Live!", async () => (await text()) === "Live!", 5000);
	await press(3);
	await waitFor("Live in 0:03", async () => (await text()) === "Live in 0:03");
	await sleep(400);
	await press(3);
	await sleep(2500);
	if ((await text()) === "Live!") throw new Error("the timer did not stop");
});

await test("keys that are not set up show the yellow dot", async () => {
	for (const key of [4, 8]) {
		const color = await pixel(key, 120, 24);
		if (!/srgb\((2[2-5]\d),(1[4-9]\d),\d{1,2}\)/.test(color)) throw new Error(`key ${key}: ${color}`);
	}
	// A set up key has no dot there.
	if (/srgb\(2[2-5]\d,1[4-9]\d/.test(await pixel(5, 120, 24))) throw new Error("key 5 has a dot");
});

await test("press, hold and double press on a visibility key", async () => {
	const { sceneItemId } = await call("GetSceneItemId", { sceneName: "Szene", sourceName: "Title Text" });
	const shown = async () => (await call("GetSceneItemEnabled", { sceneName: "Szene", sceneItemId })).sceneItemEnabled;
	await call("SetSceneItemEnabled", { sceneName: "Szene", sceneItemId, sceneItemEnabled: true });
	await sleep(400);
	await press(10, 1000);
	await waitFor("hidden by hold", async () => !(await shown()));
	await sleep(500);
	await press(10);
	await press(10);
	await waitFor("shown by double press", shown);
	await sleep(500);
	await press(10);
	await waitFor("toggled by press (after the double press wait)", async () => !(await shown()));
	await call("SetSceneItemEnabled", { sceneName: "Szene", sceneItemId, sceneItemEnabled: true });
});

await test("press, hold and double press on a media key", async () => {
	const { sceneItemId } = await call("GetSceneItemId", { sceneName: "Szene", sourceName: "Clip Player" });
	await call("SetSceneItemEnabled", { sceneName: "Szene", sceneItemId, sceneItemEnabled: true });
	await sleep(800);
	const state = async () => (await call("GetMediaInputStatus", { inputName: "Clip Player" })).mediaState;
	await press(9, 1000);
	await waitFor("stopped by hold", async () => ["OBS_MEDIA_STATE_STOPPED", "OBS_MEDIA_STATE_ENDED"].includes(await state()));
	await sleep(500);
	await press(9);
	await press(9);
	await waitFor("playing again by double press", async () => (await state()) === "OBS_MEDIA_STATE_PLAYING");
	await call("SetSceneItemEnabled", { sceneName: "Szene", sceneItemId, sceneItemEnabled: false });
});

obs.stop();
console.log(failed ? `${failed} failed` : "all passed");
process.exit(failed ? 1 : 0);
