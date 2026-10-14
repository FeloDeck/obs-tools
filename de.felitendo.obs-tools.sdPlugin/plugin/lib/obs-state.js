// Keeps the OBS state that many keys need, so each key does not have to ask for it.
import { EventEmitter } from "node:events";

import { EVENTS_ALL, EVENTS_VOLUME_METERS } from "./obs.js";

const HISTORY_SIZE = 20;

export class ObsState extends EventEmitter {
	constructor(client) {
		super();
		this.setMaxListeners(0);
		this.client = client;
		this.reset();
		this.statsUsers = 0;
		this.statsTimer = null;
		this.stats = null;
		this.lastBytes = null;
		this.meterUsers = 0;
		this.levels = new Map();

		client.on("connected", () => this.load());
		client.on("disconnected", () => {
			this.reset();
			this.emit("change", "all");
		});
		client.on("event", (type, data) => this.onEvent(type, data));
	}

	reset() {
		this.ready = false;
		this.programScene = null;
		this.previewScene = null;
		this.studioMode = false;
		this.scenes = [];
		this.history = [];
		this.record = { active: false, paused: false };
		this.stream = { active: false };
		this.replay = { active: false, available: false };
		this.virtualcam = { active: false };
		this.transition = { name: null, duration: null };
		this.fps = 30;
		this.profile = null;
		this.collection = null;
		this.lastBytes = null;
	}

	async load() {
		const requests = [
			"GetSceneList",
			"GetStudioModeEnabled",
			"GetRecordStatus",
			"GetStreamStatus",
			"GetReplayBufferStatus",
			"GetVirtualCamStatus",
			"GetCurrentSceneTransition",
			"GetVideoSettings",
			"GetProfileList",
			"GetSceneCollectionList",
		].map((requestType) => ({ requestType }));
		let results;
		try {
			results = await this.client.batch(requests);
		} catch {
			return;
		}
		const data = Object.fromEntries(results.map((r) => [r.requestType, r.requestStatus?.result ? r.responseData : null]));
		if (data.GetSceneList) this.setScenes(data.GetSceneList);
		this.studioMode = !!data.GetStudioModeEnabled?.studioModeEnabled;
		this.previewScene = this.studioMode ? data.GetSceneList?.currentPreviewSceneName ?? null : null;
		this.record = { active: !!data.GetRecordStatus?.outputActive, paused: !!data.GetRecordStatus?.outputPaused };
		this.stream = { active: !!data.GetStreamStatus?.outputActive };
		this.replay = { active: !!data.GetReplayBufferStatus?.outputActive, available: !!data.GetReplayBufferStatus };
		this.virtualcam = { active: !!data.GetVirtualCamStatus?.outputActive };
		if (data.GetCurrentSceneTransition) {
			this.transition = { name: data.GetCurrentSceneTransition.transitionName, duration: data.GetCurrentSceneTransition.transitionDuration };
		}
		this.profile = data.GetProfileList?.currentProfileName ?? null;
		this.collection = data.GetSceneCollectionList?.currentSceneCollectionName ?? null;
		if (data.GetVideoSettings?.fpsDenominator) {
			this.fps = data.GetVideoSettings.fpsNumerator / data.GetVideoSettings.fpsDenominator;
		}
		this.ready = true;
		this.emit("change", "all");
	}

	setScenes(list) {
		// OBS lists scenes bottom first. Keys want the order of the scene dock.
		this.scenes = [...(list.scenes ?? [])].sort((a, b) => b.sceneIndex - a.sceneIndex).map((s) => s.sceneName);
		if (list.currentProgramSceneName) this.programScene = list.currentProgramSceneName;
	}

	async refreshScenes() {
		try {
			this.setScenes(await this.client.call("GetSceneList"));
			this.emit("change", "scenes");
		} catch {}
	}

	// The scene shown before the current one that still exists.
	get previousScene() {
		for (let i = this.history.length - 1; i >= 0; i--) {
			const name = this.history[i];
			if (name !== this.programScene && this.scenes.includes(name)) return name;
		}
		return null;
	}

