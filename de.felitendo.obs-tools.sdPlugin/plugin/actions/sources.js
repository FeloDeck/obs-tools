import path from "node:path";

import { Action } from "./base.js";
import { diffTransforms, normalizePhase, runAnimation, totalDuration } from "../lib/animation.js";
import { activeX11Window, sessionType, xcompositeWindowValue } from "../lib/desktop.js";
import { playMedia } from "../lib/media.js";

const ID = "de.felitendo.obs-tools";

// "/home/me/Videos/Big clip.mp4" -> "Big clip", for a title.
function fileTitle(file) {
	return path.basename(String(file ?? "")).replace(/\.[^.]+$/, "");
}

async function sceneItem(action, sceneName, sourceName) {
	if (!sceneName) throw new Error("no scene");
	if (!sourceName) throw new Error("no source selected");
	try {
		return (await action.call("GetSceneItemId", { sceneName, sourceName })).sceneItemId;
	} catch {
		throw new Error(`"${sourceName}" is not in scene "${sceneName}"`);
	}
}

// Moves, scales, rotates or crops a source in phases. "Record" fills in a phase for you.
export class SourceAnimation extends Action {
	static uuid = `${ID}.sourceanimation`;
	static required = ["source"];
	static defaults = {
		scene: "",
		source: "",
		phases: [{ duration: 1000, delay: 0, easing: "easeInOut", changes: {} }],
		showAtStart: false,
		hideAtEnd: false,
		resetAtEnd: false,
		filter: "",
		filterStart: "none",
		filterEnd: "none",
	};

	sceneName() {
		return this.settings.scene || this.obsState.programScene;
	}

	render() {
		this.setState(this.running ? 1 : 0);
	}

	async onPress() {
		if (this.running) return;
		const { source } = this.settings;
		const phases = (this.settings.phases ?? []).map(normalizePhase).filter((p) => p.duration > 0 || p.delay > 0 || Object.keys(p.changes).length);
		if (!phases.length) throw new Error("the animation has no phases");
		const sceneName = this.sceneName();
		const sceneItemId = await sceneItem(this, sceneName, source);
		const { sceneItemTransform: start } = await this.call("GetSceneItemTransform", { sceneName, sceneItemId });
		this.running = true;
		this.render();
		try {
			await runAnimation(this.obs, {
				sceneName,
				sceneItemId,
				sourceName: source,
				start,
				phases,
				fps: this.obsState.fps || 30,
				options: this.settings,
			});
		} finally {
			this.running = false;
			this.render();
		}
	}

	// The settings page drives recording and import.
	async onPiMessage(message, reply) {
		if (message.type === "recordStart") {
			const sceneName = this.sceneName();
			const sceneItemId = await sceneItem(this, sceneName, this.settings.source);
			this.recording = { sceneName, sceneItemId, start: (await this.call("GetSceneItemTransform", { sceneName, sceneItemId })).sceneItemTransform };
			reply({ type: "recording", active: true });
		} else if (message.type === "recordStop") {
			const recording = this.recording;
			this.recording = null;
			if (!recording) return reply({ type: "recording", active: false });
			const { sceneItemTransform: end } = await this.call("GetSceneItemTransform", { sceneName: recording.sceneName, sceneItemId: recording.sceneItemId });
			const changes = diffTransforms(recording.start, end);
			if (!Object.keys(changes).length) return reply({ type: "recording", active: false, message: "Nothing changed. Move or resize the source in OBS between the two clicks." });
			// Put the source back where it started, so the animation can play from there.
			await this.call("SetSceneItemTransform", { sceneName: recording.sceneName, sceneItemId: recording.sceneItemId, sceneItemTransform: Object.fromEntries(Object.keys(changes).map((k) => [k, recording.start[k]])) });
			const phases = [...(this.settings.phases ?? [])].map(normalizePhase);
			const index = phases.findIndex((p) => !Object.keys(p.changes).length);
			const phase = { duration: 1000, delay: 0, easing: "easeInOut", changes };
			if (index >= 0) phases[index] = { ...phases[index], changes };
			else phases.push(phase);
			this.saveSettings({ phases });
			reply({ type: "recording", active: false, message: `Recorded ${Object.keys(changes).length} changes into phase ${index >= 0 ? index + 1 : phases.length}.` });
		} else if (message.type === "recordCancel") {
			this.recording = null;
			reply({ type: "recording", active: false });
		} else if (message.type === "preview") {
			await this.onPress();
			reply({ type: "previewDone", duration: totalDuration(this.settings.phases ?? []) });
		}
	}
}

