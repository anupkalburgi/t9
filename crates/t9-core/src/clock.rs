//! How times are displayed: the machine's local time zone, a 12-hour clock, and the zone's abbreviation,
//! e.g. `8:11 PM EST`. Stored timestamps stay UTC (RFC 3339); only display goes through here.

use std::ffi::CStr;

use chrono::{DateTime, Local};

use crate::model::Timestamp;

/// The local zone's abbreviation at `t` (`EST`, `EDT`, `IST`, …), so daylight-saving changes are right for
/// each date. Falls back to the UTC offset (`UTC-05:00`) if the system has no abbreviation.
pub fn zone(t: &Timestamp) -> String {
    let secs = t.timestamp() as libc::time_t;
    // SAFETY: localtime_r writes only into `tm`, which we own; tm_zone points to static zone data.
    let name = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&secs, &mut tm).is_null() || tm.tm_zone.is_null() {
            None
        } else {
            Some(CStr::from_ptr(tm.tm_zone).to_string_lossy().into_owned())
        }
    };
    name.filter(|n| !n.is_empty()).unwrap_or_else(|| format!("UTC{}", t.with_timezone(&Local).format("%:z")))
}

fn local(t: &Timestamp) -> DateTime<Local> {
    t.with_timezone(&Local)
}

/// `8:11 PM EST`
pub fn time(t: &Timestamp) -> String {
    format!("{} {}", local(t).format("%-I:%M %p"), zone(t))
}

/// `8:11:56 PM EST`
pub fn time_secs(t: &Timestamp) -> String {
    format!("{} {}", local(t).format("%-I:%M:%S %p"), zone(t))
}

/// `Wed 8:11 PM EST`
pub fn weekday_time(t: &Timestamp) -> String {
    format!("{} {}", local(t).format("%a %-I:%M %p"), zone(t))
}

/// `2026-11-18 8:11 PM EST`, for prose.
pub fn date_time(t: &Timestamp) -> String {
    format!("{} {}", local(t).format("%Y-%m-%d %-I:%M %p"), zone(t))
}

/// `2026-11-18  8:11 PM EST`, with the hour padded so columns line up in tables.
pub fn date_time_col(t: &Timestamp) -> String {
    format!("{} {}", local(t).format("%Y-%m-%d %l:%M %p"), zone(t))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twelve_hour_with_zone() {
        let t: Timestamp = "2026-11-18T01:05:09Z".parse().unwrap();
        let s = time(&t);
        assert!(s.contains("AM") || s.contains("PM"), "{s}");
        assert!(!s.contains(" 0"), "no leading zero on the hour: {s}");
        assert!(s.split(' ').nth(2).is_some_and(|z| !z.is_empty()), "zone shown: {s}");
        assert_eq!(date_time_col(&t).len(), date_time_col(&"2026-11-18T05:05:09Z".parse().unwrap()).len());
        assert!(time_secs(&t).matches(':').count() == 2);
    }
}
