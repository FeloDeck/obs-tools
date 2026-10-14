import fs from "node:fs";

import { Action } from "./base.js";
import { t } from "../lib/i18n.js";
import { COLORS } from "../lib/icons.js";
import { glyphKey } from "../lib/render.js";
import { isAllowed } from "../lib/twitch.js";
import { playMedia, waitForEvent } from "../lib/media.js";
import { formatDuration, timecodeSeconds } from "../lib/time.js";

const ID = "de.felitendo.obs-tools";

// Start, stop or toggle an output. Can point at another OBS on the network.
class OutputToggle extends Action {
	static pressKey = "mode";
	static ops = ["toggle", "start", "stop"];
	static defaults = { mode: "toggle", hold: "none", double: "none", showTimer: true, timerAt: "below", customConnection: false, host: "", port: 4455, password: "" };
	static output = "";

	active() {
		return this.obsState[this.constructor.output].active;
	}

	onConnected() {
		this.stopTimer?.();
		this.stopTimer = this.interval(() => this.tick(), 1000);
	}

	onDisconnected() {
		this.stopTimer?.();
		this.time = "";
	}

	// The time goes above or below the glyph, so the key draws it into the image.
	render() {
		const active = this.active();
		this.setState(active ? 1 : 0);
		if (!active) this.time = "";
		if (active && this.settings.showTimer) {
			this.ownTitle(this.time);
			this.setImage(glyphKey(this.constructor.glyph, { color: COLORS.red, title: this.time, titleAt: this.settings.timerAt }));
		} else this.resetImage();
	}

	async tick() {
		if (!this.visible || !this.settings.showTimer || !this.active()) return;
		const status = await this.call(this.constructor.statusRequest);
		this.time = status.outputPaused ? t("Paused") : formatDuration(timecodeSeconds(status.outputTimecode));
		if (this.active()) this.render();
	}

	async onPress(args) {
		const [start, stop, toggle] = this.constructor.requests;
		const mode = this.op(args);
		if (mode === "start") {
			if (!this.active()) await this.call(start);
		} else if (mode === "stop") {
			if (this.active()) await this.call(stop);
		} else {
			await this.call(toggle);
		}
	}
}

export class RecordToggle extends OutputToggle {
	static uuid = `${ID}.recordtoggle`;
	static output = "record";
	static glyph = "record";
	static statusRequest = "GetRecordStatus";
	static requests = ["StartRecord", "StopRecord", "ToggleRecord"];

	static ops = ["toggle", "start", "stop", "pause"];

	async onPress(args) {
		if (this.op(args) === "pause") {
			if (this.active()) await this.call("ToggleRecordPause");
			else throw new Error("not recording");
			return;
		}
		await super.onPress(args);
	}
}

export class StreamToggle extends OutputToggle {
	static uuid = `${ID}.streamtoggle`;
	static output = "stream";
	static glyph = "stream";
	static statusRequest = "GetStreamStatus";
	static requests = ["StartStream", "StopStream", "ToggleStream"];
}

export class VirtualCam extends Action {
	static uuid = `${ID}.virtualcam`;
	static pressKey = "mode";
	static ops = ["toggle", "start", "stop"];
	static defaults = { mode: "toggle", hold: "none", double: "none" };

	render() {
		this.setState(this.obsState.virtualcam.active ? 1 : 0);
	}

	async onPress(args) {
		const active = this.obsState.virtualcam.active;
		const mode = this.op(args);
		if (mode === "start" || (mode === "toggle" && !active)) {
			if (active) return;
			// Without v4l2loopback, OBS asks for an admin password to load it. Give the user time.
			const loaded = fs.existsSync("/sys/module/v4l2loopback");
			if (!loaded) this.log("v4l2loopback is not loaded, OBS will ask for a password to load it");
			const started = waitForEvent(this.obs, "VirtualcamStateChanged", { timeout: loaded ? 5000 : 60000, filter: (d) => d.outputActive });
			await this.call("StartVirtualCam");
			try {
				await started;
			} catch {
				throw new Error(loaded ? "the virtual camera did not start" : "the virtual camera did not start: the v4l2loopback kernel module is not loaded");
			}
		} else if (active) {
			await this.call("StopVirtualCam");
		}
	}
}

