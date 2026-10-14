import { COLORS } from "../lib/icons.js";
import { KEY_IMAGES } from "../lib/key-images.js";
import { glyphKey, statusKey, statusLabel } from "../lib/render.js";

export const LONG_PRESS_MS = 700;
export const DOUBLE_PRESS_MS = 300;
const REPEAT_DELAY_MS = 450;
const REPEAT_EVERY_MS = 140;

// Settings that hold names of OBS things, so renames in OBS can follow.
const INPUT_FIELDS = ["input", "source", "mediaInput"];
const SCENE_FIELDS = ["scene", "replayScene", "source", "animationScene"];

export class Action {
	static uuid = "";
	static defaults = {};
	// Press, hold and double press. An action that can do more than one thing lists it in `ops`
	// and keeps the press choice in the setting `pressKey`. The settings `hold` and `double` pick
	// from the same list. onPress gets the choice as `op`.
	static pressKey = "";
	static ops = [];
	// Press choices that use the key going down and up (push to talk): no hold or double press then.
	static pressOnly = [];
	// Hold can also repeat the press while the key is held ("repeat").
	static canRepeat = false;
	// Settings a key needs before it can work. Until then it shows a setup image.
	static required = [];

	constructor(plugin, event) {
		this.plugin = plugin;
		this.sd = plugin.sd;
		this.context = event.context;
		this.action = event.action;
		this.device = event.device;
		const payload = event.payload ?? {};
		this.controller = payload.controller ?? "Keypad";
		this.isInMultiAction = !!payload.isInMultiAction;
		this.state = payload.state ?? 0;
		this.settings = this.withDefaults(payload.settings);
		this.visible = false;
		this.online = false;
		this.conn = null;
		this.unsubscribe = [];
		this.timers = new Set();
		this.sent = {};
		this.imageOverridden = false;
	}

	withDefaults(settings) {
		return { ...this.constructor.defaults, ...(settings ?? {}) };
	}

	get obs() {
		return this.conn?.client;
	}

	get obsState() {
		return this.conn?.state;
	}

	log(message) {
		this.sd.log(`${this.constructor.name}: ${message}`);
	}

	// null means the default connection from the setup.
	connectionConfig() {
		const s = this.settings;
		if (s.customConnection && String(s.host ?? "").trim()) {
			return { host: String(s.host).trim(), port: Number(s.port) || 4455, password: s.password ?? "" };
		}
		return null;
	}

	attach() {
		this.detachConnection();
		const conn = this.plugin.connections.acquire(this.connectionConfig());
		this.conn = conn;
		const onStatus = () => this.refreshOnline();
		const onChange = (what) => {
			this.refreshOnline();
			if (this.online && what !== "all") this.safe(() => this.onStateChange(what));
		};
		const onEvent = (type, data) => {
			if (!this.online) return;
			this.followRenames(type, data);
			this.safe(() => this.onObsEvent(type, data));
		};
		conn.client.on("status", onStatus);
		conn.client.on("event", onEvent);
		conn.state.on("change", onChange);
		this.unsubscribe.push(() => {
			conn.client.off("status", onStatus);
			conn.client.off("event", onEvent);
			conn.state.off("change", onChange);
		});
		this.online = false;
		this.refreshOnline();
	}

	detachConnection() {
		for (const fn of this.unsubscribe.splice(0)) fn();
		if (this.online) {
			this.online = false;
			this.safe(() => this.onDisconnected());
		}
		this.conn?.release();
		this.conn = null;
	}

	refreshOnline() {
		const online = !!(this.conn?.client.connected && this.conn.state.ready);
		if (online && !this.online) {
			this.online = true;
			this.clearStatusImage();
			this.safe(async () => {
				await this.onConnected();
				this.draw();
			});
		} else if (!online) {
			if (this.online) {
				this.online = false;
				this.safe(() => this.onDisconnected());
			}
			this.showStatusImage();
		}
	}

	showStatusImage() {
		const status = this.conn?.client.status;
		if (!this.visible || status === "connecting" || status === "idle" || !status) return;
		this.statusShown = status;
		// Dials show it as text in their offline look (see dialStatus).
		if (this.controller === "Encoder") this.renderOffline();
		else this.setImage(statusKey(status), undefined, true);
	}

