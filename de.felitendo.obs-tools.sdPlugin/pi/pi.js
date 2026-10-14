// Settings page for every OBS Tools action. The fields come from schemas.js.
import { setLanguage, t, translateMessage } from "../plugin/lib/i18n.js";
import { CONTROLS, LOOK, SCHEMAS } from "./schemas.js";

const FIELD_LABELS = {
	positionX: "Position X",
	positionY: "Position Y",
	rotation: "Rotation °",
	scaleX: "Scale X",
	scaleY: "Scale Y",
	cropLeft: "Crop left",
	cropRight: "Crop right",
	cropTop: "Crop top",
	cropBottom: "Crop bottom",
	boundsWidth: "Bounds W",
	boundsHeight: "Bounds H",
};
const EASINGS = [
	["linear", "Linear"],
	["easeIn", "Ease in"],
	["easeOut", "Ease out"],
	["easeInOut", "Smooth"],
	["back", "Overshoot"],
];

let ws = null;
let context = "";
let action = "";
let schema = { fields: [] };
let settings = {};
let conn = null; // last "state" message from the plugin
let lists = {};
let listsLoaded = false;
let requestCounter = 0;
const pending = new Map();
const ui = {
	// Panels of the fields (see section in schemas.js) the user opened or closed.
	open: {},
	// The connection box under the status. null: autoOpen decides, which is true while there is a problem.
	setupOpen: null,
	autoOpen: false,
	setupMsg: null,
	setupDraft: null,
	recording: false,
	animationMsg: null,
	exportOpen: false,
	importOpen: false,
	importText: "",
	twitchMsg: null,
	pickMsg: null,
};

// ---------- connection to OpenDeck ----------

window.connectElgatoStreamDeckSocket = (port, uuid, registerEvent, info, actionInfo) => {
	context = uuid;
	document.documentElement.lang = setLanguage(JSON.parse(info || "{}").application?.language);
	const parsed = JSON.parse(actionInfo || "{}");
	action = parsed.action;
	schema = SCHEMAS[action] ?? { fields: [] };
	settings = { ...(parsed.payload?.settings ?? {}) };
	ws = new WebSocket(`ws://127.0.0.1:${port}`);
	ws.addEventListener("open", () => {
		ws.send(JSON.stringify({ event: registerEvent, uuid }));
		send({ type: "hello" });
	});
	ws.addEventListener("message", (event) => onMessage(JSON.parse(event.data)));
	render();
};

function send(payload) {
	if (ws?.readyState !== WebSocket.OPEN) return;
	ws.send(JSON.stringify({ event: "sendToPlugin", action, context, payload }));
}

function request(payload) {
	const requestId = `r${++requestCounter}`;
	return new Promise((resolve) => {
		pending.set(requestId, resolve);
		send({ ...payload, requestId });
		setTimeout(() => {
			if (pending.delete(requestId)) resolve({ type: "timeout" });
		}, 15000);
	});
}

let saveTimer = null;
function save(now = false) {
	clearTimeout(saveTimer);
	const write = () => ws?.readyState === WebSocket.OPEN && ws.send(JSON.stringify({ event: "setSettings", context, payload: settings }));
	if (now) write();
	else saveTimer = setTimeout(write, 250);
}

function onMessage(message) {
	if (message.event === "didReceiveSettings") {
		settings = { ...settings, ...(message.payload?.settings ?? {}) };
		if (!isTyping()) render();
		return;
	}
	if (message.event !== "sendToPropertyInspector") return;
	const payload = message.payload ?? {};
	if (payload.requestId && pending.has(payload.requestId)) {
		pending.get(payload.requestId)(payload);
		pending.delete(payload.requestId);
	}
	if (payload.type === "state") {
		const wasConnected = conn?.connection?.status === "connected";
		const first = !conn;
		conn = payload;
		// A new problem opens the connection box, a working connection closes it. "Connecting" while
		// OBS is away changes nothing, or the box would flicker with each try.
		if ((conn.connection.status === "connected" || problem()) && problem() !== ui.autoOpen) {
			ui.autoOpen = problem();
			ui.setupOpen = null;
		}
		// The plugin knows the defaults, so its copy wins on the first load.
		if (payload.settings && first) settings = { ...settings, ...payload.settings };
		const connected = conn.connection.status === "connected";
		// Open the help that fits the problem, once.
		if (connected && !wasConnected) loadLists();
		if (!isTyping()) render();
		else renderStatus();
	} else if (payload.type === "twitchError") {
		ui.twitchMsg = { kind: "bad", text: translateMessage(payload.message) };
		render();
	}
}

// Lists follow OBS by themselves (new scenes, sources, filters), no reload button needed.
setInterval(() => {
	if (conn?.connection?.status !== "connected" || document.hidden) return;
	const el = document.activeElement;
	if (openDropdown || (el && isTyping())) return;
	loadLists();
}, 5000);

function isTyping() {
	const el = document.activeElement;
	return !!el && (el.tagName === "TEXTAREA" || (el.tagName === "INPUT" && ["text", "number", "password"].includes(el.type)));
}

// ---------- lists from OBS ----------

