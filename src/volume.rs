// Volume math that matches the faders in the OBS audio mixer.
// OBS uses a log fader: 0 dB is the top, -96 dB and below is the bottom.
use serde_json::Value;

use crate::i18n::num;
use crate::js::{fmt, max, min, round};

const LOG_OFFSET_DB: f64 = 6.0;
const LOG_RANGE_DB: f64 = 96.0;

fn log_offset_val() -> f64 {
	-LOG_OFFSET_DB.log10()
}

fn log_range_val() -> f64 {
	-(LOG_RANGE_DB + LOG_OFFSET_DB).log10()
}

pub const MIN_DB: f64 = -100.0;
pub const BOOST_DB: f64 = 26.0;

/// dB to fader position (0..1).
pub fn db_to_fader(db: f64) -> f64 {
	if !db.is_finite() {
		return 0.0;
	}
	if db >= 0.0 {
		return 1.0;
	}
	if db <= -LOG_RANGE_DB {
		return 0.0;
	}
	(-(-db + LOG_OFFSET_DB).log10() - log_range_val()) / (log_offset_val() - log_range_val())
}

/// Fader position (0..1) to dB.
pub fn fader_to_db(position: f64) -> f64 {
	if position >= 1.0 {
		return 0.0;
	}
	if position <= 0.0 {
		return f64::NEG_INFINITY;
	}
	-(LOG_RANGE_DB + LOG_OFFSET_DB) * ((LOG_RANGE_DB + LOG_OFFSET_DB) / LOG_OFFSET_DB).powf(-position) + LOG_OFFSET_DB
}

pub fn mul_to_db(mul: f64) -> f64 {
	if mul > 0.0 { 20.0 * mul.log10() } else { f64::NEG_INFINITY }
}

/// OBS sends null for -inf, because JSON has no infinity.
pub fn read_db(response: &Value) -> f64 {
	if let Some(db) = response.get("inputVolumeDb").and_then(Value::as_f64).filter(|d| d.is_finite()) {
		return db;
	}
	if let Some(mul) = response.get("inputVolumeMul").and_then(Value::as_f64) {
		return mul_to_db(mul);
	}
	f64::NEG_INFINITY
}

/// unit "db": step in dB. unit "percent": step in fader percent.
pub fn step_volume(db: f64, unit: &str, step: f64, direction: f64, allow_boost: bool) -> f64 {
	let top = if allow_boost { BOOST_DB } else { 0.0 };
	let next = if unit == "percent" {
		if db > 0.0 {
			if direction > 0.0 { min(top, db + 1.0) } else { 0.0 }
		} else {
			fader_to_db(db_to_fader(db) + (direction * step) / 100.0)
		}
	} else {
		// Below -60 dB nothing is audible, so going up starts there.
		let from = if db.is_finite() && db >= -60.0 {
			db
		} else if direction > 0.0 {
			-60.0 - step
		} else {
			MIN_DB
		};
		from + direction * step
	};
	clamp_db(next, allow_boost)
}

pub fn clamp_db(db: f64, allow_boost: bool) -> f64 {
	let top = if allow_boost { BOOST_DB } else { 0.0 };
	if !db.is_finite() || db <= MIN_DB {
		return if db > 0.0 { top } else { f64::NEG_INFINITY };
	}
	min(top, max(MIN_DB, db))
}

/// Value for SetInputVolume.
pub fn volume_request(input_name: &str, db: f64) -> Value {
	if db.is_finite() {
		crate::obj! { "inputName" => input_name, "inputVolumeDb" => crate::js::num(round(db * 100.0) / 100.0) }
	} else {
		crate::obj! { "inputName" => input_name, "inputVolumeMul" => 0 }
	}
}

pub fn format_volume(db: f64, unit: &str) -> String {
	if unit == "percent" {
		return format!("{}%", fmt(round(db_to_fader(db) * 100.0)));
	}
	if !db.is_finite() {
		return "-inf dB".into();
	}
	format!("{}{} dB", if db > 0.0 { "+" } else { "" }, num(db, 1))
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	fn near(a: f64, b: f64, eps: f64) {
		assert!((a - b).abs() < eps, "{a} is not close to {b}");
	}

	#[test]
	fn fader_math_round_trips_like_the_obs_mixer() {
		assert_eq!(db_to_fader(0.0), 1.0);
		assert_eq!(db_to_fader(f64::NEG_INFINITY), 0.0);
		assert_eq!(db_to_fader(-100.0), 0.0);
		for db in [-60.0, -30.0, -12.0, -6.0, -1.0] {
			near(fader_to_db(db_to_fader(db)), db, 1e-6);
		}
		assert!(db_to_fader(-6.0) > db_to_fader(-12.0));
	}

	#[test]
	fn volume_steps() {
		assert_eq!(step_volume(-10.0, "db", 2.0, 1.0, false), -8.0);
		assert_eq!(step_volume(-1.0, "db", 2.0, 1.0, false), 0.0, "stops at 0 dB");
		assert_eq!(step_volume(-1.0, "db", 2.0, 1.0, true), 1.0);
		assert_eq!(step_volume(f64::NEG_INFINITY, "db", 2.0, 1.0, false), -60.0, "up from silence starts at -60 dB");
		assert_eq!(step_volume(-99.0, "db", 2.0, -1.0, false), f64::NEG_INFINITY);
		near(db_to_fader(step_volume(-12.0, "percent", 10.0, 1.0, false)), db_to_fader(-12.0) + 0.1, 1e-9);
		assert_eq!(volume_request("Mic", f64::NEG_INFINITY), json!({ "inputName": "Mic", "inputVolumeMul": 0 }));
		assert_eq!(volume_request("Mic", -6.123), json!({ "inputName": "Mic", "inputVolumeDb": -6.12 }));
		assert_eq!(read_db(&json!({ "inputVolumeDb": null, "inputVolumeMul": 0 })), f64::NEG_INFINITY);
		assert_eq!(format_volume(-6.0, "db"), "-6.0 dB");
		assert_eq!(format_volume(f64::NEG_INFINITY, "db"), "-inf dB");
		assert_eq!(format_volume(0.0, "percent"), "100%");
	}
}
