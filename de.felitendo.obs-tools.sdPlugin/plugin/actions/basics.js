// Everyday OBS actions. On Windows these come with Elgato's own OBS plugin, on Linux they do not.
import path from "node:path";

import { Action } from "./base.js";
import { COLORS, MEDIA_GLYPHS } from "../lib/icons.js";
import { glyphKey } from "../lib/render.js";
import { TEXT_GESTURES, TEXT_KINDS, textGesture } from "../lib/text-gestures.js";
import { formatDuration, parseDuration } from "../lib/time.js";

const ID = "de.felitendo.obs-tools";

// Show or hide a source in a scene.
export class SourceVisibility extends Action {
	static uuid = `${ID}.sourcevisibility`;
	static required = ["source"];
	static pressKey = "mode";
	static ops = ["toggle", "show", "hide"];
	static defaults = { scene: "", source: "", mode: "toggle", hold: "none", double: "none", showName: false };

	sceneName() {
		return this.settings.scene || this.obsState.programScene;
	}

	async onConnected() {
		await this.load();
	}

	async onSettingsChanged() {
		if (this.online) await this.load();
	}

	async onStateChange(what) {
		// Without a fixed scene the key follows the live scene.
		if (!this.settings.scene && what === "program") await this.load();
		this.render();
	}

	async load() {
		this.itemId = null;
		this.enabled = false;
		const sceneName = this.sceneName();
		if (!sceneName || !this.settings.source) return;
		try {
			this.itemId = (await this.call("GetSceneItemId", { sceneName, sourceName: this.settings.source })).sceneItemId;
			this.enabled = (await this.call("GetSceneItemEnabled", { sceneName, sceneItemId: this.itemId })).sceneItemEnabled;
		} catch {
			this.itemId = null;
		}
	}

	onObsEvent(type, data) {
		if (type === "SceneItemEnableStateChanged" && data.sceneName === this.sceneName() && data.sceneItemId === this.itemId) {
			this.enabled = data.sceneItemEnabled;
			this.render();
		} else if ((type === "SceneItemCreated" || type === "SceneItemRemoved") && data.sceneName === this.sceneName()) {
			this.load().then(() => this.render());
		}
	}

	render() {
		this.setState(this.enabled ? 1 : 0);
		this.showNameTitle(this.settings.source);
	}

	async onPress(args) {
		const sceneName = this.sceneName();
		if (this.itemId === null) await this.load();
		if (this.itemId === null) throw new Error(`"${this.settings.source}" is not in scene "${sceneName}"`);
		const mode = this.op(args);
		const enabled = mode === "show" ? true : mode === "hide" ? false : !this.enabled;
		await this.call("SetSceneItemEnabled", { sceneName, sceneItemId: this.itemId, sceneItemEnabled: enabled });
		this.enabled = enabled;
		this.render();
	}
}

// Press an OBS hotkey: by its name, or by the keys it is bound to in OBS.
export class Hotkey extends Action {
	static uuid = `${ID}.hotkey`;
	static defaults = { mode: "name", hotkey: "", context: "", key: "OBS_KEY_F13", shift: false, control: false, alt: false, command: false };

	isSetUp() {
		return this.settings.mode === "keys" ? !!this.settings.key : !!this.settings.hotkey;
	}

	async onPress() {
		const s = this.settings;
		if (s.mode === "keys") {
			if (!s.key) throw new Error("no key set");
			await this.call("TriggerHotkeyByKeySequence", { keyId: s.key, keyModifiers: { shift: !!s.shift, control: !!s.control, alt: !!s.alt, command: !!s.command } });
		} else {
			if (!s.hotkey) throw new Error("no hotkey selected");
			const data = { hotkeyName: s.hotkey };
			if (s.context) data.contextName = s.context;
			await this.call("TriggerHotkeyByName", data);
		}
		this.showOk();
	}
}

// Save a picture of a scene or source as a file.
export class Screenshot extends Action {
	static uuid = `${ID}.screenshot`;
	static defaults = { source: "", folder: "", format: "png", width: 0 };

