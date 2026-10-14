// Virtual Stream Deck for OpenDeck: a device plugin that shows the keys in a browser
// and has a small HTTP API, so plugins can be tested without hardware.
//
//   http://127.0.0.1:57190/            the deck (click keys, scroll or drag dials)
//   GET  /api/state                    images and device info as JSON
//   GET  /api/key/<n>.png              image of key n
//   GET  /api/dial/<n>.jpg             image of the touch strip above dial n
//   POST /api/key/<n>/press?ms=100     press and release (down, up also work)
//   POST /api/dial/<n>/rotate?ticks=1  turn (negative = left)
//   POST /api/dial/<n>/press           press and release (down, up also work)
//   POST /api/touch/<n>?hold=0         tap the touch strip
//
// Settings (environment): VIRTUAL_DECK_PORT, VIRTUAL_DECK_ROWS, VIRTUAL_DECK_COLUMNS, VIRTUAL_DECK_DIALS
import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const env = (name, fallback) => Number(process.env[name]) || fallback;
const HTTP_PORT = env("VIRTUAL_DECK_PORT", 57190);
const DEVICE = {
	id: "vd-virtual-deck",
	name: "Virtual Stream Deck",
	rows: env("VIRTUAL_DECK_ROWS", 3),
	columns: env("VIRTUAL_DECK_COLUMNS", 5),
	encoders: env("VIRTUAL_DECK_DIALS", 4),
	touchpoints: 0,
	type: 7,
};

const args = {};
for (let i = 2; i < process.argv.length; i++) if (process.argv[i].startsWith("-")) args[process.argv[i].slice(1)] = process.argv[++i];

const keys = new Array(DEVICE.rows * DEVICE.columns).fill(null);
const dials = new Array(DEVICE.encoders).fill(null);
let brightness = 100;
const clients = new Set();

const log = (message) => console.log(`[${new Date().toISOString()}] ${message}`);

// ---------- OpenDeck ----------

const ws = new WebSocket(`ws://127.0.0.1:${args.port}`);
const send = (event, payload) => ws.send(JSON.stringify({ event, payload }));

ws.addEventListener("open", () => {
	ws.send(JSON.stringify({ event: args.registerEvent, uuid: args.pluginUUID }));
	send("registerDevice", DEVICE);
	log(`registered ${DEVICE.id} (${DEVICE.rows}x${DEVICE.columns}, ${DEVICE.encoders} dials), UI on http://127.0.0.1:${HTTP_PORT}/`);
});
ws.addEventListener("close", () => process.exit(0));
ws.addEventListener("message", (event) => {
	const message = JSON.parse(event.data);
	if (message.device !== DEVICE.id) return;
	if (message.event === "setImage") {
		if (message.position === undefined || message.position === null) {
			keys.fill(null);
			dials.fill(null);
			broadcast({ type: "clear" });
			return;
		}
		const list = message.controller === "Encoder" ? dials : keys;
		list[message.position] = message.image ?? null;
		broadcast({ type: "image", controller: message.controller, position: message.position, image: message.image ?? null });
	} else if (message.event === "setBrightness") {
		brightness = message.brightness;
		broadcast({ type: "brightness", brightness });
	}
});

function broadcast(data) {
	const line = `data: ${JSON.stringify(data)}\n\n`;
	for (const res of clients) res.write(line);
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function keyAction(position, what, ms) {
	if (what === "down" || what === "press") send("keyDown", { device: DEVICE.id, position });
	if (what === "press") await sleep(ms);
	if (what === "up" || what === "press") send("keyUp", { device: DEVICE.id, position });
}

async function dialAction(position, what, params) {
	if (what === "rotate") send("encoderChange", { device: DEVICE.id, position, ticks: Number(params.get("ticks")) || 1 });
	if (what === "down" || what === "press") send("encoderDown", { device: DEVICE.id, position });
	if (what === "press") await sleep(Number(params.get("ms")) || 100);
	if (what === "up" || what === "press") send("encoderUp", { device: DEVICE.id, position });
}

// ---------- HTTP ----------

function dataUrlToBuffer(url) {
	const match = /^data:([^;,]+)(;base64)?,(.*)$/s.exec(url ?? "");
	if (!match) return null;
	return { type: match[1], body: match[2] ? Buffer.from(match[3], "base64") : Buffer.from(decodeURIComponent(match[3])) };
}

const server = http.createServer(async (req, res) => {
	const url = new URL(req.url, "http://localhost");
	const parts = url.pathname.split("/").filter(Boolean);
	try {
		if (req.method === "GET" && url.pathname === "/") {
			res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
			res.end(fs.readFileSync(path.join(here, "deck.html")));
		} else if (req.method === "GET" && url.pathname === "/events") {
			res.writeHead(200, { "Content-Type": "text/event-stream", "Cache-Control": "no-cache", Connection: "keep-alive" });
			res.write(`data: ${JSON.stringify({ type: "state", device: DEVICE, keys, dials, brightness })}\n\n`);
			clients.add(res);
			req.on("close", () => clients.delete(res));
		} else if (req.method === "GET" && url.pathname === "/api/state") {
			res.writeHead(200, { "Content-Type": "application/json" });
			res.end(JSON.stringify({ device: DEVICE, brightness, keys, dials }));
		} else if (req.method === "GET" && parts[0] === "api" && (parts[1] === "key" || parts[1] === "dial") && parts[2]) {
			const index = parseInt(parts[2], 10);
			const image = dataUrlToBuffer((parts[1] === "key" ? keys : dials)[index]);
			if (!image) {
				res.writeHead(404).end("no image");
				return;
			}
			res.writeHead(200, { "Content-Type": image.type });
			res.end(image.body);
		} else if (req.method === "POST" && parts[0] === "api") {
			const index = parseInt(parts[2], 10);
			if (parts[1] === "key" && index >= 0 && index < keys.length) await keyAction(index, parts[3] ?? "press", Number(url.searchParams.get("ms")) || 100);
			else if (parts[1] === "dial" && index >= 0 && index < dials.length) await dialAction(index, parts[3] ?? "press", url.searchParams);
			else if (parts[1] === "touch" && index >= 0 && index < dials.length) send("touchscreenPress", { device: DEVICE.id, position: index, x: 100, y: 50, hold: url.searchParams.get("hold") === "1" });
			else {
				res.writeHead(400).end("bad request");
				return;
			}
			res.writeHead(204).end();
		} else {
			res.writeHead(404).end("not found");
		}
	} catch (error) {
		res.writeHead(500).end(String(error));
	}
});

server.listen(HTTP_PORT, "127.0.0.1");
server.on("error", (error) => log(`HTTP server: ${error.message}`));
