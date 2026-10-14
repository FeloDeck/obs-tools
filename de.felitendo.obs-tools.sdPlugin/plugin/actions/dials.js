// Dials (Stream Deck +): transition time, scene picker and any number in a filter.
import { Action } from "./base.js";
import { COLORS } from "../lib/icons.js";
import { num, t } from "../lib/i18n.js";
import { dialIcon } from "../lib/render.js";

const ID = "de.felitendo.obs-tools";
const MIN_DURATION = 50;
const MAX_DURATION = 20000;

// Turn: transition time. Press: next transition (or take the preview live).
export class TransitionDial extends Action {
	static uuid = `${ID}.transitiondial`;
	static defaults = { step: 50, pressAction: "next", disableTouch: false };

	layout() {
		return "layouts/dial.json";
	}

	async onConnected() {
		await this.loadList();
	}

	async loadList() {
		try {
			const list = await this.call("GetSceneTransitionList");
			this.transitions = list.transitions ?? [];
		} catch {
			this.transitions = [];
		}
	}

	current() {
		return this.transitions?.find((t) => t.transitionName === this.obsState.transition.name);
	}

	renderOffline() {
		this.setFeedback({ title: t("Transition"), icon: dialIcon("transition"), value: this.dialStatus(), indicator: { value: 0 } });
	}

	render() {
		const { name, duration } = this.obsState.transition;
		const fixed = this.current()?.transitionFixed;
		this.setFeedback({
			title: name ?? t("Transition"),
			icon: dialIcon("transition"),
			value: fixed ? t("no time") : duration ? `${duration} ms` : "",
			indicator: { value: fixed || !duration ? 0 : Math.min(100, Math.round((Math.log10(duration) - Math.log10(MIN_DURATION)) / (Math.log10(5000) - Math.log10(MIN_DURATION)) * 100)) },
		});
	}

	onObsEvent(type) {
		if (type === "TransitionCreated" || type === "TransitionRemoved" || type === "TransitionNameChanged") this.loadList().then(() => this.render());
	}

	async onDialRotate(ticks) {
		if (this.current()?.transitionFixed) throw new Error(`${this.obsState.transition.name} has no duration`);
		const step = Math.max(10, Number(this.settings.step) || 50);
		const duration = Math.min(MAX_DURATION, Math.max(MIN_DURATION, (this.obsState.transition.duration ?? 300) + ticks * step));
		await this.call("SetCurrentSceneTransitionDuration", { transitionDuration: duration });
		this.obsState.transition = { ...this.obsState.transition, duration };
		this.render();
	}

	async onDialDown() {
		if (this.settings.pressAction === "nothing") return;
		if (this.settings.pressAction === "studio") {
			if (!this.obsState.studioMode) throw new Error("studio mode is off");
			await this.call("TriggerStudioModeTransition");
			return;
		}
		if (!this.transitions?.length) await this.loadList();
		const names = (this.transitions ?? []).map((t) => t.transitionName);
		if (!names.length) return;
		const next = names[(names.indexOf(this.obsState.transition.name) + 1) % names.length];
		await this.call("SetCurrentSceneTransition", { transitionName: next });
	}
}

const PICK_TIMEOUT = 6000;

// Turn to pick a scene (with a picture of it), press to switch. Or switch right away while turning.
export class SceneDial extends Action {
	static uuid = `${ID}.scenedial`;
	static defaults = { mode: "pick", disableTouch: false };

	layout() {
		return "layouts/scene-dial.json";
	}

	async onConnected() {
		this.selected = null;
		this.stopShots?.();
		this.stopShots = this.interval(() => this.refreshShot(), 2000);
		await this.refreshShot();
	}

	onDisconnected() {
		this.stopShots?.();
	}

	target() {
		return this.selected ?? this.obsState.programScene;
	}

	async refreshShot() {
		const scene = this.target();
		if (!scene || !this.visible) return;
		const token = (this.shotToken = (this.shotToken ?? 0) + 1);
		try {
			const result = await this.call("GetSourceScreenshot", { sourceName: scene, imageFormat: "jpg", imageWidth: 192, imageCompressionQuality: 70 });
			if (token !== this.shotToken) return;
			this.shot = result.imageData.replace(/^data:image\/jpg;/, "data:image/jpeg;");
			this.shotScene = scene;
		} catch {
			this.shot = null;
		}
		this.render();
	}