function neededLists() {
	const names = new Set();
	for (const field of schema.fields) if (field.list) names.add(field.list);
	return [...names];
}

async function loadLists(names = neededLists()) {
	if (!names.length) return;
	const reply = await request({ type: "getLists", lists: names, source: settings.source, scene: settings.scene, filter: settings.filter });
	if (reply.type !== "lists") return;
	const next = { ...lists, ...reply.lists };
	const changed = JSON.stringify(next) !== JSON.stringify(lists) || !listsLoaded;
	lists = next;
	listsLoaded = true;
	if (changed && !isTyping()) render();
}

// ---------- tiny DOM helper ----------

function h(tag, attrs = {}, ...children) {
	const el = document.createElement(tag);
	for (const [key, value] of Object.entries(attrs ?? {})) {
		if (value === undefined || value === null || value === false) continue;
		if (key === "class") el.className = value;
		else if (key === "style") el.style.cssText = value;
		else if (key.startsWith("on")) el.addEventListener(key.slice(2).toLowerCase(), value);
		else if (key in el) el[key] = value;
		else el.setAttribute(key, value === true ? "" : value);
	}
	for (const child of children.flat(Infinity)) {
		if (child === null || child === undefined || child === false) continue;
		el.append(child instanceof Node ? child : document.createTextNode(String(child)));
	}
	return el;
}

// Like el.append, but skips null and false (append would print "null").
function append(el, ...children) {
	el.append(...children.flat(Infinity).filter((c) => c !== null && c !== undefined && c !== false));
	return el;
}

// **bold** and _italic_ inside a (translated) sentence.
function rich(text) {
	return text.split(/(\*\*[^*]+\*\*|_[^_]+_)/).map((part) => {
		if (part.startsWith("**")) return h("b", {}, part.slice(2, -2));
		if (part.length > 2 && part.startsWith("_")) return h("i", {}, part.slice(1, -1));
		return part;
	});
}

function row(label, control, { hint, top = false } = {}) {
	return h("div", { class: `row${top ? " top" : ""}` }, h("div", { class: "name" }, label ?? ""), h("div", { class: "control" }, control), hint ? h("div", { class: "hint-line" }, hint) : null);
}

function msg(m) {
	return m ? h("div", { class: `msg ${m.kind ?? ""}` }, m.text) : null;
}

// ---------- own controls ----------
// WebKitGTK (OpenDeck) draws native drop-downs and checkboxes in the GTK style, which does not fit.
// These look like the rest of the page and work with mouse and keyboard.

const CHECK = `<svg width="13" height="10" viewBox="0 0 13 10"><path d="M1.6 5.3 4.8 8.4 11.4 1.6" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"/></svg>`;

function checkMark(on) {
	const el = h("span", { class: "dd-check" });
	if (on) el.innerHTML = CHECK;
	return el;
}

const CHEVRON = `<svg width="10" height="6" viewBox="0 0 10 6"><path d="M1 1l4 4 4-4" fill="none" stroke="currentColor" stroke-width="1.5"/></svg>`;
let openDropdown = null;
let renderPending = false;

function closeDropdown() {
	if (!openDropdown) return;
	const current = openDropdown;
	openDropdown = null;
	current.close();
	if (renderPending) {
		renderPending = false;
		render();
	}
}

document.addEventListener("mousedown", (e) => {
	if (openDropdown && !openDropdown.root.contains(e.target)) closeDropdown();
});
window.addEventListener("blur", () => closeDropdown());