// Save the replay buffer (and play it, clip it), or turn the replay buffer on or off.
export class InstantReplay extends Action {
	static uuid = `${ID}.instantreplay`;
	static pressKey = "press";
	static ops = ["save", "buffer", "bufferStart", "bufferStop"];
	static defaults = {
		press: "save",
		hold: "buffer",
		double: "none",
		autoReplay: false,
		mediaInput: "",
		replayScene: "",
		speed: 100,
		hideWhenDone: true,
		returnToScene: true,
		createClip: false,
		postClip: false,
		clipMessage: "Instant replay: {url}",
		chatCommand: false,
		command: "!replay",
		cooldown: 30,
		permission: "everyone",
	};

	// Auto replay needs its media source. Without auto replay the key just saves.
	isSetUp() {
		return !this.settings.autoReplay || !!this.settings.mediaInput;
	}

	render() {
		this.setState(this.obsState.replay.active ? 1 : 0);
	}

	wantsChat() {
		return this.visible && !!this.settings.chatCommand;
	}

	onChatMessage({ user, text }) {
		const { chatCommand, command, cooldown, permission } = this.settings;
		if (!chatCommand || !this.online) return;
		const word = String(text).trim().split(/\s+/)[0].toLowerCase();
		if (word !== String(command || "!replay").trim().toLowerCase()) return;
		if (!isAllowed(user, permission)) return;
		const now = Date.now();
		if (this.lastChat && now - this.lastChat < (Number(cooldown) || 0) * 1000) return;
		this.lastChat = now;
		this.log(`chat replay by ${user.login}`);
		this.safe(() => this.replay());
	}

	async onPress(args) {
		const op = this.op(args);
		const on = this.obsState.replay.active;
		if (op === "buffer") await this.call("ToggleReplayBuffer");
		else if (op === "bufferStart") {
			if (!on) await this.call("StartReplayBuffer");
		} else if (op === "bufferStop") {
			if (on) await this.call("StopReplayBuffer");
		} else await this.replay();
	}

	async replay() {
		if (this.busy) return;
		this.busy = true;
		try {
			const { autoReplay, mediaInput, createClip } = this.settings;
			const bufferOn = this.obsState.replay.active;
			if (!bufferOn && !createClip) throw new Error("the replay buffer is off");
			// The clip is made while the replay plays. Errors are thrown below, at the await.
			const clip = createClip ? this.clip() : null;
			clip?.catch(() => {});
			if (bufferOn) {
				const saved = waitForEvent(this.obs, "ReplayBufferSaved", { timeout: 20000 });
				await this.call("SaveReplayBuffer");
				let path;
				try {
					path = (await saved).savedReplayPath;
				} catch {
					path = (await this.call("GetLastReplayBufferReplay")).savedReplayPath;
				}
				this.log(`replay saved: ${path}`);
				if (autoReplay && mediaInput && path) await this.play(path);
			}
			if (clip) await clip;
			this.showOk();
		} finally {
			this.busy = false;
		}
	}

	async play(file) {
		const { mediaInput, replayScene, speed, hideWhenDone, returnToScene } = this.settings;
		await this.playback?.stop();
		this.playback = await playMedia(this.obs, this.obsState, {
			input: mediaInput,
			file,
			speed,
			scene: replayScene,
			showSource: true,
			hideWhenDone,
			returnToScene,
		});
		this.playback.done.then(() => (this.playback = null));
	}

	async clip() {
		const twitch = this.plugin.twitch;
		const clip = await twitch.createClip();
		this.log(`clip created: ${clip.url}`);
		if (this.settings.postClip) {
			const message = String(this.settings.clipMessage || "{url}").replaceAll("{url}", clip.url);
			await twitch.say(message.includes(clip.url) ? message : `${message} ${clip.url}`);
		}
		return clip;
	}
}
