//! `CronTime` from the `cron` package (v4): expression parsing, time zones and next-date computation,
//! including its DST handling. Field semantics are the ones of `cron` (not of the `cron` crate):
//! five or six fields (`[sec] min hour day-of-month month day-of-week`), day-of-week `0-7`
//! (`0`/`7` = Sunday), `jan..dec` / `sun..sat` aliases, `@daily`-style presets, and day-of-month
//! OR day-of-week when both are restricted.

use std::fmt;

use chrono::{DateTime, Datelike, FixedOffset, NaiveDate, NaiveDateTime, Offset, TimeZone, Timelike, Utc};

/// `CronError` (and `ExclusiveParametersError`, which extends it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronError(pub String);

impl CronError {
    pub fn new(message: impl Into<String>) -> Self {
        CronError(message.into())
    }
    /// `new ExclusiveParametersError('timeZone', 'utcOffset')`.
    pub fn exclusive(a: &str, b: &str) -> Self {
        CronError(format!("You can't specify both {a} and {b}"))
    }
    pub fn message(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CronError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CronError {}

/// A time zone as luxon understands it: an IANA name, a fixed offset (`UTC`, `UTC+3`, `UTC-04:30`)
/// or the system zone.
#[derive(Debug, Clone, PartialEq)]
pub enum CronZone {
    Iana(chrono_tz::Tz),
    /// Offset in seconds east of UTC.
    Fixed(i32),
    /// The process zone (`chrono::Local`).
    System,
}

impl CronZone {
    /// luxon's `normalizeZone(string)`: `local`/`system`/`default`, `utc`/`gmt`, `utc±H[:MM]` or an IANA
    /// name (case-insensitive, like `Intl`). Fails with `Invalid timezone.`.
    pub fn parse(name: &str) -> Result<Self, CronError> {
        let lowered = name.to_lowercase();
        match lowered.as_str() {
            "local" | "system" | "default" => return Ok(CronZone::system()),
            "utc" | "gmt" => return Ok(CronZone::Fixed(0)),
            _ => {}
        }
        if let Some(secs) = parse_utc_specifier(&lowered) {
            return Ok(CronZone::Fixed(secs));
        }
        chrono_tz::Tz::from_str_insensitive(name).map(CronZone::Iana).map_err(|_| CronError::new("Invalid timezone."))
    }

    /// The zone `Intl.DateTimeFormat().resolvedOptions().timeZone` reports: `TZ` when it names a valid
    /// zone (Node honours it on every platform), otherwise the system zone.
    pub fn system() -> Self {
        match std::env::var("TZ") {
            Ok(tz) if !tz.trim().is_empty() => match tz.trim().to_lowercase().as_str() {
                "utc" | "gmt" | "etc/utc" | "etc/gmt" => CronZone::Fixed(0),
                _ => chrono_tz::Tz::from_str_insensitive(tz.trim()).map(CronZone::Iana).unwrap_or(CronZone::System),
            },
            _ => CronZone::System,
        }
    }

    /// Offset (seconds east of UTC) in effect at `utc_ms`.
    pub fn offset_secs(&self, utc_ms: i64) -> i32 {
        let Some(utc) = DateTime::from_timestamp_millis(utc_ms) else { return 0 };
        match self {
            CronZone::Iana(tz) => tz.offset_from_utc_datetime(&utc.naive_utc()).fix().local_minus_utc(),
            CronZone::Fixed(s) => *s,
            CronZone::System => chrono::Local.offset_from_utc_datetime(&utc.naive_utc()).fix().local_minus_utc(),
        }
    }

    /// luxon's `zone.name`.
    pub fn name(&self) -> String {
        match self {
            CronZone::Iana(tz) => tz.name().to_string(),
            CronZone::Fixed(0) => "UTC".into(),
            CronZone::Fixed(s) => {
                let m = s / 60;
                let (h, mm) = (m.abs() / 60, m.abs() % 60);
                let sign = if m < 0 { '-' } else { '+' };
                if mm == 0 { format!("UTC{sign}{h}") } else { format!("UTC{sign}{h}:{mm:02}") }
            }
            CronZone::System => match CronZone::system() {
                CronZone::System => "local".into(),
                z => z.name(),
            },
        }
    }

    /// Wall-clock time in this zone at `utc_ms`.
    pub fn wall(&self, utc_ms: i64) -> NaiveDateTime {
        DateTime::from_timestamp_millis(utc_ms + self.offset_secs(utc_ms) as i64 * 1000).map(|d| d.naive_utc()).unwrap_or_default()
    }

    /// Instant of a wall-clock time in this zone, resolving gaps and overlaps like luxon
    /// (`DateTime.fromObject(..., { zone })`: provisional offset = the offset now, then `fixOffset`).
    pub fn instant(&self, wall: NaiveDateTime, now_ms: i64) -> i64 {
        let local_ts = wall.and_utc().timestamp_millis();
        let o = self.offset_secs(now_ms) as i64;
        let mut utc_guess = local_ts - o * 1000;
        let o2 = self.offset_secs(utc_guess) as i64;
        if o == o2 {
            return utc_guess;
        }
        utc_guess -= (o2 - o) * 1000;
        let o3 = self.offset_secs(utc_guess) as i64;
        if o2 == o3 {
            return utc_guess;
        }
        local_ts - o2.min(o3) * 1000
    }

    /// `utc_ms` as a `DateTime` carrying the offset of this zone.
    pub fn at(&self, utc_ms: i64) -> DateTime<FixedOffset> {
        let off = FixedOffset::east_opt(self.offset_secs(utc_ms)).unwrap_or(FixedOffset::east_opt(0).expect("utc"));
        off.timestamp_millis_opt(utc_ms).single().unwrap_or_else(|| off.timestamp_millis_opt(0).single().expect("epoch"))
    }
}

/// `FixedOffsetZone.parseSpecifier`: `utc+3`, `utc-04:30`.
fn parse_utc_specifier(s: &str) -> Option<i32> {
    let rest = s.strip_prefix("utc")?;
    if rest.is_empty() {
        return Some(0);
    }
    let (sign, rest) = match rest.as_bytes()[0] {
        b'+' => (1, &rest[1..]),
        b'-' => (-1, &rest[1..]),
        _ => return None,
    };
    let (h, m) = match rest.split_once(':') {
        Some((h, m)) => (h, Some(m)),
        None => (rest, None),
    };
    if h.is_empty() || h.len() > 2 || !h.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let hours: i32 = h.parse().ok()?;
    let minutes: i32 = match m {
        Some(m) if m.len() == 2 && m.bytes().all(|b| b.is_ascii_digit()) => m.parse().ok()?,
        Some(_) => return None,
        None => 0,
    };
    // luxon's signedOffset: the minutes take the sign of the hours (`-0:30` is negative too).
    let signed_m = if sign < 0 { -minutes } else { minutes };
    Some((sign * hours * 60 + signed_m) * 60)
}

/// What a `CronTime` is built from: an expression, or a date (a one-shot job).
#[derive(Debug, Clone, PartialEq)]
pub enum CronSource {
    Expression(String),
    Date(DateTime<Utc>),
}

impl From<&str> for CronSource {
    fn from(s: &str) -> Self {
        CronSource::Expression(s.to_string())
    }
}
impl From<String> for CronSource {
    fn from(s: String) -> Self {
        CronSource::Expression(s)
    }
}
impl From<&String> for CronSource {
    fn from(s: &String) -> Self {
        CronSource::Expression(s.clone())
    }
}
impl<Tz: TimeZone> From<DateTime<Tz>> for CronSource {
    fn from(d: DateTime<Tz>) -> Self {
        CronSource::Date(d.with_timezone(&Utc))
    }
}
impl From<std::time::SystemTime> for CronSource {
    fn from(t: std::time::SystemTime) -> Self {
        CronSource::Date(DateTime::<Utc>::from(t))
    }
}

impl fmt::Display for CronSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CronSource::Expression(s) => f.write_str(s),
            CronSource::Date(d) => write!(f, "{}", d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
        }
    }
}

