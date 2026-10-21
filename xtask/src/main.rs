// Tools to build, package and test OBS Tools. Run `cargo xtask` for the list.
mod e2e;
mod icons;
mod manifest;
mod obs_setup;
mod package;
mod profile;

use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "cargo xtask <command>

  manifest              write manifest.json, de.json and pi/lib/shared.js
  fit-glyphs            measure the glyphs (needs ImageMagick), writes src/glyph_fit.rs
  icons                 write the SVG icons in images/ (run fit-glyphs first)
  build [target...]     build the plugin into bin/<target>/ (default: this PC)
  package               build for Linux (and macOS on a Mac) and write dist/*.streamDeckPlugin
  profile <layout.json> [name]
                        write an OpenDeck profile for the virtual deck (stop OpenDeck first)
  setup-obs             create the scenes and sources the end-to-end tests expect
  make-video            make the test video and pictures (needs ffmpeg and ImageMagick)
  e2e <main|new|more|extra> [filter]
                        end-to-end test through OpenDeck, the virtual deck and OBS";

/// The folder of the repository.
pub fn root() -> PathBuf {
	PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().map(PathBuf::from).unwrap_or_default()
}

pub fn plugin_dir() -> PathBuf {
	root().join("de.felitendo.obs-tools.sdPlugin")
}

pub type Result<T = ()> = std::result::Result<T, String>;

/// Writes a file, with the folders it needs.
pub fn write(path: &std::path::Path, content: impl AsRef<[u8]>) -> Result {
	if let Some(dir) = path.parent() {
		std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
	}
	std::fs::write(path, content).map_err(|e| format!("{}: {e}", path.display()))
}

fn main() -> ExitCode {
	let args: Vec<String> = std::env::args().skip(1).collect();
	let rest = args.get(1..).unwrap_or(&[]);
	let result = match args.first().map(String::as_str) {
		Some("manifest") => manifest::run(),
		Some("fit-glyphs") => icons::fit_glyphs(),
		Some("icons") => icons::run(),
		Some("build") => package::build(rest),
		Some("package") => package::run(),
		Some("profile") => profile::run(rest),
		Some("setup-obs") => obs_setup::setup(),
		Some("make-video") => obs_setup::make_video(),
		Some("e2e") => e2e::run(rest),
		_ => Err(USAGE.to_string()),
	};
	match result {
		Ok(()) => ExitCode::SUCCESS,
		Err(error) => {
			eprintln!("{error}");
			ExitCode::FAILURE
		}
	}
}
