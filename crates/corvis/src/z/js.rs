//! JavaScript semantics needed to behave exactly like zod running on V8:
//! `Number.prototype.toString`, `String(x)`, `Number(x)`, `BigInt(x)`, `new Date(x)`,
//! `Date.prototype.toISOString`, `String.prototype.trim`, ...

use serde_json::{Value, json};

/// Reorders the keys of an object like JavaScript property enumeration (not recursive).
pub(crate) fn js_map(m: serde_json::Map<String, Value>) -> serde_json::Map<String, Value> {
    if m.keys().any(|k| k.starts_with(|c: char| c.is_ascii_digit())) {
        crate::http::json_parse::js_order(m.into_iter().collect())
    } else {
        m
    }
}

/// Reorders the keys of every object in `v` like JavaScript property enumeration (array-index
/// keys such as `"0"` first, ascending, then the rest in insertion order).
pub fn js_key_order(v: Value) -> Value {
    match v {
        Value::Object(m) => {
            let entries = m.into_iter().map(|(k, v)| (k, js_key_order(v))).collect();
            Value::Object(crate::http::json_parse::js_order(entries))
        }
        Value::Array(a) => Value::Array(a.into_iter().map(js_key_order).collect()),
        v => v,
    }
}

/// `Number.prototype.toString()` (radix 10).
pub fn num_to_string(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x == 0.0 {
        return "0".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    if x < 0.0 {
        return format!("-{}", num_to_string(-x));
    }
    // Shortest round-trip digits.
    let e = format!("{x:e}");
    let (mantissa, exp) = e.split_once('e').unwrap_or((&e, "0"));
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let exp: i32 = exp.parse().unwrap_or(0);
    let k = digits.len() as i32;
    let n = exp + 1;
    if k <= n && n <= 21 {
        let mut s = digits.clone();
        s.extend(std::iter::repeat_n('0', (n - k) as usize));
        s
    } else if 0 < n && n <= 21 {
        format!("{}.{}", &digits[..n as usize], &digits[n as usize..])
    } else if -6 < n && n <= 0 {
        format!("0.{}{}", "0".repeat((-n) as usize), digits)
    } else {
        let e = n - 1;
        let sign = if e < 0 { '-' } else { '+' };
        if k == 1 { format!("{digits}e{sign}{}", e.abs()) } else { format!("{}.{}e{sign}{}", &digits[..1], &digits[1..], e.abs()) }
    }
}

/// A finite `f64` as a JSON number (integers without the `.0`, like `JSON.stringify`).
pub fn num_json(n: f64) -> Value {
    if n == 0.0 {
        return json!(0);
    }
    if n.fract() == 0.0 && n.abs() < 9.007_199_254_740_992e15 {
        json!(n as i64)
    } else if n.is_finite() {
        json!(n)
    } else {
        Value::Null
    }
}

/// A JSON number as JavaScript sees it.
pub fn json_f64(n: &serde_json::Number) -> f64 {
    n.as_f64().unwrap_or(f64::NAN)
}

pub fn is_js_whitespace(c: char) -> bool {
    matches!(
        c,
        '\u{9}' | '\u{a}' | '\u{b}' | '\u{c}' | '\u{d}' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
    )
}

/// `String.prototype.trim()`.
pub fn js_trim(s: &str) -> &str {
    s.trim_matches(is_js_whitespace)
}

/// `str.length` (UTF-16 code units).
pub fn js_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// `String(value)` for JSON values.
pub fn to_js_string(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => {
            if let Some(i) = n.as_i64()
                && (i as f64).abs() < 9.007_199_254_740_992e15
            {
                return i.to_string();
            }
            num_to_string(json_f64(n))
        }
        Value::String(s) => s.clone(),
        Value::Array(items) => {
            items.iter().map(|i| if i.is_null() { String::new() } else { to_js_string(i) }).collect::<Vec<_>>().join(",")
        }
        Value::Object(_) => "[object Object]".into(),
    }
}

