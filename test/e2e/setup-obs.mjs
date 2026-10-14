// Creates the scenes and sources the end-to-end tests expect, in the OBS on this PC.
// Run once: node test/e2e/setup-obs.mjs (needs ~/Videos/obstools-test.mp4, see test/e2e/make-video.sh)
import { ObsClient } from "../../de.felitendo.obs-tools.sdPlugin/plugin/lib/obs.js";
import { readLocalObsConfig } from "../../de.felitendo.obs-tools.sdPlugin/plugin/lib/connections.js";
const cfg = readLocalObsConfig();
const c = new ObsClient({ port: cfg.port, password: cfg.password });
const call = async (t, d) => { try { return await c.call(t, d); } catch (e) { console.log("  skip", t, e.message); } };
c.on("connected", async () => {
  const scenes = (await call("GetSceneList")).scenes.map(s => s.sceneName);
  for (const s of ["Szene", "Scene 2", "Replay"]) if (!scenes.includes(s)) await call("CreateScene", { sceneName: s });
  await call("CreateInput", { sceneName: "Scene 2", inputName: "Blue BG", inputKind: "color_source_v3", inputSettings: { color: 0xffd08030, width: 1280, height: 800 } });
  await call("CreateInput", { sceneName: "Szene", inputName: "Dark BG", inputKind: "color_source_v3", inputSettings: { color: 0xff302020, width: 1280, height: 800 } });
  await call("CreateInput", { sceneName: "Szene", inputName: "Box", inputKind: "color_source_v3", inputSettings: { color: 0xff3030e0, width: 200, height: 200 } });
  await call("CreateInput", { sceneName: "Szene", inputName: "Clip Player", inputKind: "ffmpeg_source", inputSettings: { local_file: `${process.env.HOME}/Videos/obstools-test.mp4`, is_local_file: true } });
  await call("CreateInput", { sceneName: "Replay", inputName: "Replay Player", inputKind: "ffmpeg_source", inputSettings: {} });
  await call("CreateSourceFilter", { sourceName: "Box", filterName: "Tint", filterKind: "color_filter_v2", filterSettings: { color_multiply: 0xff00ff00 } });
  await call("CreateSourceFilter", { sourceName: "Box", filterName: "Look", filterKind: "color_filter_v2", filterSettings: { opacity: 1 } });
  await call("CreateInput", { sceneName: "Szene", inputName: "Picture", inputKind: "image_source", inputSettings: { file: `${process.env.HOME}/Pictures/obstools-brb.png` } });
  await call("CreateInput", { sceneName: "Szene", inputName: "Title Text", inputKind: "text_ft2_source_v2", inputSettings: { text: "Hello" } });
  const profiles = (await call("GetProfileList")).profiles;
  if (!profiles.includes("Test 2")) {
    const current = (await call("GetProfileList")).currentProfileName;
    await call("CreateProfile", { profileName: "Test 2" });
    await call("SetCurrentProfile", { profileName: current });
  }
  // order: Dark BG at bottom
  const items = (await call("GetSceneItemList", { sceneName: "Szene" })).sceneItems;
  console.log("Szene items:", items.map(i => `${i.sceneItemId}:${i.sourceName}`).join(", "));
  const dark = items.find(i => i.sourceName === "Dark BG"); if (dark) await call("SetSceneItemIndex", { sceneName: "Szene", sceneItemId: dark.sceneItemId, sceneItemIndex: 0 });
  const box = items.find(i => i.sourceName === "Box"); if (box) await call("SetSceneItemTransform", { sceneName: "Szene", sceneItemId: box.sceneItemId, sceneItemTransform: { positionX: 100, positionY: 100 } });
  const clip = items.find(i => i.sourceName === "Clip Player"); if (clip) { await call("SetSceneItemEnabled", { sceneName: "Szene", sceneItemId: clip.sceneItemId, sceneItemEnabled: false }); await call("SetSceneItemTransform", { sceneName: "Szene", sceneItemId: clip.sceneItemId, sceneItemTransform: { positionX: 600, positionY: 380 } }); }
  console.log("scenes:", (await call("GetSceneList")).scenes.map(s => `${s.sceneIndex}:${s.sceneName}`).join(", "));
  console.log("replay:", JSON.stringify(await call("GetReplayBufferStatus")));
  c.stop(); process.exit(0);
});
c.start();