	clearStatusImage() {
		if (!this.statusShown) return;
		this.statusShown = null;
		// Dials draw again once they are online.
		if (this.controller === "Encoder") return;
		if (this.setupShown) this.setImage(this.setupImage(), undefined, true);
		else this.resetImage();
	}

	// What a dial shows instead of its value while OBS cannot be reached: the words and colors
	// of the status image of a key. Empty while connecting.
	dialStatus() {
		const status = this.statusShown;
		if (!status) return "";
		return { value: statusLabel(status), color: status === "offline" ? COLORS.muted : COLORS.amber };
	}

	// The touch strip layout of a dial (layouts/*.json).
	layout() {
		return null;
	}

	setLayout() {
		const layout = this.controller === "Encoder" ? this.layout() : null;
		if (!layout) return;
		this.sent.feedback = null;
		this.sd.setFeedbackLayout(this.context, layout);
		this.resendFeedbackSoon();
	}

	// A key is set up when its required settings are filled in.
	isSetUp() {
		return this.constructor.required.every((key) => String(this.settings[key] ?? "").trim() !== "");
	}

	// The glyph of the action in gray with a yellow dot: "pick something first".
	setupImage() {
		return glyphKey(this.plugin.actionInfo?.get(this.action)?.icon ?? "plugin", { color: COLORS.gray, setup: true });
	}

	// Draw the key, or the setup image while it is not set up. Dials show this in their feedback.
	draw() {
		if (this.showSetup()) return;
		this.render();
	}

	drawOffline() {
		if (this.showSetup()) return;
		this.renderOffline();
	}

	showSetup() {
		if (this.controller !== "Keypad") return false;
		if (!this.isSetUp()) {
			this.setupShown = true;
			if (!this.statusShown) this.setImage(this.setupImage(), undefined, true);
			return true;
		}
		if (this.setupShown) {
			this.setupShown = false;
			this.resetImage(true);
		}
		return false;
	}

	// A key image from the manifest, drawn again with a title under the glyph.
	stateImage(state = this.state, title = "") {
		const name = this.plugin.actionInfo?.get(this.action)?.states?.[state];
		const spec = KEY_IMAGES[name] ?? { glyph: this.plugin.actionInfo?.get(this.action)?.icon ?? "plugin" };
		return glyphKey(spec.glyph, { color: spec.color ?? COLORS.white, title });
	}

	// The name of the source (or filter) as title, on keys whose pictures come from the manifest.
	showNameTitle(name) {
		this.glyphForTitle(this.settings.showName && name ? String(name) : "");
	}

	// Lifecycle from the plugin.
	appear(event) {
		this.visible = !this.isInMultiAction;
		if (event?.payload) {
			this.state = event.payload.state ?? this.state;
			this.controller = event.payload.controller ?? this.controller;
		}
		// The touch strip layout first, before any feedback is sent.
		this.setLayout();
		this.attach();
		if (!this.online) this.drawOffline();
	}

	disappear() {
		this.visible = false;
		this.clearTimers();
		this.safe(() => this.onDisappear());
		this.detachConnection();
	}

	updateSettings(settings) {
		const old = this.settings;
		const before = JSON.stringify(this.connectionConfig());
		this.settings = this.withDefaults(settings);
		if (JSON.stringify(this.connectionConfig()) !== before) this.attach();
		this.safe(async () => {
			await this.onSettingsChanged(old);
			if (this.online) this.draw();
			else this.drawOffline();
		});
	}

	saveSettings(patch) {
		this.settings = { ...this.settings, ...patch };
		this.sd.setSettings(this.context, this.settings);
	}

	// What press, hold or double press does: one of `ops`, "none", or "repeat" for hold.
	gesture(name) {
		const C = this.constructor;
		if (!C.ops.length) return name === "press" ? "press" : "none";
		const press = C.ops.includes(this.settings[C.pressKey]) ? this.settings[C.pressKey] : C.ops[0];
		if (name === "press") return press;
		if (C.pressOnly.includes(press)) return "none";
		const value = this.settings[name];
		if (name === "hold" && value === "repeat" && C.canRepeat) return "repeat";
		return C.ops.includes(value) && !C.pressOnly.includes(value) ? value : "none";
	}

	// The choice for this call of onPress.
	op(args) {
		return args?.op ?? this.gesture("press");
	}

	press(name, repeat = false) {
		return this.onPress({ op: this.gesture(name), gesture: name, repeat });
	}