// One media source, many files: each key plays its own file.
export class VideoPlayer extends Action {
	static uuid = `${ID}.videoplayer`;
	static required = ["input"];
	// play: start (pressed again while playing: see pressAgain). stop and restart do just that.
	static pressKey = "press";
	static ops = ["play", "stop", "restart"];
	static defaults = { input: "", press: "play", hold: "none", double: "none", file: "", speed: 100, scene: "", showSource: true, hideWhenDone: true, returnToScene: false, pressAgain: "stop", showName: false };

	render() {
		this.setState(this.playback ? 1 : 0);
		this.showNameTitle(fileTitle(this.settings.file) || this.settings.input);
	}

	async onPress(args) {
		const { input, file, speed, scene, showSource, hideWhenDone, returnToScene, pressAgain } = this.settings;
		if (!input) throw new Error("no media source selected");
		const op = this.op(args);
		if (op === "stop") {
			if (this.playback) await this.playback.stop();
			else await this.call("TriggerMediaInputAction", { inputName: input, mediaAction: "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_STOP" });
			return;
		}
		if (op === "play" && this.playback && pressAgain === "stop" && !this.isInMultiAction) {
			await this.playback.stop();
			return;
		}
		await this.playback?.stop();
		const playback = await playMedia(this.obs, this.obsState, { input, file, speed, scene, showSource, hideWhenDone, returnToScene });
		this.playback = playback;
		this.render();
		playback.done.then(() => {
			if (this.playback === playback) this.playback = null;
			this.render();
		});
	}

	onDisappear() {
		this.playback = null;
	}
}

// One image source, many pictures: each key shows its own picture.
export class ImageSource extends Action {
	static uuid = `${ID}.imagesource`;
	static required = ["input"];
	static pressKey = "mode";
	static ops = ["toggle", "show", "hide"];
	static defaults = { scene: "", input: "", file: "", mode: "toggle", hold: "none", double: "none", autoHide: 0, showName: false };

	sceneName() {
		return this.settings.scene || this.obsState.programScene;
	}

	render() {
		this.setState(this.showing ? 1 : 0);
		this.showNameTitle(fileTitle(this.settings.file) || this.settings.input);
	}

	onObsEvent(type, data) {
		// Someone else hid the source or loaded another picture.
		if (type === "SceneItemEnableStateChanged" && data.sceneItemId === this.itemId && !data.sceneItemEnabled) this.showing = false;
		else if (type === "InputSettingsChanged" && data.inputName === this.settings.input && data.inputSettings?.file !== this.settings.file) this.showing = false;
		else return;
		this.render();
	}

	onDisappear() {
		clearTimeout(this.hideTimer);
	}

	async hide(sceneName, itemId) {
		clearTimeout(this.hideTimer);
		await this.call("SetSceneItemEnabled", { sceneName, sceneItemId: itemId, sceneItemEnabled: false });
		this.showing = false;
		this.render();
	}

	async onPress(args) {
		const { input, file, autoHide } = this.settings;
		if (!input) throw new Error("no image source selected");
		const mode = this.op(args);
		const sceneName = this.sceneName();
		const itemId = await sceneItem(this, sceneName, input);
		this.itemId = itemId;
		const { sceneItemEnabled } = await this.call("GetSceneItemEnabled", { sceneName, sceneItemId: itemId });
		const current = (await this.call("GetInputSettings", { inputName: input })).inputSettings?.file;
		const mine = sceneItemEnabled && (!file || current === file);
		if (mode === "hide") {
			if (mine) await this.hide(sceneName, itemId);
			return;
		}
		// Toggle, pressed again while its picture is up: hide it.
		if (mode === "toggle" && mine && !this.isInMultiAction) {
			await this.hide(sceneName, itemId);
			return;
		}
		if (file && current !== file) await this.call("SetInputSettings", { inputName: input, inputSettings: { file }, overlay: true });
		await this.call("SetSceneItemEnabled", { sceneName, sceneItemId: itemId, sceneItemEnabled: true });
		this.showing = true;
		this.render();
		clearTimeout(this.hideTimer);
		if (Number(autoHide) > 0) {
			this.hideTimer = setTimeout(() => this.safe(() => (this.showing ? this.hide(sceneName, itemId) : null)), Number(autoHide) * 1000);
		}
	}
}

