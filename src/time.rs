// Times on keys and in texts, all in the same style: "4:05" or "1:02:03".

/// "5:00" -> 300, "90" -> 90, "1:02:03" -> 3723. 0 when it is not a time.
pub fn parse_duration(text: &str) -> f64 {
	let parts: Vec<&str> = crate::js::trim(text).split(':').collect();
	if parts.iter().any(|p| {
		let p = crate::js::trim(p);
		p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit())
	}) {
		return 0.0;
	}
	parts.iter().fold(0.0, |total, p| total * 60.0 + crate::js::string_to_number(p))
}

/// 300 -> "5:00", 3723 -> "1:02:03"
pub fn format_duration(seconds: f64) -> String {
	let s = crate::js::max(0.0, crate::js::round(seconds));
	let h = (s / 3600.0).floor();
	let m = ((s % 3600.0) / 60.0).floor();
	let rest = format!("{:0>2}", crate::js::fmt(s % 60.0));
	if h > 0.0 { format!("{}:{:0>2}:{rest}", crate::js::fmt(h), crate::js::fmt(m)) } else { format!("{}:{rest}", crate::js::fmt(m)) }
}

/// A timecode from OBS in whole seconds: "01:02:03.456" -> 3723.
pub fn timecode_seconds(timecode: &str) -> f64 {
	parse_duration(timecode.split('.').next().unwrap_or(""))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn timer_texts() {
		assert_eq!(parse_duration("5:00"), 300.0);
		assert_eq!(parse_duration("90"), 90.0);
		assert_eq!(parse_duration("1:02:03"), 3723.0);
		assert_eq!(parse_duration("five"), 0.0);
		assert_eq!(format_duration(300.0), "5:00");
		assert_eq!(format_duration(3723.0), "1:02:03");
		assert_eq!(timecode_seconds("00:04:05.900"), 245.0);
		assert_eq!(format_duration(timecode_seconds("01:02:03.456")), "1:02:03");
	}
}
