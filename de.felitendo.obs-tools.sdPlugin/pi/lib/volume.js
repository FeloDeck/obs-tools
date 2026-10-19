// Volume for the settings pages, like the faders in the OBS audio mixer. Keep this like src/volume.rs.
import { num } from "./i18n.js";

const LOG_OFFSET_DB = 6;
const LOG_RANGE_DB = 96;
const LOG_OFFSET_VAL = -Math.log10(LOG_OFFSET_DB);
const LOG_RANGE_VAL = -Math.log10(LOG_RANGE_DB + LOG_OFFSET_DB);

// dB to fader position (0..1).
export function dbToFader(db) {
	if (db === null || db === undefined || !Number.isFinite(db)) return 0;
	if (db >= 0) return 1;
	if (db <= -LOG_RANGE_DB) return 0;
	return (-Math.log10(-db + LOG_OFFSET_DB) - LOG_RANGE_VAL) / (LOG_OFFSET_VAL - LOG_RANGE_VAL);
}

export function formatVolume(db, unit = "db") {
	if (unit === "percent") return `${Math.round(dbToFader(db) * 100)}%`;
	if (!Number.isFinite(db)) return "-inf dB";
	return `${db > 0 ? "+" : ""}${num(db, 1)} dB`;
}