	async onPress() {
		const { source, folder, format, width } = this.settings;
		const sourceName = source || this.obsState.programScene;
		const dir = folder || (await this.call("GetRecordDirectory")).recordDirectory;
		const stamp = new Date().toISOString().slice(0, 19).replace("T", " ").replaceAll(":", "-");
		const ext = format === "jpg" ? "jpg" : "png";
		const file = path.join(dir, `Screenshot ${sourceName.replace(/[\\/:*?"<>|]/g, "_")} ${stamp}.${ext}`);
		const data = { sourceName, imageFormat: ext, imageFilePath: file };
		if (Number(width) > 0) data.imageWidth = Math.round(Number(width));
		if (ext === "jpg") data.imageCompressionQuality = 90;
		await this.call("SaveSourceScreenshot", data);
		this.log(`screenshot saved: ${file}`);
		this.showOk();
	}
}

// Chapter marker or file split while recording.
export class RecordTools extends Action {
	static uuid = `${ID}.recordtools`;
	static pressKey = "operation";
	static ops = ["chapter", "split"];
	static defaults = { operation: "chapter", hold: "none", double: "none", chapterName: "" };

	glyphName() {
		return this.settings.operation === "split" ? "split" : "chapter";
	}

	renderOffline() {
		this.setImage(glyphKey(this.glyphName(), { color: COLORS.gray }));
	}

	render() {
		this.setImage(glyphKey(this.glyphName(), { color: this.obsState.record.active ? COLORS.white : COLORS.gray }));
	}

	async onPress(args) {
		if (!this.obsState.record.active) throw new Error("not recording");
		if (this.op(args) === "split") {
			await this.call("SplitRecordFile");
		} else {
			const data = {};
			if (String(this.settings.chapterName ?? "").trim()) data.chapterName = this.settings.chapterName.trim();
			await this.call("CreateRecordChapter", data);
		}
		this.showOk();
	}
}

const MEDIA_ACTIONS = {
	play: "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_PLAY",
	pause: "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_PAUSE",
	stop: "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_STOP",
	restart: "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_RESTART",
	next: "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_NEXT",
	previous: "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_PREVIOUS",
};

// Play, pause, stop or skip in a media source. Shows the time left.
export class MediaControl extends Action {
	static uuid = `${ID}.mediacontrol`;
	static required = ["input"];
	static pressKey = "operation";
	static ops = ["playpause", "play", "pause", "stop", "restart", "forward", "back", "next", "previous"];
	static defaults = { input: "", operation: "playpause", hold: "none", double: "none", seconds: 10, showTime: true, timeAt: "below", timeShows: "left" };

	onConnected() {
		this.stopPoll?.();
		this.stopPoll = this.interval(() => this.poll(), 1000);
		return this.poll();
	}

	onDisconnected() {
		this.stopPoll?.();
	}

	async onSettingsChanged() {
		if (this.online) await this.poll();
	}

	async poll() {
		if (!this.visible || !this.settings.input) return;
		try {
			this.status = await this.call("GetMediaInputStatus", { inputName: this.settings.input });
		} catch {
			this.status = null;
		}
		this.render();
	}

	onObsEvent(type, data) {
		if (data.inputName === this.settings.input && type.startsWith("MediaInput")) this.poll();
	}

	playing() {
		return this.status?.mediaState === "OBS_MEDIA_STATE_PLAYING";
	}

	glyphName() {
		const op = this.settings.operation;
		if (op === "playpause") return this.playing() ? "pauseMedia" : "play";
		return MEDIA_GLYPHS[op] ?? "playPause";
	}

	renderOffline() {
		this.setImage(glyphKey(this.settings.operation === "playpause" ? "playPause" : MEDIA_GLYPHS[this.settings.operation] ?? "playPause"));
	}

	render() {
		const showTime = this.settings.showTime && !!this.settings.input;
		const { mediaDuration, mediaCursor } = this.status ?? {};
		const known = mediaCursor !== null && mediaCursor !== undefined;
		let time = "";
		// Time played works without a length too, like for a stream as source.
		if (known && this.settings.timeShows === "played") time = formatDuration(mediaCursor / 1000);
		else if (known && mediaDuration > 0) time = `-${formatDuration((mediaDuration - mediaCursor) / 1000)}`;
		const title = showTime && (this.playing() || this.status?.mediaState === "OBS_MEDIA_STATE_PAUSED") ? time : "";
		// The time goes above or below the glyph, so the key draws it into the image.
		this.ownTitle(title);
		this.setImage(glyphKey(this.glyphName(), { title, titleAt: this.settings.timeAt, color: this.status ? COLORS.white : COLORS.gray }));
	}

