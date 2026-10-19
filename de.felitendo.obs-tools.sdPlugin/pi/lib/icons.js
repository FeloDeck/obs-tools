// Glyphs for the pictures of keys in the settings pages. The glyphs themselves are in src/icons.rs,
// `cargo xtask manifest` copies them into shared.js. Keep this file like src/icons.rs.
import { COLORS, FIT, GLYPHS, MEDIA_GLYPHS, VOLUME_GLYPHS } from "./shared.js";

export { COLORS, GLYPHS, MEDIA_GLYPHS, VOLUME_GLYPHS };

export function escapeXml(text) {
	return String(text ?? "").replace(/[<>&"']/g, (c) => ({ "<": "&lt;", ">": "&gt;", "&": "&amp;", '"': "&quot;", "'": "&apos;" })[c]);
}

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
// FIT holds the measured bounds, see `cargo xtask icons`.
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