const UNITS: [&str; 6] = ["second", "minute", "hour", "dayOfMonth", "month", "dayOfWeek"];
const CONSTRAINTS: [(u32, u32); 6] = [(0, 59), (0, 59), (0, 23), (1, 31), (1, 12), (0, 7)];
const PARSE_DEFAULTS: [&str; 6] = ["0", "*", "*", "*", "*", "*"];
const PRESETS: [(&str, &str); 9] = [
    ("@yearly", "0 0 0 1 1 *"),
    ("@monthly", "0 0 0 1 * *"),
    ("@weekly", "0 0 0 * * 0"),
    ("@daily", "0 0 0 * * *"),
    ("@hourly", "0 0 * * * *"),
    ("@minutely", "0 * * * * *"),
    ("@secondly", "* * * * * *"),
    ("@weekdays", "0 0 0 * * 1-5"),
    ("@weekends", "0 0 0 * * 0,6"),
];
const ALIASES: [(&str, u32); 19] = [
    ("jan", 1),
    ("feb", 2),
    ("mar", 3),
    ("apr", 4),
    ("may", 5),
    ("jun", 6),
    ("jul", 7),
    ("aug", 8),
    ("sep", 9),
    ("oct", 10),
    ("nov", 11),
    ("dec", 12),
    ("sun", 0),
    ("mon", 1),
    ("tue", 2),
    ("wed", 3),
    ("thu", 4),
    ("fri", 5),
    ("sat", 6),
];
const DAY_OF_WEEK: usize = 5;

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// `new CronTime(source, timeZone?, utcOffset?)`.
#[derive(Debug, Clone, PartialEq)]
pub struct CronTime {
    pub source: CronSource,
    /// `timeZone` (absent when `utcOffset` is used, or when given as an empty string).
    pub time_zone: Option<CronZone>,
    /// `utcOffset`, in minutes.
    pub utc_offset: Option<i32>,
    /// `true` when built from a date.
    pub real_date: bool,
    /// Bit sets of the allowed values per unit (`second` .. `dayOfWeek`).
    fields: [u64; 6],
}