/// `StringToNumber` (the `Number("...")` algorithm).
pub fn string_to_number(s: &str) -> f64 {
    let t = js_trim(s);
    if t.is_empty() {
        return 0.0;
    }
    match t {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    let b = t.as_bytes();
    if b.len() > 2 && b[0] == b'0' {
        let radix = match b[1] {
            b'x' | b'X' => 16,
            b'o' | b'O' => 8,
            b'b' | b'B' => 2,
            _ => 0,
        };
        if radix != 0 {
            let mut acc = 0f64;
            for c in t[2..].chars() {
                match c.to_digit(radix) {
                    Some(d) => acc = acc * radix as f64 + d as f64,
                    None => return f64::NAN,
                }
            }
            return acc;
        }
    }
    if decimal_prefix(t) == Some(t.len()) { t.parse::<f64>().unwrap_or(f64::NAN) } else { f64::NAN }
}

/// The length of the longest prefix of `t` that is a `StrDecimalLiteral` without `Infinity` (`-1.5e3`, `.5`, `5.`);
/// `None` when no prefix is.
fn decimal_prefix(t: &str) -> Option<usize> {
    let b = t.as_bytes();
    let digits_from = |mut i: usize| {
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        i
    };
    let mut i = usize::from(matches!(b.first(), Some(b'+' | b'-')));
    let int_end = digits_from(i);
    let mut mantissa_digits = int_end - i;
    i = int_end;
    if b.get(i) == Some(&b'.') {
        let frac_end = digits_from(i + 1);
        mantissa_digits += frac_end - (i + 1);
        i = frac_end;
    }
    if mantissa_digits == 0 {
        return None;
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        let sign = usize::from(matches!(b.get(i + 1), Some(b'+' | b'-')));
        let exp_end = digits_from(i + 1 + sign);
        if exp_end > i + 1 + sign {
            i = exp_end;
        }
    }
    Some(i)
}

/// `parseFloat(string)`: the longest decimal prefix after leading white space, `NaN` when there is none.
pub fn parse_float(s: &str) -> f64 {
    let t = s.trim_start_matches(is_js_whitespace);
    for (text, value) in [("Infinity", f64::INFINITY), ("+Infinity", f64::INFINITY), ("-Infinity", f64::NEG_INFINITY)] {
        if t.starts_with(text) {
            return value;
        }
    }
    decimal_prefix(t).map_or(f64::NAN, |n| t[..n].parse::<f64>().unwrap_or(f64::NAN))
}

/// `parseInt(string, 10)`: the leading decimal digits after white space and an optional sign, `NaN` when there are
/// none.
pub fn parse_int(s: &str) -> f64 {
    let t = s.trim_start_matches(is_js_whitespace);
    let (neg, rest) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    let digits = &rest[..rest.bytes().position(|c| !c.is_ascii_digit()).unwrap_or(rest.len())];
    // The digits are a valid decimal literal, so the parse succeeds and rounds like JavaScript.
    let n = if digits.is_empty() { f64::NAN } else { digits.parse::<f64>().unwrap_or(f64::NAN) };
    if neg { -n } else { n }
}

/// `Number.prototype.toFixed(digits)`: rounded to the nearest, a tie to the larger magnitude; `String(x)` for a
/// non-finite `x` or `|x| >= 1e21`.
pub fn to_fixed(x: f64, digits: usize) -> String {
    if !x.is_finite() || x.abs() >= 1e21 {
        return num_to_string(x);
    }
    fixed_text(x, digits, fixed_precision(x, digits))
}

/// How many fraction digits `to_fixed` formats `|x|` with before it rounds to `digits` by hand: the exact expansion
/// when it is short, otherwise `digits + max(60, digits)`, so the work is bounded (a subnormal has 1074 exact digits).
///
/// The shorter expansion rounds as the exact one. Formatting at precision `p` moves `|x|` by at most `10^-p / 2`, so
/// only an `|x|` that close to a tie `(2k + 1) / (2 * 10^digits)` could round the other way. A tie has `digits + 1`
/// fraction digits, so `|x|` is not one when its expansion is longer than `p`; then `|x|` and the tie differ by at
/// least `2^e / 5^digits`, where `2^e` is the lowest bit of `|x|`, and `2^e > 10^-digits / 2^54` near a tie. That is
/// more than `10^-p / 2` once `p - digits >= 16 + 0.7 * digits`, which both `60` (for `digits <= 63`) and `digits`
/// (for `digits >= 54`) satisfy.
pub(crate) fn fixed_precision(x: f64, digits: usize) -> usize {
    exact_fraction_digits(x).min(digits.saturating_add(digits.max(60))).max(digits)
}

/// `|x|` formatted with `precision` fraction digits, then rounded half up to `digits` by hand (Rust's own rounding of a
/// tie is to even), with the sign of `x`.
pub(crate) fn fixed_text(x: f64, digits: usize, precision: usize) -> String {
    let exact = format!("{:.*}", precision, x.abs());
    let (int, frac) = exact.split_once('.').unwrap_or((exact.as_str(), ""));
    let mut kept: Vec<u8> = int.bytes().chain(frac.bytes().take(digits)).collect();
    if frac.as_bytes().get(digits).is_some_and(|d| *d >= b'5') {
        let mut i = kept.len();
        loop {
            if i == 0 {
                kept.insert(0, b'1');
                break;
            }
            i -= 1;
            if kept[i] == b'9' {
                kept[i] = b'0';
            } else {
                kept[i] += 1;
                break;
            }
        }
    }
    let point = kept.len() - digits;
    let mut out = String::with_capacity(kept.len() + 2);
    if x < 0.0 {
        out.push('-');
    }
    out.extend(kept[..point].iter().map(|b| char::from(*b)));
    if digits > 0 {
        out.push('.');
        out.extend(kept[point..].iter().map(|b| char::from(*b)));
    }
    out
}

/// How many fraction digits the exact decimal expansion of a finite `x` has.
pub(crate) fn exact_fraction_digits(x: f64) -> usize {
    if x == 0.0 {
        return 0;
    }
    let bits = x.to_bits();
    let biased = ((bits >> 52) & 0x7ff) as i64;
    let mantissa = bits & ((1 << 52) - 1);
    let (mantissa, exponent) = if biased == 0 { (mantissa, -1074) } else { (mantissa | (1 << 52), biased - 1075) };
    let exponent = exponent + i64::from(mantissa.trailing_zeros());
    usize::try_from(-exponent).unwrap_or(0)
}

/// `Number(value)` for JSON values (`None` = `undefined`).
pub fn to_number(v: Option<&Value>) -> f64 {
    match v {
        None => f64::NAN,
        Some(Value::Null) => 0.0,
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        Some(Value::Number(n)) => json_f64(n),
        Some(Value::String(s)) => string_to_number(s),
        Some(a @ Value::Array(_)) => string_to_number(&to_js_string(a)),
        Some(Value::Object(_)) => f64::NAN,
    }
}

/// `Boolean(value)` (truthiness).
pub fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => {
            let f = json_f64(n);
            f != 0.0 && !f.is_nan()
        }
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

/// `BigInt(value)`; `None` when JavaScript would throw, and for a number whose integer value does not fit in `i128`
/// (JavaScript's BigInt has no bound; Corvis refuses what it cannot represent instead of saturating).
pub fn to_bigint(v: Option<&Value>) -> Option<i128> {
    match v? {
        Value::Bool(b) => Some(i128::from(*b)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                return Some(i128::from(i));
            }
            if let Some(u) = n.as_u64() {
                return Some(i128::from(u));
            }
            let f = json_f64(n);
            // -2^127 is i128::MIN; 2^127 is one past i128::MAX. Both are exact in f64.
            const LIMIT: f64 = 170_141_183_460_469_231_731_687_303_715_884_105_728.0;
            (f.fract() == 0.0 && (-LIMIT..LIMIT).contains(&f)).then_some(f as i128)
        }
        Value::String(s) => string_to_bigint(s),
        Value::Array(_) => string_to_bigint(&to_js_string(v?)),
        _ => None,
    }
}

