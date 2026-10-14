// Volume math that matches the faders in the OBS audio mixer.
// OBS uses a log fader: 0 dB is the top, -96 dB and below is the bottom.
import { num } from "./i18n.js";

const LOG_OFFSET_DB = 6;
const LOG_RANGE_DB = 96;
const LOG_OFFSET_VAL = -Math.log10(LOG_OFFSET_DB);
const LOG_RANGE_VAL = -Math.log10(LOG_RANGE_DB + LOG_OFFSET_DB);

export const MIN_DB = -100;
export const BOOST_DB = 26;

// dB to fader position (0..1).
export function dbToFader(db) {
	if (db === null || db === undefined || !Number.isFinite(db)) return 0;
	if (db >= 0) return 1;
	if (db <= -LOG_RANGE_DB) return 0;
	return (-Math.log10(-db + LOG_OFFSET_DB) - LOG_RANGE_VAL) / (LOG_OFFSET_VAL - LOG_RANGE_VAL);
}

// Fader position (0..1) to dB.
export function faderToDb(position) {
	if (position >= 1) return 0;
	if (position <= 0) return -Infinity;
	return -(LOG_RANGE_DB + LOG_OFFSET_DB) * Math.pow((LOG_RANGE_DB + LOG_OFFSET_DB) / LOG_OFFSET_DB, -position) + LOG_OFFSET_DB;
}

export function mulToDb(mul) {
	return mul > 0 ? 20 * Math.log10(mul) : -Infinity;
}

// OBS sends null for -inf, because JSON has no infinity.
export function readDb(response) {
	if (Number.isFinite(response?.inputVolumeDb)) return response.inputVolumeDb;
	if (typeof response?.inputVolumeMul === "number") return mulToDb(response.inputVolumeMul);
	return -Infinity;
}

// unit "db": step in dB. unit "percent": step in fader percent.
export function stepVolume(db, { unit = "db", step = 1, direction = 1, allowBoost = false }) {
	const max = allowBoost ? BOOST_DB : 0;
	let next;
	if (unit === "percent") {
		if (db > 0) next = direction > 0 ? Math.min(max, db + 1) : 0;
		else next = faderToDb(dbToFader(db) + (direction * step) / 100);
	} else {
		// Below -60 dB nothing is audible, so going up starts there.
		const from = Number.isFinite(db) && db >= -60 ? db : direction > 0 ? -60 - step : MIN_DB;
		next = from + direction * step;
	}
	return clampDb(next, allowBoost);
}

export function clampDb(db, allowBoost = false) {
	const max = allowBoost ? BOOST_DB : 0;
	if (!Number.isFinite(db) || db <= MIN_DB) return db > 0 ? max : -Infinity;
	return Math.min(max, Math.max(MIN_DB, db));
}

// Value for SetInputVolume.
export function volumeRequest(inputName, db) {
	return Number.isFinite(db) ? { inputName, inputVolumeDb: Math.round(db * 100) / 100 } : { inputName, inputVolumeMul: 0 };
}

export function formatVolume(db, unit = "db") {
	if (unit === "percent") return `${Math.round(dbToFader(db) * 100)}%`;
	if (!Number.isFinite(db)) return "-inf dB";
	return `${db > 0 ? "+" : ""}${num(db, 1)} dB`;
}
