// The SVG icons in images/, from the glyphs in src/icons.rs.
use std::fs;
use std::process::Command;

use obs_tools::icons::{GLYPHS, GlyphOpts, action_icon_svg, colors, glyph};
use obs_tools::key_images::KEY_IMAGES;
use obs_tools::render::{GlyphKey, glyph_key_svg};

use crate::{Result, plugin_dir, root, write};

/// Writes images/ (and pi/lib/shared.js, which holds the glyphs too).
pub fn run() -> Result {
	let images = plugin_dir().join("images");
	let _ = fs::remove_dir_all(&images);
	for (name, _) in GLYPHS.iter() {
		write(&images.join("actions").join(format!("{name}.svg")), action_icon_svg(name, "0 0 24 24"))?;
	}
	for (name, spec) in KEY_IMAGES {
		write(
			&images.join("keys").join(format!("{name}.svg")),
			glyph_key_svg(spec.glyph, &GlyphKey { color: spec.color.unwrap_or(colors::WHITE), ..Default::default() }) + "\n",
		)?;
	}

	// Plugin icon like the Feishin plugin: the blades of the OBS logo in black on a light circle. The
	// category icon in the action list is tiny, so there the blades are white, without the circle.
	// The logo belongs to the OBS Project: xtask/assets/obs-logo.svg is the file from OBS Studio
	// (GPL-2.0), unchanged. The blades are its last path.
	let obs = fs::read_to_string(root().join("xtask").join("assets").join("obs-logo.svg")).map_err(|e| format!("obs-logo.svg: {e}"))?;
	let blades = last_path_d(&obs).ok_or("no path in obs-logo.svg")?;
	// Center of mass of the blades (486, 482) and how far their tips reach from it (418). The stroke
	// makes the thin blades as bold as the Feishin glyph.
	let blades_at = |c: f64, r: f64, color: &str| {
		let k = r / 418.0;
		format!(
			r#"<path transform="translate({} {}) scale({})" d="{blades}" fill="{color}" stroke="{color}" stroke-width="44" stroke-linejoin="round"/>"#,
			obs_tools::js::to_fixed(c - 486.0 * k, 2),
			obs_tools::js::to_fixed(c - 482.0 * k, 2),
			obs_tools::js::to_fixed(k, 4)
		)
	};
	let disc = r##"<defs><linearGradient id="bg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#DFDFDF"/><stop offset="1" stop-color="#FFFFFF"/></linearGradient><filter id="shadow" x="-20%" y="-20%" width="140%" height="140%"><feDropShadow dx="0" dy="1.5" stdDeviation="2" flood-color="#000000" flood-opacity="0.3"/></filter></defs><circle cx="72" cy="72" r="72" fill="url(#bg)"/>"##;
	let svg_of = |size: u32, body: &str| {
		format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="0 0 144 144">{body}</svg>"#) + "\n"
	};
	write(&images.join("plugin.svg"), svg_of(144, &format!(r#"{disc}<g filter="url(#shadow)">{}</g>"#, blades_at(72.0, 48.0, "#000000"))))?;
	write(&images.join("category.svg"), svg_of(40, &blades_at(72.0, 66.0, "#FFFFFF")))?;
	// Dials: icon in the touch strip, and the image in the dial slot of OpenDeck.
	for name in ["volume", "transition", "scene", "filter"] {
		write(&images.join("encoder").join(format!("{name}.svg")), action_icon_svg(name, "-4 -4 32 32"))?;
		write(&images.join("encoder").join(format!("{name}-key.svg")), action_icon_svg(name, "-8 -8 40 40"))?;
	}
	write(&root().join(obs_tools::pi::SHARED_JS), obs_tools::pi::shared_js())?;
	println!("icons written to {}", images.display());
	Ok(())
}

// The d attribute of the last <path> in an SVG file.
fn last_path_d(svg: &str) -> Option<String> {
	let mut found = None;
	for (at, _) in svg.match_indices("<path") {
		let next = svg[at + 5..].chars().next();
		if !next.is_some_and(|c| c.is_whitespace() || c == '>' || c == '/') {
			continue;
		}
		let tag_end = svg[at..].find('>').map_or(svg.len(), |e| at + e);
		let tag = &svg[at..tag_end];
		let mut search = 0;
		while let Some(i) = tag[search..].find("d=\"") {
			let start = search + i;
			let before = tag[..start].chars().last();
			if before.is_some_and(|c| c.is_whitespace()) {
				let value_start = start + 3;
				if let Some(len) = tag[value_start..].find('"') {
					found = Some(tag[value_start..value_start + len].to_string());
				}
				break;
			}
			search = start + 3;
		}
	}
	found
}

const PX: f64 = 20.0; // pixels per grid unit
const PAD: f64 = 4.0; // grid units around the 24 box, for strokes that stick out
const OPTICAL: f64 = 0.5; // 0 = middle of the box, 1 = center of mass

/// Measures every glyph and writes src/glyph_fit.rs, so icons can be centered and sized evenly.
///
/// Centered by its box alone, a glyph with its weight on one side looks off: a play triangle seems
/// too far left, a speaker too. So the center used is halfway between the middle of the box and the
/// center of mass of the ink (the optical center).
pub fn fit_glyphs() -> Result {
	let tmp = std::env::temp_dir().join(format!("glyphs-{}", std::process::id()));
	fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
	let mut lines = Vec::new();
	for (name, _) in GLYPHS.iter() {
		let file = tmp.join(format!("{name}.svg"));
		let size = (24.0 + PAD * 2.0) * PX;
		let body = glyph(name, &GlyphOpts { fit: false, color: "#000000", ..Default::default() });
		let svg = format!(
			r#"<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="{} {} {} {}">{body}</svg>"#,
			-PAD,
			-PAD,
			24.0 + PAD * 2.0,
			24.0 + PAD * 2.0
		);
		write(&file, svg)?;
		// Raw 8-bit gray pixels: 0 is ink, 255 is paper.
		let output = Command::new("magick")
			.args(["-background", "white"])
			.arg(&file)
			.args(["-flatten", "-colorspace", "gray", "-depth", "8", "gray:-"])
			.output()
			.map_err(|e| format!("ImageMagick (magick) is needed: {e}"))?;
		if !output.status.success() {
			return Err(format!("magick failed for {name}: {}", String::from_utf8_lossy(&output.stderr)));
		}
		let raw = output.stdout;
		// ImageMagick may render at another size than asked, so measure against what it made.
		let width = (raw.len() as f64).sqrt().round() as usize;
		let px = width as f64 / (24.0 + PAD * 2.0);
		let (mut x0, mut y0, mut x1, mut y1): (f64, f64, f64, f64) = (width as f64, width as f64, -1.0, -1.0);
		let (mut mass, mut mx, mut my) = (0.0, 0.0, 0.0);
		for y in 0..width {
			for x in 0..width {
				let ink = (255.0 - f64::from(raw[y * width + x])) / 255.0;
				if ink < 0.1 {
					continue;
				}
				let (xf, yf) = (x as f64, y as f64);
				x0 = x0.min(xf);
				y0 = y0.min(yf);
				x1 = x1.max(xf);
				y1 = y1.max(yf);
				mass += ink;
				mx += ink * (xf + 0.5);
				my += ink * (yf + 0.5);
			}
		}
		let (bx, by) = ((x0 + x1 + 1.0) / 2.0, (y0 + y1 + 1.0) / 2.0);
		let cx = bx + OPTICAL * (mx / mass - bx);
		let cy = by + OPTICAL * (my / mass - by);
		let round = |v: f64| obs_tools::js::round(v * 1000.0) / 1000.0;
		lines.push(format!(
			"\t(\"{name}\", Fit {{ cx: {:?}, cy: {:?}, w: {:?}, h: {:?} }}),",
			round(cx / px - PAD),
			round(cy / px - PAD),
			round((x1 - x0 + 1.0) / px),
			round((y1 - y0 + 1.0) / px)
		));
	}
	let _ = fs::remove_dir_all(&tmp);
	let out = root().join("src").join("glyph_fit.rs");
	let code = format!(
		"// Written by `cargo xtask fit-glyphs`: center and size of every glyph, in grid units.\nuse crate::icons::Fit;\n\npub static FIT: &[(&str, Fit)] = &[\n{}\n];\n",
		lines.join("\n")
	);
	write(&out, code)?;
	println!("measured {} glyphs -> src/glyph_fit.rs", lines.len());
	Ok(())
}
