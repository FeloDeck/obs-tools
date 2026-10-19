// Glyphs drawn on a 24x24 grid. Bold, like the other FeloDeck plugins: a body is always filled
// (details are cut out of it), round 2.4px strokes only for lines like arrows and waves.
// Used for the static icons (`cargo xtask icons`), the settings pages and keys drawn at runtime.
use std::collections::HashMap;
use std::sync::LazyLock;

use crate::glyph_fit::FIT;
use crate::js::{fmt, round};

pub mod colors {
	pub const WHITE: &str = "#FFFFFF";
	pub const GRAY: &str = "#6B6B6B";
	pub const MUTED: &str = "#9A9AA2";
	pub const RED: &str = "#E5484D";
	pub const GREEN: &str = "#30A46C";
	pub const BLUE: &str = "#3E8EF7";
	pub const AMBER: &str = "#F5A524";
}

pub static COLORS: &[(&str, &str)] = &[
	("white", colors::WHITE),
	("gray", colors::GRAY),
	("muted", colors::MUTED),
	("red", colors::RED),
	("green", colors::GREEN),
	("blue", colors::BLUE),
	("amber", colors::AMBER),
];

fn gear(cx: f64, cy: f64, outer: f64, inner: f64, teeth: usize) -> String {
	let mut points = Vec::new();
	let step = (std::f64::consts::PI * 2.0) / (teeth * 4) as f64;
	for i in 0..teeth * 4 {
		let r = if i % 4 < 2 { outer } else { inner };
		let a = i as f64 * step - std::f64::consts::PI / 2.0 + step / 2.0;
		points.push(format!("{} {}", crate::js::to_fixed(cx + r * a.cos(), 2), crate::js::to_fixed(cy + r * a.sin(), 2)));
	}
	format!("M{}Z", points.join("L"))
}

const FILL: &str = r#"fill="currentColor""#;

// A filled body with details cut out of it. Cuts are lines in the stroke width, or shapes with
// fill="#000"; add puts parts back on top of a cut.
fn cut(id: &str, body: &str, cuts: &str, add: &str) -> String {
	format!(
		r##"<mask id="cut-{id}"><g fill="#FFF" stroke="none">{body}</g><g fill="none" stroke="#000">{cuts}</g><g fill="none" stroke="#FFF">{add}</g></mask><rect x="-4" y="-4" width="32" height="32" {FILL} stroke="none" mask="url(#cut-{id})"/>"##
	)
}

