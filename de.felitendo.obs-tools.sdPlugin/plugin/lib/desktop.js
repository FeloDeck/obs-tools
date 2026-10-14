// Small desktop helpers: a native file picker and the focused X11 window.
import { execFile } from "node:child_process";
import os from "node:os";
import path from "node:path";

function run(command, args, { timeout = 0 } = {}) {
	return new Promise((resolve) => {
		execFile(command, args, { timeout, maxBuffer: 1024 * 1024 }, (error, stdout, stderr) => {
			resolve({ ok: !error, code: error?.code, stdout: String(stdout ?? ""), stderr: String(stderr ?? "") });
		});
	});
}

const IMAGE_PATTERNS = ["*.png", "*.jpg", "*.jpeg", "*.gif", "*.webp", "*.bmp", "*.tga", "*.psd"];
const VIDEO_PATTERNS = ["*.mp4", "*.mkv", "*.mov", "*.webm", "*.flv", "*.avi", "*.ts", "*.m4v", "*.gif", "*.mp3", "*.wav", "*.ogg", "*.flac"];

// Opens a file dialog. Resolves with the path, or null when the user cancels.
export async function pickFile({ title = "Choose a file", start = "", images = false, patterns = images ? IMAGE_PATTERNS : VIDEO_PATTERNS } = {}) {
	const dir = start ? path.dirname(start) : path.join(os.homedir(), images ? "Pictures" : "Videos");
	const kind = images ? "Pictures" : "Media files";
	const kdialog = await run("kdialog", ["--title", title, "--getopenfilename", start || dir, `${kind} (${patterns.join(" ")})|All files (*)`]);
	if (kdialog.ok) return kdialog.stdout.trim() || null;
	if (kdialog.code !== "ENOENT") return null;
	for (const tool of ["zenity", "qarma"]) {
		const result = await run(tool, ["--file-selection", `--title=${title}`, `--filename=${start || dir + "/"}`, `--file-filter=${kind} | ${patterns.join(" ")}`, "--file-filter=All files | *"]);
		if (result.ok) return result.stdout.trim() || null;
		if (result.code !== "ENOENT") return null;
	}
	throw new Error("No file dialog found. Install kdialog or zenity, or type the path.");
}

// Opens a folder dialog. Resolves with the path, or null when the user cancels.
export async function pickFolder({ title = "Choose a folder", start = "" } = {}) {
	const dir = start || os.homedir();
	const kdialog = await run("kdialog", ["--title", title, "--getexistingdirectory", dir]);
	if (kdialog.ok) return kdialog.stdout.trim() || null;
	if (kdialog.code !== "ENOENT") return null;
	for (const tool of ["zenity", "qarma"]) {
		const result = await run(tool, ["--file-selection", "--directory", `--title=${title}`, `--filename=${dir}/`]);
		if (result.ok) return result.stdout.trim() || null;
		if (result.code !== "ENOENT") return null;
	}
	throw new Error("No folder dialog found. Install kdialog or zenity, or type the path.");
}

export function sessionType() {
	if (process.env.XDG_SESSION_TYPE) return process.env.XDG_SESSION_TYPE;
	if (process.env.WAYLAND_DISPLAY) return "wayland";
	if (process.env.DISPLAY) return "x11";
	return "unknown";
}

function parseXprop(text, name) {
	const line = text.split("\n").find((l) => l.startsWith(name));
	if (!line) return null;
	return line.slice(line.indexOf("=") + 1).trim();
}

// The focused X11 window as { id, name, cls }, or null.
export async function activeX11Window() {
	const root = await run("xprop", ["-root", "_NET_ACTIVE_WINDOW"], { timeout: 3000 });
	if (!root.ok) {
		if (root.code === "ENOENT") throw new Error("xprop is missing (package xorg-xprop)");
		return null;
	}
	const match = root.stdout.match(/window id # (0x[0-9a-f]+)/i);
	if (!match || match[1] === "0x0") return null;
	const id = parseInt(match[1], 16);
	const info = await run("xprop", ["-id", match[1], "_NET_WM_NAME", "WM_NAME", "WM_CLASS"], { timeout: 3000 });
	const name = (parseXprop(info.stdout, "_NET_WM_NAME") || parseXprop(info.stdout, "WM_NAME") || "").replace(/^"|"$/g, "");
	const classes = (parseXprop(info.stdout, "WM_CLASS") || "").match(/"([^"]*)"/g)?.map((s) => s.slice(1, -1)) ?? [];
	// OBS uses the first WM_CLASS string.
	return { id, name, cls: classes[0] || "unknown" };
}

// The value OBS uses for "capture_window" in an xcomposite window capture.
export function xcompositeWindowValue({ id, name, cls }) {
	return `${id}\r\n${name}\r\n${cls}`;
}
