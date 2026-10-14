// Measures every glyph and writes plugin/lib/glyph-fit.js, so icons can be centered and sized evenly.
// Needs ImageMagick. Run after changing a glyph: node scripts/fit-glyphs.mjs (make icons does it too)
//
// Centered by its box alone, a glyph with its weight on one side looks off: a play triangle seems
// too far left, a speaker too. So the center used is halfway between the middle of the box and the
// center of mass of the ink (the optical center).
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { GLYPHS, glyph } from "../de.felitendo.obs-tools.sdPlugin/plugin/lib/icons.js";

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), "..");
const out = path.join(root, "de.felitendo.obs-tools.sdPlugin", "plugin", "lib", "glyph-fit.js");
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "glyphs-"));
const PX = 20; // pixels per grid unit
const PAD = 4; // grid units around the 24 box, for strokes that stick out
const OPTICAL = 0.5; // 0 = middle of the box, 1 = center of mass

const fit = {};
for (const name of Object.keys(GLYPHS)) {
	const file = path.join(tmp, `${name}.svg`);
	const size = (24 + PAD * 2) * PX;
	fs.writeFileSync(file, `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="${-PAD} ${-PAD} ${24 + PAD * 2} ${24 + PAD * 2}">${glyph(name, { fit: false, color: "#000000" })}</svg>`);
	// Raw 8-bit gray pixels: 0 is ink, 255 is paper.
	const raw = execFileSync("magick", ["-background", "white", file, "-flatten", "-colorspace", "gray", "-depth", "8", "gray:-"]);
	// ImageMagick may render at another size than asked, so measure against what it made.
	const width = Math.round(Math.sqrt(raw.length));
	const px = width / (24 + PAD * 2);
	let [x0, y0, x1, y1, mass, mx, my] = [width, width, -1, -1, 0, 0, 0];
	for (let y = 0; y < width; y++) {
		for (let x = 0; x < width; x++) {
			const ink = (255 - raw[y * width + x]) / 255;
			if (ink < 0.1) continue;
			x0 = Math.min(x0, x);
			y0 = Math.min(y0, y);
			x1 = Math.max(x1, x);
			y1 = Math.max(y1, y);
			mass += ink;
			mx += ink * (x + 0.5);
			my += ink * (y + 0.5);
		}
	}
	const [bx, by] = [(x0 + x1 + 1) / 2, (y0 + y1 + 1) / 2];
	const cx = bx + OPTICAL * (mx / mass - bx);
	const cy = by + OPTICAL * (my / mass - by);
	const round = (v) => Math.round(v * 1000) / 1000;
	fit[name] = { cx: round(cx / px - PAD), cy: round(cy / px - PAD), w: round((x1 - x0 + 1) / px), h: round((y1 - y0 + 1) / px) };
}
fs.rmSync(tmp, { recursive: true });
fs.writeFileSync(out, `// Written by scripts/fit-glyphs.mjs: center and size of every glyph, in grid units.\nexport const FIT = ${JSON.stringify(fit, null, "\t")};\n`);
console.log(`measured ${Object.keys(fit).length} glyphs -> ${path.relative(root, out)}`);
