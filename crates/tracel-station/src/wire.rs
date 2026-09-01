use std::time::SystemTime;

use chrono::{DateTime, NaiveDateTime, Utc};

pub fn station_timestamp(value: &str) -> Option<SystemTime> {
    DateTime::parse_from_rfc3339(value)
        .map(|parsed| parsed.with_timezone(&Utc))
        .or_else(|_| {
            NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f")
                .map(|naive| naive.and_utc())
        })
        .ok()
        .map(SystemTime::from)
}
