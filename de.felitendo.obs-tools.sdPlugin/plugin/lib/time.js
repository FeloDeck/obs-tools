// Times on keys and in texts, all in the same style: "4:05" or "1:02:03".
// The settings page imports this file too, so keep it free of Node imports.

// "5:00" -> 300, "90" -> 90, "1:02:03" -> 3723. 0 when it is not a time.
export function parseDuration(text) {
	const parts = String(text ?? "").trim().split(":");
	if (!parts.length || parts.some((p) => !/^\d+$/.test(p.trim()))) return 0;
	return parts.reduce((total, p) => total * 60 + Number(p), 0);
}

// 300 -> "5:00", 3723 -> "1:02:03"
export function formatDuration(seconds) {
	const s = Math.max(0, Math.round(seconds));
	const h = Math.floor(s / 3600);
	const m = Math.floor((s % 3600) / 60);
	const rest = String(s % 60).padStart(2, "0");
	return h ? `${h}:${String(m).padStart(2, "0")}:${rest}` : `${m}:${rest}`;
}

// A timecode from OBS in whole seconds: "01:02:03.456" -> 3723.
export function timecodeSeconds(timecode) {
	return parseDuration(String(timecode ?? "").split(".")[0]);
}