impl CronTime {
    pub fn new(source: impl Into<CronSource>, time_zone: Option<&str>, utc_offset: Option<i32>) -> Result<Self, CronError> {
        if time_zone.is_some() && utc_offset.is_some() {
            return Err(CronError::exclusive("timeZone", "utcOffset"));
        }
        let mut zone = None;
        if let Some(tz) = time_zone.filter(|t| !t.is_empty()) {
            zone = Some(CronZone::parse(tz)?);
        }
        if time_zone.is_none() && utc_offset.is_none() {
            zone = Some(CronZone::system());
        }
        let source = source.into();
        let mut t = CronTime { real_date: matches!(source, CronSource::Date(_)), source, time_zone: zone, utc_offset, fields: [0; 6] };
        if let CronSource::Expression(s) = &t.source {
            let s = s.clone();
            t.parse(&s)?;
        }
        Ok(t)
    }

    /// `CronTime.validateCronExpression(expr)`.
    pub fn validate_cron_expression(expr: &str) -> Result<(), CronError> {
        CronTime::new(expr, None, None).map(|_| ())
    }

    /// The zone dates are computed in: `utcOffset` as `UTC±hh:mm`, else `timeZone`, else UTC.
    pub fn zone(&self) -> Result<CronZone, CronError> {
        if let Some(off) = self.utc_offset {
            let hours = off / 60;
            if hours.abs() > 99 {
                return Err(CronError::new("ERROR: You specified an invalid UTC offset."));
            }
            return Ok(CronZone::Fixed(off * 60));
        }
        Ok(self.time_zone.clone().unwrap_or(CronZone::Fixed(0)))
    }

