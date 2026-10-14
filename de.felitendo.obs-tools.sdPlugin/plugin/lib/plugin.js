// Routes OpenDeck events to the action instances and answers the settings pages.
import fs from "node:fs";
import path from "node:path";

import { ConnectionManager, DEFAULT_GLOBAL, connectionKey, readLocalObsConfig, resolveConnection } from "./connections.js";
import { pickFile, pickFolder, sessionType } from "./desktop.js";
import { Twitch } from "./twitch.js";
import { filterRange, filterValueLabel } from "../actions/dials.js";

const AUDIO_CAP = 1 << 1;
const MEDIA_KINDS = new Set(["ffmpeg_source", "vlc_source"]);
const AUDIO_KINDS = new Set(["pulse_input_capture", "pulse_output_capture", "alsa_input_capture", "jack_output_capture", "pipewire-audio-capture-source", "wasapi_input_capture", "wasapi_output_capture", "wasapi_process_output_capture", "coreaudio_input_capture", "coreaudio_output_capture", "ffmpeg_source", "vlc_source", "browser_source"]);

export class Plugin {
	constructor(sd, actionClasses) {
		this.sd = sd;
		this.classes = new Map(actionClasses.map((c) => [c.uuid, c]));
		this.instances = new Map();
		this.openInspectors = new Set();
		// Icon and state images of every action, by UUID (from manifest.json).
		const manifest = JSON.parse(fs.readFileSync(new URL("../../manifest.json", import.meta.url), "utf8"));
		this.actionInfo = new Map(manifest.Actions.map((a) => [a.UUID, { icon: path.basename(a.Icon), states: (a.States ?? []).map((st) => path.basename(st.Image ?? "")) }]));
		this.globalSettings = { ...DEFAULT_GLOBAL };
		this.connections = new ConnectionManager((message) => sd.log(message));
		this.twitch = new Twitch({
			save: (config) => this.saveGlobal({ twitch: config }),
			log: (message) => sd.log(message),
		});

		this.connections.on("default-changed", (config) => {
			const key = connectionKey(config);
			for (const instance of this.instances.values()) {
				if (!instance.connectionConfig() && instance.conn && instance.conn.key !== key) instance.attach();
			}
			this.broadcastState();
		});
		this.connections.on("status", () => this.broadcastState());
		this.twitch.on("status", () => this.broadcastState());
		this.twitch.on("error", (error) => {
			sd.warn(`Twitch: ${error.message}`);
			this.broadcast({ type: "twitchError", message: error.message });
		});
		this.twitch.on("message", (message) => {
			for (const instance of this.instances.values()) instance.onChatMessage?.(message);
		});

		sd.on("message", (message) => this.onMessage(message));
	}

	saveGlobal(patch) {
		this.globalSettings = { ...this.globalSettings, ...patch };
		this.sd.setGlobalSettings(this.globalSettings);
		this.connections.setGlobalSettings(this.globalSettings);
		this.twitch.configure(this.globalSettings.twitch);
		this.broadcastState();
	}

	instanceFor(message) {
		let instance = this.instances.get(message.context);
		if (!instance) {
			const Class = this.classes.get(message.action);
			if (!Class) return null;
			instance = new Class(this, message.event === "sendToPlugin" ? { ...message, payload: {} } : message);
			this.instances.set(message.context, instance);
			// Multi action children never get willAppear, they only press.
			if (message.event !== "willAppear") instance.attach();
		}
		return instance;
	}

	// Press events carry the settings, which matters for multi action children.
	withSettings(message) {
		const instance = this.instanceFor(message);
		const settings = message.payload?.settings;
		if (instance && settings && JSON.stringify(instance.withDefaults(settings)) !== JSON.stringify(instance.settings)) {
			instance.updateSettings(settings);
		}
		return instance;
	}

	updateChatNeeds() {
		const wanted = [...this.instances.values()].some((i) => i.wantsChat?.());
		this.twitch.setChatWanted(wanted);
	}