	onEvent(type, data) {
		switch (type) {
			case "CurrentProgramSceneChanged": {
				const old = this.programScene;
				if (old && old !== data.sceneName) {
					this.history = this.history.filter((name) => name !== old);
					this.history.push(old);
					if (this.history.length > HISTORY_SIZE) this.history.shift();
				}
				this.programScene = data.sceneName;
				this.emit("change", "program");
				break;
			}
			case "CurrentPreviewSceneChanged":
				this.previewScene = data.sceneName;
				this.emit("change", "preview");
				break;
			case "StudioModeStateChanged":
				this.studioMode = data.studioModeEnabled;
				if (!this.studioMode) this.previewScene = null;
				else this.client.call("GetCurrentPreviewScene").then((r) => {
					this.previewScene = r.sceneName ?? r.currentPreviewSceneName ?? null;
					this.emit("change", "preview");
				}).catch(() => {});
				this.emit("change", "studio");
				break;
			case "SceneListChanged":
			case "SceneCreated":
			case "SceneRemoved":
				this.refreshScenes();
				break;
			case "SceneNameChanged":
				this.history = this.history.map((name) => (name === data.oldSceneName ? data.sceneName : name));
				if (this.programScene === data.oldSceneName) this.programScene = data.sceneName;
				if (this.previewScene === data.oldSceneName) this.previewScene = data.sceneName;
				this.refreshScenes();
				break;
			case "RecordStateChanged":
				this.record = {
					active: data.outputActive,
					paused: data.outputState === "OBS_WEBSOCKET_OUTPUT_PAUSED",
				};
				this.emit("change", "record");
				break;
			case "StreamStateChanged":
				this.stream = { active: data.outputActive };
				this.emit("change", "stream");
				break;
			case "ReplayBufferStateChanged":
				this.replay = { active: data.outputActive, available: true };
				this.emit("change", "replay");
				break;
			case "VirtualcamStateChanged":
				this.virtualcam = { active: data.outputActive };
				this.emit("change", "virtualcam");
				break;
			case "CurrentSceneTransitionChanged":
				this.transition = { ...this.transition, name: data.transitionName };
				this.client.call("GetCurrentSceneTransition").then((r) => {
					this.transition = { name: r.transitionName, duration: r.transitionDuration };
					this.emit("change", "transition");
				}).catch(() => {});
				this.emit("change", "transition");
				break;
			case "CurrentSceneTransitionDurationChanged":
				this.transition = { ...this.transition, duration: data.transitionDuration };
				this.emit("change", "transition");
				break;
			case "CurrentProfileChanged":
				this.profile = data.profileName;
				this.emit("change", "profile");
				break;
			case "CurrentSceneCollectionChanged":
				// Everything about scenes changed, start over.
				this.history = [];
				this.load();
				break;
			case "InputVolumeMeters":
				// Inputs that play nothing are left out, so start fresh each time (no stuck meters).
				this.levels = new Map((data.inputs ?? []).map((input) => [input.inputName, levelFromMeter(input.inputLevelsMul)]));
				this.emit("meters", this.levels);
				break;
		}
	}

	// Audio levels only come while at least one key shows them, they are a lot of traffic.
	subscribeMeters() {
		this.meterUsers++;
		this.client.setEventSubscriptions(EVENTS_ALL | EVENTS_VOLUME_METERS);
		let done = false;
		return () => {
			if (done) return;
			done = true;
			this.meterUsers = Math.max(0, this.meterUsers - 1);
			if (!this.meterUsers) {
				this.client.setEventSubscriptions(EVENTS_ALL);
				this.levels.clear();
			}
		};
	}

	// Stats are polled once a second while at least one key shows them.
	subscribeStats() {
		this.statsUsers++;
		if (!this.statsTimer) {
			this.pollStats();
			this.statsTimer = setInterval(() => this.pollStats(), 1000);
		}
		let done = false;
		return () => {
			if (done) return;
			done = true;
			this.statsUsers--;
			if (this.statsUsers <= 0) {
				this.statsUsers = 0;
				clearInterval(this.statsTimer);
				this.statsTimer = null;
			}
		};
	}

	async pollStats() {
		if (!this.client.connected) return;
		try {
			const results = await this.client.batch([{ requestType: "GetStats" }, { requestType: "GetStreamStatus" }]);
			const [stats, stream] = results.map((r) => (r.requestStatus?.result ? r.responseData : null));
			if (!stats) return;
			if (stream) stream.kbps = this.bitrate(stream);
			this.stats = { ...stats, stream };
			this.emit("stats", this.stats);
		} catch {}
	}

	// Upload speed of the stream in kbit/s, from the bytes sent between two polls.
	bitrate(stream) {
		const now = Date.now();
		const bytes = stream.outputBytes ?? 0;
		const last = this.lastBytes;
		this.lastBytes = stream.outputActive ? { bytes, now } : null;
		if (!stream.outputActive || !last || bytes < last.bytes || now === last.now) return null;
		return ((bytes - last.bytes) * 8) / (now - last.now);
	}
}

// OBS sends [magnitude, peak, input peak] per channel as multipliers. Keys show the loudest peak in dB.
export function levelFromMeter(channels = []) {
	let peak = 0;
	let magnitude = 0;
	for (const [mag = 0, pk = 0] of channels) {
		peak = Math.max(peak, pk);
		magnitude = Math.max(magnitude, mag);
	}
	const db = (v) => (v > 0 ? 20 * Math.log10(v) : -Infinity);
	return { peak: db(peak), magnitude: db(magnitude) };
}