// options: [{ value, label, missing }]
function dropdown({ value, options, placeholder = t("Choose…"), onChange, focusKey }) {
	const current = options.find((o) => o.value === value);
	const root = h("div", { class: "dropdown" });
	const button = h(
		"button",
		{
			type: "button",
			class: `dd-button${current?.missing ? " missing" : ""}`,
			"data-focus-key": focusKey,
			onclick: () => (openDropdown?.root === root ? closeDropdown() : open()),
			onkeydown: (e) => {
				if (["ArrowDown", "ArrowUp", "Enter", " "].includes(e.key)) {
					e.preventDefault();
					open();
				}
			},
		},
		h("span", { class: `dd-label${current ? "" : " placeholder"}` }, current ? current.label : placeholder),
	);
	button.insertAdjacentHTML("beforeend", `<span class="dd-chevron">${CHEVRON}</span>`);
	root.append(button);

	function open() {
		closeDropdown();
		root.classList.add("open");
		const pop = h("div", { class: "dd-pop", role: "listbox" });
		const search = options.length > 10 ? h("input", { type: "text", class: "dd-search", placeholder: t("Search…") }) : null;
		const list = h("div", { class: "dd-list" });
		let shown = options;
		let active = Math.max(0, options.findIndex((o) => o.value === value));
		const pick = (option) => {
			closeDropdown();
			if (option && option.value !== value) onChange(option.value);
			const again = focusKey && document.querySelector(`[data-focus-key="${CSS.escape(focusKey)}"]`);
			(again || button).focus();
		};
		const mark = () => {
			[...list.children].forEach((el, i) => el.classList.toggle("active", i === active));
			list.children[active]?.scrollIntoView({ block: "nearest" });
		};
		const draw = () => {
			list.replaceChildren(
				...shown.map((o, i) =>
					h(
						"div",
						{
							class: `dd-item${o.value === value ? " selected" : ""}${o.missing ? " missing" : ""}`,
							role: "option",
							onmousedown: (e) => (e.preventDefault(), pick(o)),
							onmouseenter: () => ((active = i), mark()),
						},
						checkMark(o.value === value),
						h("span", { class: "dd-text" }, o.label),
					),
				),
			);
			if (!shown.length) list.append(h("div", { class: "dd-empty" }, t("Nothing found")));
			mark();
		};
		const onKey = (e) => {
			const handled = () => (e.preventDefault(), e.stopPropagation());
			if (e.key === "ArrowDown" || e.key === "ArrowUp") {
				handled();
				active = Math.min(shown.length - 1, Math.max(0, active + (e.key === "ArrowDown" ? 1 : -1)));
				mark();
			} else if (e.key === "Enter" || (e.key === " " && document.activeElement !== search)) {
				// Stop it here, or the button would open the list again.
				handled();
				pick(shown[active]);
			} else if (e.key === "Escape" || e.key === "Tab") {
				handled();
				closeDropdown();
				button.focus();
			} else if (search && e.key.length === 1 && document.activeElement !== search) {
				search.focus();
			}
		};
		if (search) {
			search.addEventListener("input", () => {
				const q = search.value.trim().toLowerCase();
				shown = q ? options.filter((o) => String(o.label).toLowerCase().includes(q)) : options;
				active = 0;
				draw();
			});
			pop.append(search);
		}
		pop.append(list);
		root.append(pop);
		draw();
		// Open upwards when there is more room above, and never taller than the page.
		const rect = button.getBoundingClientRect();
		const below = window.innerHeight - rect.bottom - 8;
		const above = rect.top - 8;
		const up = below < 160 && above > below;
		pop.classList.toggle("up", up);
		list.style.maxHeight = `${Math.max(90, Math.min(240, (up ? above : below) - (search ? 40 : 8)))}px`;
		mark();
		document.addEventListener("keydown", onKey, true);
		if (search) search.focus();
		openDropdown = {
			root,
			close: () => {
				document.removeEventListener("keydown", onKey, true);
				pop.remove();
				root.classList.remove("open");
			},
		};
	}
	return root;
}

function checkbox(checked, text, onChange) {
	return h(
		"label",
		{ class: "check" },
		h("input", { type: "checkbox", checked: !!checked, onchange: (e) => onChange(e.target.checked) }),
		h("span", { class: "check-box" }),
		text ? h("span", { class: "check-text" }, text) : null,
	);
}

// ---------- rendering ----------

function render() {
	if (openDropdown) {
		renderPending = true;
		return;
	}
	const app = document.getElementById("app");
	const focus = document.activeElement?.dataset?.focusKey;
	const parts = [
		renderStatusCard(),
		setupOpen() ? renderSetup() : null,
		renderKeyError(),
		schema.help ? h("p", { class: "note" }, t(schema.help)) : null,
		...renderFields(),
		schema.hint ? h("p", { class: "note" }, t(schema.hint)) : null,
	];
	app.replaceChildren(...parts.flat(Infinity).filter(Boolean));
	// WebKitGTK in OpenDeck sometimes paints only parts of the page after a big change and keeps old
	// pixels elsewhere. A tiny change of the opacity makes it paint the whole page again.
	app.style.opacity = app.style.opacity === "0.999" ? "" : "0.999";
	if (focus) document.querySelector(`[data-focus-key="${CSS.escape(focus)}"]`)?.focus();
}

function renderStatus() {
	const old = document.querySelector(".status");
	if (old) old.replaceWith(renderStatusCard());
}

function statusText() {
	if (!conn) return { kind: "busy", title: t("Checking connection to OBS…") };
	const c = conn.connection;
	const where = `${c.host}:${c.port}`;
	// The plugin sends its messages in English.
	const error = c.error ? translateMessage(c.error) : "";
	switch (c.status) {
		case "connected": {
			// All good: a quiet line, the details only on hover.
			const detail = c.custom ? t("This key uses another PC") : "";
			return { kind: "ok", quiet: true, title: t("Connected to OBS {version}", { version: c.version?.obs ?? "" }).replace(/\s+/g, " ").trim(), hint: where, detail };
		}
		case "auth":
			return { kind: "warn", title: t(c.error?.includes("asks") ? "OBS asks for a password" : "Wrong password"), hint: t("{where} · check the password below", { where }) };
		case "offline":
			return { kind: "bad", title: t("OBS is not reachable"), hint: t("{error}. Is OBS running with the WebSocket server on?", { error: error || where }) };
		case "error":
			return { kind: "bad", title: t("OBS error"), hint: error || where };
		default:
			return { kind: "warn busy", title: t("Connecting to OBS…"), hint: where };
	}
}

// Why the key showed its alert last time. It goes away after the next press that works.
function renderKeyError() {
	if (!conn?.keyError) return null;
	return h("div", { class: "msg bad key-error" }, t("Last error: {error}", { error: translateMessage(conn.keyError) }));
}