pub fn string_to_bigint(s: &str) -> Option<i128> {
    let t = js_trim(s);
    if t.is_empty() {
        return Some(0);
    }
    let b = t.as_bytes();
    if b.len() > 2 && b[0] == b'0' {
        let radix = match b[1] {
            b'x' | b'X' => 16,
            b'o' | b'O' => 8,
            b'b' | b'B' => 2,
            _ => 0,
        };
        if radix != 0 {
            return i128::from_str_radix(&t[2..], radix).ok();
        }
    }
    let (neg, digits) = match b[0] {
        b'-' => (true, &t[1..]),
        b'+' => (false, &t[1..]),
        _ => (false, t),
    };
    if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let n: i128 = digits.parse().ok()?;
    Some(if neg { -n } else { n })
}

// ---------------------------------------------------------------------------
// Dates.
// ---------------------------------------------------------------------------

const MAX_TIME: f64 = 8.64e15;

/// No year beyond this one has a valid time value (ECMAScript dates span ±8.64e15 ms around 1970, years -271821 to
/// 275760).
const MAX_YEAR: u64 = 275_760;

/// Days from 1970-01-01, in `i128` so it is total for any `i64` arguments.
fn days_from_civil(y: i64, m: i64, d: i64) -> i128 {
    let (y, m, d) = (i128::from(y), i128::from(m), i128::from(d));
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `MakeDate(MakeDay(y, m, d), MakeTime(h, min, s, ms))` in UTC; NaN for a year no valid date has.
fn make_utc(y: i64, mon: i64, d: i64, h: i64, mi: i64, s: i64, ms: i64) -> f64 {
    if y.unsigned_abs() > MAX_YEAR {
        return f64::NAN;
    }
    let time = i128::from(h) * 3_600_000 + i128::from(mi) * 60_000 + i128::from(s) * 1000 + i128::from(ms);
    (days_from_civil(y, mon, d) * 86_400_000 + time) as f64
}

/// A run of ASCII digits as a number; `None` when it does not fit in `i64` (V8 answers Invalid Date for such a
/// numeral, never `0`).
fn numeral(digits: &[char]) -> Option<i64> {
    digits.iter().try_fold(0i64, |acc, c| acc.checked_mul(10)?.checked_add(i64::from(c.to_digit(10)?)))
}

fn local_offset_ms(utc_guess_ms: f64) -> f64 {
    use chrono::{Local, Offset, TimeZone};
    let secs = (utc_guess_ms / 1000.0).floor() as i64;
    match Local.timestamp_opt(secs, 0) {
        chrono::LocalResult::Single(dt) | chrono::LocalResult::Ambiguous(dt, _) => f64::from(dt.offset().fix().local_minus_utc()) * 1000.0,
        chrono::LocalResult::None => 0.0,
    }
}

/// Local wall-clock time to UTC milliseconds.
fn local_to_utc(local_ms: f64) -> f64 {
    let off = local_offset_ms(local_ms);
    let guess = local_ms - off;
    let off2 = local_offset_ms(guess);
    local_ms - off2
}

fn time_clip(t: f64) -> f64 {
    if !t.is_finite() || t.abs() > MAX_TIME { f64::NAN } else { t.trunc() + 0.0 }
}

/// `Date.parse(string)` (ISO format per ECMA-262 plus the common V8 fallbacks).
pub fn parse_date(s: &str) -> f64 {
    if let Some(t) = parse_iso_date(s) {
        return t;
    }
    parse_legacy_date(s)
}

/// The ECMAScript date time string format alone (`Date.parse` without V8's fallbacks): `None` for any other text,
/// including a day past the end of its month; NaN for a date outside the ECMAScript range.
pub(crate) fn parse_iso_date(s: &str) -> Option<f64> {
    let b = s.as_bytes();
    let mut i = 0;
    let num = |i: &mut usize, n: usize| -> Option<i64> {
        if *i + n > b.len() || !b[*i..*i + n].iter().all(u8::is_ascii_digit) {
            return None;
        }
        let v = s[*i..*i + n].parse().ok()?;
        *i += n;
        Some(v)
    };
    let year = if b.first().is_some_and(|c| *c == b'+' || *c == b'-') {
        let neg = b[0] == b'-';
        i += 1;
        let y = num(&mut i, 6)?;
        if neg && y == 0 {
            return None;
        }
        if neg { -y } else { y }
    } else {
        num(&mut i, 4)?
    };
    let (mut month, mut day) = (1, 1);
    if i < b.len() && b[i] == b'-' {
        i += 1;
        month = num(&mut i, 2)?;
        if i < b.len() && b[i] == b'-' {
            i += 1;
            day = num(&mut i, 2)?;
        }
    }
    let (mut h, mut mi, mut sec, mut ms) = (0, 0, 0, 0);
    let mut has_time = false;
    let mut offset: Option<i64> = None;
    if i < b.len() && (b[i] == b'T' || b[i] == b't') {
        has_time = true;
        i += 1;
        h = num(&mut i, 2)?;
        if i >= b.len() || b[i] != b':' {
            return None;
        }
        i += 1;
        mi = num(&mut i, 2)?;
        if i < b.len() && b[i] == b':' {
            i += 1;
            sec = num(&mut i, 2)?;
            if i < b.len() && (b[i] == b'.' || b[i] == b',') {
                i += 1;
                let start = i;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
                if i == start {
                    return None;
                }
                let frac = &s[start..i.min(start + 3)];
                ms = format!("{frac:0<3}").parse().ok()?;
            }
        }
        if i < b.len() {
            if b[i] == b'Z' || b[i] == b'z' {
                offset = Some(0);
                i += 1;
            } else if b[i] == b'+' || b[i] == b'-' {
                let sign = if b[i] == b'-' { -1 } else { 1 };
                i += 1;
                let oh = num(&mut i, 2)?;
                if i >= b.len() || b[i] != b':' {
                    return None;
                }
                i += 1;
                let om = num(&mut i, 2)?;
                if oh > 23 || om > 59 {
                    return None;
                }
                offset = Some(sign * (oh * 60 + om));
            }
        }
    }
    if i != b.len() {
        return None;
    }
    if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
        return None;
    }
    if h > 24 || mi > 59 || sec > 59 || (h == 24 && (mi > 0 || sec > 0 || ms > 0)) {
        return None;
    }
    let t = make_utc(year, month, day, h, mi, sec, ms);
    let t = match (has_time, offset) {
        (false, _) => t,
        (true, Some(off)) => t - off as f64 * 60_000.0,
        (true, None) => local_to_utc(t),
    };
    Some(time_clip(t))
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
                29
            } else {
                28
            }
        }
    }
}