	onStateChange(what) {
		if (what === "program" || what === "preview" || what === "scenes") {
			if (this.selected && !this.obsState.scenes.includes(this.selected)) this.selected = null;
			this.refreshShot();
		}
		this.render();
	}

	renderOffline() {
		this.setFeedback({ title: t("Scene"), name: "", info: this.dialStatus(), preview: "" });
	}

	render() {
		const state = this.obsState;
		const scene = this.target();
		const index = state.scenes.indexOf(scene);
		const live = scene === state.programScene;
		const preview = state.studioMode && scene === state.previewScene;
		let info = `${index + 1} / ${state.scenes.length}`;
		if (live) info = t("LIVE");
		else if (preview) info = t("PREVIEW");
		this.setFeedback({
			title: t(this.selected && !live ? "Press to switch" : "Scene"),
			name: { value: scene ?? "", color: live ? COLORS.red : preview ? COLORS.green : COLORS.white },
			info: { value: info, color: live ? COLORS.red : preview ? COLORS.green : COLORS.muted },
			preview: this.shotScene === scene && this.shot ? this.shot : "",
		});
	}

	async onDialRotate(ticks) {
		const scenes = this.obsState.scenes;
		if (!scenes.length) return;
		const from = scenes.indexOf(this.target());
		const next = scenes[(((from + ticks) % scenes.length) + scenes.length) % scenes.length];
		clearTimeout(this.pickTimer);
		if (this.settings.mode === "direct") {
			// Wait a moment, so a fast turn does not switch through every scene.
			clearTimeout(this.switchTimer);
			this.selected = next;
			this.render();
			this.switchTimer = setTimeout(() => this.safe(() => this.switchTo(next)), 200);
			return;
		}
		this.selected = next;
		this.render();
		this.refreshShot();
		this.pickTimer = setTimeout(() => {
			this.selected = null;
			this.refreshShot();
		}, PICK_TIMEOUT);
	}

	async switchTo(scene) {
		const state = this.obsState;
		if (state.studioMode && !this.isInMultiAction) {
			if (state.previewScene === scene) await this.call("TriggerStudioModeTransition");
			else await this.call("SetCurrentPreviewScene", { sceneName: scene });
		} else {
			await this.call("SetCurrentProgramScene", { sceneName: scene });
		}
		clearTimeout(this.pickTimer);
		this.selected = null;
	}

	async onDialDown() {
		await this.switchTo(this.target());
	}
}

// Good steps and limits for the numbers in the filters that come with OBS.
export const FILTER_RANGES = {
	color_filter: { gamma: [-3, 3, 0.05], contrast: [-4, 4, 0.05], brightness: [-1, 1, 0.01], saturation: [-1, 5, 0.05], hue_shift: [-180, 180, 2], opacity: [0, 1, 0.01] },
	gain_filter: { db: [-30, 30, 0.5] },
	sharpness_filter: { sharpness: [0, 1, 0.01] },
	noise_suppress_filter: { suppress_level: [-60, 0, 1] },
	noise_gate_filter: { open_threshold: [-96, 0, 1], close_threshold: [-96, 0, 1] },
	compressor_filter: { ratio: [1, 32, 0.5], threshold: [-60, 0, 1], output_gain: [-32, 32, 0.5], attack_time: [1, 500, 5], release_time: [1, 1000, 10] },
	limiter_filter: { threshold: [-60, 0, 1], release_time: [1, 1000, 10] },
	expander_filter: { ratio: [1, 20, 0.5], threshold: [-60, 0, 1] },
	basic_eq_filter: { low: [-20, 20, 0.5], mid: [-20, 20, 0.5], high: [-20, 20, 0.5] },
	scroll_filter: { speed_x: [-500, 500, 10], speed_y: [-500, 500, 10] },
	crop_filter: { left: [0, 4000, 2], right: [0, 4000, 2], top: [0, 4000, 2], bottom: [0, 4000, 2] },
	gpu_delay: { delay_ms: [0, 500, 10] },
	async_delay_filter: { delay_ms: [0, 20000, 50] },
	chroma_key_filter: { similarity: [1, 1000, 5], smoothness: [1, 1000, 5], spill: [1, 1000, 5], opacity: [0, 1, 0.01], contrast: [-4, 4, 0.05], brightness: [-1, 1, 0.01], gamma: [-1, 1, 0.01] },
	color_key_filter: { similarity: [1, 1000, 5], smoothness: [1, 1000, 5], opacity: [0, 1, 0.01], contrast: [-4, 4, 0.05], brightness: [-1, 1, 0.01], gamma: [-1, 1, 0.01] },
	luma_key_filter: { luma_max: [0, 1, 0.01], luma_min: [0, 1, 0.01], luma_max_smooth: [0, 1, 0.01], luma_min_smooth: [0, 1, 0.01] },
	mask_filter: { opacity: [0, 1, 0.01] },
};