function problem() {
	return ["auth", "offline", "error"].includes(conn?.connection?.status);
}

// The connection box under the status: open while there is a problem, or when opened with the button.
function setupOpen() {
	return ui.setupOpen ?? ui.autoOpen;
}

function renderStatusCard() {
	const s = statusText();
	const open = setupOpen();
	const button = h("button", { class: "status-action", "aria-expanded": String(open), onclick: () => ((ui.setupOpen = !open), render()) }, t(open ? "Close" : "Change"));
	if (s.quiet) {
		return h(
			"section",
			{ class: `status ${s.kind} quiet`, "aria-live": "polite", title: s.detail || undefined },
			h("span", { class: "status-dot" }),
			h("span", { class: "status-title" }, s.title),
			h("span", { class: "status-hint" }, s.hint),
			button,
		);
	}
	return h(
		"section",
		{ class: `status ${s.kind}`, "aria-live": "polite" },
		h("span", { class: "status-dot" }),
		h("div", {}, h("div", { class: "status-title" }, s.title), s.hint ? h("div", { class: "status-hint" }, s.hint) : null),
		button,
	);
}

// Only needed while there is no connection.
function renderGuide() {
	return [
		h("div", { class: "box-title" }, t("How to connect OBS")),
		h(
			"ol",
			{ class: "steps" },
			h("li", {}, rich(t("Open **OBS** and go to **Tools → WebSocket Server Settings**."))),
			h("li", {}, rich(t("Turn on **Enable WebSocket server**."))),
			h("li", {}, rich(t("OBS on this PC? Click **Read port and password from OBS** below."))),
			h("li", {}, t("OBS on another PC? Enter its address, port and password below.")),
		),
		h("p", { class: "note" }, t("The status at the top turns green as soon as the connection works.")),
	];
}

function renderSetup() {
	const g = conn?.global ?? { host: "127.0.0.1", port: 4455 };
	const draft = (ui.setupDraft ??= { host: g.host, port: g.port, password: null });
	const field = (key, attrs) =>
		h("input", {
			...attrs,
			"data-focus-key": `setup-${key}`,
			oninput: (e) => (draft[key] = e.target.value),
		});
	return h(
		"section",
		{ class: "connection" },
		conn?.connection?.status === "connected" ? null : renderGuide(),
		// Text, checkboxes and messages line up with the fields.
		row("", h("div", { class: "note" }, t("These settings apply to all OBS Tools actions."))),
		row("", h("div", { class: "buttons" }, h("button", { class: "btn", onclick: readLocalObs }, t("Read port and password from OBS")))),
		ui.localMsg ? row("", msg(ui.localMsg)) : null,
		row(t("Host"), field("host", { type: "text", value: draft.host ?? "", placeholder: "127.0.0.1" })),
		row(t("Port"), field("port", { type: "number", value: draft.port, min: 1, max: 65535 })),
		row(
			t("Password"),
			field("password", {
				type: "password",
				value: draft.password ?? "",
				placeholder: t(g.hasPassword ? "unchanged" : "none"),
				autocomplete: "off",
			}),
		),
		row(
			"",
			h(
				"div",
				{ class: "buttons" },
				h("button", { class: "btn", onclick: testSetup }, t("Test")),
				h("button", { class: "btn primary", onclick: saveSetup }, t("Save and reconnect")),
			),
		),
		ui.setupMsg ? row("", msg(ui.setupMsg)) : null,
		schema.remote ? renderKeyConnection() : null,
	);
}

function setupPayload() {
	const d = ui.setupDraft;
	return { host: d.host, port: Number(d.port) || 4455, password: d.password === "" && conn?.global?.hasPassword ? null : d.password };
}

// Reads the OBS settings file on this PC, but only when the user clicks the button.
async function readLocalObs() {
	const r = await request({ type: "readLocalObs" });
	if (!r.found) ui.localMsg = { kind: "warn", text: t("No OBS settings found on this PC. Start OBS once, or type port and password.") };
	else if (!r.enabled) ui.localMsg = { kind: "warn", text: t("The WebSocket server is off. In OBS: Tools → WebSocket Server Settings → Enable WebSocket server.") };
	else ui.localMsg = { kind: "ok", text: t(r.authRequired ? "Found OBS on this PC: port {port}, password read from OBS." : "Found OBS on this PC: port {port}, no password.", { port: r.port }) };
	ui.setupMsg = null;
	ui.setupDraft = null;
	render();
}

async function testSetup() {
	ui.setupMsg = { kind: "", text: t("Testing…") };
	render();
	const result = await request({ type: "testConnection", settings: setupPayload() });
	ui.setupMsg = result.ok
		? { kind: "ok", text: t("It works: OBS {version} on {host}:{port}", { version: result.version?.obs ?? "", host: result.host, port: result.port }) }
		: { kind: "bad", text: `${result.message ? translateMessage(result.message) : t("No connection")} (${result.host}:${result.port})` };
	render();
}

async function saveSetup() {
	await request({ type: "saveConnection", settings: setupPayload() });
	ui.setupMsg = { kind: "ok", text: t("Saved. Connecting…") };
	ui.localMsg = null;
	ui.setupDraft = null;
	render();
}