const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];

/// Subset of V8's legacy date parser: numeric `Y-M-D`/`Y/M/D`/`M/D/Y` dates, month names,
/// optional weekday, `HH:MM[:SS[.mmm]]`, `AM/PM`, `Z`/`UTC`/`GMT` and `+hhmm` offsets.
fn parse_legacy_date(s: &str) -> f64 {
    let lower = s.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return f64::NAN;
    }
    let mut nums: Vec<i64> = Vec::new();
    let mut month: Option<i64> = None;
    let mut time: Option<(i64, i64, i64, i64)> = None;
    let mut offset: Option<i64> = None;
    let mut pm: Option<bool> = None;
    let mut date_sep: Option<char> = None;
    let chars: Vec<char> = lower.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            let Some(n) = numeral(&chars[start..i]) else { return f64::NAN };
            if i < chars.len() && chars[i] == ':' && time.is_none() {
                // time
                let mut parts = vec![n];
                let mut ms = 0;
                while i < chars.len() && chars[i] == ':' {
                    i += 1;
                    let st = i;
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                    match numeral(&chars[st..i]) {
                        Some(part) if st < i => parts.push(part),
                        _ => return f64::NAN,
                    }
                }
                if i < chars.len() && chars[i] == '.' {
                    i += 1;
                    let st = i;
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                    let frac: String = chars[st..i.min(st + 3)].iter().collect();
                    ms = format!("{frac:0<3}").parse().unwrap_or(0);
                }
                if parts.len() > 3 {
                    return f64::NAN;
                }
                time = Some((parts[0], parts[1], *parts.get(2).unwrap_or(&0), ms));
                continue;
            }
            nums.push(n);
            if i < chars.len() && (chars[i] == '-' || chars[i] == '/') && nums.len() < 3 {
                if date_sep.is_some_and(|d| d != chars[i]) {
                    return f64::NAN;
                }
                date_sep = Some(chars[i]);
                i += 1;
            }
            continue;
        }
        if c == '+' || (c == '-' && time.is_some()) {
            let sign = if c == '-' { -1 } else { 1 };
            i += 1;
            let st = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == ':') {
                i += 1;
            }
            let raw: Vec<char> = chars[st..i].iter().copied().filter(|c| *c != ':').collect();
            let Some(v) = numeral(&raw) else { return f64::NAN };
            let mins = if raw.len() <= 2 { v * 60 } else { (v / 100) * 60 + v % 100 };
            offset = Some(sign * mins);
            continue;
        }
        if c.is_ascii_alphabetic() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_alphabetic() {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            match word.as_str() {
                "am" | "a" => pm = Some(false),
                "pm" | "p" => pm = Some(true),
                "z" | "utc" | "gmt" | "ut" => offset = Some(offset.unwrap_or(0)),
                "t" => {}
                w if w.len() >= 3 => {
                    if let Some(m) = MONTHS.iter().position(|m| w.starts_with(m)) {
                        month = Some(m as i64 + 1);
                    } else if ["sun", "mon", "tue", "wed", "thu", "fri", "sat"].iter().any(|d| w.starts_with(d)) {
                    } else {
                        return f64::NAN;
                    }
                }
                _ => return f64::NAN,
            }
            continue;
        }
        if matches!(c, ' ' | ',' | '.' | '(' | ')' | '-' | '/') {
            i += 1;
            continue;
        }
        return f64::NAN;
    }
    let (y, m, d) = match (month, nums.as_slice()) {
        (Some(m), [d, y]) => {
            if *d > 31 {
                (*d, m, *y)
            } else {
                (*y, m, *d)
            }
        }
        (Some(m), [d]) => (2001, m, *d),
        (None, [a, b, c]) => {
            if *a > 31 || date_sep == Some('-') && *a > 12 {
                (*a, *b, *c)
            } else {
                (*c, *a, *b)
            }
        }
        (None, [a, b]) if date_sep.is_some() => {
            if *a > 12 {
                (*a, *b, 1)
            } else {
                (2001, *a, *b)
            }
        }
        _ => return f64::NAN,
    };
    let y = if (0..50).contains(&y) && nums.iter().all(|n| *n < 100) {
        y + 2000
    } else if (50..100).contains(&y) && nums.iter().all(|n| *n < 100) {
        y + 1900
    } else {
        y
    };
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return f64::NAN;
    }
    let (mut h, mi, sec, ms) = time.unwrap_or((0, 0, 0, 0));
    if let Some(p) = pm {
        if h > 12 {
            return f64::NAN;
        }
        if p && h < 12 {
            h += 12;
        } else if !p && h == 12 {
            h = 0;
        }
    }
    if h > 24 || mi > 59 || sec > 59 {
        return f64::NAN;
    }
    let t = make_utc(y, m, d, h, mi, sec, ms);
    let t = match offset {
        Some(off) => t - off as f64 * 60_000.0,
        None => local_to_utc(t),
    };
    time_clip(t)
}

