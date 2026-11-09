//! `Accept-Encoding` negotiation, ported from negotiator 0.6.4 (`lib/encoding.js`) with the
//! arguments `compression` 1.8.2 passes: the supported list `br, gzip, deflate, identity` and the
//! preferred list `br, gzip`. The JavaScript number semantics that the original relies on
//! (`parseFloat`, `NaN` in comparisons) are reproduced because they decide the result for odd headers.

use std::cmp::Ordering;

/// The encodings the compression offers, in the order of `compression`'s `SUPPORTED_ENCODING`.
pub(super) const SUPPORTED: [&str; 4] = ["br", "gzip", "deflate", "identity"];
/// `PREFERRED_ENCODING`: wins ties between equally acceptable encodings.
pub(super) const PREFERRED: [&str; 2] = ["br", "gzip"];

struct Spec {
    encoding: String,
    q: f64,
    /// Position in the header (the appended `identity` takes the number of entries).
    i: usize,
}

struct Priority {
    q: f64,
    s: i32,
    o: i64,
    index: usize,
}

/// JavaScript's whitespace (`\s` and `String.prototype.trim`).
fn is_js_space(c: char) -> bool {
    c.is_whitespace() || c == '\u{feff}'
}

/// `parseFloat`: the longest decimal prefix after leading whitespace; `NaN` when there is none.
pub(super) fn parse_float(s: &str) -> f64 {
    let t = s.trim_start_matches(is_js_space);
    let b = t.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    if t[i..].starts_with("Infinity") {
        return if b.first() == Some(&b'-') { f64::NEG_INFINITY } else { f64::INFINITY };
    }
    let digits_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let mut digits = i - digits_start;
    if i < b.len() && b[i] == b'.' {
        let mut j = i + 1;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        digits += j - (i + 1);
        if digits > 0 {
            i = j;
        }
    }
    if digits == 0 {
        return f64::NAN;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        let exp_start = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > exp_start {
            i = j;
        }
    }
    t[..i].parse::<f64>().unwrap_or(f64::NAN)
}

/// `Number(string)`: the whole trimmed string must be a number (`NaN` otherwise); the empty string is 0.
pub(super) fn js_number(s: &str) -> f64 {
    let t = s.trim_matches(is_js_space);
    if t.is_empty() {
        return 0.0;
    }
    let radix = match t.get(..2) {
        Some("0x" | "0X") => Some(16),
        Some("0o" | "0O") => Some(8),
        Some("0b" | "0B") => Some(2),
        _ => None,
    };
    if let Some(radix) = radix {
        let digits = &t[2..];
        if digits.is_empty() {
            return f64::NAN;
        }
        return digits.chars().try_fold(0f64, |acc, c| c.to_digit(radix).map(|d| acc * radix as f64 + d as f64)).unwrap_or(f64::NAN);
    }
    match t {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    if !t.bytes().all(|c| c.is_ascii_digit() || matches!(c, b'+' | b'-' | b'.' | b'e' | b'E')) {
        return f64::NAN;
    }
    t.parse::<f64>().unwrap_or(f64::NAN)
}

/// `simpleEncodingRegExp`: `^\s*([^\s;]+)\s*(?:;(.*))?$`.
fn parse_entry(s: &str, i: usize) -> Option<Spec> {
    let rest = s.trim_start_matches(is_js_space);
    let name_end = rest.find(|c: char| is_js_space(c) || c == ';').unwrap_or(rest.len());
    if name_end == 0 {
        return None;
    }
    let name = &rest[..name_end];
    let rest = rest[name_end..].trim_start_matches(is_js_space);
    let params = match rest {
        "" => None,
        r => Some(r.strip_prefix(';')?),
    };
    let mut q = 1.0;
    if let Some(params) = params.filter(|p| !p.is_empty()) {
        for p in params.split(';') {
            let mut kv = p.trim_matches(is_js_space).split('=');
            if kv.next() == Some("q") {
                q = kv.next().map_or(f64::NAN, parse_float);
                break;
            }
        }
    }
    Some(Spec { encoding: name.to_string(), q, i })
}

/// `specify`: 1 for the exact name (case-insensitive), 0 for `*`, `None` otherwise.
fn specify(encoding: &str, spec: &Spec) -> Option<i32> {
    if spec.encoding.eq_ignore_ascii_case(encoding) {
        Some(1)
    } else if spec.encoding == "*" {
        Some(0)
    } else {
        None
    }
}

fn parse_accept_encoding(accept: &str) -> Vec<Spec> {
    let mut out = Vec::new();
    let mut has_identity = false;
    let mut min_quality = 1.0f64;
    let entries: Vec<&str> = accept.split(',').collect();
    for (i, raw) in entries.iter().enumerate() {
        if let Some(spec) = parse_entry(raw.trim_matches(is_js_space), i) {
            has_identity = has_identity || specify("identity", &spec).is_some();
            // `encoding.q || 1`: a zero or `NaN` quality counts as 1.
            let q = if spec.q == 0.0 || spec.q.is_nan() { 1.0 } else { spec.q };
            min_quality = min_quality.min(q);
            out.push(spec);
        }
    }
    if !has_identity {
        out.push(Spec { encoding: "identity".into(), q: min_quality, i: entries.len() });
    }
    out
}

/// JavaScript `a || b || c` over numbers: the first truthy value, else the last one.
fn js_or(values: [f64; 3]) -> f64 {
    for v in &values[..2] {
        if *v != 0.0 && !v.is_nan() {
            return *v;
        }
    }
    values[2]
}

fn priority_of(encoding: &str, accepted: &[Spec], index: usize) -> Priority {
    let mut best = Priority { q: 0.0, s: 0, o: -1, index };
    for spec in accepted {
        let Some(s) = specify(encoding, spec) else { continue };
        let diff = js_or([(best.s - s) as f64, best.q - spec.q, (best.o - spec.i as i64) as f64]);
        if diff < 0.0 {
            best = Priority { q: spec.q, s, o: spec.i as i64, index };
        }
    }
    best
}

fn preferred_index(encoding: &str) -> Option<usize> {
    PREFERRED.iter().position(|p| *p == encoding)
}

fn compare(a: &Priority, b: &Priority, provided: &[&str]) -> Ordering {
    let number = |n: f64| {
        if n < 0.0 {
            Ordering::Less
        } else if n > 0.0 {
            Ordering::Greater
        } else {
            Ordering::Equal
        }
    };
    if a.q != b.q {
        return number(b.q - a.q);
    }
    match (preferred_index(provided[a.index]), preferred_index(provided[b.index])) {
        (None, None) => number(js_or([(b.s - a.s) as f64, (a.o - b.o) as f64, (a.index as i64 - b.index as i64) as f64])),
        (Some(x), Some(y)) => x.cmp(&y),
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
    }
}

/// `new Negotiator(req).encoding(SUPPORTED, PREFERRED)`: the best encoding for the `Accept-Encoding`
/// header value (`None` when the header is absent), or `None` when nothing is acceptable.
pub(super) fn negotiate(accept_encoding: Option<&str>) -> Option<&'static str> {
    negotiate_among(accept_encoding, &SUPPORTED)
}