// Recording and streaming keys can talk to another OBS than the other keys.
function renderKeyConnection() {
	const custom = !!settings.customConnection;
	const section = h(
		"div",
		{ class: "panel-section" },
		row("", h("div", { class: "section-title" }, t("Only this key"))),
		row("", checkbox(custom, t("Use another OBS for this key"), (on) => setValue("customConnection", on, true))),
	);
	if (custom) {
		append(section,
			row(t("Host"), textInput("host", { placeholder: "192.168.1.20" })),
			row(t("Port"), textInput("port", { type: "number", min: 1, max: 65535, placeholder: "4455" })),
			row(t("Password"), textInput("password", { type: "password", placeholder: t("none"), autocomplete: "off" })),
			row(
				"",
				h(
					"button",
					{
						class: "btn",
						onclick: async () => {
							ui.remoteMsg = { kind: "", text: t("Testing…") };
							render();
							const r = await request({ type: "testConnection", direct: true, settings: { host: settings.host, port: settings.port, password: settings.password ?? "" } });
							ui.remoteMsg = r.ok ? { kind: "ok", text: t("It works: OBS {version}", { version: r.version?.obs ?? "" }) } : { kind: "bad", text: r.message ? translateMessage(r.message) : t("No connection") };
							render();
						},
					},
					t("Test"),
				),
			),
			ui.remoteMsg ? row("", msg(ui.remoteMsg)) : null,
		);
	}
	return section;
}

function visible(field) {
	return !field.showIf || field.showIf(settings);
}

// What the key is about at the top, then what it does, how it looks, and the closed sections.
function renderFields() {
	const top = [];
	const look = [];
	const panels = new Map();
	for (const field of schema.fields) {
		if (!visible(field)) continue;
		const el = renderField(field);
		if (!el) continue;
		if (!field.section) top.push(el);
		else if (field.section === LOOK) look.push(el);
		else {
			if (!panels.has(field.section)) panels.set(field.section, { fields: [], wanted: false });
			const p = panels.get(field.section);
			p.fields.push(el);
			if (field.openIf?.(settings)) p.wanted = true;
		}
	}
	const out = [...top];
	if (panels.has(CONTROLS)) out.push(section(CONTROLS, true, panels.get(CONTROLS).fields));
	if (look.length) out.push(section(schema.lookTitle ?? "On the key", true, look));
	for (const [title, p] of panels) if (title !== CONTROLS) out.push(section(title, p.wanted, p.fields));
	return out;
}

// A small heading that folds the fields below it. The fields keep the grid of the rows above.
function section(title, open, fields) {
	const id = `${action}:${title}`;
	return h(
		"details",
		{ class: "section", open: ui.open[id] ?? open, ontoggle: (e) => (ui.open[id] = e.target.open) },
		h("summary", { class: "heading" }, t(title)),
		...fields,
	);
}

// Several settings at once, from one choice.
function setValues(values) {
	Object.assign(settings, values);
	save(true);
	render();
}

// Pictures on one page share the ids of their gradients and clips. Give each picture its own.
function ownIds(svg, suffix) {
	return svg.replace(/\bid="([^"]+)"/g, `id="$1-${suffix}"`).replace(/url\(#([^)]+)\)/g, `url(#$1-${suffix})`);
}

function setValue(key, value, rerender = false) {
	settings[key] = value;
	save(rerender);
	if (key === "source") loadLists(["filters", "filterSettings"]);
	if (key === "filter" && neededLists().includes("filterSettings")) loadLists(["filterSettings"]);
	if (key === "scene" && neededLists().includes("sceneItems")) loadLists(["sceneItems"]);
	if (rerender) render();
}

function textInput(key, attrs = {}) {
	return h("input", {
		type: "text",
		...attrs,
		value: settings[key] ?? "",
		"data-focus-key": key,
		oninput: (e) => setValue(key, e.target.value),
		onchange: () => (save(true), render()),
	});
}

function numberInput(field) {
	const { key } = field;
	const input = h("input", {
		type: "number",
		value: settings[key] ?? "",
		min: field.min,
		max: field.max,
		step: field.step,
		placeholder: field.placeholder ? t(field.placeholder) : undefined,
		"data-focus-key": key,
		oninput: (e) => setValue(key, e.target.value === "" ? (field.allowEmpty ? "" : settings[key]) : Number(e.target.value)),
		onchange: () => (save(true), render()),
	});
	// − and + instead of the tiny native arrows.
	const bump = (direction) => {
		const step = Number(field.step) > 0 ? Number(field.step) : 1;
		const from = input.value === "" ? Number(field.min ?? 0) || 0 : Number(input.value);
		let next = Math.round((from + direction * step) / step) * step;
		if (field.min !== undefined) next = Math.max(Number(field.min), next);
		if (field.max !== undefined) next = Math.min(Number(field.max), next);
		next = Math.round(next * 1000) / 1000;
		input.value = next;
		setValue(key, next);
		save(true);
	};
	return h(
		"div",
		{ class: "stepper" },
		h("button", { type: "button", class: "step-btn", tabindex: -1, onclick: () => bump(-1), "aria-label": t("Less") }, "−"),
		input,
		h("button", { type: "button", class: "step-btn", tabindex: -1, onclick: () => bump(1), "aria-label": t("More") }, "+"),
	);
}

