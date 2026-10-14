// Twitch: read chat for !replay (no login needed), create clips and post them (login needed).
import { EventEmitter } from "node:events";

const IRC_URL = "wss://irc-ws.chat.twitch.tv:443";
const SCOPES = "clips:edit user:write:chat";
const RECONNECT_MS = 5000;

export function parseIrc(line) {
	const message = { tags: {}, prefix: null, command: null, params: [] };
	let rest = line;
	if (rest.startsWith("@")) {
		const space = rest.indexOf(" ");
		for (const pair of rest.slice(1, space).split(";")) {
			const index = pair.indexOf("=");
			if (index < 0) message.tags[pair] = "";
			else message.tags[pair.slice(0, index)] = pair.slice(index + 1);
		}
		rest = rest.slice(space + 1);
	}
	if (rest.startsWith(":")) {
		const space = rest.indexOf(" ");
		message.prefix = rest.slice(1, space);
		rest = rest.slice(space + 1);
	}
	let trailing = null;
	const index = rest.indexOf(" :");
	if (index >= 0) {
		trailing = rest.slice(index + 2);
		rest = rest.slice(0, index);
	}
	const parts = rest.split(" ").filter(Boolean);
	message.command = parts.shift() ?? null;
	message.params = trailing === null ? parts : [...parts, trailing];
	return message;
}

export function chatUser(message) {
	const badges = new Set((message.tags.badges || "").split(",").filter(Boolean).map((b) => b.split("/")[0]));
	const login = message.prefix?.split("!")[0] ?? "";
	return {
		login,
		name: message.tags["display-name"] || login,
		broadcaster: badges.has("broadcaster"),
		mod: badges.has("moderator") || message.tags.mod === "1",
		vip: badges.has("vip") || message.tags.vip === "1",
		sub: badges.has("subscriber") || badges.has("founder") || message.tags.subscriber === "1",
	};
}

// permission: everyone, subs, vips, mods
export function isAllowed(user, permission) {
	if (user.broadcaster || permission === "everyone") return true;
	if (permission === "mods") return user.mod;
	if (permission === "vips") return user.mod || user.vip;
	if (permission === "subs") return user.mod || user.vip || user.sub;
	return false;
}

