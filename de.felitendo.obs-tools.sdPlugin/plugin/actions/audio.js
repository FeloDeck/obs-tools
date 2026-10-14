import { Action } from "./base.js";
import { COLORS, VOLUME_GLYPHS } from "../lib/icons.js";
import { t } from "../lib/i18n.js";
import { dialIcon, glyphKey, meterLevel } from "../lib/render.js";
import { clampDb, dbToFader, faderToDb, formatVolume, readDb, stepVolume, volumeRequest } from "../lib/volume.js";

const ID = "de.felitendo.obs-tools";

const MONITOR_TYPES = {
	none: "OBS_MONITORING_TYPE_NONE",
	monitorOnly: "OBS_MONITORING_TYPE_MONITOR_ONLY",
	monitorAndOutput: "OBS_MONITORING_TYPE_MONITOR_AND_OUTPUT",
};

// Shared by the key and the dial: keeps volume and mute of one input up to date.
class InputAction extends Action {
	async onConnected() {
		await this.loadInput();
		this.startMeter();
	}

	onDisconnected() {
		this.stopMeter?.();
	}

	onDisappear() {
		this.stopMeter?.();
	}

	async onSettingsChanged(old) {
		if (old.input !== this.settings.input && this.online) await this.loadInput();
		if (old.input !== this.settings.input || old.showMeter !== this.settings.showMeter) this.startMeter();
	}

	// Live audio level, at most 10 updates a second and only when it changes visibly.
	startMeter() {
		this.stopMeter?.();
		this.level = 0;
		if (!this.settings.showMeter || !this.online || !this.visible) return;
		const state = this.obsState;
		const unsubscribe = state.subscribeMeters();
		const onMeters = (levels) => {
			const now = Date.now();
			if (now - (this.meterAt ?? 0) < 100) return;
			const level = Math.round(meterLevel(levels.get(this.settings.input)?.peak) * 40) / 40;
			if (level === this.level) return;
			this.meterAt = now;
			this.level = level;
			this.render();
		};
		state.on("meters", onMeters);
		this.stopMeter = () => {
			unsubscribe();
			state.off("meters", onMeters);
			this.stopMeter = null;
			this.level = 0;
		};
	}

	async loadInput() {
		this.db = null;
		this.muted = false;
		this.monitorType = null;
		const input = this.settings.input;
		if (!input) return;
		try {
			const [volume, mute] = await Promise.all([this.call("GetInputVolume", { inputName: input }), this.call("GetInputMute", { inputName: input })]);
			this.db = readDb(volume);
			this.muted = mute.inputMuted;
		} catch {
			this.db = null;
		}
	}

	onObsEvent(type, data) {
		if (data.inputName !== this.settings.input) return;
		if (type === "InputVolumeChanged") this.db = readDb(data);
		else if (type === "InputMuteStateChanged") this.muted = data.inputMuted;
		else if (type === "InputAudioMonitorTypeChanged") this.monitorType = data.monitorType;
		else return;
		this.render();
	}

	async changeVolume(direction, step) {
		const { input, unit, allowBoost } = this.settings;
		if (!input) throw new Error("no audio source selected");
		if (this.db === null) await this.loadInput();
		const next = stepVolume(this.db ?? -Infinity, { unit, step: Number(step) || 1, direction, allowBoost: !!allowBoost });
		this.db = next;
		await this.call("SetInputVolume", volumeRequest(input, next));
		this.render();
	}

	async setVolume(value) {
		const { input, unit, allowBoost } = this.settings;
		if (!input) throw new Error("no audio source selected");
		const number = Number(value);
		let db;
		if (unit === "percent") {
			db = clampDb(faderToDb(Math.max(0, Math.min(100, number)) / 100), allowBoost);
		} else {
			db = clampDb(number, !!allowBoost);
		}
		this.db = db;
		await this.call("SetInputVolume", volumeRequest(input, db));
		this.render();
	}

	async toggleMute() {
		const { input } = this.settings;
		if (!input) throw new Error("no audio source selected");
		const result = await this.call("ToggleInputMute", { inputName: input });
		this.muted = result.inputMuted;
		this.render();
	}
}

// Volume up, down, to a fixed value, or mute. Hold keeps going by default.
export class SourceVolume extends InputAction {
	static uuid = `${ID}.sourcevolume`;
	static required = ["input"];
	static pressKey = "mode";
	static ops = ["up", "down", "set", "mute"];
	static canRepeat = true;
	static defaults = { input: "", mode: "up", hold: "repeat", double: "none", unit: "db", step: 2, value: -10, allowBoost: false, showValue: true, valueAt: "below", showMeter: true };

	glyphName() {
		return VOLUME_GLYPHS[this.settings.mode] ?? "volume";
	}

	renderOffline() {
		this.setImage(glyphKey(this.glyphName()));
	}

	// Like the volume keys of the Feishin plugin: the value is the title.
	render() {
		const { showValue, valueAt, unit, input } = this.settings;
		const meter = this.settings.showMeter && input ? (this.muted ? 0 : this.level ?? 0) : null;
		const value = this.db === null ? "?" : this.muted ? t("Muted") : formatVolume(this.db, unit);
		const title = showValue && input ? value : "";
		this.ownTitle(title);
		this.setImage(glyphKey(this.muted ? "mute" : this.glyphName(), { color: this.muted ? COLORS.red : COLORS.white, meter, title, titleAt: valueAt }));
	}

