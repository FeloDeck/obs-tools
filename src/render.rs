// Key images as SVG data URLs (OpenDeck draws them on a 144x144 canvas).
// Same look as the other FeloDeck plugins: no background, a bold glyph in the middle.
// pi/lib/render.js draws the same pictures for the settings pages, keep the two alike.
use base64::Engine;

use crate::i18n::t;
use crate::icons::{GlyphOpts, colors, escape_xml, glyph, glyph_height};
use crate::js::{fmt, max, min, round};
use crate::obs::Status;

pub const SIZE: f64 = 144.0;
const FONT: &str = "'Inter','Noto Sans','DejaVu Sans','Liberation Sans',sans-serif";
// Like the titles OpenDeck draws (font size 11 is about 22 px on a key), but bold to read it on
// the deck. The same in the Feishin plugin.
const TITLE_FONT: &str = "'Liberation Sans',Arial,sans-serif";
// The glyph takes 24 of 40 units, like viewBox="-8 -8 40 40".
const GLYPH_SIZE: f64 = (SIZE * 24.0) / 40.0;
const GLYPH_OFFSET: f64 = (SIZE - GLYPH_SIZE) / 2.0;
// Space between a glyph (or picture) and its title.
const TITLE_GAP: f64 = 16.0;

pub fn svg_url(svg: &str) -> String {
	format!("data:image/svg+xml;base64,{}", base64::engine::general_purpose::STANDARD.encode(svg))
}