fn build() -> Vec<(&'static str, String)> {
	let fill = FILL;
	let speaker = format!(r#"<path d="M3.5 9.2h3.2L11.5 5v14l-4.8-4.2H3.5a1 1 0 0 1-1-1v-3.6a1 1 0 0 1 1-1z" {fill} stroke-width="1.6"/>"#);
	let s = |text: &str| text.to_string();
	vec![
		(
			"plugin",
			cut(
				"plugin",
				&format!(r#"<path d="{}"/>"#, gear(12.0, 12.0, 10.6, 8.2, 8)),
				r##"<circle cx="12" cy="12" r="3.4" fill="#000" stroke="none"/>"##,
				"",
			),
		),
		(
			"scene",
			format!(
				r#"<rect x="1" y="7.5" width="17" height="13" rx="3" {fill} stroke="none"/><path d="M7 4h12.5a2.5 2.5 0 0 1 2.5 2.5v9"/>"#
			),
		),
		(
			"animation",
			format!(r#"<rect x="10.5" y="5.5" width="12" height="13" rx="2.6" {fill} stroke="none"/><path d="M2 8h5M3.5 12h4M2 16h5"/>"#),
		),
		(
			"studio",
			format!(
				r#"<rect x="1" y="4.5" width="10" height="13.5" rx="2.2" {fill} stroke="none"/><rect x="13" y="4.5" width="10" height="13.5" rx="2.2" {fill} stroke="none"/><path d="M6 21.5h12"/>"#
			),
		),
		(
			"video",
			cut(
				"video",
				r#"<rect x="1" y="3" width="22" height="18" rx="3.4"/>"#,
				r##"<path d="M10 8.6 15.5 12 10 15.4z" fill="#000" stroke-width="1.6"/>"##,
				"",
			),
		),
		(
			"videoStop",
			cut(
				"videoStop",
				r#"<rect x="1" y="3" width="22" height="18" rx="3.4"/>"#,
				r##"<rect x="8.5" y="8.5" width="7" height="7" rx="1.2" fill="#000" stroke="none"/>"##,
				"",
			),
		),
		("volume", format!(r#"{speaker}<path d="M15.2 8.8a4.6 4.6 0 0 1 0 6.4M18.3 5.8a9 9 0 0 1 0 12.4"/>"#)),
		("volumeUp", format!(r#"{speaker}<path d="M15.5 12h6M18.5 9v6"/>"#)),
		("volumeDown", format!(r#"{speaker}<path d="M15.5 12h6"/>"#)),
		("volumeSet", format!(r#"{speaker}<path d="M15.5 10h6M15.5 14h6"/>"#)),
		("mute", format!(r#"{speaker}<path d="m15.5 9.2 5.6 5.6M21.1 9.2l-5.6 5.6"/>"#)),
		("unmuted", format!(r#"{speaker}<path d="M15.2 8.8a4.6 4.6 0 0 1 0 6.4M18.3 5.8a9 9 0 0 1 0 12.4"/>"#)),
		(
			"monitor",
			format!(
				r#"<path d="M3.5 17v-5a8.5 8.5 0 0 1 17 0v5"/><path d="M20.5 14v4.5a2 2 0 0 1-2 2H17v-6.5zM3.5 14v4.5a2 2 0 0 0 2 2H7v-6.5z" {fill}/>"#
			),
		),
		(
			"dropped",
			cut(
				"dropped",
				r#"<rect x="1" y="4" width="22" height="16" rx="3"/>"#,
				r#"<path d="M6.5 4v16M17.5 4v16M12 8.4v4.2M12 15.6v.1"/>"#,
				"",
			),
		),
		(
			"cpu",
			cut(
				"cpu",
				r#"<rect x="4.8" y="4.8" width="14.4" height="14.4" rx="2.6"/>"#,
				r#"<rect x="9.6" y="9.6" width="4.8" height="4.8" rx=".8"/>"#,
				"",
			) + r#"<path d="M9 2v2.5M15 2v2.5M9 19.5V22M15 19.5V22M2 9h2.5M2 15h2.5M19.5 9H22M19.5 15H22"/>"#,
		),
		(
			"previous",
			format!(r#"<path d="M19 6 9.5 12l9.5 6z" {fill}/><rect x="4.5" y="5" width="3.2" height="14" rx="1.2" {fill} stroke="none"/>"#),
		),
		(
			"next",
			format!(r#"<path d="M5 6l9.5 6L5 18z" {fill}/><rect x="16.3" y="5" width="3.2" height="14" rx="1.2" {fill} stroke="none"/>"#),
		),
		("back", format!(r#"<path d="M9.5 9H15a5 5 0 0 1 0 10H8"/><path d="M10.5 4.5 4 9l6.5 4.5z" {fill} stroke-width="1.6"/>"#)),
		(
			"replay",
			format!(
				r#"<path d="M3.5 12a8.5 8.5 0 1 0 2.5-6l-2.5 2.5"/><path d="M3.5 3.5v5h5"/><path d="m10.5 9 4.5 3-4.5 3z" {fill} stroke-width="1.6"/>"#
			),
		),
		(
			"browser",
			cut(
				"browser",
				r#"<circle cx="12" cy="12" r="10.4"/>"#,
				r#"<path d="M1.5 12h21M12 1.6c2.6 2.6 3.9 6 3.9 10.4s-1.3 7.8-3.9 10.4c-2.6-2.6-3.9-6-3.9-10.4s1.3-7.8 3.9-10.4z"/>"#,
				"",
			),
		),
		("record", format!(r#"<circle cx="12" cy="12" r="9.5" {fill} stroke="none"/>"#)),
		("pause", cut("pause", r#"<circle cx="12" cy="12" r="10.2"/>"#, r#"<path d="M9.6 8.6v6.8M14.4 8.6v6.8"/>"#, "")),
		(
			"stream",
			format!(
				r#"<circle cx="12" cy="12" r="2.3" {fill} stroke="none"/><path d="M16.2 7.8a6 6 0 0 1 0 8.4M7.8 16.2a6 6 0 0 1 0-8.4M19.1 4.9a10 10 0 0 1 0 14.2M4.9 19.1a10 10 0 0 1 0-14.2"/>"#
			),
		),
		(
			"transition",
			s(
				r#"<path d="M2.5 7h3.2c2.6 0 3.9 1.6 5.3 5s2.7 5 5.3 5h4.2M2.5 17h3.2c1.4 0 2.4-.5 3.2-1.3M13.1 8.3c.8-.8 1.8-1.3 3.1-1.3h4.3M18 4l3 3-3 3M18 14l3 3-3 3"/>"#,
			),
		),
		("filter", format!(r#"<path d="M3 4.5h18l-7 8.2v6.3l-4 2v-8.3z" {fill} stroke-width="1.8"/>"#)),
		(
			"virtualcam",
			format!(
				r#"<rect x="2" y="6" width="13.5" height="12" rx="2.5" {fill} stroke="none"/><path d="m17 10.3 5-3v9.4l-5-3z" {fill} stroke-width="1.6"/>"#
			),
		),
		(
			"window",
			cut(
				"window",
				r#"<rect x="1" y="3" width="22" height="18" rx="3.4"/>"#,
				r##"<path d="M1 8.5h22M12 11.4v.6M12 17v.6M8.8 14.5h.6M14.6 14.5h.6"/><circle cx="12" cy="14.5" r="1.4" fill="#000" stroke="none"/>"##,
				"",
			),
		),
		(
			"dial",
			cut("dial", r#"<circle cx="12" cy="13" r="8.7"/>"#, r#"<path d="m12 13 3.2-3.8"/>"#, "")
				+ r#"<path d="M3.5 6.2 5 7.7M20.5 6.2 19 7.7M12 1.5v1.6"/>"#,
		),
		("power", s(r#"<path d="M12 3v9"/><path d="M18.4 6.6a9 9 0 1 1-12.8 0"/>"#)),
		("lock", format!(r#"<rect x="5" y="11" width="14" height="10" rx="2" {fill} stroke="none"/><path d="M8 11V7a4 4 0 0 1 8 0v4"/>"#)),
		(
			"twitch",
			cut(
				"twitch",
				r##"<path d="M4 2 2.4 6.6V20H7v3h3.4l3-3h4.2l4.8-4.8V2z" stroke="#FFF" stroke-width="1.4" stroke-linejoin="round"/>"##,
				r#"<path d="M11 7.6v4.2M16 7.6v4.2"/>"#,
				"",
			),
		),
		(
			"eye",
			cut(
				"eye",
				r##"<path d="M.6 12S5 4 12 4s11.4 8 11.4 8S19 20 12 20 .6 12 .6 12z" stroke="#FFF" stroke-width="1.2" stroke-linejoin="round"/>"##,
				r#"<circle cx="12" cy="12" r="3.9"/>"#,
				"",
			),
		),
		(
			"eyeOff",
			cut(
				"eyeOff",
				r##"<path d="M.6 12S5 4 12 4s11.4 8 11.4 8S19 20 12 20 .6 12 .6 12z" stroke="#FFF" stroke-width="1.2" stroke-linejoin="round"/>"##,
				r#"<path d="m3 3 18 18" stroke-width="6"/>"#,
				r#"<path d="m3 3 18 18"/>"#,
			),
		),
		(
			"keyboard",
			cut(
				"keyboard",
				r#"<rect x="1" y="5" width="22" height="15" rx="3.2"/>"#,
				r#"<path d="M5.8 9.6h.01M9.3 9.6h.01M12.8 9.6h.01M16.3 9.6h.01M7 15h10"/>"#,
				"",
			),
		),
		(
			"camera",
			cut(
				"camera",
				r##"<path d="M3 8.6a2 2 0 0 1 2-2h2.2L8.8 4h6.4l1.6 2.6H19a2 2 0 0 1 2 2v9.4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" stroke="#FFF" stroke-width="2.4" stroke-linejoin="round"/>"##,
				r#"<circle cx="12" cy="13.2" r="4.4"/>"#,
				"",
			),
		),
		("chapter", format!(r#"<path d="M6.5 3.5h11v17L12 16.6l-5.5 3.9z" {fill} stroke-width="1.6"/>"#)),
		("split", s(r#"<circle cx="6" cy="6.5" r="3"/><circle cx="6" cy="17.5" r="3"/><path d="M8.6 8 20 18.5M8.6 16 20 5.5"/>"#)),
		("play", format!(r#"<path d="M7 4.6 19.2 12 7 19.4z" {fill} stroke-width="1.6"/>"#)),
		(
			"pauseMedia",
			format!(
				r#"<rect x="5.5" y="4.5" width="4.6" height="15" rx="1.4" {fill} stroke="none"/><rect x="13.9" y="4.5" width="4.6" height="15" rx="1.4" {fill} stroke="none"/>"#
			),
		),
		(
			"playPause",
			format!(
				r#"<path d="M3 5.5 12 12l-9 6.5z" {fill} stroke-width="1.6"/><rect x="14" y="5.5" width="3" height="13" rx="1" {fill} stroke="none"/><rect x="19" y="5.5" width="3" height="13" rx="1" {fill} stroke="none"/>"#
			),
		),
		("stop", format!(r#"<rect x="5.5" y="5.5" width="13" height="13" rx="2.2" {fill} stroke="none"/>"#)),
		("restart", s(r#"<path d="M20 12a8 8 0 1 1-2.4-5.7L20 8.5"/><path d="M20 3.5v5h-5"/>"#)),
		(
			"prevTrack",
			format!(r#"<path d="M19 6 9.5 12l9.5 6z" {fill}/><rect x="4.5" y="5" width="3.2" height="14" rx="1.2" {fill} stroke="none"/>"#),
		),
		(
			"nextTrack",
			format!(r#"<path d="M5 6l9.5 6L5 18z" {fill}/><rect x="16.3" y="5" width="3.2" height="14" rx="1.2" {fill} stroke="none"/>"#),
		),
		("forward", format!(r#"<path d="M3 6.5 11 12l-8 5.5zM12.5 6.5l8 5.5-8 5.5z" {fill} stroke-width="1.6"/>"#)),
		("rewind", format!(r#"<path d="M21 6.5 13 12l8 5.5zM11.5 6.5l-8 5.5 8 5.5z" {fill} stroke-width="1.6"/>"#)),
		(
			"profile",
			format!(r#"<circle cx="12" cy="7.5" r="4.6" {fill} stroke="none"/><path d="M3.6 20.4a8.4 8.4 0 0 1 16.8 0z" {fill}/>"#),
		),
		(
			"collection",
			format!(
				r#"<path d="M12 3 2.5 8 12 13l9.5-5z" {fill} stroke-width="1.6"/><path d="m2.5 12.4 9.5 5 9.5-5M2.5 16.4l9.5 5 9.5-5"/>"#
			),
		),
		(
			"image",
			cut(
				"image",
				r#"<rect x="1.5" y="3" width="21" height="18" rx="3.4"/>"#,
				r##"<circle cx="8" cy="8.8" r="2.1" fill="#000" stroke="none"/><path d="m5.5 17 4-4.6 2.9 3 2.4-2.4 3.7 4z" fill="#000" stroke-width="1.6"/>"##,
				"",
			),
		),
		("text", s(r#"<path d="M5 7V4.5h14V7M12 4.5v15M8.5 19.5h7"/>"#)),
		(
			"health",
			cut(
				"health",
				r##"<path d="M2.4 18a9.6 9.6 0 1 1 19.2 0z" stroke="#FFF" stroke-width="2.4" stroke-linejoin="round"/>"##,
				r##"<path d="m12 15.6 3.6-4.4"/><circle cx="12" cy="16" r="2.2" fill="#000" stroke="none"/>"##,
				"",
			),
		),
	]
}

/// Every glyph by name, in the order they are written above.
pub static GLYPHS: LazyLock<Vec<(&'static str, String)>> = LazyLock::new(build);

static INDEX: LazyLock<HashMap<&'static str, usize>> =
	LazyLock::new(|| GLYPHS.iter().enumerate().map(|(i, (name, _))| (*name, i)).collect());

pub fn glyph_body(name: &str) -> Option<&'static str> {
	INDEX.get(name).map(|&i| GLYPHS[i].1.as_str())
}

pub fn escape_xml(text: &str) -> String {
	let mut out = String::with_capacity(text.len());
	for c in text.chars() {
		match c {
			'<' => out.push_str("&lt;"),
			'>' => out.push_str("&gt;"),
			'&' => out.push_str("&amp;"),
			'"' => out.push_str("&quot;"),
			'\'' => out.push_str("&apos;"),
			_ => out.push(c),
		}
	}
	out
}

/// The glyph of a volume key for each mode. Mute shows the plain speaker.
pub fn volume_glyph(mode: &str) -> Option<&'static str> {
	VOLUME_GLYPHS.iter().find(|(m, _)| *m == mode).map(|(_, g)| *g)
}

pub static VOLUME_GLYPHS: &[(&str, &str)] = &[("up", "volumeUp"), ("down", "volumeDown"), ("set", "volumeSet")];

/// The glyph of a media key for each operation. Play / pause shows pause while it plays.
pub fn media_glyph(op: &str) -> Option<&'static str> {
	MEDIA_GLYPHS.iter().find(|(m, _)| *m == op).map(|(_, g)| *g)
}

pub static MEDIA_GLYPHS: &[(&str, &str)] = &[
	("play", "play"),
	("pause", "pauseMedia"),
	("stop", "stop"),
	("restart", "restart"),
	("next", "nextTrack"),
	("previous", "prevTrack"),
	("forward", "forward"),
	("back", "rewind"),
];

/// Center and size of a glyph as measured, in grid units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
	pub cx: f64,
	pub cy: f64,
	pub w: f64,
	pub h: f64,
}

pub fn fit_of(name: &str) -> Option<Fit> {
	FIT.iter().find(|(n, _)| *n == name).map(|(_, f)| *f)
}

// How much of the 24 grid a glyph may use: wide ones up to MAX, compact ones less,
// so a circle does not look bigger than a wide eye.
const MAX: f64 = 20.5;
const AREA: f64 = 17.5;

fn fit_scale(b: &Fit) -> f64 {
	crate::js::min(MAX / crate::js::max(b.w, b.h), AREA / (b.w * b.h).sqrt())
}

/// How tall a glyph is drawn, in units of the 24 grid.
pub fn glyph_height(name: &str) -> f64 {
	fit_of(name).map_or(24.0, |b| b.h * fit_scale(&b))
}

fn n3(v: f64) -> f64 {
	round(v * 1000.0) / 1000.0
}

// The fitted box of a glyph: centered on (12, 12) and with an even optical size.
fn fit_transform(name: &str) -> (String, f64) {
	let Some(b) = fit_of(name) else { return (String::new(), 1.0) };
	let k = fit_scale(&b);
	(format!("translate(12 12) scale({}) translate({} {})", fmt(n3(k)), fmt(n3(-b.cx)), fmt(n3(-b.cy))), k)
}

pub struct GlyphOpts<'a> {
	pub x: f64,
	pub y: f64,
	pub size: f64,
	pub color: &'a str,
	pub stroke: f64,
	/// false draws it as written, used to measure it.
	pub fit: bool,
}

impl Default for GlyphOpts<'_> {
	fn default() -> Self {
		GlyphOpts { x: 0.0, y: 0.0, size: 24.0, color: colors::WHITE, stroke: 2.4, fit: true }
	}
}

/// A glyph placed at (x, y) with the given size in pixels.
pub fn glyph(name: &str, o: &GlyphOpts) -> String {
	let body = glyph_body(name).or_else(|| glyph_body("plugin")).unwrap_or("");
	let (inner, scale) = if o.fit { fit_transform(name) } else { (String::new(), 1.0) };
	// Keep the stroke the same on screen, whatever the glyph was scaled by.
	let width = n3(o.stroke / scale);
	let color = o.color;
	let outer = format!(
		r#"<g transform="translate({} {}) scale({})" fill="none" stroke="{color}" color="{color}" stroke-width="{}" stroke-linecap="round" stroke-linejoin="round">"#,
		fmt(n3(o.x)),
		fmt(n3(o.y)),
		fmt(n3(o.size / 24.0)),
		fmt(width)
	);
	if inner.is_empty() { format!("{outer}{body}</g>") } else { format!(r#"{outer}<g transform="{inner}">{body}</g></g>"#) }
}

/// Icon for the action list: white glyph, no background.
pub fn action_icon_svg(name: &str, view_box: &str) -> String {
	format!(
		r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40" viewBox="{view_box}">{}</svg>"#,
		glyph(name, &GlyphOpts::default())
	) + "\n"
}