    /// `sendAt()`: the next date (or the date itself for a one-shot `CronTime`).
    pub fn send_at(&self) -> Result<DateTime<FixedOffset>, CronError> {
        let zone = self.zone()?;
        if let CronSource::Date(d) = &self.source {
            let ms = d.timestamp_millis();
            if now_ms() > ms {
                return Err(CronError::new("WARNING: Date in past. Will never be fired."));
            }
            return Ok(zone.at(ms));
        }
        self.next_date_from(now_ms(), &zone)
    }

    /// `sendAt(i)`: the next `i` dates.
    pub fn send_at_n(&self, i: usize) -> Result<Vec<DateTime<FixedOffset>>, CronError> {
        if self.real_date {
            return self.send_at().map(|d| vec![d]);
        }
        let zone = self.zone()?;
        let mut date = now_ms();
        let mut out = Vec::with_capacity(i);
        for _ in 0..i {
            let next = self.next_date_from(date, &zone)?;
            date = next.timestamp_millis();
            out.push(next);
        }
        Ok(out)
    }

    /// `getTimeout()`: milliseconds until the next date (negative when it is already past).
    pub fn get_timeout(&self) -> Result<i64, CronError> {
        Ok(self.send_at()?.timestamp_millis() - now_ms())
    }

    /// `getNextDateFrom(start, timeZone?)`. Without a zone, `start` is read in the system zone (what
    /// happens with a JS `Date`).
    pub fn get_next_date_from(&self, start: DateTime<Utc>, time_zone: Option<&str>) -> Result<DateTime<FixedOffset>, CronError> {
        let zone = match time_zone {
            Some(z) => CronZone::parse(z)?,
            None => CronZone::system(),
        };
        self.next_date_from(start.timestamp_millis(), &zone)
    }

    /// The next matching date strictly after the second of `start_ms`, in `zone`.
    pub fn next_date_from(&self, start_ms: i64, zone: &CronZone) -> Result<DateTime<FixedOffset>, CronError> {
        let now = now_ms();
        let first = zone.wall(start_ms).with_nanosecond(0).unwrap_or_default();
        let mut date = first;
        let max_match = DateTime::from_timestamp_millis(now)
            .and_then(|d| d.checked_add_months(chrono::Months::new(96)))
            .map(|d| d.timestamp_millis())
            .unwrap_or(i64::MAX);
        let [second, minute, hour, dom, month, dow] = self.fields;
        let has = |set: u64, v: u32| set & (1u64 << v) != 0;
        let count = |set: u64| set.count_ones();
        loop {
            if date.and_utc().timestamp_millis() > max_match {
                return Err(CronError::new(format!(
                    "Something went wrong. No execution date was found in the next 8 years.\n\t\t\t\t\t\t\tPlease provide the following string if you would like to help debug:\n\t\t\t\t\t\t\tTime Zone: {} - Cron String: {} - UTC offset: 0 - current Date: {}",
                    zone.name(),
                    self.source,
                    CronZone::system().at(now).to_rfc3339_opts(chrono::SecondsFormat::Millis, false)
                )));
            }
            if !has(month, date.month()) && count(month) != 12 {
                let (y, m) = if date.month() == 12 { (date.year() + 1, 1) } else { (date.year(), date.month() + 1) };
                date = naive(y, m, 1, 0, 0, 0);
                continue;
            }
            let wd = date.weekday().num_days_from_sunday();
            let in_dom = has(dom, date.day());
            let in_dow = has(dow, wd);
            let dom_all = count(dom) == 31;
            let dow_all = count(dow) == 7;
            if (!in_dom && !dom_all && !(in_dow && !dow_all)) || (!in_dow && !dow_all && !(in_dom && !dom_all)) {
                let next = date.date().succ_opt().unwrap_or(date.date());
                date = next.and_hms_opt(0, 0, 0).expect("midnight");
                continue;
            }
            if !has(hour, date.hour()) && count(hour) != 24 {
                date = (date + chrono::Duration::hours(1)).with_minute(0).and_then(|d| d.with_second(0)).expect("valid");
                continue;
            }
            if !has(minute, date.minute()) && count(minute) != 60 {
                date = (date + chrono::Duration::minutes(1)).with_second(0).expect("valid");
                continue;
            }
            if date == first || (!has(second, date.second()) && count(second) != 60) {
                date += chrono::Duration::seconds(1);
                continue;
            }
            break;
        }

        let (expected_hour, expected_minute) = (date.hour(), date.minute());
        let mut ms = zone.instant(date, now);
        let wall = zone.wall(ms);
        let reference = zone.instant(naive(wall.year(), 1, 1, 0, 0, 0), now);
        let non_dst_offset = zone.offset_secs(reference);
        if (expected_hour != wall.hour() || expected_minute != wall.minute()) && non_dst_offset != zone.offset_secs(ms) {
            while zone.offset_secs(ms - 60_000) != non_dst_offset {
                ms -= 60_000;
            }
            return Ok(zone.at(ms));
        }
        let h = |t: i64| zone.wall(t).hour();
        let m = |t: i64| zone.wall(t).minute();
        let hour_test = ms - 3_600_000;
        let two_hour_test = ms - 7_200_000;
        if (h(hour_test) == h(ms) || h(two_hour_test) == h(hour_test)) && hour_test > start_ms {
            ms = hour_test;
        }
        let half_hour_test = ms - 1_800_000;
        if (m(half_hour_test) == m(ms) || m(hour_test) == m(half_hour_test)) && half_hour_test > start_ms {
            ms = half_hour_test;
        }
        Ok(zone.at(ms))
    }