pub fn is_color(value: &str) -> bool {
	value.len() == 7 && value.starts_with('#') && value[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

fn svg(content: &str) -> String {
	let s = fmt(SIZE);
	format!(
		r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{s}" height="{s}" viewBox="0 0 {s} {s}">{content}</svg>"#
	)
}

fn flat(color: Option<&str>) -> String {
	match color {
		Some(c) if !c.is_empty() => format!(r#"<rect width="{0}" height="{0}" fill="{c}"/>"#, fmt(SIZE)),
		_ => String::new(),
	}
}

struct Text<'a> {
	x: f64,
	y: f64,
	size: f64,
	color: &'a str,
	weight: u32,
}

impl Default for Text<'_> {
	fn default() -> Self {
		Text { x: SIZE / 2.0, y: 0.0, size: 0.0, color: colors::WHITE, weight: 700 }
	}
}

fn text(value: &str, o: Text) -> String {
	format!(
		r#"<text x="{}" y="{}" font-family="{FONT}" font-size="{}" font-weight="{}" fill="{}" text-anchor="middle">{}</text>"#,
		fmt(o.x),
		fmt(o.y),
		fmt(o.size),
		o.weight,
		o.color,
		escape_xml(value)
	)
}

/// Length as pi/lib/render.js counts it, in UTF-16 units.
fn js_len(text: &str) -> usize {
	text.encode_utf16().count()
}

/// Font size that keeps a line inside the key. Capital letters are wider.
pub fn fit_size(value: &str, max_size: f64, width: f64) -> f64 {
	let length = max(1.0, js_len(value) as f64);
	let per_char = if value == value.to_uppercase() && value.chars().any(char::is_alphabetic) { 0.68 } else { 0.58 };
	max(12.0, min(max_size, (width / (length * per_char)).floor()))
}

/// Audio level from dB to 0..1, like the meters in OBS (-60 dB to 0 dB).
pub fn meter_level(db: f64) -> f64 {
	if !db.is_finite() {
		return 0.0;
	}
	max(0.0, min(1.0, (db + 60.0) / 60.0))
}

// Vertical level meter at the right edge: green, then yellow from -9 dB, red from -3 dB.
fn meter_svg(level: f64) -> String {
	let (x, y, w, h) = (125.0, 18.0, 9.0, 108.0);
	let fill = round(h * level);
	let stop = |offset: f64, color: &str| format!(r#"<stop offset="{}" stop-color="{color}"/>"#, fmt(offset));
	let mut out = format!(
		r#"<defs><linearGradient id="lvl" x1="0" y1="1" x2="0" y2="0">{}{}{}{}{}{}</linearGradient>"#,
		stop(0.0, colors::GREEN),
		stop(0.85, colors::GREEN),
		stop(0.85, colors::AMBER),
		stop(0.95, colors::AMBER),
		stop(0.95, colors::RED),
		stop(1.0, colors::RED)
	);
	out += &format!(
		r#"<clipPath id="lvlc"><rect x="{}" y="{}" width="{}" height="{}"/></clipPath></defs>"#,
		fmt(x),
		fmt(y + h - fill),
		fmt(w),
		fmt(fill)
	);
	out += &format!(
		r##"<rect x="{}" y="{}" width="{}" height="{}" rx="4.5" fill="#FFFFFF" fill-opacity="0.14"/>"##,
		fmt(x),
		fmt(y),
		fmt(w),
		fmt(h)
	);
	if fill > 0.0 {
		out += &format!(
			r#"<rect x="{}" y="{}" width="{}" height="{}" rx="4.5" fill="url(#lvl)" clip-path="url(#lvlc)"/>"#,
			fmt(x),
			fmt(y),
			fmt(w),
			fmt(h)
		);
	}
	out
}

/// A title the key draws itself, in the font of an OpenDeck title.
#[derive(Debug, Clone, PartialEq)]
pub struct TitleLine {
	pub line: String,
	pub size: f64,
	/// How tall digits and capitals are, for placing the line.
	pub ink: f64,
}

/// 22 px like OpenDeck draws them, smaller for long text, and cut with "…" when even the smallest
/// size is too wide.
pub fn title_line(value: &str, width: f64) -> TitleLine {
	let mut line = crate::js::trim(value).to_string();
	let per_char = 0.56;
	let size = max(15.0, min(22.0, (width / (max(1.0, js_len(&line) as f64) * per_char)).floor()));
	let fits = (width / (size * per_char)).floor();
	if js_len(&line) as f64 > fits {
		// Cut by UTF-16 units, like pi/lib/render.js. Half an emoji becomes U+FFFD there too.
		let units: Vec<u16> = line.encode_utf16().take(max(1.0, fits - 1.0) as usize).collect();
		let start = String::from_utf16_lossy(&units);
		line = format!("{}…", start.trim_end_matches(crate::js::is_js_space));
	}
	TitleLine { line, size, ink: round(size * 0.72) }
}

fn title_text(line: &TitleLine, x: f64, y: f64, stroke: bool) -> String {
	let outline = if stroke { r##" stroke="#000" stroke-width="4" paint-order="stroke""## } else { "" };
	format!(
		r#"<text x="{}" y="{}" font-family="{TITLE_FONT}" font-size="{}" font-weight="700" fill="{}" text-anchor="middle"{outline}>{}</text>"#,
		fmt(x),
		fmt(round(y * 10.0) / 10.0),
		fmt(line.size),
		colors::WHITE,
		escape_xml(&line.line)
	)
}

// Something of the given height with a title above or below it, together in the middle of the key.
// Returns where it starts and where the title stands.
fn with_title_layout(height: f64, line: &TitleLine, above: bool) -> (f64, f64) {
	let top = (SIZE - height - TITLE_GAP - line.ink) / 2.0;
	if above { (top + line.ink + TITLE_GAP, top + line.ink) } else { (top, SIZE - top) }
}

/// What a glyph key shows besides the glyph.
#[derive(Clone)]
pub struct GlyphKey<'a> {
	pub color: &'a str,
	/// 0..1 adds a level meter on the right (the glyph moves a bit to the left).
	pub meter: Option<f64>,
	/// A yellow dot in the corner, for a key that still needs its settings.
	pub setup: bool,
	/// A line of text "above" or "below" the glyph (title_at), close to it, and glyph and title
	/// together in the middle of the key. OpenDeck cannot move its own title, so keys draw it here.
	pub title: &'a str,
	pub title_at: &'a str,
}

impl Default for GlyphKey<'_> {
	fn default() -> Self {
		GlyphKey { color: colors::WHITE, meter: None, setup: false, title: "", title_at: "below" }
	}
}

pub fn glyph_key_svg(name: &str, o: &GlyphKey) -> String {
	let dot = if o.setup { format!(r#"<circle cx="120" cy="24" r="10" fill="{}"/>"#, colors::AMBER) } else { String::new() };
	svg(&(glyph_content(name, o) + &dot))
}

fn glyph_content(name: &str, o: &GlyphKey) -> String {
	let shift = if o.meter.is_none() { 0.0 } else { 9.0 };
	let mut y = GLYPH_OFFSET;
	let mut label = String::new();
	if !o.title.is_empty() {
		let line = title_line(o.title, if o.meter.is_none() { 128.0 } else { 110.0 });
		let height = (glyph_height(name) * GLYPH_SIZE) / 24.0;
		let (top, baseline) = with_title_layout(height, &line, o.title_at == "above");
		y = GLYPH_OFFSET + top - (SIZE - height) / 2.0;
		label = title_text(&line, SIZE / 2.0 - shift, baseline, false);
	}
	let mut content =
		glyph(name, &GlyphOpts { x: GLYPH_OFFSET - shift, y, size: GLYPH_SIZE, color: o.color, ..Default::default() }) + &label;
	if let Some(meter) = o.meter {
		content += &meter_svg(meter);
	}
	content
}

pub fn glyph_key(name: &str, o: &GlyphKey) -> String {
	svg_url(&glyph_key_svg(name, o))
}

/// Big value with a small label above and an optional line or bar below.
#[derive(Clone)]
pub struct StatKey<'a> {
	pub label: &'a str,
	pub value: &'a str,
	pub sub: &'a str,
	pub bar: Option<f64>,
	pub color: &'a str,
	/// A flat color behind everything (used for flashing).
	pub alert: Option<&'a str>,
	/// A small colored dot before the label (used as a status light).
	pub dot: Option<&'a str>,
}

impl Default for StatKey<'_> {
	fn default() -> Self {
		StatKey { label: "", value: "", sub: "", bar: None, color: colors::BLUE, alert: None, dot: None }
	}
}

pub fn stat_key_svg(o: &StatKey) -> String {
	let alert = o.alert.filter(|a| !a.is_empty());
	let dot = o.dot.filter(|d| !d.is_empty());
	let dim = if alert.is_some() { colors::WHITE } else { colors::MUTED };
	let mut content = flat(alert);
	// Longer labels (other languages) get a smaller font.
	let size = min(18.0, fit_size(o.label, 18.0, if dot.is_some() { 108.0 } else { 128.0 }));
	if let Some(dot) = dot {
		let width = js_len(o.label) as f64 * size * 0.59;
		content += &format!(r#"<circle cx="{}" cy="36" r="6" fill="{dot}"/>"#, fmt(SIZE / 2.0 - width / 2.0 - 4.0));
		content += &text(o.label, Text { x: SIZE / 2.0 + 8.0, y: 42.0, size, color: dim, ..Default::default() });
	} else {
		content += &text(o.label, Text { y: 42.0, size, color: dim, ..Default::default() });
	}
	content += &text(o.value, Text { y: 90.0, size: fit_size(o.value, 46.0, 124.0), color: colors::WHITE, ..Default::default() });
	if !o.sub.is_empty() {
		content += &text(o.sub, Text { y: 120.0, size: fit_size(o.sub, 19.0, 124.0), weight: 600, color: dim, ..Default::default() });
	}
	if let Some(bar) = o.bar {
		let pct = max(0.0, min(1.0, bar));
		let y = if o.sub.is_empty() { 110 } else { 126 };
		content += &format!(r##"<rect x="24" y="{y}" width="96" height="8" rx="4" fill="#FFFFFF" fill-opacity="0.18"/>"##);
		if pct > 0.0 {
			content += &format!(
				r#"<rect x="24" y="{y}" width="{}" height="8" rx="4" fill="{}"/>"#,
				fmt(max(8.0, 96.0 * pct)),
				if alert.is_some() { colors::WHITE } else { o.color }
			);
		}
	}
	svg(&content)
}

pub fn stat_key(o: &StatKey) -> String {
	svg_url(&stat_key_svg(o))
}

/// Scene preview with a colored frame for live (program) or preview.
#[derive(Clone)]
pub struct SceneKey<'a> {
	pub image: Option<&'a str>,
	pub border: Option<&'a str>,
	pub glyph_name: &'a str,
	/// "fit" shows the whole scene as a picture, "fill" crops the scene to fill the whole key.
	pub fit: &'a str,
	/// The name, under the glyph or the picture (both together in the middle), or over a picture
	/// that fills the key.
	pub title: &'a str,
}

impl Default for SceneKey<'_> {
	fn default() -> Self {
		SceneKey { image: None, border: None, glyph_name: "scene", fit: "fit", title: "" }
	}
}

pub fn scene_key_svg(o: &SceneKey) -> String {
	let mut content;
	match o.image.filter(|i| !i.is_empty()) {
		None => content = glyph_content(o.glyph_name, &GlyphKey { title: o.title, ..Default::default() }),
		Some(image) => {
			let href = match image.strip_prefix("data:image/jpg;") {
				Some(rest) => format!("data:image/jpeg;{rest}"),
				None => image.to_string(),
			};
			let line = if o.title.is_empty() { None } else { Some(title_line(o.title, 128.0)) };
			if o.fit == "fill" {
				let s = fmt(SIZE);
				content = format!(
					r#"<image x="0" y="0" width="{s}" height="{s}" preserveAspectRatio="xMidYMid slice" href="{href}" xlink:href="{href}"/>"#
				);
				if let Some(line) = &line {
					content += &title_text(line, SIZE / 2.0, SIZE - 16.0, true);
				}
			} else {
				// 120 x 76 fits 16:9 and 16:10 canvases. Top aligned in its box.
				let (top, baseline) = match &line {
					Some(line) => with_title_layout(76.0, line, false),
					None => ((SIZE - 76.0) / 2.0, 0.0),
				};
				let top = fmt(top);
				content = format!(r#"<defs><clipPath id="pic"><rect x="12" y="{top}" width="120" height="76" rx="6"/></clipPath></defs>"#);
				content += &format!(
					r#"<image x="12" y="{top}" width="120" height="76" preserveAspectRatio="xMidYMin meet" clip-path="url(#pic)" href="{href}" xlink:href="{href}"/>"#
				);
				if let Some(line) = &line {
					content += &title_text(line, SIZE / 2.0, baseline, false);
				}
			}
		}
	}
	if let Some(border) = o.border.filter(|b| !b.is_empty()) {
		let side = fmt(SIZE - 8.0);
		content += &format!(r#"<rect x="4" y="4" width="{side}" height="{side}" rx="12" fill="none" stroke="{border}" stroke-width="8"/>"#);
	}
	svg(&content)
}

pub fn scene_key(o: &SceneKey) -> String {
	svg_url(&scene_key_svg(o))
}

/// Why OBS cannot be reached, in a word or two. Keys show it with a glyph, dials as text.
pub fn status_label(status: Status) -> String {
	t(match status {
		Status::Auth => "Password?",
		Status::Error => "OBS error",
		_ => "OBS offline",
	})
}

/// Shown on every key while OBS cannot be reached.
pub fn status_key(status: Status) -> String {
	let auth = matches!(status, Status::Auth | Status::Error);
	let color = if auth { colors::AMBER } else { colors::GRAY };
	let mut content = glyph(if auth { "lock" } else { "power" }, &GlyphOpts { x: 44.0, y: 22.0, size: 56.0, color, ..Default::default() });
	let label = status_label(status);
	content += &text(
		&label,
		Text {
			y: 112.0,
			size: fit_size(&label, 20.0, 132.0),
			color: if auth { colors::AMBER } else { colors::MUTED },
			..Default::default()
		},
	);
	svg_url(&svg(&content))
}

/// The icon in the touch strip of a dial, like images/encoder/<name>.svg.
/// setup: gray with a yellow dot, like the setup image of a key.
pub fn dial_icon(name: &str, setup: bool) -> String {
	let mut content = glyph(name, &GlyphOpts { color: if setup { colors::GRAY } else { colors::WHITE }, ..Default::default() });
	if setup {
		content += &format!(r#"<circle cx="25" cy="-1" r="2.8" fill="{}"/>"#, colors::AMBER);
	}
	svg_url(&format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40" viewBox="-4 -4 32 32">{content}</svg>"#))
}

#[cfg(test)]
mod tests {
	use super::*;

	fn decode(url: &str) -> String {
		let data = url.split(',').nth(1).unwrap_or("");
		String::from_utf8(base64::engine::general_purpose::STANDARD.decode(data).unwrap_or_default()).unwrap_or_default()
	}

	#[test]
	fn svg_data_urls_with_escaped_text() {
		let url = stat_key(&StatKey { label: "A<B", value: "1&2", ..Default::default() });
		assert!(url.starts_with("data:image/svg+xml;base64,"));
		let svg = decode(&url);
		assert!(svg.contains("A&lt;B") && svg.contains("1&amp;2"));
		assert!(fit_size("a very long scene name", 40.0, 124.0) < 40.0);
		let key = decode(&glyph_key("record", &GlyphKey::default()));
		assert!(!key.contains(r#"<rect width="144""#), "keys have no background");
		assert!(key.contains("translate(28.8 28.8)"), "glyph sits in the middle");
		assert!(svg_url("<svg/>").len() > 20);
	}

	#[test]
	fn long_titles_get_smaller_then_cut() {
		assert_eq!(title_line("Mic", 128.0).size, 22.0);
		let long = title_line("Bildschirmaufnahme (PipeWire)", 128.0);
		assert!(long.size < 22.0);
		assert!(long.line.ends_with('…'));
	}

	#[test]
	fn level_meter_covers_minus_60_to_0_db() {
		assert_eq!(meter_level(f64::NEG_INFINITY), 0.0);
		assert_eq!(meter_level(0.0), 1.0);
		assert_eq!(meter_level(-30.0), 0.5);
		assert_eq!(meter_level(-90.0), 0.0);
	}
}