function selectInput(field) {
	// Lists from the plugin hold names, or { value, label } when the name is not readable.
	const fixed = field.options;
	// Lists can hold English names from the plugin (hotkeys, filter values), translated here.
	const listLabel = (item) => {
		const name = field.translate ? translateMessage(item.label) : item.label;
		return item.range ? t("{name} ({min} to {max})", { name, min: item.range[0], max: item.range[1] }) : name;
	};
	const items = fixed ?? (lists[field.list] ?? []).map((item) => (typeof item === "string" ? [item, item] : [item.value, listLabel(item)]));
	const value = settings[field.key] ?? "";
	// Fixed options are our texts, lists hold names from OBS (never translated).
	const options = items.map(([v, label]) => ({ value: v, label: fixed ? t(label) : label }));
	if (field.empty !== undefined) options.unshift({ value: "", label: t(field.empty) });
	if (value && !options.some((o) => o.value === value)) {
		const known = fixed || (listsLoaded && field.list in lists);
		options.push({ value, label: known ? t("{value} (not found)", { value }) : value, missing: known });
	}
	const offline = conn?.connection?.status !== "connected";
	const placeholder = t(field.list && offline && !items.length ? "OBS not connected" : "Choose…");
	return dropdown({ value, options, placeholder, focusKey: field.key, onChange: (v) => setValue(field.key, v, true) });
}

function renderField(field) {
	const { key } = field;
	const label = field.label ? t(field.label) : field.label;
	const hint = field.hint ? t(field.hint) : field.hint;
	const placeholder = field.placeholder ? t(field.placeholder) : field.placeholder;
	switch (field.type) {
		case "help":
		case "note":
			return h("p", { class: "note" }, t(field.text));
		case "select":
			return row(label, selectInput(field), { hint });
		case "text":
			return row(label, textInput(key, { placeholder }), { hint });
		case "number": {
			const unit = typeof field.unit === "function" ? field.unit(settings) : field.unit;
			return row(label, [numberInput(field), unit ? h("span", { class: "unit" }, t(unit)) : null], { hint });
		}
		case "checkbox":
			return row(label, checkbox(settings[key], field.text ? t(field.text) : "", (on) => setValue(key, on, true)));
		case "color":
			return row(
				label,
				h("label", { class: "swatch", style: `background:${settings[key] || "#ff0000"}`, title: t("Pick a color") }, h("input", { type: "color", value: settings[key] || "#ff0000", onchange: (e) => setValue(key, e.target.value, true) })),
			);
		case "textarea":
			return row(label, h("textarea", { value: settings[key] ?? "", placeholder, rows: 4, "data-focus-key": key, oninput: (e) => setValue(key, e.target.value), onchange: () => save(true) }), { top: true, hint });
		case "segmented":
			return row(
				label,
				h(
					"div",
					{ class: "segmented" },
					field.options.map(([v, text]) => h("button", { class: (settings[key] ?? field.options[0][0]) === v ? "on" : "", onclick: () => setValue(key, v, true) }, t(text))),
				),
			);
		case "tiles": {
			const current = field.value ? field.value(settings) : (settings[key] ?? field.options[0][0]);
			return row(
				label,
				h(
					"div",
					{ class: "tiles", role: "radiogroup" },
					field.options.map(([v, text], i) => {
						const on = current === v;
						const pick = () => (field.apply ? setValues(field.apply(v)) : setValue(key, v, true));
						return h(
							"button",
							{ class: `tile${on ? " on" : ""}`, role: "radio", "aria-checked": String(on), onclick: pick },
							h("span", { class: "tile-key", innerHTML: ownIds(field.preview(v, settings), `${key}-${i}`) }),
							h("span", { class: "tile-name" }, t(text)),
						);
					}),
				),
				{ top: true },
			);
		}
		case "file":
		case "folder":
			return [
				row(label, [textInput(key, { placeholder }), h("button", { class: "btn", onclick: () => pickFile(key, field.type === "folder", field.images) }, t("Browse…"))], { hint }),
				ui.pickMsg ? row("", msg(ui.pickMsg)) : null,
			];
		case "phases":
			return renderPhases();
		case "twitch":
			return renderTwitch();
	}
	return null;
}

async function pickFile(key, folder = false, images = false) {
	ui.pickMsg = { kind: "", text: t(folder ? "A folder dialog opened on your desktop…" : "A file dialog opened on your desktop…") };
	render();
	const title = t(folder ? "Choose a folder" : images ? "Choose a picture" : "Choose a video");
	const reply = await request({ type: "pickFile", key, folder, images, start: settings[key] ?? "", title });
	ui.pickMsg = reply.error ? { kind: "bad", text: translateMessage(reply.error) } : null;
	if (reply.file) setValue(key, reply.file, true);
	else render();
}

// ---------- Source Animation ----------