    /// `toJSON()`: each unit as `*` or its values joined by `,`.
    pub fn to_json(&self) -> Vec<String> {
        (0..6).map(|u| self.wc_or_all(u)).collect()
    }

    fn wc_or_all(&self, unit: usize) -> String {
        if self.has_all(unit) {
            return "*".into();
        }
        (0..64).filter(|v| self.fields[unit] & (1u64 << v) != 0).map(|v| v.to_string()).collect::<Vec<_>>().join(",")
    }

    // `_hasAll` (its loop stops one short of the upper bound, like the original).
    fn has_all(&self, unit: usize) -> bool {
        let (low, high) = CONSTRAINTS[unit];
        let high = if unit == DAY_OF_WEEK { high - 1 } else { high };
        (low..high).all(|i| self.fields[unit] & (1u64 << i) != 0)
    }

    /// Whether `v` is allowed for `unit` (`"second"`, ..., `"dayOfWeek"`).
    pub fn matches(&self, unit: &str, v: u32) -> bool {
        UNITS.iter().position(|u| *u == unit).is_some_and(|i| v < 64 && self.fields[i] & (1u64 << v) != 0)
    }

    fn parse(&mut self, source: &str) -> Result<(), CronError> {
        let mut s = source.to_lowercase();
        if let Some((_, p)) = PRESETS.iter().find(|(k, _)| *k == s) {
            s = p.to_string();
        }
        let chars: Vec<char> = s.chars().collect();
        let mut replaced = String::with_capacity(s.len());
        let mut i = 0;
        while i < chars.len() {
            if chars[i].is_ascii_alphabetic() {
                let mut j = i;
                while j < chars.len() && j - i < 3 && chars[j].is_ascii_alphabetic() {
                    j += 1;
                }
                let alias: String = chars[i..j].iter().collect();
                match ALIASES.iter().find(|(k, _)| *k == alias) {
                    Some((_, v)) => replaced.push_str(&v.to_string()),
                    None => return Err(CronError::new(format!("Unknown alias: {alias}"))),
                }
                i = j;
            } else {
                replaced.push(chars[i]);
                i += 1;
            }
        }
        let units: Vec<&str> = replaced.split_whitespace().collect();
        if units.len() < 5 {
            return Err(CronError::new("Too few fields"));
        }
        if units.len() > 6 {
            return Err(CronError::new("Too many fields"));
        }
        let shift = 6 - units.len();
        for unit in 0..6 {
            let cur = if unit < shift { PARSE_DEFAULTS[unit] } else { units[unit - shift] };
            self.parse_field(cur, unit)?;
        }
        Ok(())
    }

