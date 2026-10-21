// Builds the plugin binaries and the plugin file for the plugin store.
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use zip::write::SimpleFileOptions;

use crate::manifest::{TARGETS, binary_path};
use crate::{Result, plugin_dir, root};

/// The oldest glibc the release runs with (Debian 10, Ubuntu 18.10). cargo-zigbuild links against it.
const GLIBC: &str = "2.28";

fn target_dir() -> PathBuf {
	std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from).unwrap_or_else(|| root().join("target"))
}

fn exec(command: &mut Command) -> Result {
	let status = command.status().map_err(|e| format!("{command:?}: {e}"))?;
	if status.success() { Ok(()) } else { Err(format!("{command:?} failed")) }
}

fn host() -> Result<String> {
	let output = Command::new("rustc").arg("-vV").output().map_err(|e| format!("rustc: {e}"))?;
	String::from_utf8_lossy(&output.stdout)
		.lines()
		.find_map(|l| l.strip_prefix("host: ").map(String::from))
		.ok_or_else(|| "rustc did not say its host".into())
}

fn install_binary(built: &Path, target: &str) -> Result {
	let to = plugin_dir().join(binary_path(target));
	if let Some(dir) = to.parent() {
		fs::create_dir_all(dir).map_err(|e| e.to_string())?;
	}
	fs::copy(built, &to).map_err(|e| format!("{}: {e}", built.display()))?;
	println!("{}", to.strip_prefix(root()).unwrap_or(&to).display());
	Ok(())
}

fn is_macos(target: &str) -> bool {
	target.ends_with("-apple-darwin")
}

/// Builds for this PC, or for the given targets: Linux with cargo-zigbuild (an old glibc, so it
/// runs on older distributions too), macOS with cargo (it needs the Apple SDK, so only on a Mac).
pub fn build(targets: &[String]) -> Result {
	if targets.is_empty() {
		exec(Command::new("cargo").args(["build", "--release", "--package", "obs-tools"]).current_dir(root()))?;
		let host = host()?;
		return install_binary(&target_dir().join("release").join("obs-tools"), &host);
	}
	for target in targets {
		if is_macos(target) {
			exec(Command::new("cargo").args(["build", "--release", "--package", "obs-tools", "--target", target]).current_dir(root()))
				.map_err(|e| format!("{e}. macOS targets only build on a Mac, with the Rust library of the target (rustup target add)."))?;
			install_binary(&target_dir().join(target).join("release").join("obs-tools"), target)?;
			continue;
		}
		let zigbuild = Command::new("cargo")
			.args(["zigbuild", "--release", "--package", "obs-tools", "--target", &format!("{target}.{GLIBC}")])
			.current_dir(root())
			.status()
			.map_err(|e| format!("cargo zigbuild: {e}"))?;
		if !zigbuild.success() {
			return Err("cargo zigbuild failed. It needs zig and cargo-zigbuild (cargo install --locked cargo-zigbuild), and the Rust library of the target (rustup target add, or the rust-aarch64-gnu package on Arch).".into());
		}
		install_binary(&target_dir().join(target).join("release").join("obs-tools"), target)?;
	}
	Ok(())
}

fn add_dir(zip: &mut zip::ZipWriter<fs::File>, dir: &Path, base: &Path) -> Result {
	let mut entries: Vec<PathBuf> = fs::read_dir(dir).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).collect();
	entries.sort();
	for path in entries {
		let name = path.strip_prefix(base).map_err(|e| e.to_string())?.to_string_lossy().replace('\\', "/");
		if path.is_dir() {
			zip.add_directory(format!("{name}/"), SimpleFileOptions::default().unix_permissions(0o755)).map_err(|e| e.to_string())?;
			add_dir(zip, &path, base)?;
		} else {
			let executable = name.contains("/bin/");
			let options = SimpleFileOptions::default()
				.compression_method(zip::CompressionMethod::Deflated)
				.unix_permissions(if executable { 0o755 } else { 0o644 });
			zip.start_file(name, options).map_err(|e| e.to_string())?;
			zip.write_all(&fs::read(&path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
		}
	}
	Ok(())
}

/// dist/de.felitendo.obs-tools.streamDeckPlugin, with binaries for Linux and macOS. The macOS ones
/// only build on a Mac, so elsewhere the file only has the Linux ones.
pub fn run() -> Result {
	let _ = fs::remove_dir_all(plugin_dir().join("bin"));
	let (targets, skipped): (Vec<String>, Vec<String>) =
		TARGETS.iter().map(|t| t.to_string()).partition(|t| cfg!(target_os = "macos") || !is_macos(t));
	build(&targets)?;
	let dist = root().join("dist");
	let _ = fs::remove_dir_all(&dist);
	fs::create_dir_all(&dist).map_err(|e| e.to_string())?;
	let file = dist.join("de.felitendo.obs-tools.streamDeckPlugin");
	let mut zip = zip::ZipWriter::new(fs::File::create(&file).map_err(|e| e.to_string())?);
	let dir = plugin_dir();
	let base = dir.parent().unwrap_or(&dir).to_path_buf();
	let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
	zip.add_directory(format!("{name}/"), SimpleFileOptions::default().unix_permissions(0o755)).map_err(|e| e.to_string())?;
	add_dir(&mut zip, &dir, &base)?;
	zip.finish().map_err(|e| e.to_string())?;
	let version = fs::read_to_string(root().join("VERSION")).unwrap_or_default();
	println!("dist/de.felitendo.obs-tools.streamDeckPlugin ({})", version.trim());
	if !skipped.is_empty() {
		println!("Left out {}: they only build on a Mac.", skipped.join(", "));
	}
	Ok(())
}
