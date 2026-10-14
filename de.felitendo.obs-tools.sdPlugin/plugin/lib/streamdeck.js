// Connection to OpenDeck (or any host that speaks the Stream Deck SDK protocol).
import { EventEmitter } from "node:events";

export function parseArgs(argv) {
	const args = {};
	for (let i = 0; i < argv.length; i++) {
		if (argv[i].startsWith("-") && i + 1 < argv.length) {
			args[argv[i].replace(/^-+/, "")] = argv[++i];
		}
	}
	return args;
}

export class StreamDeck extends EventEmitter {
	constructor({ port, pluginUUID, registerEvent, info }) {
		super();
		this.port = port;
		this.uuid = pluginUUID;
		this.registerEvent = registerEvent;
		this.info = typeof info === "string" ? JSON.parse(info || "{}") : info || {};
		this.ws = null;
		this.queue = [];
	}

	connect() {
		const ws = new WebSocket(`ws://127.0.0.1:${this.port}`);
		this.ws = ws;
		ws.addEventListener("open", () => {
			ws.send(JSON.stringify({ event: this.registerEvent, uuid: this.uuid }));
			for (const message of this.queue.splice(0)) ws.send(message);
			this.emit("open");
		});
		ws.addEventListener("message", (event) => {
			let message;
			try {
				message = JSON.parse(event.data);
			} catch {
				return;
			}
			this.emit("message", message);
		});
		ws.addEventListener("close", () => this.emit("close"));
		ws.addEventListener("error", (event) => this.emit("error", event?.error || event));
	}

	send(message) {
		const text = JSON.stringify(message);
		if (this.ws?.readyState === WebSocket.OPEN) this.ws.send(text);
		else this.queue.push(text);
	}

	setTitle(context, title, state) {
		const payload = { title: title ?? "", target: 0 };
		if (state !== undefined) payload.state = state;
		this.send({ event: "setTitle", context, payload });
	}

	// image: data URL, path inside the plugin, or "" to go back to the default image.
	setImage(context, image, state) {
		const payload = { image: image ?? "", target: 0 };
		if (state !== undefined) payload.state = state;
		this.send({ event: "setImage", context, payload });
	}

	setState(context, state) {
		this.send({ event: "setState", context, payload: { state } });
	}

	setFeedback(context, payload) {
		this.send({ event: "setFeedback", context, payload });
	}

	setFeedbackLayout(context, layout) {
		this.send({ event: "setFeedbackLayout", context, payload: { layout } });
	}

	showOk(context) {
		this.send({ event: "showOk", context });
	}

	showAlert(context) {
		this.send({ event: "showAlert", context });
	}

	setSettings(context, settings) {
		this.send({ event: "setSettings", context, payload: settings });
	}

	setGlobalSettings(settings) {
		this.send({ event: "setGlobalSettings", context: this.uuid, payload: settings });
	}

	getGlobalSettings() {
		this.send({ event: "getGlobalSettings", context: this.uuid });
	}

	sendToPropertyInspector(context, payload) {
		this.send({ event: "sendToPropertyInspector", context, payload });
	}

	openUrl(url) {
		this.send({ event: "openUrl", payload: { url } });
	}

	// Goes to the plugin log file (OpenDeck catches stdout).
	log(message) {
		console.log(`[${new Date().toISOString()}] ${message}`);
	}

	// Also goes to the OpenDeck log.
	warn(message) {
		console.warn(`[${new Date().toISOString()}] WARN ${message}`);
		this.send({ event: "logMessage", payload: { message: String(message) } });
	}
}