/// `new Date(value)` for JSON values; returns the time value (NaN = Invalid Date).
pub fn to_date(v: Option<&Value>) -> f64 {
    match v {
        None => f64::NAN,
        Some(Value::String(s)) => parse_date(s),
        Some(Value::Array(_)) => parse_date(&to_js_string(v.unwrap())),
        Some(Value::Object(_)) => f64::NAN,
        Some(other) => time_clip(to_number(Some(other))),
    }
}

/// `Date.prototype.toISOString()`; `None` for invalid dates.
pub fn date_to_iso(t: f64) -> Option<String> {
    if !t.is_finite() || t.abs() > MAX_TIME {
        return None;
    }
    let t = t as i64;
    let days = t.div_euclid(86_400_000);
    let ms_of_day = t.rem_euclid(86_400_000);
    let (y, m, d) = civil_from_days(days);
    let h = ms_of_day / 3_600_000;
    let mi = (ms_of_day / 60_000) % 60;
    let s = (ms_of_day / 1000) % 60;
    let ms = ms_of_day % 1000;
    let year = if (0..=9999).contains(&y) {
        format!("{y:04}")
    } else if y < 0 {
        format!("-{:06}", -y)
    } else {
        format!("+{y:06}")
    };
    Some(format!("{year}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{ms:03}Z"))
}