export class BrowserSource extends Action {
	static uuid = `${ID}.browsersource`;
	static required = ["input"];
	static pressKey = "operation";
	static ops = ["refresh", "url", "cycle", "visibility"];
	static defaults = { input: "", operation: "refresh", hold: "none", double: "none", url: "", urls: "", index: 0, showName: false };

	render() {
		this.showNameTitle(this.settings.input);
	}

	urlList() {
		return String(this.settings.urls ?? "")
			.split("\n")
			.map((u) => u.trim())
			.filter(Boolean);
	}

	async onPress(args) {
		const { input, url } = this.settings;
		const operation = this.op(args);
		if (!input) throw new Error("no browser source selected");
		if (operation === "refresh") {
			await this.call("PressInputPropertiesButton", { inputName: input, propertyName: "refreshnocache" });
		} else if (operation === "url") {
			if (!url) throw new Error("no URL set");
			await this.call("SetInputSettings", { inputName: input, inputSettings: { url }, overlay: true });
		} else if (operation === "cycle") {
			const urls = this.urlList();
			if (!urls.length) throw new Error("no URLs in the list");
			const index = ((Number(this.settings.index) || 0) + 1) % urls.length;
			await this.call("SetInputSettings", { inputName: input, inputSettings: { url: urls[index] }, overlay: true });
			this.saveSettings({ index });
		} else if (operation === "visibility") {
			const sceneName = this.obsState.programScene;
			const sceneItemId = await sceneItem(this, sceneName, input);
			const { sceneItemEnabled } = await this.call("GetSceneItemEnabled", { sceneName, sceneItemId });
			await this.call("SetSceneItemEnabled", { sceneName, sceneItemId, sceneItemEnabled: !sceneItemEnabled });
		}
		this.showOk();
	}
}

export class FilterToggle extends Action {
	static uuid = `${ID}.filtertoggle`;
	static required = ["source", "filter"];
	static pressKey = "mode";
	static ops = ["toggle", "enable", "disable"];
	static defaults = { source: "", filter: "", mode: "toggle", hold: "none", double: "none", showName: false };

	async onConnected() {
		await this.load();
	}

	async onSettingsChanged(old) {
		if ((old.source !== this.settings.source || old.filter !== this.settings.filter) && this.online) await this.load();
	}

	async load() {
		this.enabled = false;
		const { source, filter } = this.settings;
		if (!source || !filter) return;
		try {
			this.enabled = (await this.call("GetSourceFilter", { sourceName: source, filterName: filter })).filterEnabled;
		} catch {
			this.enabled = false;
		}
	}

	onObsEvent(type, data) {
		if (type === "SourceFilterEnableStateChanged" && data.sourceName === this.settings.source && data.filterName === this.settings.filter) {
			this.enabled = data.filterEnabled;
			this.render();
		}
	}

	render() {
		this.setState(this.enabled ? 1 : 0);
		this.showNameTitle(this.settings.filter);
	}

	async onPress(args) {
		const { source, filter } = this.settings;
		const mode = this.op(args);
		if (!source || !filter) throw new Error("no filter selected");
		let enabled;
		if (mode === "enable") enabled = true;
		else if (mode === "disable") enabled = false;
		else enabled = !(await this.call("GetSourceFilter", { sourceName: source, filterName: filter })).filterEnabled;
		await this.call("SetSourceFilterEnabled", { sourceName: source, filterName: filter, filterEnabled: enabled });
		this.enabled = enabled;
		this.render();
	}
}

// Points a Window Capture (Xcomposite) source at the window that has focus.
export class FocusedWindow extends Action {
	static uuid = `${ID}.focusedwindow`;
	static required = ["input"];
	static defaults = { input: "", delay: 0 };

	async onPress() {
		const { input, delay } = this.settings;
		if (!input) throw new Error("no window capture source selected");
		// Fails early when the source is missing.
		await this.call("GetInputSettings", { inputName: input });
		if (Number(delay) > 0) await new Promise((r) => setTimeout(r, Number(delay) * 1000));
		const window = await activeX11Window();
		if (!window) {
			throw new Error(sessionType() === "wayland" ? "no X11 window has focus (Wayland apps cannot be picked)" : "no focused window found");
		}
		await this.call("SetInputSettings", { inputName: input, inputSettings: { capture_window: xcompositeWindowValue(window) }, overlay: true });
		this.log(`window capture "${input}" -> ${window.name} (${window.cls})`);
		this.showOk();
	}
}
