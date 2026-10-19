// The pictures of keys, for the settings pages. src/render.rs draws the real keys: keep the two alike
// (cargo test compares them).
import { COLORS, escapeXml, glyph, glyphHeight } from "./icons.js";

export const SIZE = 144;
const FONT = `'Inter','Noto Sans','DejaVu Sans','Liberation Sans',sans-serif`;
// Like the titles OpenDeck draws (font size 11 is about 22 px on a key), but bold to read it on
// the deck. The same in the Feishin plugin.
const TITLE_FONT = `'Liberation Sans',Arial,sans-serif`;
// The glyph takes 24 of 40 units, like viewBox="-8 -8 40 40".
const GLYPH_SIZE = (SIZE * 24) / 40;
const GLYPH_OFFSET = (SIZE - GLYPH_SIZE) / 2;
// Space between a glyph (or picture) and its title.
const TITLE_GAP = 16;

export function isColor(value) {
	return /^#[0-9a-f]{6}$/i.test(String(value ?? ""));
}

function svg(content) {
	return `<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="${SIZE}" height="${SIZE}" viewBox="0 0 ${SIZE} ${SIZE}">${content}</svg>`;
}

function flat(color) {
	return color ? `<rect width="${SIZE}" height="${SIZE}" fill="${color}"/>` : "";
}

function text(value, { x = SIZE / 2, y, size, color = COLORS.white, weight = 700, anchor = "middle", font = FONT }) {
	return `<text x="${x}" y="${y}" font-family="${font}" font-size="${size}" font-weight="${weight}" fill="${color}" text-anchor="${anchor}">${escapeXml(value)}</text>`;
}

// Font size that keeps a line inside the key. Capital letters are wider.
export function fitSize(value, max, width = 124) {
	const text = String(value ?? "");
	const length = Math.max(1, text.length);
	const perChar = text === text.toUpperCase() && /\p{L}/u.test(text) ? 0.68 : 0.58;
	return Math.max(12, Math.min(max, Math.floor(width / (length * perChar))));
}

// Audio level from dB to 0..1, like the meters in OBS (-60 dB to 0 dB).
export function meterLevel(db) {
	if (!Number.isFinite(db)) return 0;
	return Math.max(0, Math.min(1, (db + 60) / 60));
}

// Vertical level meter at the right edge: green, then yellow from -9 dB, red from -3 dB.
function meterSvg(level) {
	const x = 125;
	const y = 18;
	const w = 9;
	const h = 108;
	const fill = Math.round(h * level);
	const stop = (offset, color) => `<stop offset="${offset}" stop-color="${color}"/>`;
	let out = `<defs><linearGradient id="lvl" x1="0" y1="1" x2="0" y2="0">${stop(0, COLORS.green)}${stop(0.85, COLORS.green)}${stop(0.85, COLORS.amber)}${stop(0.95, COLORS.amber)}${stop(0.95, COLORS.red)}${stop(1, COLORS.red)}</linearGradient>`;
	out += `<clipPath id="lvlc"><rect x="${x}" y="${y + h - fill}" width="${w}" height="${fill}"/></clipPath></defs>`;
	out += `<rect x="${x}" y="${y}" width="${w}" height="${h}" rx="4.5" fill="#FFFFFF" fill-opacity="0.14"/>`;
	if (fill > 0) out += `<rect x="${x}" y="${y}" width="${w}" height="${h}" rx="4.5" fill="url(#lvl)" clip-path="url(#lvlc)"/>`;
	return out;
}

// A title the key draws itself, in the font of an OpenDeck title: 22 px like OpenDeck draws them,
// smaller for long text, and cut with "…" when even the smallest size is too wide.
// ink: how tall digits and capitals are, for placing the line.
export function titleLine(value, width = 128) {
	let line = String(value ?? "").trim();
	const perChar = 0.56;
	const size = Math.max(15, Math.min(22, Math.floor(width / (Math.max(1, line.length) * perChar))));
	const fits = Math.floor(width / (size * perChar));
	if (line.length > fits) line = `${line.slice(0, Math.max(1, fits - 1)).trimEnd()}…`;
	return { line, size, ink: Math.round(size * 0.72) };
}

function titleText(line, { x = SIZE / 2, y, stroke = false }) {
	const outline = stroke ? ` stroke="#000" stroke-width="4" paint-order="stroke"` : "";
	return `<text x="${x}" y="${Math.round(y * 10) / 10}" font-family="${TITLE_FONT}" font-size="${line.size}" font-weight="700" fill="${COLORS.white}" text-anchor="middle"${outline}>${escapeXml(line.line)}</text>`;
}

// Something of the given height with a title above or below it, together in the middle of the key.
// top: where it starts. baseline: where the title stands.
function withTitleLayout(height, line, above) {
	const top = (SIZE - height - TITLE_GAP - line.ink) / 2;
	return above ? { top: top + line.ink + TITLE_GAP, baseline: top + line.ink } : { top, baseline: SIZE - top };
}

