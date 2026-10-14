import { FIT } from "./glyph-fit.js";

// Glyphs drawn on a 24x24 grid. Bold, like the other FeloDeck plugins: a body is always filled
// (details are cut out of it), round 2.4px strokes only for lines like arrows and waves.
// Used for the static icons (scripts/build-icons.mjs) and for keys drawn at runtime.

function gear(cx, cy, outer, inner, teeth) {
	const points = [];
	const step = (Math.PI * 2) / (teeth * 4);
	for (let i = 0; i < teeth * 4; i++) {
		const r = i % 4 < 2 ? outer : inner;
		const a = i * step - Math.PI / 2 + step / 2;
		points.push(`${(cx + r * Math.cos(a)).toFixed(2)} ${(cy + r * Math.sin(a)).toFixed(2)}`);
	}
	return `M${points.join("L")}Z`;
}

const fill = `fill="currentColor"`;
// A filled body with details cut out of it. Cuts are lines in the stroke width, or shapes with
// fill="#000"; add puts parts back on top of a cut.
const cut = (id, body, cuts, add = "") =>
	`<mask id="cut-${id}"><g fill="#FFF" stroke="none">${body}</g><g fill="none" stroke="#000">${cuts}</g><g fill="none" stroke="#FFF">${add}</g></mask>` +
	`<rect x="-4" y="-4" width="32" height="32" ${fill} stroke="none" mask="url(#cut-${id})"/>`;
const speaker = `<path d="M3.5 9.2h3.2L11.5 5v14l-4.8-4.2H3.5a1 1 0 0 1-1-1v-3.6a1 1 0 0 1 1-1z" ${fill} stroke-width="1.6"/>`;