export function normalizeChannel(name) {
	return String(name ?? "")
		.trim()
		.replace(/^https?:\/\/(www\.)?twitch\.tv\//i, "")
		.replace(/^#/, "")
		.split(/[/?]/)[0]
		.toLowerCase();
}

export class Twitch extends EventEmitter {
	constructor({ save, log = () => {} }) {
		super();
		this.save = save;
		this.log = log;
		this.config = {};
		this.chatWanted = false;
		this.ws = null;
		this.chatChannel = null;
		this.chatConnected = false;
		this.reconnectTimer = null;
		this.deviceFlow = null;
	}

	configure(config = {}) {
		this.config = { ...config };
		this.updateChat();
	}

	get channel() {
		return normalizeChannel(this.config.channel || this.config.login);
	}

	status() {
		return {
			channel: this.channel,
			login: this.config.login || "",
			authorized: !!this.config.token,
			scopes: this.config.scopes || [],
			chatConnected: this.chatConnected,
			chatWanted: this.chatWanted,
			deviceFlow: this.deviceFlow ? { userCode: this.deviceFlow.userCode, uri: this.deviceFlow.uri } : null,
		};
	}

	setChatWanted(wanted) {
		if (wanted === this.chatWanted) return;
		this.chatWanted = wanted;
		this.updateChat();
	}

	updateChat() {
		const channel = this.chatWanted ? this.channel : "";
		if (channel === this.chatChannel && (this.ws || !channel)) return;
		this.closeChat();
		this.chatChannel = channel || null;
		if (channel) this.openChat();
	}

	closeChat() {
		clearTimeout(this.reconnectTimer);
		const ws = this.ws;
		this.ws = null;
		if (ws) {
			try {
				ws.close();
			} catch {}
		}
		if (this.chatConnected) {
			this.chatConnected = false;
			this.emit("status");
		}
	}

	openChat() {
		const channel = this.chatChannel;
		const ws = new WebSocket(IRC_URL);
		this.ws = ws;
		ws.addEventListener("open", () => {
			ws.send("CAP REQ :twitch.tv/tags twitch.tv/commands");
			ws.send("PASS SCHMOOPIIE");
			ws.send(`NICK justinfan${Math.floor(10000 + Math.random() * 80000)}`);
			ws.send(`JOIN #${channel}`);
		});
		ws.addEventListener("message", (event) => {
			for (const line of String(event.data).split("\r\n")) {
				if (!line) continue;
				const message = parseIrc(line);
				if (message.command === "PING") ws.send(`PONG :${message.params[0] ?? "tmi.twitch.tv"}`);
				else if (message.command === "JOIN" && !this.chatConnected) {
					this.chatConnected = true;
					this.log(`Twitch chat: joined #${channel}`);
					this.emit("status");
				} else if (message.command === "RECONNECT") ws.close();
				else if (message.command === "PRIVMSG") {
					this.emit("message", { user: chatUser(message), text: message.params[1] ?? "", channel });
				}
			}
		});
		ws.addEventListener("close", () => {
			if (this.ws !== ws) return;
			this.ws = null;
			if (this.chatConnected) {
				this.chatConnected = false;
				this.emit("status");
			}
			this.reconnectTimer = setTimeout(() => {
				if (!this.ws && this.chatChannel) this.openChat();
			}, RECONNECT_MS);
		});
		ws.addEventListener("error", () => {});
	}

	// Login with a Client ID of a "Public" app from dev.twitch.tv (device code flow).
	async startDeviceFlow(clientId) {
		clientId = String(clientId ?? "").trim();
		if (!clientId) throw new Error("Enter the Client ID of your Twitch app first");
		const response = await fetch("https://id.twitch.tv/oauth2/device", {
			method: "POST",
			body: new URLSearchParams({ client_id: clientId, scopes: SCOPES }),
		});
		const data = await response.json();
		if (!response.ok) throw new Error(data.message || `Twitch said ${response.status}`);
		const flow = {
			clientId,
			deviceCode: data.device_code,
			userCode: data.user_code,
			uri: data.verification_uri,
			expires: Date.now() + data.expires_in * 1000,
			interval: Math.max(1, data.interval || 5) * 1000,
		};
		this.deviceFlow = flow;
		this.emit("status");
		this.pollDeviceFlow(flow);
		return { userCode: flow.userCode, uri: flow.uri };
	}

	async pollDeviceFlow(flow) {
		while (this.deviceFlow === flow && Date.now() < flow.expires) {
			await new Promise((r) => setTimeout(r, flow.interval));
			if (this.deviceFlow !== flow) return;
			try {
				const response = await fetch("https://id.twitch.tv/oauth2/token", {
					method: "POST",
					body: new URLSearchParams({
						client_id: flow.clientId,
						scopes: SCOPES,
						device_code: flow.deviceCode,
						grant_type: "urn:ietf:params:oauth:grant-type:device_code",
					}),
				});
				const data = await response.json();
				if (response.ok && data.access_token) {
					this.deviceFlow = null;
					await this.useToken(data.access_token, { refreshToken: data.refresh_token, clientId: flow.clientId });
					return;
				}
				if (data.message && data.message !== "authorization_pending" && data.message !== "slow_down") {
					throw new Error(data.message);
				}
			} catch (error) {
				this.deviceFlow = null;
				this.emit("error", error);
				this.emit("status");
				return;
			}
		}
		if (this.deviceFlow === flow) {
			this.deviceFlow = null;
			this.emit("error", new Error("The Twitch login code expired"));
			this.emit("status");
		}
	}

	cancelDeviceFlow() {
		this.deviceFlow = null;
		this.emit("status");
	}

	// Checks a token and stores it with the user it belongs to.
	async useToken(token, { refreshToken = "", clientId = "" } = {}) {
		token = String(token ?? "").trim().replace(/^oauth:/i, "");
		const response = await fetch("https://id.twitch.tv/oauth2/validate", { headers: { Authorization: `OAuth ${token}` } });
		const data = await response.json().catch(() => ({}));
		if (!response.ok) throw new Error(data.message || "Twitch did not accept the token");
		this.config = {
			...this.config,
			token,
			refreshToken: refreshToken || "",
			clientId: data.client_id || clientId,
			login: data.login,
			userId: data.user_id,
			scopes: data.scopes || [],
			channel: this.config.channel || data.login,
		};
		await this.save(this.config);
		this.updateChat();
		this.emit("status");
		return this.status();
	}

	async logout() {
		this.config = { channel: this.config.channel || "" };
		await this.save(this.config);
		this.emit("status");
	}

	async refresh() {
		if (!this.config.refreshToken || !this.config.clientId) return false;
		const response = await fetch("https://id.twitch.tv/oauth2/token", {
			method: "POST",
			body: new URLSearchParams({ grant_type: "refresh_token", refresh_token: this.config.refreshToken, client_id: this.config.clientId }),
		});
		const data = await response.json().catch(() => ({}));
		if (!response.ok || !data.access_token) return false;
		this.config = { ...this.config, token: data.access_token, refreshToken: data.refresh_token || this.config.refreshToken };
		await this.save(this.config);
		return true;
	}

	async helix(method, path, { query = {}, body } = {}, retry = true) {
		if (!this.config.token) throw new Error("Log in to Twitch in the Instant Replay settings first");
		const url = new URL(`https://api.twitch.tv/helix/${path}`);
		for (const [key, value] of Object.entries(query)) url.searchParams.set(key, value);
		const response = await fetch(url, {
			method,
			headers: {
				Authorization: `Bearer ${this.config.token}`,
				"Client-Id": this.config.clientId,
				...(body ? { "Content-Type": "application/json" } : {}),
			},
			body: body ? JSON.stringify(body) : undefined,
		});
		if (response.status === 401 && retry && (await this.refresh())) return this.helix(method, path, { query, body }, false);
		const data = await response.json().catch(() => ({}));
		if (!response.ok) throw new Error(data.message || `Twitch said ${response.status}`);
		return data;
	}

	async broadcasterId() {
		const channel = this.channel;
		if (!channel) throw new Error("No Twitch channel set");
		if (channel === this.config.login && this.config.userId) return this.config.userId;
		const data = await this.helix("GET", "users", { query: { login: channel } });
		if (!data.data?.[0]) throw new Error(`Twitch channel ${channel} not found`);
		return data.data[0].id;
	}

	// Clips only work while the channel is live.
	async createClip() {
		const id = await this.broadcasterId();
		const data = await this.helix("POST", "clips", { query: { broadcaster_id: id } });
		const clip = data.data?.[0];
		if (!clip) throw new Error("Twitch did not create a clip");
		return { id: clip.id, url: `https://clips.twitch.tv/${clip.id}`, editUrl: clip.edit_url };
	}

	async say(message) {
		const id = await this.broadcasterId();
		const data = await this.helix("POST", "chat/messages", { body: { broadcaster_id: id, sender_id: this.config.userId, message } });
		const result = data.data?.[0];
		if (result && result.is_sent === false) throw new Error(result.drop_reason?.message || "Twitch dropped the message");
	}
}
