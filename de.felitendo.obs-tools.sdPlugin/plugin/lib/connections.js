// Shares one OBS connection between all keys that point at the same OBS.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { EventEmitter } from "node:events";

import { ObsClient } from "./obs.js";
import { ObsState } from "./obs-state.js";

const IDLE_STOP_DELAY = 30000;

export const DEFAULT_GLOBAL = {
	host: "127.0.0.1",
	port: 4455,
	password: "",
};

function configHome() {
	return process.env.XDG_CONFIG_HOME || path.join(os.homedir(), ".config");
}

// Where OBS keeps the obs-websocket settings on Linux (native and Flatpak).
export function localConfigPaths() {
	const tail = path.join("obs-studio", "plugin_config", "obs-websocket", "config.json");
	return [
		path.join(configHome(), tail),
		path.join(os.homedir(), ".var", "app", "com.obsproject.Studio", "config", tail),
	];
}

export function readLocalObsConfig(paths = localConfigPaths()) {
	let best = null;
	for (const file of paths) {
		try {
			const stat = fs.statSync(file);
			const data = JSON.parse(fs.readFileSync(file, "utf8"));
			const entry = {
				path: file,
				mtime: stat.mtimeMs,
				enabled: data.server_enabled !== false,
				port: Number(data.server_port) || 4455,
				authRequired: data.auth_required !== false,
				password: data.server_password || "",
			};
			if (!best || entry.mtime > best.mtime) best = entry;
		} catch {}
	}
	return best;
}

// Turns the saved settings into the host, port and password to use.
export function resolveConnection(settings = {}) {
	const merged = { ...DEFAULT_GLOBAL, ...settings };
	return {
		host: String(merged.host || "127.0.0.1").trim(),
		port: Number(merged.port) || 4455,
		password: merged.password ?? "",
	};
}

export function connectionKey({ host, port, password }) {
	return `${String(host).toLowerCase()}|${port}|${password}`;
}

export class ConnectionManager extends EventEmitter {
	constructor(log = () => {}) {
		super();
		this.setMaxListeners(0);
		this.log = log;
		this.entries = new Map();
		this.globalSettings = { ...DEFAULT_GLOBAL };
		this.defaultConfig = resolveConnection(this.globalSettings);
	}

	setGlobalSettings(settings) {
		this.globalSettings = { ...DEFAULT_GLOBAL, ...settings };
		this.checkDefault(true);
	}

	checkDefault(force = false) {
		const next = resolveConnection(this.globalSettings);
		const changed = connectionKey(next) !== connectionKey(this.defaultConfig);
		this.defaultConfig = next;
		if (changed || force) this.emit("default-changed", next);
		if (changed) this.log(`OBS connection is now ${next.host}:${next.port}`);
	}

	// config: null for the default connection, or { host, port, password }.
	acquire(config) {
		const resolved = config ? { host: config.host || "127.0.0.1", port: Number(config.port) || 4455, password: config.password ?? "" } : this.defaultConfig;
		const key = connectionKey(resolved);
		let entry = this.entries.get(key);
		if (!entry) {
			const client = new ObsClient({ ...resolved, name: key });
			const state = new ObsState(client);
			client.on("status", (status, error) => {
				this.log(`OBS ${resolved.host}:${resolved.port} ${status}${error ? `: ${error}` : ""}`);
				this.emit("status", key, status, error);
			});
			entry = { key, client, state, users: 0, stopTimer: null };
			this.entries.set(key, entry);
			client.start();
		}
		clearTimeout(entry.stopTimer);
		entry.users++;
		let released = false;
		return {
			key,
			client: entry.client,
			state: entry.state,
			release: () => {
				if (released) return;
				released = true;
				entry.users--;
				if (entry.users <= 0) {
					clearTimeout(entry.stopTimer);
					entry.stopTimer = setTimeout(() => {
						if (entry.users > 0) return;
						entry.client.stop();
						this.entries.delete(key);
					}, IDLE_STOP_DELAY);
				}
			},
		};
	}

	// Used by the setup page to try settings before saving them.
	async test(config, timeout = 4000) {
		const client = new ObsClient({ ...config });
		try {
			return await new Promise((resolve) => {
				const timer = setTimeout(() => resolve({ ok: false, status: client.status, message: client.lastError || "No answer from OBS" }), timeout);
				client.on("status", (status, error) => {
					if (status === "connected") {
						clearTimeout(timer);
						resolve({ ok: true, status, version: client.version });
					} else if (status === "auth" || status === "offline" || status === "error") {
						clearTimeout(timer);
						resolve({ ok: false, status, message: error });
					}
				});
				client.start();
			});
		} finally {
			client.stop();
		}
	}
}