	async onPress(args) {
		const { step, value } = this.settings;
		const mode = this.op(args);
		if (mode === "mute") {
			if (!args?.repeat) await this.toggleMute();
		} else if (mode === "set") {
			if (!args?.repeat) await this.setVolume(value);
		} else {
			await this.changeVolume(mode === "down" ? -1 : 1, step);
		}
	}
}

export class MuteToggle extends InputAction {
	static uuid = `${ID}.mutetoggle`;
	static required = ["input"];
	static pressKey = "mode";
	static ops = ["toggle", "mute", "unmute", "talk", "holdMute"];
	static pressOnly = ["talk", "holdMute"];
	static defaults = { input: "", mode: "toggle", hold: "none", double: "none", showName: false };

	render() {
		this.setState(this.muted ? 1 : 0);
		this.showNameTitle(this.settings.input);
	}

	async setMuted(muted) {
		await this.call("SetInputMute", { inputName: this.settings.input, inputMuted: muted });
		this.muted = muted;
		this.render();
	}

	// talk: on only while the key is held (push to talk). holdMute: off only while held.
	async onPress(args) {
		const { input } = this.settings;
		const mode = this.op(args);
		if (!input) throw new Error("no audio source selected");
		if (mode === "toggle") await this.toggleMute();
		else if (mode === "talk") await this.setMuted(false);
		else if (mode === "holdMute") await this.setMuted(true);
		else await this.setMuted(mode === "mute");
	}

	async onRelease() {
		const { input, mode } = this.settings;
		if (!input || this.isInMultiAction) return;
		if (mode === "talk") await this.setMuted(true);
		else if (mode === "holdMute") await this.setMuted(false);
	}
}

export class MonitorSet extends InputAction {
	static uuid = `${ID}.monitorset`;
	static required = ["input"];
	// set: the chosen monitoring (press again: off, if wanted). off: monitoring off.
	static pressKey = "press";
	static ops = ["set", "off"];
	static defaults = { input: "", press: "set", hold: "none", double: "none", monitorType: "monitorAndOutput", pressAgainOff: true, showName: false };

	async loadInput() {
		await super.loadInput();
		if (!this.settings.input) return;
		try {
			this.monitorType = (await this.call("GetInputAudioMonitorType", { inputName: this.settings.input })).monitorType;
		} catch {
			this.monitorType = null;
		}
	}

	target() {
		return MONITOR_TYPES[this.settings.monitorType] ?? MONITOR_TYPES.monitorAndOutput;
	}

	render() {
		this.setState(this.monitorType === this.target() ? 1 : 0);
		this.showNameTitle(this.settings.input);
	}

	async onPress(args) {
		const { input, pressAgainOff } = this.settings;
		if (!input) throw new Error("no audio source selected");
		const target = this.target();
		const off = this.op(args) === "off" || (pressAgainOff && this.monitorType === target && !this.isInMultiAction);
		const next = off ? MONITOR_TYPES.none : target;
		await this.call("SetInputAudioMonitorType", { inputName: input, monitorType: next });
		this.monitorType = next;
		this.render();
	}
}

// Stream Deck + dial: turn for volume, press or touch to mute (or what pressAction says).
export class InputVolume extends InputAction {
	static uuid = `${ID}.inputvolume`;
	static required = ["input"];
	static defaults = { input: "", unit: "db", step: 1, allowBoost: false, pressAction: "mute", disableTouch: false, showMeter: true };

	layout() {
		return this.settings.showMeter ? "layouts/volume-meter.json" : "layouts/dial.json";
	}

	async onSettingsChanged(old) {
		if (old.showMeter !== this.settings.showMeter) this.setLayout();
		await super.onSettingsChanged(old);
	}

	// Without OBS or settings: the name, the status, and the setup dot on the icon.
	renderOffline() {
		const feedback = { title: this.settings.input || t("Volume"), icon: dialIcon("volume", { setup: !this.isSetUp() }), value: this.dialStatus(), indicator: { value: 0 } };
		if (this.settings.showMeter) feedback.meter = { value: 0 };
		this.setFeedback(feedback);
	}

	render() {
		if (!this.isSetUp()) return this.renderOffline();
		const { input, unit } = this.settings;
		const value = this.db === null ? "?" : this.muted ? t("Muted") : formatVolume(this.db, unit);
		const position = this.db === null ? 0 : Math.round(dbToFader(this.db) * 100);
		const feedback = {
			title: input,
			icon: dialIcon("volume"),
			value: { value, color: this.muted ? COLORS.red : COLORS.white },
			indicator: { value: position, bar_fill_c: this.muted ? COLORS.gray : COLORS.green },
		};
		if (this.settings.showMeter) feedback.meter = { value: this.muted ? 0 : Math.round((this.level ?? 0) * 100) };
		this.setFeedback(feedback);
	}

	async onDialRotate(ticks) {
		await this.changeVolume(Math.sign(ticks), Math.abs(ticks) * (Number(this.settings.step) || 1));
	}

	async onDialDown() {
		if (this.settings.pressAction === "reset") await this.setVolume(this.settings.unit === "percent" ? 100 : 0);
		else if (this.settings.pressAction !== "nothing") await this.toggleMute();
	}
}
