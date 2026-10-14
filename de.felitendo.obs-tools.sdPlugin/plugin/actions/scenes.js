import { Action } from "./base.js";
import { COLORS } from "../lib/icons.js";
import { glyphKey, isColor, sceneKey } from "../lib/render.js";

const ID = "de.felitendo.obs-tools";

// Press once to put the scene in preview (studio mode), press again to go live.
export class SmartScene extends Action {
	static uuid = `${ID}.smartscene`;
	static required = ["scene"];
	// switch: smart (studio mode: preview first, then live). live: straight live. preview: only to preview.
	static pressKey = "press";
	static ops = ["switch", "live", "preview"];
	static defaults = { scene: "", press: "switch", hold: "none", double: "none", title: "", showTitle: true, showPreview: true, previewFit: "fit", previewInterval: 2, liveColor: COLORS.red, previewColor: COLORS.green };

	onConnected() {
		this.shot = null;
		this.startPreview();
		return this.refreshShot();
	}

	onDisconnected() {
		this.stopPreview?.();
	}

	onDisappear() {
		this.stopPreview?.();
	}

	renderOffline() {
		this.ownTitle(this.title());
		this.setImage(sceneKey({ title: this.title() }));
	}

	startPreview() {
		this.stopPreview?.();
		const seconds = Number(this.settings.previewInterval);
		if (this.settings.showPreview && seconds > 0) this.stopPreview = this.interval(() => this.refreshShot(), Math.max(500, seconds * 1000));
	}

	async onSettingsChanged(old) {
		if (old.scene !== this.settings.scene || old.showPreview !== this.settings.showPreview) this.shot = null;
		if (!this.online) return this.renderOffline();
		this.startPreview();
		await this.refreshShot();
	}

	onStateChange(what) {
		this.render();
		if (what === "program" || what === "preview") this.refreshShot();
	}

	async refreshShot() {
		const { scene, showPreview } = this.settings;
		if (!this.visible || !this.online || !scene || !showPreview) return;
		try {
			const result = await this.call("GetSourceScreenshot", { sourceName: scene, imageFormat: "jpg", imageWidth: 256, imageCompressionQuality: 70 });
			this.shot = result.imageData;
		} catch {
			this.shot = null;
		}
		this.render();
	}

	title() {
		const { showTitle, title, scene } = this.settings;
		return showTitle ? title || scene : "";
	}

	render() {
		const { scene, liveColor, previewColor, showPreview, previewFit } = this.settings;
		const state = this.obsState;
		const live = !!scene && state.programScene === scene;
		const preview = !!scene && state.studioMode && state.previewScene === scene;
		const border = live ? (isColor(liveColor) ? liveColor : COLORS.red) : preview ? (isColor(previewColor) ? previewColor : COLORS.green) : null;
		this.ownTitle(this.title());
		this.setImage(sceneKey({ image: showPreview ? this.shot : null, border, fit: previewFit, title: this.title() }));
	}

	async onPress(args) {
		const { scene } = this.settings;
		if (!scene) throw new Error("no scene selected");
		const state = this.obsState;
		const op = this.op(args);
		if (op === "preview") {
			if (!state.studioMode) throw new Error("studio mode is off");
			await this.call("SetCurrentPreviewScene", { sceneName: scene });
		} else if (op === "switch" && state.studioMode && !this.isInMultiAction) {
			if (state.previewScene === scene) await this.call("TriggerStudioModeTransition");
			else await this.call("SetCurrentPreviewScene", { sceneName: scene });
		} else {
			await this.call("SetCurrentProgramScene", { sceneName: scene });
		}
	}
}

export class StudioMode extends Action {
	static uuid = `${ID}.studiomode`;
	static pressKey = "mode";
	static ops = ["toggle", "enable", "disable", "take"];
	static defaults = { mode: "toggle", hold: "none", double: "none" };

	render() {
		this.setState(this.obsState.studioMode ? 1 : 0);
	}

	async onPress(args) {
		const current = this.obsState.studioMode;
		const mode = this.op(args);
		// take: the transition button of OBS, the preview goes live.
		if (mode === "take") {
			if (!current) throw new Error("studio mode is off");
			await this.call("TriggerStudioModeTransition");
			return;
		}
		const enabled = mode === "enable" ? true : mode === "disable" ? false : !current;
		await this.call("SetStudioModeEnabled", { studioModeEnabled: enabled });
	}
}

// Back to the scene that was live before the current one.
export class PreviousScene extends Action {
	static uuid = `${ID}.previousscene`;
	static defaults = { showName: true };

	render() {
		const previous = this.obsState.previousScene ?? "";
		this.glyphForTitle(this.settings.showName ? previous : "");
	}

	async onPress() {
		const previous = this.obsState.previousScene;
		if (!previous) throw new Error("no previous scene yet");
		await this.call("SetCurrentProgramScene", { sceneName: previous });
	}
}

export class NextScene extends Action {
	static uuid = `${ID}.nextscene`;
	static pressKey = "direction";
	static ops = ["next", "previous"];
	static defaults = { direction: "next", hold: "none", double: "none", wrap: true, showName: true };

	target(programScene = this.obsState.programScene, direction = this.settings.direction) {
		const { scenes } = this.obsState;
		if (!scenes.length) return null;
		const index = scenes.indexOf(programScene);
		let next = index + (direction === "previous" ? -1 : 1);
		if (next < 0 || next >= scenes.length) {
			if (!this.settings.wrap) return null;
			next = (next + scenes.length) % scenes.length;
		}
		return scenes[next];
	}

	// The icon points where a press goes: on in the list, or up.
	glyphName() {
		return this.settings.direction === "previous" ? "previous" : "next";
	}

	renderOffline() {
		this.setImage(glyphKey(this.glyphName()));
	}

	render() {
		const title = this.settings.showName ? (this.target() ?? "") : "";
		this.ownTitle(title);
		this.setImage(glyphKey(this.glyphName(), { title }));
	}

	async onPress(args) {
		// Ask OBS, the cached scene can lag behind a fast double press.
		const { sceneName } = await this.call("GetCurrentProgramScene");
		const target = this.target(sceneName, this.op(args));
		if (!target) throw new Error("no next scene");
		await this.call("SetCurrentProgramScene", { sceneName: target });
	}
}

export class SetTransition extends Action {
	static uuid = `${ID}.settransition`;
	static required = ["transition"];
	static defaults = { transition: "", duration: "", showName: true };

	active() {
		const { transition, duration } = this.settings;
		const current = this.obsState.transition;
		if (!transition || current.name !== transition) return false;
		return duration === "" || duration === null || current.duration === null || Number(duration) === current.duration;
	}

	// Offline it is not the active transition, so it is gray.
	renderOffline() {
		this.setState(0);
		this.renderTitle();
	}

	render() {
		this.setState(this.active() ? 1 : 0);
		this.renderTitle();
	}

	renderTitle() {
		const { showName, transition } = this.settings;
		this.glyphForTitle(showName ? transition : "");
	}

	async onPress() {
		const { transition, duration } = this.settings;
		if (!transition) throw new Error("no transition selected");
		await this.call("SetCurrentSceneTransition", { transitionName: transition });
		if (duration !== "" && duration !== null && Number(duration) >= 50) {
			try {
				await this.call("SetCurrentSceneTransitionDuration", { transitionDuration: Math.round(Number(duration)) });
			} catch (error) {
				// Fixed transitions like Cut have no duration.
				this.log(`duration not set: ${error.message}`);
			}
		}
		this.render();
	}
}
