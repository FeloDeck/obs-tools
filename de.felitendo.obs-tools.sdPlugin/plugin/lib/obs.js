// Small obs-websocket v5 client. Uses the WebSocket built into Node 22+.
import { createHash } from "node:crypto";
import { lookup } from "node:dns/promises";
import { EventEmitter } from "node:events";
import { isIP } from "node:net";

const OP = {
	HELLO: 0,
	IDENTIFY: 1,
	IDENTIFIED: 2,
	REIDENTIFY: 3,
	EVENT: 5,
	REQUEST: 6,
	REQUEST_RESPONSE: 7,
	REQUEST_BATCH: 8,
	REQUEST_BATCH_RESPONSE: 9,
};

// Every non high-volume category (2047).
export const EVENTS_ALL = (1 << 11) - 1;
// Audio levels of all inputs, about 20 times a second.
export const EVENTS_VOLUME_METERS = 1 << 16;

export const BatchExecution = {
	SERIAL_REALTIME: 0,
	SERIAL_FRAME: 1,
	PARALLEL: 2,
};

const CLOSE_AUTH_FAILED = 4009;
const CLOSE_RPC_VERSION = 4010;

const RETRY_DELAYS = [1000, 2000, 3000, 5000];
const AUTH_RETRY_DELAY = 15000;

export function authResponse(password, salt, challenge) {
	const secret = createHash("sha256").update(password + salt).digest("base64");
	return createHash("sha256").update(secret + challenge).digest("base64");
}

export class ObsError extends Error {
	constructor(requestType, status) {
		super(status?.comment || `${requestType} failed (code ${status?.code})`);
		this.requestType = requestType;
		this.code = status?.code;
	}
}

/**
 * Status values:
 *   idle        not started
 *   connecting  waiting for the socket or the handshake
 *   connected   identified, requests work
 *   offline     nothing listens on host:port (OBS closed or server off)
 *   auth        wrong or missing password
 *   error       anything else, see lastError
 */
export class ObsClient extends EventEmitter {
	constructor({ host = "127.0.0.1", port = 4455, password = "", name = "obs" } = {}) {
		super();
		this.setMaxListeners(0);
		this.host = host;
		this.port = Number(port) || 4455;
		this.password = password ?? "";
		this.name = name;
		this.status = "idle";
		this.lastError = "";
		this.version = null;
		this.ws = null;
		this.pending = new Map();
		this.nextId = 1;
		this.retry = 0;
		this.retryTimer = null;
		this.stopped = true;
		this.eventSubscriptions = EVENTS_ALL;
	}

	// Change which events OBS sends, also on a running connection.
	setEventSubscriptions(mask) {
		if (mask === this.eventSubscriptions) return;
		this.eventSubscriptions = mask;
		if (this.connected) this.ws.send(JSON.stringify({ op: OP.REIDENTIFY, d: { eventSubscriptions: mask } }));
	}

	get url() {
		const host = this.host.includes(":") && !this.host.startsWith("[") ? `[${this.host}]` : this.host;
		return `ws://${host}:${this.port}`;
	}

	get connected() {
		return this.status === "connected";
	}

	start() {
		if (!this.stopped) return;
		this.stopped = false;
		this.open();
	}

	stop() {
		this.stopped = true;
		clearTimeout(this.retryTimer);
		this.retryTimer = null;
		const wasConnected = this.connected;
		this.dropSocket();
		this.cleanup("stopped");
		if (wasConnected) this.emit("disconnected");
		this.setStatus("idle");
	}

	// Reconnect now, for example after the user saved new settings.
	reconnect() {
		clearTimeout(this.retryTimer);
		this.retry = 0;
		if (this.ws) {
			const wasConnected = this.connected;
			this.dropSocket();
			this.cleanup("reconnect");
			if (wasConnected) this.emit("disconnected");
		}
		if (!this.stopped) this.open();
	}

	// Close the socket without running its close handler.
	dropSocket() {
		const ws = this.ws;
		this.ws = null;
		if (ws) {
			try {
				ws.close(1000);
			} catch {}
		}
	}

	setStatus(status, error = "") {
		const changed = status !== this.status || error !== this.lastError;
		this.status = status;
		this.lastError = error;
		if (changed) this.emit("status", status, error);
	}

