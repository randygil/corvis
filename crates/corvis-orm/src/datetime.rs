//! `DateTime`: the equivalent of the JavaScript `Date` that JavaScript clients return for
//! `DateTime` fields.
//!
//! - Millisecond precision (values are truncated to milliseconds, like a JS `Date`).
//! - Serializes like `JSON.stringify(date)` / `date.toISOString()`:
//!   `"2026-01-02T03:04:05.678Z"` (always 3 fraction digits and `Z`).
//! - Deserializes from an ISO-8601 string (any offset or precision, or a plain
//!   `YYYY-MM-DD`) or from a number of milliseconds since the epoch.
//! - `@db.Date` columns use the same type: they read as midnight UTC.
//!
//! It derefs to `chrono::DateTime<Utc>`, so every chrono method is available.

use std::fmt;
use std::ops::Deref;
use std::str::FromStr;

use chrono::{DateTime as ChronoDateTime, Datelike, NaiveDate, NaiveDateTime, SecondsFormat, TimeZone, Timelike, Utc};

/// A JavaScript-`Date`-compatible UTC timestamp with millisecond precision.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DateTime(ChronoDateTime<Utc>);

fn truncate_ms(d: ChronoDateTime<Utc>) -> ChronoDateTime<Utc> {
    let ns = d.nanosecond();
    let keep = ns - ns % 1_000_000;
    d.with_nanosecond(keep).unwrap_or(d)
}

impl DateTime {
    /// `new Date()`
    pub fn now() -> DateTime {
        DateTime(truncate_ms(Utc::now()))
    }

    /// `new Date(ms)`
    pub fn from_millis(ms: i64) -> Option<DateTime> {
        ChronoDateTime::<Utc>::from_timestamp_millis(ms).map(DateTime)
    }

    /// `Date.UTC(y, m0, d, h, mi, s, ms)` with a 1-based month.
    pub fn from_ymd_hms_milli(y: i32, month: u32, day: u32, h: u32, mi: u32, s: u32, ms: u32) -> Option<DateTime> {
        Utc.with_ymd_and_hms(y, month, day, h, mi, s).single().map(|d| DateTime(d + chrono::Duration::milliseconds(ms as i64)))
    }

    /// Midnight UTC of a calendar date (the value returned for `@db.Date`).
    pub fn from_date(d: NaiveDate) -> DateTime {
        DateTime(d.and_hms_opt(0, 0, 0).unwrap().and_utc())
    }

    /// `date.getTime()`
    pub fn get_time(&self) -> i64 {
        self.0.timestamp_millis()
    }
    /// `date.getTime()` (chrono naming).
    pub fn timestamp_millis(&self) -> i64 {
        self.0.timestamp_millis()
    }

    /// `date.toISOString()` / `date.toJSON()`.
    pub fn to_iso_string(&self) -> String {
        let y = self.0.year();
        if (0..=9999).contains(&y) {
            self.0.to_rfc3339_opts(SecondsFormat::Millis, true)
        } else {
            // Expanded years: `+012345-01-01T00:00:00.000Z` / `-000001-...`.
            let sign = if y < 0 { '-' } else { '+' };
            format!(
                "{sign}{:06}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
                y.unsigned_abs(),
                self.0.month(),
                self.0.day(),
                self.0.hour(),
                self.0.minute(),
                self.0.second(),
                self.0.nanosecond() / 1_000_000
            )
        }
    }

    /// The calendar date in UTC (for `@db.Date`).
    pub fn date_naive_utc(&self) -> NaiveDate {
        self.0.date_naive()
    }

    /// The wrapped chrono value.
    pub fn inner(&self) -> ChronoDateTime<Utc> {
        self.0
    }

    /// Parses an ISO-8601 timestamp (`Z`, offsets, any fraction digits), `YYYY-MM-DD HH:MM:SS`
    /// (as UTC) or `YYYY-MM-DD` (midnight UTC), like `new Date(str)` for those formats.
    pub fn parse(s: &str) -> Option<DateTime> {
        let s = s.trim();
        if let Ok(d) = ChronoDateTime::parse_from_rfc3339(s) {
            return Some(DateTime(truncate_ms(d.with_timezone(&Utc))));
        }
        for fmt in ["%Y-%m-%dT%H:%M:%S%.f%:z", "%Y-%m-%d %H:%M:%S%.f%:z", "%Y-%m-%d %H:%M:%S%.f%#z"] {
            if let Ok(d) = ChronoDateTime::parse_from_str(s, fmt) {
                return Some(DateTime(truncate_ms(d.with_timezone(&Utc))));
            }
        }
        for fmt in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M", "%Y-%m-%dT%H:%M:%S"] {
            if let Ok(n) = NaiveDateTime::parse_from_str(s, fmt) {
                return Some(DateTime(truncate_ms(n.and_utc())));
            }
        }
        if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
            return Some(DateTime::from_date(d));
        }
        None
    }
}