    fn parse_field(&mut self, value: &str, unit: usize) -> Result<(), CronError> {
        let name = UNITS[unit];
        let (low, high) = CONSTRAINTS[unit];
        for field in value.split(',') {
            if let Some(idx) = field.find('*')
                && idx != 0
            {
                return Err(CronError::new(format!("Field ({field}) has an invalid wildcard expression")));
            }
        }
        let value = value.replace('*', &format!("{low}-{high}"));
        for range in value.split(',') {
            let Some((lower, upper, step)) = parse_range(range) else {
                return Err(CronError::new(format!("Field ({name}) cannot be parsed")));
            };
            let was_step_defined = step.is_some();
            let step = step.unwrap_or(1.0);
            if step == 0.0 {
                return Err(CronError::new(format!("Field ({name}) has a step of zero")));
            }
            if let Some(u) = upper
                && lower > u
            {
                return Err(CronError::new(format!("Field ({name}) has an invalid range")));
            }
            let out_of_range = lower < low as f64 || upper.is_some_and(|u| u > high as f64) || (upper.is_none() && lower > high as f64);
            if out_of_range {
                return Err(CronError::new(format!("Field value ({value}) is out of range")));
            }
            let lower = lower.max(low as f64).min(high as f64);
            let upper = match upper {
                Some(u) => u.min(high as f64),
                None if was_step_defined => high as f64,
                None => lower,
            };
            let mut pointer = lower;
            loop {
                self.fields[unit] |= 1u64 << (pointer as u32);
                pointer += step;
                if pointer > upper {
                    break;
                }
            }
            if unit == DAY_OF_WEEK {
                let set = &mut self.fields[unit];
                if *set & 1 == 0 && *set & (1 << 7) != 0 {
                    *set |= 1;
                }
                *set &= !(1 << 7);
            }
        }
        Ok(())
    }
}

impl fmt::Display for CronTime {
    /// `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_json().join(" "))
    }
}

/// `^(\d+)(?:-(\d+))?(?:\/(\d+))?$` → (lower, upper, step), numbers as `parseInt` reads them.
fn parse_range(s: &str) -> Option<(f64, Option<f64>, Option<f64>)> {
    fn digits(s: &str) -> (&str, &str) {
        let n = s.bytes().take_while(|b| b.is_ascii_digit()).count();
        s.split_at(n)
    }
    let (lo, rest) = digits(s);
    if lo.is_empty() {
        return None;
    }
    let mut rest = rest;
    let mut upper = None;
    if let Some(r) = rest.strip_prefix('-') {
        let (hi, r) = digits(r);
        if hi.is_empty() {
            return None;
        }
        upper = Some(hi.parse::<f64>().ok()?);
        rest = r;
    }
    let mut step = None;
    if let Some(r) = rest.strip_prefix('/') {
        let (st, r) = digits(r);
        if st.is_empty() {
            return None;
        }
        step = Some(st.parse::<f64>().ok()?);
        rest = r;
    }
    if !rest.is_empty() {
        return None;
    }
    Some((lo.parse::<f64>().ok()?, upper, step))
}

fn naive(y: i32, m: u32, d: u32, h: u32, mi: u32, s: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(y, m, d).and_then(|d| d.and_hms_opt(h, mi, s)).unwrap_or_default()
}