	async open() {
		if (this.stopped) return;
		// While retrying, keep showing why the last try failed.
		if (!["offline", "auth", "error"].includes(this.status)) this.setStatus("connecting");
		// The WebSocket error does not say why it failed, so check the name first.
		if (!isIP(this.host.replace(/^\[|\]$/g, "")) && this.host !== "localhost") {
			try {
				await lookup(this.host);
			} catch {
				if (this.stopped) return;
				this.setStatus("offline", `Host ${this.host} not found`);
				this.scheduleRetry();
				return;
			}
			if (this.stopped || this.ws) return;
		}
		let ws;
		try {
			ws = new WebSocket(this.url, "obswebsocket.json");
		} catch (error) {
			this.setStatus("error", error.message);
			this.scheduleRetry();
			return;
		}
		this.ws = ws;
		let identified = false;
		let closeCode = 0;
		let attemptError = "";

		ws.addEventListener("message", (event) => {
			let message;
			try {
				message = JSON.parse(event.data);
			} catch {
				return;
			}
			const { op, d } = message;
			if (op === OP.HELLO) {
				const identify = { rpcVersion: 1, eventSubscriptions: this.eventSubscriptions };
				if (d.authentication) {
					if (!this.password) {
						closeCode = CLOSE_AUTH_FAILED;
						attemptError = "OBS asks for a password";
						ws.close(1000);
						return;
					}
					identify.authentication = authResponse(this.password, d.authentication.salt, d.authentication.challenge);
				}
				this.version = { obs: null, websocket: d.obsWebSocketVersion, rpc: d.rpcVersion };
				ws.send(JSON.stringify({ op: OP.IDENTIFY, d: identify }));
			} else if (op === OP.IDENTIFIED) {
				if (identified) return; // answer to a Reidentify
				identified = true;
				this.retry = 0;
				this.call("GetVersion")
					.then((version) => {
						this.version = { ...this.version, obs: version.obsVersion, platform: version.platformDescription };
					})
					.catch(() => {})
					.finally(() => {
						if (this.ws !== ws) return;
						this.setStatus("connected");
						this.emit("connected");
					});
			} else if (op === OP.EVENT) {
				this.emit("event", d.eventType, d.eventData ?? {});
				this.emit(d.eventType, d.eventData ?? {});
			} else if (op === OP.REQUEST_RESPONSE || op === OP.REQUEST_BATCH_RESPONSE) {
				const pending = this.pending.get(d.requestId);
				if (!pending) return;
				this.pending.delete(d.requestId);
				clearTimeout(pending.timer);
				if (op === OP.REQUEST_BATCH_RESPONSE) {
					pending.resolve(d.results ?? []);
				} else if (d.requestStatus?.result) {
					pending.resolve(d.responseData ?? {});
				} else {
					pending.reject(new ObsError(d.requestType, d.requestStatus));
				}
			}
		});

		const closed = (event) => {
			if (this.ws !== ws) return;
			const code = closeCode || event.code;
			this.ws = null;
			const wasConnected = identified;
			this.cleanup("connection closed");
			if (wasConnected) this.emit("disconnected");
			if (code === CLOSE_AUTH_FAILED) {
				this.setStatus("auth", attemptError || "Wrong password");
			} else if (code === CLOSE_RPC_VERSION) {
				this.setStatus("error", "This OBS version is not supported (needs obs-websocket 5)");
			} else if (!wasConnected) {
				this.setStatus("offline", attemptError || `Nothing answers on ${this.host}:${this.port}`);
			} else {
				this.setStatus("offline", event.reason || "OBS closed the connection");
			}
			this.scheduleRetry();
		};
		ws.addEventListener("close", closed);

		ws.addEventListener("error", (event) => {
			const message = event?.error?.cause?.code || event?.error?.message || event?.message || "";
			if (/ECONNREFUSED/.test(message)) attemptError = `Nothing answers on ${this.host}:${this.port}`;
			else if (/ENOTFOUND|EAI_AGAIN/.test(message)) attemptError = `Host ${this.host} not found`;
			// Node's own text when nothing answers. "Nothing answers" says it better.
			else if (message && !/non-101/.test(message)) attemptError = message;
			// Some Node versions never send "close" when the socket did not open, so the try would hang.
			if (ws.readyState === WebSocket.CONNECTING) {
				closed({ code: 1006, reason: "" });
				try {
					ws.close();
				} catch {}
			}
		});
	}

	scheduleRetry() {
		if (this.stopped) return;
		clearTimeout(this.retryTimer);
		const delay = this.status === "auth" ? AUTH_RETRY_DELAY : RETRY_DELAYS[Math.min(this.retry, RETRY_DELAYS.length - 1)];
		this.retry++;
		this.retryTimer = setTimeout(() => this.open(), delay);
	}

	cleanup(reason) {
		for (const [, pending] of this.pending) {
			clearTimeout(pending.timer);
			pending.reject(new Error(`OBS not connected (${reason})`));
		}
		this.pending.clear();
	}

	send(op, d, timeout) {
		return new Promise((resolve, reject) => {
			if (!this.ws || this.ws.readyState !== WebSocket.OPEN) {
				reject(new Error("OBS not connected"));
				return;
			}
			const requestId = String(this.nextId++);
			const timer = setTimeout(() => {
				this.pending.delete(requestId);
				reject(new Error("OBS did not answer in time"));
			}, timeout);
			this.pending.set(requestId, { resolve, reject, timer });
			this.ws.send(JSON.stringify({ op, d: { ...d, requestId } }));
		});
	}

	call(requestType, requestData = {}, { timeout = 10000 } = {}) {
		return this.send(OP.REQUEST, { requestType, requestData }, timeout);
	}

	// requests: [{ requestType, requestData }]. Resolves with the raw results.
	batch(requests, { executionType = BatchExecution.SERIAL_REALTIME, haltOnFailure = false, timeout } = {}) {
		const wait = timeout ?? Math.max(10000, requests.length * 100);
		return this.send(OP.REQUEST_BATCH, { executionType, haltOnFailure, requests }, wait);
	}
}
