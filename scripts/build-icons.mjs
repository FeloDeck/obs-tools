// Writes the SVG icons in images/ from the glyphs in plugin/lib/icons.js.
// Run after changing a glyph: node scripts/build-icons.mjs
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { COLORS, GLYPHS, actionIconSvg } from "../de.felitendo.obs-tools.sdPlugin/plugin/lib/icons.js";
import { glyphKeySvg } from "../de.felitendo.obs-tools.sdPlugin/plugin/lib/render.js";
import { KEY_IMAGES } from "../de.felitendo.obs-tools.sdPlugin/plugin/lib/key-images.js";

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), "..", "de.felitendo.obs-tools.sdPlugin", "images");


const write = (file, content) => {
	fs.mkdirSync(path.dirname(file), { recursive: true });
	fs.writeFileSync(file, content);
};

fs.rmSync(root, { recursive: true, force: true });
for (const name of Object.keys(GLYPHS)) write(path.join(root, "actions", `${name}.svg`), actionIconSvg(name));
for (const [name, spec] of Object.entries(KEY_IMAGES)) {
	write(path.join(root, "keys", `${name}.svg`), glyphKeySvg(spec.glyph, { color: spec.color ?? COLORS.white }) + "\n");
}

// Plugin icon like the Feishin plugin: the blades of the OBS logo in black on a light circle. The
// category icon in the action list is tiny, so there the blades are white, without the circle.
// The logo belongs to the OBS Project: scripts/assets/obs-logo.svg is the file from OBS Studio
// (GPL-2.0), unchanged. The blades are its last path.
const obsSvg = fs.readFileSync(path.join(path.dirname(fileURLToPath(import.meta.url)), "assets", "obs-logo.svg"), "utf8");
const blades = [...obsSvg.matchAll(/<path\b[^>]*\bd="([^"]+)"/g)].at(-1)[1];
// Center of mass of the blades (486, 482) and how far their tips reach from it (418). The stroke
// makes the thin blades as bold as the Feishin glyph.
const bladesAt = (c, r, color) => {
	const k = r / 418;
	return `<path transform="translate(${(c - 486 * k).toFixed(2)} ${(c - 482 * k).toFixed(2)}) scale(${k.toFixed(4)})" d="${blades}" fill="${color}" stroke="${color}" stroke-width="44" stroke-linejoin="round"/>`;
};
const disc =
	`<defs><linearGradient id="bg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#DFDFDF"/><stop offset="1" stop-color="#FFFFFF"/></linearGradient>` +
	`<filter id="shadow" x="-20%" y="-20%" width="140%" height="140%"><feDropShadow dx="0" dy="1.5" stdDeviation="2" flood-color="#000000" flood-opacity="0.3"/></filter></defs>` +
	`<circle cx="72" cy="72" r="72" fill="url(#bg)"/>`;
const svgOf = (size, body) => `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="0 0 144 144">${body}</svg>\n`;
write(path.join(root, "plugin.svg"), svgOf(144, disc + `<g filter="url(#shadow)">${bladesAt(72, 48, "#000000")}</g>`));
write(path.join(root, "category.svg"), svgOf(40, bladesAt(72, 66, "#FFFFFF")));
// Dials: icon in the touch strip, and the image in the dial slot of OpenDeck.
for (const name of ["volume", "transition", "scene", "filter"]) {
	write(path.join(root, "encoder", `${name}.svg`), actionIconSvg(name, "-4 -4 32 32"));
	write(path.join(root, "encoder", `${name}-key.svg`), actionIconSvg(name, "-8 -8 40 40"));
}

console.log(`icons written to ${root}`);