	// Key handling. In a multi action there are no gestures, every press is a short press.
	async keyDown(event) {
		this.isInMultiAction = !!event.payload?.isInMultiAction;
		if (!(await this.ensureOnline())) return;
		const hold = this.isInMultiAction ? "none" : this.gesture("hold");
		const double = this.isInMultiAction ? "none" : this.gesture("double");
		clearTimeout(this.repeatTimer);
		if (double === "none" && (hold === "none" || hold === "repeat")) {
			// Right away on key down. "repeat" goes on while the key is held.
			this.gesturing = false;
			if (hold === "repeat") this.startRepeat(REPEAT_DELAY_MS);
			await this.act(() => this.press("press"));
			return;
		}
		// Wait for key up (or the long press) to know which gesture it is.
		this.gesturing = true;
		this.doubleGesture = double;
		// A second press while the first one waits for its double press.
		this.secondPress = !!this.singleTimer;
		clearTimeout(this.singleTimer);
		this.singleTimer = null;
		this.longFired = false;
		clearTimeout(this.longTimer);
		this.longTimer = null;
		if (hold === "none") return;
		this.longTimer = setTimeout(
			async () => {
				this.longFired = true;
				// The first press of the pair still counts.
				if (this.secondPress) await this.act(() => this.press("press"));
				if (hold === "repeat") {
					await this.act(() => this.press("press"));
					this.startRepeat(REPEAT_EVERY_MS);
				} else if (await this.act(() => this.press("hold"))) this.showOk();
			},
			hold === "repeat" ? REPEAT_DELAY_MS : LONG_PRESS_MS,
		);
	}

	startRepeat(delay) {
		const tick = async () => {
			await this.act(() => this.press("press", true));
			this.repeatTimer = setTimeout(tick, REPEAT_EVERY_MS);
		};
		this.repeatTimer = setTimeout(tick, delay);
	}

	async keyUp() {
		clearTimeout(this.repeatTimer);
		if (this.gesturing) {
			this.gesturing = false;
			clearTimeout(this.longTimer);
			this.longTimer = null;
			if (this.longFired || !this.online) return;
			if (this.secondPress) {
				this.secondPress = false;
				await this.act(() => this.press("double"));
			} else if (this.doubleGesture !== "none") {
				// Wait a moment: a second press makes it a double press.
				this.singleTimer = setTimeout(() => {
					this.singleTimer = null;
					this.act(() => this.press("press"));
				}, DOUBLE_PRESS_MS);
			} else {
				await this.act(() => this.press("press"));
			}
			return;
		}
		// Not act(): letting go of the key must not clear the error of the press.
		await this.safe(() => this.onRelease());
	}

	// A fresh instance (for example in a multi action) may still be connecting.
	async ensureOnline(timeout = 3000) {
		if (this.online) return true;
		if (!this.conn) this.attach();
		const until = Date.now() + timeout;
		while (!this.online && Date.now() < until) await new Promise((r) => setTimeout(r, 50));
		if (!this.online) this.showAlert();
		return this.online;
	}

	async safe(fn) {
		try {
			return await fn();
		} catch (error) {
			this.reportError(error);
			return undefined;
		}
	}

	// Something the user did (press, turn, touch). Success clears the last error.
	async act(fn) {
		try {
			await fn();
			this.clearError();
			return true;
		} catch (error) {
			this.reportError(error);
			return false;
		}
	}

	// The settings page shows the last error, so the user sees why the key showed an alert.
	reportError(error) {
		const message = String(error?.message ?? error);
		this.log(`error: ${message}`);
		this.showAlert();
		if (this.lastError === message) return;
		this.lastError = message;
		this.plugin.stateChanged?.(this.context);
	}

	clearError() {
		if (!this.lastError) return;
		this.lastError = null;
		this.plugin.stateChanged?.(this.context);
	}

	// Errors bubble up to safe(), which shows the alert.
	call(type, data) {
		if (!this.obs) return Promise.reject(new Error("no connection"));
		return this.obs.call(type, data);
	}

	followRenames(type, data) {
		const patch = {};
		if (type === "InputNameChanged") {
			for (const key of INPUT_FIELDS) if (this.settings[key] && this.settings[key] === data.oldInputName) patch[key] = data.inputName;
		} else if (type === "SceneNameChanged") {
			for (const key of SCENE_FIELDS) if (this.settings[key] && this.settings[key] === data.oldSceneName) patch[key] = data.sceneName;
		} else if (type === "SourceFilterNameChanged") {
			if (this.settings.filter === data.oldFilterName && this.settings.source === data.sourceName) patch.filter = data.filterName;
		}
		if (Object.keys(patch).length) {
			this.saveSettings(patch);
			this.safe(async () => {
				await this.onSettingsChanged(this.settings);
				this.draw();
			});
		}
	}