impl Deref for DateTime {
    type Target = ChronoDateTime<Utc>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<ChronoDateTime<Utc>> for DateTime {
    fn from(d: ChronoDateTime<Utc>) -> Self {
        DateTime(truncate_ms(d))
    }
}
impl<Tz: TimeZone> From<&ChronoDateTime<Tz>> for DateTime {
    fn from(d: &ChronoDateTime<Tz>) -> Self {
        DateTime(truncate_ms(d.with_timezone(&Utc)))
    }
}
impl From<chrono::DateTime<chrono::FixedOffset>> for DateTime {
    fn from(d: chrono::DateTime<chrono::FixedOffset>) -> Self {
        DateTime(truncate_ms(d.with_timezone(&Utc)))
    }
}
impl From<NaiveDate> for DateTime {
    fn from(d: NaiveDate) -> Self {
        DateTime::from_date(d)
    }
}
impl From<NaiveDateTime> for DateTime {
    /// Interpreted as UTC.
    fn from(d: NaiveDateTime) -> Self {
        DateTime(truncate_ms(d.and_utc()))
    }
}
impl From<DateTime> for ChronoDateTime<Utc> {
    fn from(d: DateTime) -> Self {
        d.0
    }
}
impl From<DateTime> for NaiveDate {
    fn from(d: DateTime) -> Self {
        d.0.date_naive()
    }
}

impl std::ops::Add<chrono::Duration> for DateTime {
    type Output = DateTime;
    fn add(self, rhs: chrono::Duration) -> DateTime {
        DateTime(truncate_ms(self.0 + rhs))
    }
}
impl std::ops::Sub<chrono::Duration> for DateTime {
    type Output = DateTime;
    fn sub(self, rhs: chrono::Duration) -> DateTime {
        DateTime(truncate_ms(self.0 - rhs))
    }
}
impl std::ops::Sub<DateTime> for DateTime {
    type Output = chrono::Duration;
    fn sub(self, rhs: DateTime) -> chrono::Duration {
        self.0 - rhs.0
    }
}

impl fmt::Display for DateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_iso_string())
    }
}

impl fmt::Debug for DateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DateTime({})", self.to_iso_string())
    }
}

impl FromStr for DateTime {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        DateTime::parse(s).ok_or_else(|| format!("Invalid DateTime: {s}"))
    }
}

impl serde::Serialize for DateTime {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_iso_string())
    }
}

impl<'de> serde::Deserialize<'de> for DateTime {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl serde::de::Visitor<'_> for V {
            type Value = DateTime;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("an ISO-8601 date string or milliseconds since the epoch")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<DateTime, E> {
                DateTime::parse(v).ok_or_else(|| E::custom(format!("Invalid DateTime: {v}")))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<DateTime, E> {
                DateTime::from_millis(v).ok_or_else(|| E::custom("DateTime out of range"))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<DateTime, E> {
                self.visit_i64(v as i64)
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<DateTime, E> {
                self.visit_i64(v as i64)
            }
        }
        d.deserialize_any(V)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn js_date_format() {
        let d = DateTime::parse("2026-01-02T03:04:05Z").unwrap();
        assert_eq!(serde_json::to_string(&d).unwrap(), "\"2026-01-02T03:04:05.000Z\"");
        let d = DateTime::parse("2026-01-02T03:04:05.123456-04:00").unwrap();
        assert_eq!(d.to_iso_string(), "2026-01-02T07:04:05.123Z");
        assert_eq!(DateTime::parse("2026-03-01").unwrap().to_iso_string(), "2026-03-01T00:00:00.000Z");
        let n = DateTime::now();
        assert_eq!(n.nanosecond() % 1_000_000, 0);
        let back: DateTime = serde_json::from_str("1700000000123").unwrap();
        assert_eq!(back.get_time(), 1700000000123);
    }
}
