// Writes an OpenDeck profile for the virtual deck, so tests do not need drag and drop.
// Stop OpenDeck first, it keeps profiles in memory.
//
//   node tools/virtual-deck/make-profile.mjs <layout.json> [profile name]
//
// layout.json: { "keys": { "0": { "action": "de.felitendo.obs-tools.smartscene", "settings": {...} } },
//                "dials": { "0": { "action": "...", "settings": {...} } } }
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const CONFIG = path.join(process.env.XDG_CONFIG_HOME || path.join(os.homedir(), ".config"), "opendeck");
const DEVICE = "vd-virtual-deck";
const [layoutFile, profileName = "OBS Tools Test"] = process.argv.slice(2);
if (!layoutFile) {
	console.error("usage: make-profile.mjs <layout.json> [profile name]");
	process.exit(1);
}
const layout = JSON.parse(fs.readFileSync(layoutFile, "utf8"));
const rows = Number(process.env.VIRTUAL_DECK_ROWS) || 3;
const columns = Number(process.env.VIRTUAL_DECK_COLUMNS) || 5;
const dials = Number(process.env.VIRTUAL_DECK_DIALS) || 4;

const manifests = new Map();
function pluginFor(uuid) {
	for (const dir of fs.readdirSync(path.join(CONFIG, "plugins"))) {
		const file = path.join(CONFIG, "plugins", dir, "manifest.json");
		if (!fs.existsSync(file)) continue;
		if (!manifests.has(dir)) manifests.set(dir, JSON.parse(fs.readFileSync(file, "utf8")));
		const action = manifests.get(dir).Actions?.find((a) => a.UUID === uuid);
		if (action) return { dir, manifest: manifests.get(dir), action };
	}
	throw new Error(`no installed plugin has the action ${uuid}`);
}

// Same lookup as OpenDeck: prefer .svg, then @2x.png, then .png. Paths are relative to the config dir.
function icon(dir, file) {
	const base = path.join(CONFIG, "plugins", dir, file);
	for (const ext of [".svg", "@2x.png", ".png"]) if (fs.existsSync(base + ext)) return path.relative(CONFIG, base + ext);
	return path.relative(CONFIG, base + ".png");
}

function state(dir, s, actionIcon) {
	return {
		image: s.Image ? icon(dir, s.Image) : actionIcon,
		image_scale: 100,
		background_colour: "#000000",
		name: s.Name ?? "",
		text: s.Title ?? "",
		show: s.ShowTitle ?? true,
		colour: s.TitleColor ?? "#FFFFFF",
		stroke_colour: "#000000",
		alignment: s.TitleAlignment ?? "middle",
		family: s.FontFamily ?? "Liberation Sans",
		style: s.FontStyle ?? "Regular",
		size: Number(s.FontSize ?? 16),
		stroke_size: 3,
		underline: !!s.FontUnderline,
	};
}

const BUILTIN = {
	"opendeck.multiaction": { name: "Multi Action", image: "opendeck/multi-action.png", tooltip: "Execute multiple actions" },
	"opendeck.toggleaction": { name: "Toggle Action", image: "opendeck/toggle-action.png", tooltip: "Cycle through multiple actions" },
};

// Multi and toggle actions from OpenDeck itself, with plugin actions as children.
function builtin(position, spec) {
	const info = BUILTIN[spec.action];
	const states = [state("", { Image: "" }, info.image)];
	states[0].image = info.image;
	return {
		action: {
			name: info.name,
			uuid: spec.action,
			plugin: "opendeck",
			tooltip: info.tooltip,
			icon: info.image,
			disable_automatic_states: false,
			visible_in_action_list: true,
			supported_in_multi_actions: false,
			property_inspector: "",
			controllers: ["Keypad"],
			encoder: null,
			states,
		},
		context: `Keypad.${position}.0`,
		states: structuredClone(states),
		current_state: 0,
		settings: {},
		children: (spec.children ?? []).map((child, i) => instance("Keypad", position, child, i + 1)),
	};
}

function instance(controller, position, spec, index = 0) {
	if (BUILTIN[spec.action]) return builtin(position, spec);
	const { dir, manifest, action } = pluginFor(spec.action);
	const actionIcon = icon(dir, action.Icon);
	const states = action.States.map((s) => state(dir, s, actionIcon));
	const encoder = action.Encoder
		? {
				icon: action.Encoder.Icon ? icon(dir, action.Encoder.Icon) : "",
				stack_color: "",
				trigger_description: {
					long_touch: action.Encoder.TriggerDescription?.LongTouch ?? "",
					push: action.Encoder.TriggerDescription?.Push ?? "",
					rotate: action.Encoder.TriggerDescription?.Rotate ?? "",
					touch: action.Encoder.TriggerDescription?.Touch ?? "",
				},
				background: "",
				layout: action.Encoder.layout ?? "$X1",
			}
		: null;
	return {
		action: {
			name: action.Name,
			uuid: action.UUID,
			plugin: dir,
			tooltip: action.Tooltip ?? "",
			icon: actionIcon,
			disable_automatic_states: !!action.DisableAutomaticStates,
			visible_in_action_list: action.VisibleInActionsList ?? true,
			supported_in_multi_actions: action.SupportedInMultiActions ?? true,
			property_inspector: path.relative(CONFIG, path.join(CONFIG, "plugins", dir, action.PropertyInspectorPath ?? manifest.PropertyInspectorPath ?? "")),
			controllers: action.Controllers ?? ["Keypad"],
			encoder,
			states,
		},
		context: `${controller}.${position}.${index}`,
		states: structuredClone(states),
		current_state: 0,
		settings: expandHome(spec.settings ?? {}),
		children: null,
	};
}

// "~/" at the start of a text setting means the home folder.
function expandHome(value) {
	if (typeof value === "string") return value.startsWith("~/") ? path.join(os.homedir(), value.slice(2)) : value;
	if (Array.isArray(value)) return value.map(expandHome);
	if (value && typeof value === "object") return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, expandHome(v)]));
	return value;
}

const profile = { keys: new Array(rows * columns).fill(null), sliders: new Array(dials).fill(null), infobars: [] };
for (const [position, spec] of Object.entries(layout.keys ?? {})) profile.keys[Number(position)] = instance("Keypad", Number(position), spec);
for (const [position, spec] of Object.entries(layout.dials ?? {})) profile.sliders[Number(position)] = instance("Encoder", Number(position), spec);

const dir = path.join(CONFIG, "profiles", DEVICE);
fs.mkdirSync(dir, { recursive: true });
fs.writeFileSync(path.join(dir, `${profileName}.json`), JSON.stringify(profile, null, 2));
fs.writeFileSync(path.join(CONFIG, "profiles", `${DEVICE}.json`), JSON.stringify({ selected_profile: profileName }, null, 2));
console.log(`wrote profile "${profileName}" for ${DEVICE}`);