	// Timers that stop when the key goes away.
	interval(fn, ms) {
		const id = setInterval(() => this.safe(fn), ms);
		this.timers.add(id);
		return () => {
			clearInterval(id);
			this.timers.delete(id);
		};
	}

	clearTimers() {
		for (const id of this.timers) clearInterval(id);
		this.timers.clear();
		clearTimeout(this.longTimer);
		clearTimeout(this.repeatTimer);
		clearTimeout(this.singleTimer);
		this.singleTimer = null;
	}

	// Output helpers. They skip sends that would not change anything.
	setTitle(title, state) {
		if (!this.visible) return;
		const key = `title:${state}`;
		if (this.sent[key] === title) return;
		this.sent[key] = title;
		if (state === undefined) for (const k of Object.keys(this.sent)) if (k.startsWith("title:")) this.sent[k] = title;
		this.sd.setTitle(this.context, title, state);
	}

	// Keys draw their titles themselves, close to the glyph. A title in OpenDeck would be a second
	// one (older versions set it), so it is cleared while the key shows its own. Without one, a title
	// the user typed stays.
	ownTitle(title) {
		if (title) this.setTitle("");
	}

	// force: also while the setup or status image is shown.
	setImage(image, state, force = false) {
		if (!this.visible) return;
		if (!force && (this.setupShown || this.statusShown)) return;
		const key = `image:${state}`;
		if (this.sent[key] === image) return;
		this.sent[key] = image;
		if (state === undefined) for (const k of Object.keys(this.sent)) if (k.startsWith("image:")) this.sent[k] = image;
		this.imageOverridden = true;
		this.sd.setImage(this.context, image, state);
	}

	// The manifest pictures have the glyph in the middle. With a title, draw it with the title below.
	glyphForTitle(title) {
		this.ownTitle(title);
		if (title) this.setImage(this.stateImage(this.state, title));
		else this.resetImage();
	}

	// Back to the image from the manifest (or the one the user picked).
	resetImage(force = false) {
		if (!force && (this.setupShown || this.statusShown)) return;
		if (!this.imageOverridden) return;
		this.imageOverridden = false;
		for (const k of Object.keys(this.sent)) if (k.startsWith("image:")) delete this.sent[k];
		if (this.visible) this.sd.setImage(this.context, "");
	}

	setState(state) {
		if (!this.visible && !this.isInMultiAction) return;
		if (this.state === state && this.sent.state === state) return;
		this.state = state;
		this.sent.state = state;
		this.sd.setState(this.context, state);
	}

	// OpenDeck loads a custom touch strip layout again after the key appeared,
	// which drops what was sent before. So send it again a little later.
	resendFeedbackSoon() {
		for (const ms of [700, 2500]) {
			const timer = setTimeout(() => {
				this.timers.delete(timer);
				if (!this.visible) return;
				this.sent.feedback = null;
				if (this.online) this.safe(() => this.render());
				else this.renderOffline();
			}, ms);
			this.timers.add(timer);
		}
	}

	setFeedback(payload) {
		if (!this.visible) return;
		const text = JSON.stringify(payload);
		if (this.sent.feedback === text) return;
		this.sent.feedback = text;
		this.sd.setFeedback(this.context, payload);
	}

	showOk() {
		if (this.visible) this.sd.showOk(this.context);
	}

	showAlert() {
		if (this.visible) this.sd.showAlert(this.context);
	}

	sendToPI(payload) {
		this.sd.sendToPropertyInspector(this.context, payload);
	}


	// Hooks for subclasses.
	onDisappear() {}
	onConnected() {}
	onDisconnected() {}
	onStateChange() {
		this.draw();
	}
	onObsEvent() {}
	onSettingsChanged() {}
	onPress() {}
	onRelease() {}
	onDialRotate() {}
	onDialDown() {}
	onDialUp() {}
	// A touch does what a press does, unless touches are off.
	onTouchTap(payload) {
		if (!this.settings.disableTouch) return this.onDialDown(payload);
	}
	onPiMessage() {}
	render() {}
	renderOffline() {}
}