export const GLYPHS = {
	plugin: cut("plugin", `<path d="${gear(12, 12, 10.6, 8.2, 8)}"/>`, `<circle cx="12" cy="12" r="3.4" fill="#000" stroke="none"/>`),
	scene: `<rect x="1" y="7.5" width="17" height="13" rx="3" ${fill} stroke="none"/><path d="M7 4h12.5a2.5 2.5 0 0 1 2.5 2.5v9"/>`,
	animation: `<rect x="10.5" y="5.5" width="12" height="13" rx="2.6" ${fill} stroke="none"/><path d="M2 8h5M3.5 12h4M2 16h5"/>`,
	studio: `<rect x="1" y="4.5" width="10" height="13.5" rx="2.2" ${fill} stroke="none"/><rect x="13" y="4.5" width="10" height="13.5" rx="2.2" ${fill} stroke="none"/><path d="M6 21.5h12"/>`,
	video: cut("video", `<rect x="1" y="3" width="22" height="18" rx="3.4"/>`, `<path d="M10 8.6 15.5 12 10 15.4z" fill="#000" stroke-width="1.6"/>`),
	videoStop: cut("videoStop", `<rect x="1" y="3" width="22" height="18" rx="3.4"/>`, `<rect x="8.5" y="8.5" width="7" height="7" rx="1.2" fill="#000" stroke="none"/>`),
	volume: `${speaker}<path d="M15.2 8.8a4.6 4.6 0 0 1 0 6.4M18.3 5.8a9 9 0 0 1 0 12.4"/>`,
	volumeUp: `${speaker}<path d="M15.5 12h6M18.5 9v6"/>`,
	volumeDown: `${speaker}<path d="M15.5 12h6"/>`,
	volumeSet: `${speaker}<path d="M15.5 10h6M15.5 14h6"/>`,
	mute: `${speaker}<path d="m15.5 9.2 5.6 5.6M21.1 9.2l-5.6 5.6"/>`,
	unmuted: `${speaker}<path d="M15.2 8.8a4.6 4.6 0 0 1 0 6.4M18.3 5.8a9 9 0 0 1 0 12.4"/>`,
	monitor: `<path d="M3.5 17v-5a8.5 8.5 0 0 1 17 0v5"/><path d="M20.5 14v4.5a2 2 0 0 1-2 2H17v-6.5zM3.5 14v4.5a2 2 0 0 0 2 2H7v-6.5z" ${fill}/>`,
	dropped: cut("dropped", `<rect x="1" y="4" width="22" height="16" rx="3"/>`, `<path d="M6.5 4v16M17.5 4v16M12 8.4v4.2M12 15.6v.1"/>`),
	cpu: cut("cpu", `<rect x="4.8" y="4.8" width="14.4" height="14.4" rx="2.6"/>`, `<rect x="9.6" y="9.6" width="4.8" height="4.8" rx=".8"/>`) + `<path d="M9 2v2.5M15 2v2.5M9 19.5V22M15 19.5V22M2 9h2.5M2 15h2.5M19.5 9H22M19.5 15H22"/>`,
	previous: `<path d="M19 6 9.5 12l9.5 6z" ${fill}/><rect x="4.5" y="5" width="3.2" height="14" rx="1.2" ${fill} stroke="none"/>`,
	next: `<path d="M5 6l9.5 6L5 18z" ${fill}/><rect x="16.3" y="5" width="3.2" height="14" rx="1.2" ${fill} stroke="none"/>`,
	back: `<path d="M9.5 9H15a5 5 0 0 1 0 10H8"/><path d="M10.5 4.5 4 9l6.5 4.5z" ${fill} stroke-width="1.6"/>`,
	replay: `<path d="M3.5 12a8.5 8.5 0 1 0 2.5-6l-2.5 2.5"/><path d="M3.5 3.5v5h5"/><path d="m10.5 9 4.5 3-4.5 3z" ${fill} stroke-width="1.6"/>`,
	browser: cut("browser", `<circle cx="12" cy="12" r="10.4"/>`, `<path d="M1.5 12h21M12 1.6c2.6 2.6 3.9 6 3.9 10.4s-1.3 7.8-3.9 10.4c-2.6-2.6-3.9-6-3.9-10.4s1.3-7.8 3.9-10.4z"/>`),
	record: `<circle cx="12" cy="12" r="9.5" ${fill} stroke="none"/>`,
	pause: cut("pause", `<circle cx="12" cy="12" r="10.2"/>`, `<path d="M9.6 8.6v6.8M14.4 8.6v6.8"/>`),
	stream: `<circle cx="12" cy="12" r="2.3" ${fill} stroke="none"/><path d="M16.2 7.8a6 6 0 0 1 0 8.4M7.8 16.2a6 6 0 0 1 0-8.4M19.1 4.9a10 10 0 0 1 0 14.2M4.9 19.1a10 10 0 0 1 0-14.2"/>`,
	transition: `<path d="M2.5 7h3.2c2.6 0 3.9 1.6 5.3 5s2.7 5 5.3 5h4.2M2.5 17h3.2c1.4 0 2.4-.5 3.2-1.3M13.1 8.3c.8-.8 1.8-1.3 3.1-1.3h4.3M18 4l3 3-3 3M18 14l3 3-3 3"/>`,
	filter: `<path d="M3 4.5h18l-7 8.2v6.3l-4 2v-8.3z" ${fill} stroke-width="1.8"/>`,
	virtualcam: `<rect x="2" y="6" width="13.5" height="12" rx="2.5" ${fill} stroke="none"/><path d="m17 10.3 5-3v9.4l-5-3z" ${fill} stroke-width="1.6"/>`,
	window: cut("window", `<rect x="1" y="3" width="22" height="18" rx="3.4"/>`, `<path d="M1 8.5h22M12 11.4v.6M12 17v.6M8.8 14.5h.6M14.6 14.5h.6"/><circle cx="12" cy="14.5" r="1.4" fill="#000" stroke="none"/>`),
	dial: cut("dial", `<circle cx="12" cy="13" r="8.7"/>`, `<path d="m12 13 3.2-3.8"/>`) + `<path d="M3.5 6.2 5 7.7M20.5 6.2 19 7.7M12 1.5v1.6"/>`,
	power: `<path d="M12 3v9"/><path d="M18.4 6.6a9 9 0 1 1-12.8 0"/>`,
	lock: `<rect x="5" y="11" width="14" height="10" rx="2" ${fill} stroke="none"/><path d="M8 11V7a4 4 0 0 1 8 0v4"/>`,
	twitch: cut("twitch", `<path d="M4 2 2.4 6.6V20H7v3h3.4l3-3h4.2l4.8-4.8V2z" stroke="#FFF" stroke-width="1.4" stroke-linejoin="round"/>`, `<path d="M11 7.6v4.2M16 7.6v4.2"/>`),
	eye: cut("eye", `<path d="M.6 12S5 4 12 4s11.4 8 11.4 8S19 20 12 20 .6 12 .6 12z" stroke="#FFF" stroke-width="1.2" stroke-linejoin="round"/>`, `<circle cx="12" cy="12" r="3.9"/>`),
	eyeOff: cut("eyeOff", `<path d="M.6 12S5 4 12 4s11.4 8 11.4 8S19 20 12 20 .6 12 .6 12z" stroke="#FFF" stroke-width="1.2" stroke-linejoin="round"/>`, `<path d="m3 3 18 18" stroke-width="6"/>`, `<path d="m3 3 18 18"/>`),
	keyboard: cut("keyboard", `<rect x="1" y="5" width="22" height="15" rx="3.2"/>`, `<path d="M5.8 9.6h.01M9.3 9.6h.01M12.8 9.6h.01M16.3 9.6h.01M7 15h10"/>`),
	camera: cut("camera", `<path d="M3 8.6a2 2 0 0 1 2-2h2.2L8.8 4h6.4l1.6 2.6H19a2 2 0 0 1 2 2v9.4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" stroke="#FFF" stroke-width="2.4" stroke-linejoin="round"/>`, `<circle cx="12" cy="13.2" r="4.4"/>`),
	chapter: `<path d="M6.5 3.5h11v17L12 16.6l-5.5 3.9z" ${fill} stroke-width="1.6"/>`,
	split: `<circle cx="6" cy="6.5" r="3"/><circle cx="6" cy="17.5" r="3"/><path d="M8.6 8 20 18.5M8.6 16 20 5.5"/>`,
	play: `<path d="M7 4.6 19.2 12 7 19.4z" ${fill} stroke-width="1.6"/>`,
	pauseMedia: `<rect x="5.5" y="4.5" width="4.6" height="15" rx="1.4" ${fill} stroke="none"/><rect x="13.9" y="4.5" width="4.6" height="15" rx="1.4" ${fill} stroke="none"/>`,
	playPause: `<path d="M3 5.5 12 12l-9 6.5z" ${fill} stroke-width="1.6"/><rect x="14" y="5.5" width="3" height="13" rx="1" ${fill} stroke="none"/><rect x="19" y="5.5" width="3" height="13" rx="1" ${fill} stroke="none"/>`,
	stop: `<rect x="5.5" y="5.5" width="13" height="13" rx="2.2" ${fill} stroke="none"/>`,
	restart: `<path d="M20 12a8 8 0 1 1-2.4-5.7L20 8.5"/><path d="M20 3.5v5h-5"/>`,
	prevTrack: `<path d="M19 6 9.5 12l9.5 6z" ${fill}/><rect x="4.5" y="5" width="3.2" height="14" rx="1.2" ${fill} stroke="none"/>`,
	nextTrack: `<path d="M5 6l9.5 6L5 18z" ${fill}/><rect x="16.3" y="5" width="3.2" height="14" rx="1.2" ${fill} stroke="none"/>`,
	forward: `<path d="M3 6.5 11 12l-8 5.5zM12.5 6.5l8 5.5-8 5.5z" ${fill} stroke-width="1.6"/>`,
	rewind: `<path d="M21 6.5 13 12l8 5.5zM11.5 6.5l-8 5.5 8 5.5z" ${fill} stroke-width="1.6"/>`,
	profile: `<circle cx="12" cy="7.5" r="4.6" ${fill} stroke="none"/><path d="M3.6 20.4a8.4 8.4 0 0 1 16.8 0z" ${fill}/>`,
	collection: `<path d="M12 3 2.5 8 12 13l9.5-5z" ${fill} stroke-width="1.6"/><path d="m2.5 12.4 9.5 5 9.5-5M2.5 16.4l9.5 5 9.5-5"/>`,
	image: cut("image", `<rect x="1.5" y="3" width="21" height="18" rx="3.4"/>`, `<circle cx="8" cy="8.8" r="2.1" fill="#000" stroke="none"/><path d="m5.5 17 4-4.6 2.9 3 2.4-2.4 3.7 4z" fill="#000" stroke-width="1.6"/>`),
	text: `<path d="M5 7V4.5h14V7M12 4.5v15M8.5 19.5h7"/>`,
	health: cut("health", `<path d="M2.4 18a9.6 9.6 0 1 1 19.2 0z" stroke="#FFF" stroke-width="2.4" stroke-linejoin="round"/>`, `<path d="m12 15.6 3.6-4.4"/><circle cx="12" cy="16" r="2.2" fill="#000" stroke="none"/>`),
};