/// A date string in RFC 3339 / ISO 8601 form (what a Rust `DateTime` serializes to).
pub fn looks_like_iso_datetime(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 10 && b[4] == b'-' && b[7] == b'-' && b[..4].iter().all(u8::is_ascii_digit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_formatting() {
        assert_eq!(num_to_string(1e21), "1e+21");
        assert_eq!(num_to_string(123e-20), "1.23e-18");
        assert_eq!(num_to_string(0.000001), "0.000001");
        assert_eq!(num_to_string(1e-7), "1e-7");
        assert_eq!(num_to_string(1.5e300), "1.5e+300");
        assert_eq!(num_to_string(9007199254740992.0), "9007199254740992");
        assert_eq!(num_to_string(0.1), "0.1");
        assert_eq!(num_to_string(-2.5), "-2.5");
        assert_eq!(num_to_string(100.0), "100");
        assert_eq!(num_to_string(3.4028234663852886e38), "3.4028234663852886e+38");
    }

    #[test]
    fn coercions() {
        assert_eq!(string_to_number(" 0x1F "), 31.0);
        assert!(string_to_number("-0x1").is_nan());
        assert!(string_to_number("1_0").is_nan());
        assert_eq!(string_to_number(".5"), 0.5);
        assert_eq!(string_to_number("5."), 5.0);
        assert_eq!(string_to_number("+.5e1"), 5.0);
        assert_eq!(string_to_number(""), 0.0);
        assert!(string_to_number("infinity").is_nan());
        assert_eq!(to_number(Some(&json!([]))), 0.0);
        assert_eq!(to_number(Some(&json!([7]))), 7.0);
        assert_eq!(to_js_string(&json!([1, [2, null], null])), "1,2,,");
        assert_eq!(date_to_iso(parse_date("2024-01-01")).unwrap(), "2024-01-01T00:00:00.000Z");
        assert_eq!(date_to_iso(to_date(Some(&json!(null)))).unwrap(), "1970-01-01T00:00:00.000Z");
        assert!(parse_date("x").is_nan());
        assert_eq!(date_to_iso(parse_date("2024-02-29T10:20:30.5+02:00")).unwrap(), "2024-02-29T08:20:30.500Z");
    }

    /// JavaScript's BigInt has no bound; a number whose integer value does not fit in `i128` is refused, never
    /// saturated.
    #[test]
    fn bigint_of_a_number_outside_i128_is_refused() {
        assert_eq!(to_bigint(Some(&json!(1e39))), None);
        assert_eq!(to_bigint(Some(&json!(-1e39))), None);
        assert_eq!(to_bigint(Some(&json!(2f64.powi(127)))), None);
        assert_eq!(to_bigint(Some(&json!(-(2f64.powi(127))))), Some(i128::MIN));
        // `BigInt(1e38)` in JavaScript: the f64 nearest to 1e38.
        assert_eq!(to_bigint(Some(&json!(1e38))), Some(99_999_999_999_999_997_748_809_823_456_034_029_568));
    }

    /// Expected values printed by Node 24 (`x.toFixed(d)`, `parseFloat(s)`, `parseInt(s, 10)`).
    #[test]
    fn to_fixed_parse_float_and_parse_int() {
        for (x, d, want) in [
            (0.5, 0, "1"),
            (2.5, 0, "3"),
            (1.005, 2, "1.00"),
            (1.45, 1, "1.4"),
            (-1.5, 0, "-2"),
            (-0.0000001, 2, "-0.00"),
            (0.0, 2, "0.00"),
            (1e21, 2, "1e+21"),
            (123.456, 10, "123.4560000000"),
            // An exact tie (1e14 + 0.125 is representable): rounded to the larger magnitude.
            (1e14 + 0.125, 2, "100000000000000.13"),
            (0.30000000000000004, 17, "0.30000000000000004"),
            // Below a unit of the last digit: the zero text, with the sign of a negative number.
            (5e-324, 2, "0.00"),
            (-1e-300, 2, "-0.00"),
            (-0.0, 2, "0.00"),
            (0.004999999999999999, 2, "0.00"),
            (0.005, 2, "0.01"),
        ] {
            assert_eq!(to_fixed(x, d), want, "({x}).toFixed({d})");
        }
        for (s, want) in
            [(" 1.5abc", 1.5), ("0x10", 0.0), (".5", 0.5), ("5.", 5.0), ("-.5e1x", -5.0), ("1e", 1.0), ("Infinityx", f64::INFINITY)]
        {
            assert_eq!(parse_float(s), want, "parseFloat({s:?})");
        }
        assert_eq!(parse_float("-Infinity"), f64::NEG_INFINITY);
        assert_eq!(parse_float("  \u{a0} 7"), 7.0);
        assert!(parse_float("+.").is_nan() && parse_float("").is_nan());
        for (s, want) in [("030", 30.0), ("-12x", -12.0), ("+5", 5.0), (" 9007199254740993", 9007199254740992.0), ("1e3", 1.0)] {
            assert_eq!(parse_int(s), want, "parseInt({s:?}, 10)");
        }
        assert!(parse_int("x").is_nan());
        assert!(parse_int("-0") == 0.0 && parse_int("-0").is_sign_negative());
    }

    /// A year, an offset or a numeral too large to compute is an invalid date (V8 answers Invalid Date for each;
    /// `date.coerce.edges` in the zod fixture), never a panic.
    #[test]
    fn dates_out_of_range_are_invalid() {
        for input in ["999999999999999999-01-01", "Jan 1 999999999999999999", "1/1/2000 10:00 +9999999999999999"] {
            assert!(parse_date(input).is_nan(), "{input}");
        }
        assert!(parse_date("1/1/99999999999999999999 UTC").is_nan(), "an over-long numeral is not 0");
    }

    /// `make_utc` of `{y}-01-01` computed the way the parser computed it before it was made total, in wrapping `i64`
    /// arithmetic (what a release build without `overflow-checks` does).
    fn wrapped_time(y: i64) -> i64 {
        let y = y - 1;
        let era = y / 400;
        let yoe = y.wrapping_sub(era.wrapping_mul(400));
        let doe = (yoe * 365 + yoe / 4 - yoe / 100) + (153 * 10 + 2) / 5;
        era.wrapping_mul(146_097).wrapping_add(doe).wrapping_sub(719_468).wrapping_mul(86_400_000)
    }

    /// A year whose wrapped time value lands inside the valid range would have been a valid date in a release build.
    #[test]
    fn a_year_whose_wrapped_time_looks_valid_is_invalid() {
        // A margin of one day for the local time zone offset.
        let looks_valid = |t: i64| t.unsigned_abs() < 8_640_000_000_000_000 - 86_400_000;
        // Consecutive years move the wrapped value by one year of milliseconds; a prime stride spreads it over the
        // whole `i64` range, where about one candidate in a thousand lands inside the valid range.
        let year = (0..100_000)
            .map(|k| 100_000_000_000_000_000 + k * 1_000_003)
            .find(|y| looks_valid(wrapped_time(*y)))
            .expect("a candidate inside the valid range");
        assert!(parse_date(&format!("{year}-01-01")).is_nan(), "{year}");
    }
}