/// [`negotiate`] over a subset of [`SUPPORTED`] (used when an encoding is not available right now).
pub(super) fn negotiate_among(accept_encoding: Option<&str>, supported: &[&'static str]) -> Option<&'static str> {
    let accepted = parse_accept_encoding(accept_encoding.unwrap_or(""));
    let mut priorities: Vec<Priority> = supported.iter().enumerate().map(|(i, e)| priority_of(e, &accepted, i)).collect();
    priorities.retain(|p| p.q > 0.0);
    priorities.sort_by(|a, b| compare(a, b, supported));
    priorities.first().map(|p| supported[p.index])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_float_takes_a_leading_number() {
        assert_eq!(parse_float("0.5abc"), 0.5);
        assert_eq!(parse_float("  .5"), 0.5);
        assert_eq!(parse_float("1e-1"), 0.1);
        assert_eq!(parse_float("1e"), 1.0);
        assert_eq!(parse_float("-2"), -2.0);
        assert_eq!(parse_float("Infinity"), f64::INFINITY);
        assert!(parse_float("abc").is_nan());
        assert!(parse_float("").is_nan());
        assert!(parse_float(".").is_nan());
        assert!(parse_float("NaN").is_nan());
    }

    #[test]
    fn js_number_needs_the_whole_string() {
        assert_eq!(js_number("1024"), 1024.0);
        assert_eq!(js_number(" 12 "), 12.0);
        assert_eq!(js_number(""), 0.0);
        assert_eq!(js_number("0x10"), 16.0);
        assert_eq!(js_number("1e3"), 1000.0);
        assert!(js_number("12abc").is_nan());
        assert!(js_number("inf").is_nan());
        assert!(js_number("nan").is_nan());
        assert!(js_number("5e").is_nan());
    }

    #[test]
    fn basic_preferences() {
        assert_eq!(negotiate(None), Some("identity"));
        assert_eq!(negotiate(Some("gzip, deflate, br")), Some("br"));
        assert_eq!(negotiate(Some("br;q=0.5, gzip")), Some("gzip"));
        assert_eq!(negotiate(Some("deflate")), Some("deflate"));
        assert_eq!(negotiate(Some("identity;q=0, gzip;q=0")), None);
        assert_eq!(negotiate(Some("*")), Some("br"));
        assert_eq!(negotiate(Some("gzip;q=0")), Some("identity"));
    }
}