export const COLORS = {
	white: "#FFFFFF",
	gray: "#6B6B6B",
	muted: "#9A9AA2",
	red: "#E5484D",
	green: "#30A46C",
	blue: "#3E8EF7",
	amber: "#F5A524",
};

export function escapeXml(text) {
	return String(text ?? "").replace(/[<>&"']/g, (c) => ({ "<": "&lt;", ">": "&gt;", "&": "&amp;", '"': "&quot;", "'": "&apos;" })[c]);
}

// The glyph of a volume key for each mode. Mute shows the plain speaker.
export const VOLUME_GLYPHS = { up: "volumeUp", down: "volumeDown", set: "volumeSet" };

// The glyph of a media key for each operation. Play / pause shows pause while it plays.
export const MEDIA_GLYPHS = { play: "play", pause: "pauseMedia", stop: "stop", restart: "restart", next: "nextTrack", previous: "prevTrack", forward: "forward", back: "rewind" };

// How much of the 24 grid a glyph may use: wide ones up to MAX, compact ones less,
// so a circle does not look bigger than a wide eye.
const MAX = 20.5;
const AREA = 17.5;

const fitScale = (box) => Math.min(MAX / Math.max(box.w, box.h), AREA / Math.sqrt(box.w * box.h));

// How tall a glyph is drawn, in units of the 24 grid.
export function glyphHeight(name) {
	const box = FIT[name];
	return box ? box.h * fitScale(box) : 24;
}

// The fitted box of a glyph: centered on (12, 12) and with an even optical size.
// FIT holds the measured bounds, see scripts/fit-glyphs.mjs.
function fitTransform(name) {
	const box = FIT[name];
	if (!box) return { inner: "", scale: 1 };
	const k = fitScale(box);
	const n = (v) => Math.round(v * 1000) / 1000;
	return { inner: `translate(12 12) scale(${n(k)}) translate(${n(-box.cx)} ${n(-box.cy)})`, scale: k };
}

// A glyph placed at (x, y) with the given size in pixels.
// fit: false draws it as written, used to measure it.
export function glyph(name, { x = 0, y = 0, size = 24, color = COLORS.white, stroke = 2.4, fit = true } = {}) {
	const body = GLYPHS[name] ?? GLYPHS.plugin;
	const n = (v) => Math.round(v * 1000) / 1000;
	const { inner, scale } = fit ? fitTransform(name) : { inner: "", scale: 1 };
	// Keep the stroke the same on screen, whatever the glyph was scaled by.
	const width = n(stroke / scale);
	const outer = `<g transform="translate(${n(x)} ${n(y)}) scale(${n(size / 24)})" fill="none" stroke="${color}" color="${color}" stroke-width="${width}" stroke-linecap="round" stroke-linejoin="round">`;
	return inner ? `${outer}<g transform="${inner}">${body}</g></g>` : `${outer}${body}</g>`;
}

// Icon for the action list: white glyph, no background.
export function actionIconSvg(name, viewBox = "0 0 24 24") {
	return `<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40" viewBox="${viewBox}">${glyph(name)}</svg>\n`;
}