function phases() {
	return Array.isArray(settings.phases) ? settings.phases : [];
}

function renderPhases() {
	const list = phases();
	if (!Array.isArray(settings.phases) && conn) settings.phases = list;
	const wrap = h("div", { class: "box" }, h("div", { class: "box-title" }, t("Phases")));
	list.forEach((phase, index) => {
		phase.changes ??= {};
		const setPhase = (k, v) => {
			phase[k] = v;
			save();
		};
		const grid = h("div", { class: "phase-grid" });
		for (const [field, label] of Object.entries(FIELD_LABELS)) {
			const value = phase.changes[field] ?? "";
			append(grid, 
				h(
					"label",
					{},
					t(label),
					h("input", {
						type: "number",
						step: field.startsWith("scale") ? 0.01 : 1,
						value,
						class: value !== "" && Number(value) !== 0 ? "changed" : "",
						"data-focus-key": `phase-${index}-${field}`,
						oninput: (e) => {
							if (e.target.value === "" || Number(e.target.value) === 0) delete phase.changes[field];
							else phase.changes[field] = Number(e.target.value);
							e.target.classList.toggle("changed", e.target.value !== "" && Number(e.target.value) !== 0);
							save();
						},
					}),
				),
			);
		}
		const move = (dir) => {
			const target = index + dir;
			if (target < 0 || target >= list.length) return;
			[list[index], list[target]] = [list[target], list[index]];
			save(true);
			render();
		};
		append(wrap, 
			h(
				"div",
				{ class: "phase" },
				h(
					"div",
					{ class: "phase-head" },
					h("span", { class: "label" }, t("Phase {n}", { n: index + 1 })),
					h("button", { class: "btn icon", title: t("Move up"), disabled: index === 0, onclick: () => move(-1) }, "▲"),
					h("button", { class: "btn icon", title: t("Move down"), disabled: index === list.length - 1, onclick: () => move(1) }, "▼"),
					h("button", { class: "btn icon danger", title: t("Remove"), onclick: () => (list.splice(index, 1), save(true), render()) }, "✕"),
				),
				h(
					"div",
					{ class: "phase-grid" },
					h("label", {}, t("Duration ms"), h("input", { type: "number", min: 0, step: 100, value: phase.duration ?? 1000, "data-focus-key": `phase-${index}-duration`, oninput: (e) => setPhase("duration", Number(e.target.value) || 0) })),
					h("label", {}, t("Wait ms"), h("input", { type: "number", min: 0, step: 100, value: phase.delay ?? 0, "data-focus-key": `phase-${index}-delay`, oninput: (e) => setPhase("delay", Number(e.target.value) || 0) })),
					h(
						"label",
						{},
						t("Easing"),
						(() => {
							return dropdown({
								value: phase.easing ?? "linear",
								options: EASINGS.map(([v, l]) => ({ value: v, label: t(l) })),
								focusKey: `phase-${index}-easing`,
								onChange: (v) => (setPhase("easing", v), render()),
							});
						})(),
					),
				),
				h("div", { class: "note", style: "margin:6px 0 4px" }, t("Change by (empty = no change):")),
				grid,
			),
		);
	});
	const recordButtons = ui.recording
		? [
				h("button", { class: "btn primary", onclick: () => record("recordStop") }, t("■ Stop recording")),
				h("button", { class: "btn", onclick: () => record("recordCancel") }, t("Cancel")),
			]
		: [h("button", { class: "btn", onclick: () => record("recordStart"), disabled: !settings.source }, t("● Record"))];
	append(wrap, 
		h(
			"div",
			{ class: "buttons" },
			h("button", { class: "btn", onclick: () => (list.push({ duration: 1000, delay: 0, easing: "easeInOut", changes: {} }), save(true), render()) }, t("+ Add phase")),
			...recordButtons,
			h("button", { class: "btn", onclick: playAnimation, disabled: !settings.source }, t("▶ Play")),
			h("button", { class: "btn", onclick: () => ((ui.exportOpen = !ui.exportOpen), (ui.importOpen = false), render()) }, t("Export")),
			h("button", { class: "btn", onclick: () => ((ui.importOpen = !ui.importOpen), (ui.exportOpen = false), render()) }, t("Import")),
		),
		msg(ui.animationMsg),
	);
	if (ui.recording) append(wrap, h("div", { class: "msg warn" }, t("Recording: now move, resize, rotate or crop the source in OBS. Then press Stop recording.")));
	if (ui.exportOpen) {
		const text = JSON.stringify(exportData(), null, 2);
		const area = h("textarea", { value: text, rows: 8, readOnly: true });
		append(wrap, 
			h("div", { class: "msg" }, t("Copy this text to share the animation or keep a backup:")),
			area,
			h(
				"div",
				{ class: "buttons", style: "margin-top:6px" },
				h(
					"button",
					{
						class: "btn",
						onclick: async () => {
							area.select();
							let ok = false;
							try {
								await navigator.clipboard.writeText(text);
								ok = true;
							} catch {
								ok = document.execCommand?.("copy");
							}
							ui.animationMsg = ok ? { kind: "ok", text: t("Copied.") } : { kind: "warn", text: t("Select the text and press Ctrl+C.") };
							render();
						},
					},
					t("Copy"),
				),
			),
		);
	}
	if (ui.importOpen) {
		const area = h("textarea", { value: ui.importText, rows: 8, placeholder: t("Paste an exported animation here"), "data-focus-key": "import", oninput: (e) => (ui.importText = e.target.value) });
		append(wrap, area, h("div", { class: "buttons", style: "margin-top:6px" }, h("button", { class: "btn primary", onclick: importAnimation }, t("Import"))));
	}
	return wrap;
}