// Readable names for the numbers in the filters above. Others get their key, a bit tidied up.
const FILTER_VALUE_LABELS = {
	gamma: "Gamma",
	contrast: "Contrast",
	brightness: "Brightness",
	saturation: "Saturation",
	hue_shift: "Hue shift",
	opacity: "Opacity",
	db: "Gain",
	sharpness: "Sharpness",
	suppress_level: "Suppression",
	open_threshold: "Open threshold",
	close_threshold: "Close threshold",
	ratio: "Ratio",
	threshold: "Threshold",
	output_gain: "Output gain",
	attack_time: "Attack",
	release_time: "Release",
	low: "Low",
	mid: "Mid",
	high: "High",
	speed_x: "Speed X",
	speed_y: "Speed Y",
	left: "Left",
	right: "Right",
	top: "Top",
	bottom: "Bottom",
	delay_ms: "Delay",
	similarity: "Similarity",
	smoothness: "Smoothness",
	spill: "Spill reduction",
};

export function filterValueLabel(key) {
	const text = String(key ?? "").replace(/_/g, " ");
	return FILTER_VALUE_LABELS[key] ?? text.charAt(0).toUpperCase() + text.slice(1);
}

export function filterRange(kind, key) {
	const base = String(kind ?? "").replace(/_v\d+$/, "");
	return FILTER_RANGES[base]?.[key] ?? null;
}

function formatNumber(value, step) {
	const decimals = Math.min(3, Math.max(0, -Math.floor(Math.log10(step || 1))));
	return num(value, decimals);
}

// Turn to change one number of a filter (blur, gain, opacity...). Press to turn the filter on or off.
export class FilterDial extends Action {
	static uuid = `${ID}.filterdial`;
	static required = ["source", "filter", "setting"];
	static defaults = { source: "", filter: "", setting: "", step: "", min: "", max: "", pressAction: "toggle", disableTouch: false };

	layout() {
		return "layouts/dial.json";
	}

	async onConnected() {
		await this.load();
		// OBS sends no event when filter settings change, so check now and then.
		this.stopPoll?.();
		this.stopPoll = this.interval(() => this.refresh(), 2000);
	}

	onDisconnected() {
		this.stopPoll?.();
	}

	async onSettingsChanged(old) {
		const s = this.settings;
		if (this.online && (old.source !== s.source || old.filter !== s.filter || old.setting !== s.setting)) await this.load();
	}

	// Fresh settings from OBS. Skipped while the dial is turned quickly.
	async refresh(maxAge = 0) {
		if (!this.filter || !this.visible || Date.now() - (this.readAt ?? 0) < maxAge) return;
		const { source, filter } = this.settings;
		try {
			const fresh = await this.call("GetSourceFilter", { sourceName: source, filterName: filter });
			this.readAt = Date.now();
			const changed = JSON.stringify(fresh.filterSettings) !== JSON.stringify(this.filter.filterSettings) || fresh.filterEnabled !== this.filter.filterEnabled;
			this.filter = fresh;
			if (changed) this.render();
		} catch {}
	}

	async load() {
		this.filter = null;
		const { source, filter } = this.settings;
		if (!source || !filter) return;
		try {
			this.filter = await this.call("GetSourceFilter", { sourceName: source, filterName: filter });
			this.defaults = (await this.call("GetSourceFilterDefaultSettings", { filterKind: this.filter.filterKind })).defaultFilterSettings ?? {};
		} catch {
			this.filter = null;
		}
	}