	async onMessage(message) {
		const { event, context } = message;
		try {
			switch (event) {
				case "didReceiveGlobalSettings":
					this.globalSettings = { ...DEFAULT_GLOBAL, ...(message.payload?.settings ?? {}) };
					this.connections.setGlobalSettings(this.globalSettings);
					this.twitch.configure(this.globalSettings.twitch);
					this.broadcastState();
					break;
				case "willAppear":
					this.instanceFor(message)?.appear(message);
					this.updateChatNeeds();
					break;
				case "willDisappear": {
					const instance = this.instances.get(context);
					instance?.disappear();
					this.instances.delete(context);
					this.updateChatNeeds();
					break;
				}
				case "keyDown":
					await this.withSettings(message)?.keyDown(message);
					break;
				case "keyUp":
					await this.withSettings(message)?.keyUp(message);
					break;
				case "dialRotate": {
					const instance = this.withSettings(message);
					if (instance && (await instance.ensureOnline())) await instance.act(() => instance.onDialRotate(message.payload.ticks, message.payload.pressed));
					break;
				}
				case "dialDown":
				case "dialUp":
				case "touchTap": {
					const instance = this.withSettings(message);
					if (!instance || !(await instance.ensureOnline())) break;
					const hook = { dialDown: "onDialDown", dialUp: "onDialUp", touchTap: "onTouchTap" }[event];
					await instance.act(() => instance[hook](message.payload));
					break;
				}
				case "didReceiveSettings":
					this.instanceFor(message)?.updateSettings(message.payload?.settings);
					this.updateChatNeeds();
					break;
				case "propertyInspectorDidAppear":
					this.openInspectors.add(context);
					this.sendState(context);
					break;
				case "propertyInspectorDidDisappear":
					this.openInspectors.delete(context);
					break;
				case "sendToPlugin":
					await this.onInspectorMessage(message);
					break;
				case "systemDidWakeUp":
					for (const entry of this.connections.entries.values()) entry.client.reconnect();
					break;
			}
		} catch (error) {
			this.sd.warn(`${event}: ${error?.stack ?? error}`);
		}
	}

	// State shown at the top of every settings page.
	stateFor(context) {
		const instance = this.instances.get(context);
		const conn = instance?.conn;
		const resolved = this.connections.defaultConfig;
		const client = conn?.client;
		return {
			type: "state",
			connection: {
				status: client?.status ?? "idle",
				error: client?.lastError ?? "",
				version: client?.version ?? null,
				host: client?.host ?? resolved.host,
				port: client?.port ?? resolved.port,
				custom: !!instance?.connectionConfig(),
			},
			global: {
				host: this.globalSettings.host,
				port: this.globalSettings.port,
				hasPassword: !!this.globalSettings.password,
			},
			twitch: this.twitch.status(),
			session: sessionType(),
			settings: instance?.settings ?? null,
			keyError: instance?.lastError ?? null,
		};
	}

	// Something on the settings page of this key changed, like its last error.
	stateChanged(context) {
		if (this.openInspectors.has(context)) this.sendState(context);
	}

	sendState(context) {
		this.sd.sendToPropertyInspector(context, this.stateFor(context));
	}

	broadcastState() {
		for (const context of this.openInspectors) this.sendState(context);
	}

	broadcast(payload) {
		for (const context of this.openInspectors) this.sd.sendToPropertyInspector(context, payload);
	}