const EXPORT_KEYS = ["phases", "showAtStart", "hideAtEnd", "resetAtEnd", "filter", "filterStart", "filterEnd"];

function exportData() {
	return { type: "obs-tools-animation", version: 1, ...Object.fromEntries(EXPORT_KEYS.map((k) => [k, settings[k]])) };
}

function importAnimation() {
	try {
		const data = JSON.parse(ui.importText);
		if (!Array.isArray(data.phases)) throw new Error(t("no phases in the text"));
		for (const key of EXPORT_KEYS) if (key in data) settings[key] = data[key];
		save(true);
		ui.importOpen = false;
		ui.importText = "";
		ui.animationMsg = { kind: "ok", text: t("Imported {n} phase(s).", { n: data.phases.length }) };
	} catch (error) {
		ui.animationMsg = { kind: "bad", text: t("Could not import: {error}", { error: error.message }) };
	}
	render();
}

async function record(type) {
	save(true);
	const reply = await request({ type });
	if (reply.type === "error") ui.animationMsg = { kind: "bad", text: translateMessage(reply.message) };
	else if (reply.type === "recording") {
		ui.recording = !!reply.active;
		ui.animationMsg = reply.message ? { kind: reply.message.startsWith("Nothing") ? "warn" : "ok", text: translateMessage(reply.message) } : null;
	}
	render();
}

async function playAnimation() {
	save(true);
	ui.animationMsg = { kind: "", text: t("Playing…") };
	render();
	const reply = await request({ type: "preview" });
	ui.animationMsg = reply.type === "error" ? { kind: "bad", text: translateMessage(reply.message) } : null;
	render();
}

// ---------- Twitch ----------

function renderTwitch() {
	const tw = conn?.twitch ?? {};
	const box = h("div", { class: "box" }, h("div", { class: "box-title" }, t("Twitch account (all keys)")));
	append(box, 
		row(
			t("Channel"),
			h("input", {
				type: "text",
				value: tw.channel ?? "",
				placeholder: t("your channel name"),
				"data-focus-key": "twitch-channel",
				onchange: (e) => send({ type: "twitchSave", channel: e.target.value }),
			}),
			{ hint: tw.chatWanted ? (tw.chatConnected ? t("Reading chat of #{channel}", { channel: tw.channel }) : t("Connecting to chat…")) : t("Chat is read without login. Clips need a login.") },
		),
	);
	if (tw.authorized) {
		append(box, 
			row(t("Login"), [h("span", {}, t("Logged in as {login}", { login: tw.login })), h("button", { class: "btn", onclick: () => send({ type: "twitchLogout" }) }, t("Log out"))]),
		);
		const missing = ["clips:edit", "user:write:chat"].filter((s) => !(tw.scopes ?? []).includes(s));
		if (missing.length) append(box, h("div", { class: "msg warn" }, t("The token is missing: {scopes}", { scopes: missing.join(", ") })));
	} else if (tw.deviceFlow) {
		append(box, 
			row(t("Code"), [h("span", { class: "code" }, tw.deviceFlow.userCode), h("button", { class: "btn", onclick: () => send({ type: "openUrl", url: tw.deviceFlow.uri }) }, t("Open page"))]),
			h("div", { class: "msg" }, t("Enter the code on the Twitch page that opened in your browser. This page updates when you are done.")),
			row("", h("button", { class: "btn", onclick: () => send({ type: "twitchCancel" }) }, t("Cancel"))),
		);
	} else {
		const clientId = h("input", { type: "text", placeholder: t("Client ID of your Twitch app"), "data-focus-key": "twitch-client" });
		const token = h("input", { type: "password", placeholder: t("or paste an access token"), autocomplete: "off", "data-focus-key": "twitch-token" });
		append(box, 
			h("details", { class: "inline" }, h("summary", {}, t("Log in for clips")), h("p", { class: "note" }, t("Create an app on dev.twitch.tv (Client Type: Public, redirect URL http://localhost), then paste its Client ID here.")),
				row(t("Client ID"), [clientId, h("button", { class: "btn primary", onclick: () => ((ui.twitchMsg = null), send({ type: "twitchDeviceFlow", clientId: clientId.value })) }, t("Log in"))]),
				row(t("Token"), [token, h("button", { class: "btn", onclick: () => ((ui.twitchMsg = null), send({ type: "twitchToken", token: token.value })) }, t("Use token"))], { hint: t("Scopes needed: clips:edit and user:write:chat") }),
			),
		);
	}
	if (ui.twitchMsg) append(box, msg(ui.twitchMsg));
	return box;
}