	// [min, max, step]: from the settings page, the table above, or a guess.
	range() {
		const s = this.settings;
		const known = filterRange(this.filter?.filterKind, s.setting);
		const guessMax = Math.abs(Number(this.defaults?.[s.setting]) || 0) <= 1 ? 1 : 100;
		const min = s.min !== "" && s.min !== null ? Number(s.min) : known ? known[0] : 0;
		const max = s.max !== "" && s.max !== null ? Number(s.max) : known ? known[1] : guessMax;
		const step = Number(s.step) > 0 ? Number(s.step) : known ? known[2] : (max - min) / 100;
		return [min, max, step];
	}

	value() {
		const key = this.settings.setting;
		const v = this.filter?.filterSettings?.[key] ?? this.defaults?.[key];
		return typeof v === "number" ? v : Number(v) || 0;
	}

	onObsEvent(type, data) {
		if (data.sourceName !== this.settings.source || data.filterName !== this.settings.filter || !this.filter) return;
		if (type === "SourceFilterSettingsChanged") this.filter.filterSettings = data.filterSettings;
		else if (type === "SourceFilterEnableStateChanged") this.filter.filterEnabled = data.filterEnabled;
		else return;
		this.render();
	}

	// Without OBS or settings: the name, the status, and the setup dot on the icon.
	renderOffline() {
		this.setFeedback({ title: this.settings.filter || t("Filter"), icon: dialIcon("filter", { setup: !this.isSetUp() }), value: this.dialStatus(), indicator: { value: 0 } });
	}

	render() {
		if (!this.isSetUp()) return this.renderOffline();
		const { filter, setting } = this.settings;
		if (!this.filter) {
			// The filter is not in OBS (any more).
			this.setFeedback({ title: filter, icon: dialIcon("filter"), value: "?", indicator: { value: 0 } });
			return;
		}
		const [min, max, step] = this.range();
		const value = this.value();
		const on = this.filter.filterEnabled;
		this.setFeedback({
			title: `${filter} · ${t(filterValueLabel(setting))}`,
			icon: dialIcon("filter"),
			value: { value: on ? formatNumber(value, step) : t("off"), color: on ? COLORS.white : COLORS.gray },
			indicator: { value: max > min ? Math.round(((value - min) / (max - min)) * 100) : 0, bar_fill_c: on ? COLORS.white : COLORS.gray },
		});
	}

	async onDialRotate(ticks) {
		const { source, filter, setting } = this.settings;
		if (!this.filter) await this.load();
		if (!this.filter || !setting) throw new Error("no filter setting selected");
		await this.refresh(300);
		const [min, max, step] = this.range();
		const value = Math.min(max, Math.max(min, Math.round((this.value() + ticks * step) / step) * step));
		const rounded = Math.round(value * 10000) / 10000;
		const isInt = Number.isInteger(this.defaults?.[setting]) && Number.isInteger(step);
		await this.call("SetSourceFilterSettings", { sourceName: source, filterName: filter, filterSettings: { [setting]: isInt ? Math.round(rounded) : rounded }, overlay: true });
		this.filter.filterSettings = { ...this.filter.filterSettings, [setting]: rounded };
		this.readAt = Date.now();
		this.render();
	}

	async onDialDown() {
		const { source, filter, setting, pressAction } = this.settings;
		if (!this.filter) await this.load();
		if (!this.filter) throw new Error("no filter selected");
		if (pressAction === "reset" && setting) {
			const value = this.defaults?.[setting] ?? 0;
			await this.call("SetSourceFilterSettings", { sourceName: source, filterName: filter, filterSettings: { [setting]: value }, overlay: true });
			this.filter.filterSettings = { ...this.filter.filterSettings, [setting]: value };
		} else if (pressAction === "toggle") {
			const enabled = !this.filter.filterEnabled;
			await this.call("SetSourceFilterEnabled", { sourceName: source, filterName: filter, filterEnabled: enabled });
			this.filter.filterEnabled = enabled;
		}
		this.render();
	}
}