	async onInspectorMessage(message) {
		const { context, payload = {} } = message;
		const instance = this.instanceFor(message);
		const reply = (data) => this.sd.sendToPropertyInspector(context, { ...data, requestId: payload.requestId });
		this.openInspectors.add(context);

		switch (payload.type) {
			case "hello":
				this.sendState(context);
				break;
			case "getLists":
				reply({ type: "lists", lists: await this.lists(instance, payload.lists ?? [], payload) });
				break;
			case "saveConnection": {
				const { host, port, password } = payload.settings ?? {};
				const patch = { host: String(host ?? "").trim() || "127.0.0.1", port: Number(port) || 4455 };
				if (password !== undefined && password !== null) patch.password = password;
				this.saveGlobal(patch);
				this.connections.entries.get(connectionKey(this.connections.defaultConfig))?.client.reconnect();
				reply({ type: "saved" });
				break;
			}
			// Only when the user asks for it: take port and password from OBS on this PC.
			case "readLocalObs": {
				const local = readLocalObsConfig();
				if (local) {
					this.saveGlobal({ host: "127.0.0.1", port: local.port, password: local.authRequired ? local.password : "" });
					this.connections.entries.get(connectionKey(this.connections.defaultConfig))?.client.reconnect();
				}
				reply({ type: "localObs", found: !!local, port: local?.port, authRequired: local?.authRequired, enabled: local?.enabled });
				break;
			}
			case "testConnection": {
				const settings = { ...this.globalSettings, ...(payload.settings ?? {}) };
				if (payload.settings?.password === undefined || payload.settings?.password === null) settings.password = this.globalSettings.password;
				const resolved = payload.direct ? { host: settings.host, port: Number(settings.port) || 4455, password: settings.password ?? "" } : resolveConnection(settings);
				const result = await this.connections.test(resolved);
				reply({ type: "testResult", ...result, host: resolved.host, port: resolved.port });
				break;
			}
			case "pickFile": {
				try {
					const file = payload.folder ? await pickFolder({ title: payload.title, start: payload.start }) : await pickFile({ title: payload.title, start: payload.start, images: payload.images });
					reply({ type: "pickedFile", key: payload.key, file });
				} catch (error) {
					reply({ type: "pickedFile", key: payload.key, file: null, error: error.message });
				}
				break;
			}
			case "openUrl":
				if (/^https?:\/\//.test(payload.url ?? "")) this.sd.openUrl(payload.url);
				break;
			case "twitchSave":
				this.saveGlobal({ twitch: { ...(this.globalSettings.twitch ?? {}), channel: String(payload.channel ?? "").trim() } });
				break;
			case "twitchDeviceFlow":
				try {
					const flow = await this.twitch.startDeviceFlow(payload.clientId);
					this.sd.openUrl(flow.uri);
					reply({ type: "twitchFlow", ...flow });
				} catch (error) {
					reply({ type: "twitchError", message: error.message });
				}
				break;
			case "twitchToken":
				try {
					await this.twitch.useToken(payload.token);
				} catch (error) {
					reply({ type: "twitchError", message: error.message });
				}
				break;
			case "twitchCancel":
				this.twitch.cancelDeviceFlow();
				break;
			case "twitchLogout":
				await this.twitch.logout();
				break;
			default:
				if (!instance) break;
				try {
					await instance.onPiMessage(payload, reply);
				} catch (error) {
					instance.log(`settings page: ${error.message}`);
					reply({ type: "error", for: payload.type, message: error.message });
				}
		}
	}

	// Lists for the drop downs, read from the OBS this key talks to.
	async lists(instance, names, payload) {
		const out = {};
		const client = instance?.conn?.client;
		if (!client?.connected) return out;
		const call = (type, data) => client.call(type, data).catch(() => null);
		let inputs = null;
		const getInputs = async () => (inputs ??= (await call("GetInputList"))?.inputs ?? []);
		for (const name of names) {
			if (name === "scenes") {
				const list = await call("GetSceneList");
				out.scenes = (list?.scenes ?? []).sort((a, b) => b.sceneIndex - a.sceneIndex).map((s) => s.sceneName);
			} else if (name === "inputs") {
				out.inputs = (await getInputs()).map((i) => i.inputName);
			} else if (name === "audioInputs") {
				out.audioInputs = (await getInputs()).filter((i) => (i.inputKindCaps !== undefined ? i.inputKindCaps & AUDIO_CAP : AUDIO_KINDS.has(i.unversionedInputKind))).map((i) => i.inputName);
			} else if (name === "mediaInputs") {
				out.mediaInputs = (await getInputs()).filter((i) => MEDIA_KINDS.has(i.unversionedInputKind)).map((i) => i.inputName);
			} else if (name === "imageInputs") {
				out.imageInputs = (await getInputs()).filter((i) => i.unversionedInputKind === "image_source").map((i) => i.inputName);
			} else if (name === "browserInputs") {
				out.browserInputs = (await getInputs()).filter((i) => i.unversionedInputKind === "browser_source").map((i) => i.inputName);
			} else if (name === "windowInputs") {
				out.windowInputs = (await getInputs()).filter((i) => i.unversionedInputKind === "xcomposite_input").map((i) => i.inputName);
			} else if (name === "sources") {
				const list = await call("GetSceneList");
				const scenes = (list?.scenes ?? []).sort((a, b) => b.sceneIndex - a.sceneIndex).map((s) => s.sceneName);
				out.sources = [...(await getInputs()).map((i) => i.inputName), ...scenes];
			} else if (name === "transitions") {
				out.transitions = ((await call("GetSceneTransitionList"))?.transitions ?? []).map((t) => t.transitionName);
			} else if (name === "filters") {
				const source = payload.source;
				out.filters = source ? ((await call("GetSourceFilterList", { sourceName: source }))?.filters ?? []).map((f) => f.filterName) : [];
			} else if (name === "textInputs") {
				out.textInputs = (await getInputs()).filter((i) => String(i.unversionedInputKind).startsWith("text_")).map((i) => i.inputName);
			} else if (name === "profiles") {
				out.profiles = (await call("GetProfileList"))?.profiles ?? [];
			} else if (name === "collections") {
				out.collections = (await call("GetSceneCollectionList"))?.sceneCollections ?? [];
			} else if (name === "hotkeys") {
				const names = [...new Set((await call("GetHotkeyList"))?.hotkeys ?? [])];
				out.hotkeys = names.map((value) => ({ value, label: hotkeyLabel(value) })).sort((a, b) => a.label.localeCompare(b.label));
			} else if (name === "filterSettings") {
				out.filterSettings = await this.filterSettings(call, payload.source, payload.filter);
			} else if (name === "sceneItems") {
				const scene = payload.scene || instance.obsState?.programScene;
				out.sceneItems = scene ? ((await call("GetSceneItemList", { sceneName: scene }))?.sceneItems ?? []).map((i) => i.sourceName) : [];
			}
		}
		return out;
	}

	// The number settings of a filter, for the filter dial.
	async filterSettings(call, sourceName, filterName) {
		if (!sourceName || !filterName) return [];
		const filter = await call("GetSourceFilter", { sourceName, filterName });
		if (!filter) return [];
		const defaults = (await call("GetSourceFilterDefaultSettings", { filterKind: filter.filterKind }))?.defaultFilterSettings ?? {};
		const all = { ...defaults, ...(filter.filterSettings ?? {}) };
		return Object.keys(all)
			.filter((key) => typeof all[key] === "number" && !key.startsWith("color"))
			.sort()
			.map((key) => {
				const range = filterRange(filter.filterKind, key);
				return { value: key, label: filterValueLabel(key), range };
			});
	}
}

// "OBSBasic.StartRecording" -> "Start recording", "libobs.mute" -> "Mute (source)"
export function hotkeyLabel(name) {
	const [group, ...rest] = String(name).split(".");
	const words = rest
		.join(" ")
		.replace(/([a-z])([A-Z])/g, "$1 $2")
		.replace(/[_.]/g, " ")
		.toLowerCase()
		.trim();
	const label = (words.charAt(0).toUpperCase() + words.slice(1)).replace(/virtual cam$/, "virtual camera");
	const groups = { OBSBasic: "", libobs: "source", MediaSource: "media source", ReplayBuffer: "replay buffer" };
	const where = group in groups ? groups[group] : group;
	return where ? `${label} (${where})` : label;
}