// A centered glyph.
// meter: 0..1 adds a level meter on the right (the glyph moves a bit to the left).
// setup: a yellow dot in the corner, for a key that still needs its settings.
// title: a line of text "above" or "below" the glyph (titleAt), close to it, and glyph and title
// together in the middle of the key. OpenDeck cannot move its own title, so keys draw it here.
export function glyphKeySvg(name, { color = COLORS.white, meter = null, setup = false, title = "", titleAt = "below" } = {}) {
	return svg(glyphContent(name, { color, meter, title, titleAt }) + (setup ? `<circle cx="120" cy="24" r="10" fill="${COLORS.amber}"/>` : ""));
}

function glyphContent(name, { color = COLORS.white, meter = null, title = "", titleAt = "below" } = {}) {
	const shift = meter === null ? 0 : 9;
	let y = GLYPH_OFFSET;
	let label = "";
	if (title) {
		const line = titleLine(title, meter === null ? 128 : 110);
		const height = (glyphHeight(name) * GLYPH_SIZE) / 24;
		const { top, baseline } = withTitleLayout(height, line, titleAt === "above");
		y = GLYPH_OFFSET + top - (SIZE - height) / 2;
		label = titleText(line, { x: SIZE / 2 - shift, y: baseline });
	}
	let content = glyph(name, { x: GLYPH_OFFSET - shift, y, size: GLYPH_SIZE, color }) + label;
	if (meter !== null) content += meterSvg(meter);
	return content;
}

// Big value with a small label above and an optional line or bar below.
// alert: a flat color behind everything (used for flashing).
// dot: a small colored dot before the label (used as a status light).
export function statKeySvg({ label = "", value = "", sub = "", bar = null, color = COLORS.blue, alert = null, dot = null }) {
	const dim = alert ? COLORS.white : COLORS.muted;
	let content = flat(alert);
	// Longer labels (other languages) get a smaller font.
	const size = Math.min(18, fitSize(label, 18, dot ? 108 : 128));
	if (dot) {
		const width = String(label).length * size * 0.59;
		content += `<circle cx="${SIZE / 2 - width / 2 - 4}" cy="36" r="6" fill="${dot}"/>`;
		content += text(label, { x: SIZE / 2 + 8, y: 42, size, color: dim });
	} else content += text(label, { y: 42, size, color: dim });
	content += text(value, { y: 90, size: fitSize(value, 46), color: COLORS.white });
	if (sub) content += text(sub, { y: 120, size: fitSize(sub, 19), weight: 600, color: dim });
	if (bar !== null) {
		const pct = Math.max(0, Math.min(1, bar));
		const y = sub ? 126 : 110;
		content += `<rect x="24" y="${y}" width="96" height="8" rx="4" fill="#FFFFFF" fill-opacity="0.18"/>`;
		if (pct > 0) content += `<rect x="24" y="${y}" width="${Math.max(8, 96 * pct)}" height="8" rx="4" fill="${alert ? COLORS.white : color}"/>`;
	}
	return svg(content);
}

// Scene preview with a colored frame for live (program) or preview.
// fit: "fit" shows the whole scene as a picture, "fill" crops the scene to fill the whole key.
// title: the name, under the glyph or the picture (both together in the middle), or over a
// picture that fills the key.
export function sceneKeySvg({ image = null, border = null, glyphName = "scene", fit = "fit", title = "" }) {
	let content = "";
	if (!image) {
		content = glyphContent(glyphName, { title });
	} else {
		const href = image.replace(/^data:image\/jpg;/, "data:image/jpeg;");
		const line = title ? titleLine(title) : null;
		if (fit === "fill") {
			content = `<image x="0" y="0" width="${SIZE}" height="${SIZE}" preserveAspectRatio="xMidYMid slice" href="${href}" xlink:href="${href}"/>`;
			if (line) content += titleText(line, { y: SIZE - 16, stroke: true });
		} else {
			// 120 x 76 fits 16:9 and 16:10 canvases. Top aligned in its box.
			const { top, baseline } = line ? withTitleLayout(76, line, false) : { top: (SIZE - 76) / 2 };
			content = `<defs><clipPath id="pic"><rect x="12" y="${top}" width="120" height="76" rx="6"/></clipPath></defs>`;
			content += `<image x="12" y="${top}" width="120" height="76" preserveAspectRatio="xMidYMin meet" clip-path="url(#pic)" href="${href}" xlink:href="${href}"/>`;
			if (line) content += titleText(line, { y: baseline });
		}
	}
	if (border) content += `<rect x="4" y="4" width="${SIZE - 8}" height="${SIZE - 8}" rx="12" fill="none" stroke="${border}" stroke-width="8"/>`;
	return svg(content);
}