	async onPress(args) {
		const { input, seconds } = this.settings;
		const operation = this.op(args);
		if (!input) throw new Error("no media source selected");
		if (operation === "forward" || operation === "back") {
			const offset = Math.round((Number(seconds) || 10) * 1000) * (operation === "back" ? -1 : 1);
			await this.call("OffsetMediaInputCursor", { inputName: input, mediaCursorOffset: offset });
		} else {
			let action = MEDIA_ACTIONS[operation];
			if (operation === "playpause") {
				const status = await this.call("GetMediaInputStatus", { inputName: input });
				const state = status.mediaState;
				if (state === "OBS_MEDIA_STATE_PLAYING") action = MEDIA_ACTIONS.pause;
				else if (state === "OBS_MEDIA_STATE_ENDED" || state === "OBS_MEDIA_STATE_STOPPED" || state === "OBS_MEDIA_STATE_NONE") action = MEDIA_ACTIONS.restart;
				else action = MEDIA_ACTIONS.play;
			}
			await this.call("TriggerMediaInputAction", { inputName: input, mediaAction: action });
		}
		setTimeout(() => this.poll(), 150);
	}
}

// Switch the OBS profile or scene collection.
export class ProfileSwitch extends Action {
	static uuid = `${ID}.profile`;
	static required = ["name"];
	static defaults = { kind: "profile", name: "", showName: true };

	current() {
		return this.settings.kind === "collection" ? this.obsState.collection : this.obsState.profile;
	}

	glyphName() {
		return this.settings.kind === "collection" ? "collection" : "profile";
	}

	title() {
		return this.settings.showName ? String(this.settings.name ?? "") : "";
	}

	renderOffline() {
		this.ownTitle(this.title());
		this.setImage(glyphKey(this.glyphName(), { title: this.title(), color: COLORS.gray }));
	}

	render() {
		const active = !!this.settings.name && this.current() === this.settings.name;
		this.ownTitle(this.title());
		this.setImage(glyphKey(this.glyphName(), { title: this.title(), color: active ? COLORS.white : COLORS.gray }));
	}

	async onPress() {
		const { kind, name } = this.settings;
		if (!name) throw new Error(kind === "collection" ? "no scene collection selected" : "no profile selected");
		if (this.current() === name) return;
		if (kind === "collection") await this.call("SetCurrentSceneCollection", { sceneCollectionName: name });
		else await this.call("SetCurrentProfile", { profileName: name });
	}
}

// Change the text of a text source: a fixed text, the next line of a list, or a counter.
export class SetText extends Action {
	static uuid = `${ID}.settext`;
	static required = ["input"];
	// press, hold and double each pick one of TEXT_ACTIONS. shown: what the key shows ("text", "line" or "count").
	static defaults = {
		input: "",
		press: "set",
		hold: "none",
		double: "none",
		text: "",
		lines: "",
		index: -1,
		template: "{n}",
		start: 0,
		count: 0,
		step: 1,
		timerText: "{t}",
		timerLength: "5:00",
		timerEnd: "",
		shown: "",
		showText: true,
	};

	// Press, hold and double press pick from TEXT_ACTIONS (the base class does the rest).
	gesture(name) {
		return textGesture(this.settings, name);
	}

	lines() {
		return String(this.settings.lines ?? "")
			.split("\n")
			.map((l) => l.trimEnd())
			.filter(Boolean);
	}

	counterText(count) {
		const template = String(this.settings.template || "{n}");
		return template.includes("{n}") ? template.replaceAll("{n}", String(count)) : `${template}${count}`;
	}

	// The timer text: "{t}" becomes the time left. At 0 the end text, if there is one.
	timerText(seconds) {
		const end = String(this.settings.timerEnd ?? "");
		if (seconds <= 0 && end) return end;
		const template = String(this.settings.timerText || "{t}");
		const time = formatDuration(seconds);
		return template.includes("{t}") ? template.replaceAll("{t}", time) : `${template}${time}`;
	}

	// What the key shows: what was set last, or what a press would set.
	preview() {
		const s = this.settings;
		const kind = s.shown || TEXT_KINDS[this.gesture("press")] || "";
		if (kind === "count") return this.counterText(Number(s.count) || 0);
		if (kind === "timer") return this.timerText(this.timeLeft ?? parseDuration(s.timerLength));
		if (kind === "line") return this.lines()[Number(s.index)] ?? "";
		if (kind === "text") return s.text;
		return "";
	}

	// A new start number puts the counter there.
	async onSettingsChanged(old) {
		const start = Number(this.settings.start) || 0;
		if ((Number(old.start) || 0) !== start) this.saveSettings({ count: start });
		if (this.online && (old.input !== this.settings.input || old.text !== this.settings.text)) await this.readToggle();
	}

