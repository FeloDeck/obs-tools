// Second page: monitoring, set volume, scene above, and an OpenDeck multi action.
// Needs the profile from layout-extra.json.
import { ObsClient } from "../../de.felitendo.obs-tools.sdPlugin/plugin/lib/obs.js";
import { readLocalObsConfig } from "../../de.felitendo.obs-tools.sdPlugin/plugin/lib/connections.js";
import { faderToDb } from "../../de.felitendo.obs-tools.sdPlugin/plugin/lib/volume.js";

const DECK = process.env.VIRTUAL_DECK_URL || "http://127.0.0.1:57190";
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const press = (key, ms = 80) => fetch(`${DECK}/api/key/${key}/press?ms=${ms}`, { method: "POST" });
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
	throw new Error(`timeout waiting for ${what}`);
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
const input = "Desktop-Audio";

await call("SetCurrentProgramScene", { sceneName: "Szene" });
await call("SetInputAudioMonitorType", { inputName: input, monitorType: "OBS_MONITORING_TYPE_NONE" });
await sleep(500);

await test("monitor set, press again turns it off", async () => {
	await press(0);
	await waitFor("monitor only", async () => (await call("GetInputAudioMonitorType", { inputName: input })).monitorType === "OBS_MONITORING_TYPE_MONITOR_ONLY");
	await sleep(300);
	await press(0);
	await waitFor("monitor off", async () => (await call("GetInputAudioMonitorType", { inputName: input })).monitorType === "OBS_MONITORING_TYPE_NONE");
});

await test("set volume to 50% of the fader", async () => {
	await call("SetInputVolume", { inputName: input, inputVolumeDb: 0 });
	await press(8);
	const target = faderToDb(0.5);
	await waitFor(`${target.toFixed(2)} dB`, async () => Math.abs((await call("GetInputVolume", { inputName: input })).inputVolumeDb - target) < 0.05);
	await call("SetInputVolume", { inputName: input, inputVolumeDb: 0 });
});

await test("scene above (previous in the list)", async () => {
	const { scenes } = await call("GetSceneList");
	const order = [...scenes].sort((a, b) => b.sceneIndex - a.sceneIndex).map((s) => s.sceneName);
	const expected = order[(order.indexOf("Szene") - 1 + order.length) % order.length];
	await press(5);
	await waitFor(`program = ${expected}`, async () => (await call("GetCurrentProgramScene")).sceneName === expected);
	await call("SetCurrentProgramScene", { sceneName: "Szene" });
	await sleep(300);
});

await test("multi action: studio on, mute, scene goes live directly", async () => {
	await call("SetStudioModeEnabled", { studioModeEnabled: false });
	await call("SetInputMute", { inputName: input, inputMuted: false });
	await sleep(300);
	await press(9);
	await waitFor("studio mode on", async () => (await call("GetStudioModeEnabled")).studioModeEnabled);
	await waitFor("muted", async () => (await call("GetInputMute", { inputName: input })).inputMuted);
	await waitFor("program = Scene 2", async () => (await call("GetCurrentProgramScene")).sceneName === "Scene 2", 6000);
	// "enable" and "mute" must not toggle back on a second run.
	await sleep(500);
	await press(9);
	await sleep(1200);
	if (!(await call("GetStudioModeEnabled")).studioModeEnabled) throw new Error("second run turned studio mode off");
	if (!(await call("GetInputMute", { inputName: input })).inputMuted) throw new Error("second run unmuted");
	await call("SetStudioModeEnabled", { studioModeEnabled: false });
	await call("SetInputMute", { inputName: input, inputMuted: false });
	await call("SetCurrentProgramScene", { sceneName: "Szene" });
});

await test("image source: picture per key, auto hide, press again hides", async () => {
	const { sceneItemId } = await call("GetSceneItemId", { sceneName: "Szene", sourceName: "Picture" });
	const shown = async () => (await call("GetSceneItemEnabled", { sceneName: "Szene", sceneItemId })).sceneItemEnabled;
	const file = async () => (await call("GetInputSettings", { inputName: "Picture" })).inputSettings.file;
	await call("SetSceneItemEnabled", { sceneName: "Szene", sceneItemId, sceneItemEnabled: false });
	await sleep(300);
	await press(12);
	await waitFor("GG shown", async () => (await shown()) && (await file()).endsWith("obstools-gg.png"));
	await waitFor("hidden after 2 s", async () => !(await shown()), 4000);
	await press(13);
	await waitFor("BRB shown", async () => (await shown()) && (await file()).endsWith("obstools-brb.png"));
	await sleep(400);
	await press(13);
	await waitFor("hidden by second press", async () => !(await shown()));
});

await test("set text: count down from 3, stops at 0, double press, hold", async () => {
	const text = async () => (await call("GetInputSettings", { inputName: "Title Text" })).inputSettings.text;
	const pressAndWait = async (expected, ms) => {
		await press(14, ms);
		await waitFor(expected, async () => (await text()) === expected);
		// A pause, or the next press would make a double press.
		await sleep(500);
	};
	await pressAndWait("Live in 3", 1000);
	await pressAndWait("Live in 2");
	await pressAndWait("Live in 1");
	await pressAndWait("Live in 0");
	await pressAndWait("Live in 0");
	await press(14);
	await press(14);
	await waitFor("Live in 1 (double press counts up)", async () => (await text()) === "Live in 1");
	await sleep(500);
	await pressAndWait("Live in 3", 1000);
});

obs.stop();
console.log(failed ? `${failed} failed` : "all passed");
process.exit(failed ? 1 : 0);