	// Text on or off: the key is white while the source has this key's text, else gray.
	usesToggle() {
		return TEXT_GESTURES.some((gesture) => textGesture(this.settings, gesture) === "toggle");
	}

	async onConnected() {
		await this.readToggle();
	}

	async readToggle() {
		const { input, text } = this.settings;
		if (!this.usesToggle() || !input) return;
		try {
			const { inputSettings } = await this.call("GetInputSettings", { inputName: input });
			this.textOn = !!text && inputSettings.text === text;
		} catch {
			this.textOn = false;
		}
	}

	// The text can also change in OBS or by another key.
	onObsEvent(type, data) {
		if (type !== "InputSettingsChanged" || data.inputName !== this.settings.input || !this.usesToggle()) return;
		if (typeof data.inputSettings?.text !== "string") return;
		this.textOn = !!this.settings.text && data.inputSettings.text === this.settings.text;
		this.render();
	}

	renderOffline() {
		this.render();
	}

	render() {
		const title = this.settings.showText ? String(this.preview() ?? "").split("\n")[0].trim() : "";
		const off = this.usesToggle() && !this.textOn;
		this.ownTitle(title);
		this.setImage(glyphKey("text", { title, color: off ? COLORS.gray : COLORS.white }));
	}

	async setText(text) {
		const { input } = this.settings;
		if (!input) throw new Error("no text source selected");
		await this.call("SetInputSettings", { inputName: input, inputSettings: { text }, overlay: true });
	}

	async setCount(count) {
		await this.setText(this.counterText(count));
		this.saveSettings({ count });
	}

	async setLine(index) {
		const lines = this.lines();
		if (!lines.length) throw new Error("the list is empty");
		index = ((index % lines.length) + lines.length) % lines.length;
		await this.setText(lines[index]);
		this.saveSettings({ index });
	}

	// Counts down by itself, once a second. Running: the next start stops it.
	async toggleTimer() {
		if (this.stopTimer) {
			this.stopTimer();
			this.stopTimer = null;
			return;
		}
		const length = parseDuration(this.settings.timerLength);
		if (!length) throw new Error("no timer length set");
		const end = Date.now() + length * 1000;
		let shown = null;
		const tick = async () => {
			const left = Math.max(0, Math.ceil((end - Date.now()) / 1000));
			if (left !== shown) {
				shown = left;
				this.timeLeft = left;
				await this.setText(this.timerText(left));
				this.render();
			}
			if (left === 0 && this.stopTimer) {
				this.stopTimer();
				this.stopTimer = null;
			}
		};
		this.stopTimer = this.interval(tick, 200);
		await tick();
	}

	// The timer stops with the key (another profile), so do not keep a dead one.
	onDisappear() {
		this.stopTimer = null;
	}

	// One step of the counter. It never goes below 0.
	async count(direction) {
		const step = Number(this.settings.step) || 1;
		await this.setCount(Math.max(0, (Number(this.settings.count) || 0) + direction * step));
	}

	async run(what) {
		const s = this.settings;
		switch (what) {
			case "set":
				await this.setText(String(s.text ?? ""));
				break;
			// Off when the source shows this text, else on.
			case "toggle": {
				if (!s.input) throw new Error("no text source selected");
				const text = String(s.text ?? "");
				const { inputSettings } = await this.call("GetInputSettings", { inputName: s.input });
				const on = !!text && inputSettings.text === text;
				await this.setText(on ? "" : text);
				this.textOn = !on;
				break;
			}
			// index -1 means nothing was set yet, so the first "next" sets line 0.
			case "next":
				await this.setLine((Number(s.index) || 0) + 1);
				break;
			case "previous":
				await this.setLine((Number(s.index) || 0) - 1);
				break;
			case "first":
				await this.setLine(0);
				break;
			case "up":
				await this.count(1);
				break;
			case "down":
				await this.count(-1);
				break;
			case "reset":
				await this.setCount(Number(s.start) || 0);
				break;
			case "timer":
				await this.toggleTimer();
				break;
			case "clear":
				await this.setText("");
				break;
			default:
				return;
		}
		if (s.shown !== TEXT_KINDS[what]) this.saveSettings({ shown: TEXT_KINDS[what] });
		// Hold or double press can change the text too.
		if (what !== "toggle") await this.readToggle();
		this.render();
	}

	onPress(args) {
		return this.run(this.op(args));
	}
}
