//! The one place that reads the corvis-specific query parameters of the database URL
//! (`connection_limit`, `pool_timeout`). They are removed before the URL reaches the driver,
//! which would ignore them (Postgres) or reject them (SQLite); every other pair is kept as is.

use crate::db::Dialect;
use crate::error::{InitializationError, OrmError, Result};

/// `pool_timeout`: whole seconds, `0` meaning no limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PoolTimeout {
    Seconds(u64),
    Unlimited,
}

/// Parameters read from the query string of the database URL.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct UrlParams {
    pub(crate) connection_limit: Option<u32>,
    pub(crate) pool_timeout: Option<PoolTimeout>,
}

fn invalid(what: &str, value: &str) -> OrmError {
    // Only the value of a pool parameter is echoed (bounded, without control characters), never
    // the rest of the URL: it carries the password.
    let shown: String = value.chars().filter(|c| !c.is_control()).take(40).collect();
    OrmError::Initialization(InitializationError {
        message: format!("The provided database string is invalid. {what}, got `{shown}`."),
        error_code: Some("P1013".into()),
    })
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
            if let Some(v) = hex {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Unsigned decimal digits only (no sign, no fraction, no spaces).
fn digits(v: &str) -> Option<&str> {
    (!v.is_empty() && v.bytes().all(|b| b.is_ascii_digit())).then_some(v)
}

fn parse_limit(raw: &str) -> Result<u32> {
    let v = percent_decode(raw);
    match digits(&v).and_then(|d| d.parse::<u32>().ok()) {
        Some(n) if n >= 1 => Ok(n),
        _ => Err(invalid("`connection_limit` must be a positive integer", &v)),
    }
}

fn parse_timeout(raw: &str) -> Option<PoolTimeout> {
    let v = percent_decode(raw);
    let d = digits(&v)?;
    // Digits that overflow `u64` are a wait longer than any clock: no limit.
    Some(match d.parse::<u64>() {
        Ok(0) | Err(_) => PoolTimeout::Unlimited,
        Ok(n) => PoolTimeout::Seconds(n),
    })
}

/// Splits `url` into the URL for the driver and the parameters read from it.
///
/// The query string is cut at the first `?` and on `&`. A pair is removed when its key is exactly
/// `connection_limit` or `pool_timeout`; the others are copied byte for byte, in order, and the
/// `?` disappears when nothing remains. Values are percent-decoded; a repeated key keeps its last
/// value (what the original client does).
///
/// `connection_limit` must be an integer of at least 1. An invalid `pool_timeout` fails too,
/// except for SQLite, where the original client accepts and ignores any value.
pub(crate) fn split_url_params(url: &str, dialect: Dialect) -> Result<(String, UrlParams)> {
    let url = url.trim();
    let Some((head, query)) = url.split_once('?') else {
        return Ok((url.to_string(), UrlParams::default()));
    };
    let mut params = UrlParams::default();
    let mut kept: Vec<&str> = Vec::new();
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match key {
            "connection_limit" => params.connection_limit = Some(parse_limit(value)?),
            "pool_timeout" => match parse_timeout(value) {
                Some(t) => params.pool_timeout = Some(t),
                None if dialect == Dialect::Sqlite => params.pool_timeout = None,
                None => return Err(invalid("`pool_timeout` must be a whole number of seconds", &percent_decode(value))),
            },
            _ => kept.push(pair),
        }
    }
    let driver = if kept.is_empty() { head.to_string() } else { format!("{head}?{}", kept.join("&")) };
    Ok((driver, params))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pg(url: &str) -> Result<(String, UrlParams)> {
        split_url_params(url, Dialect::Postgres)
    }
    fn sq(url: &str) -> Result<(String, UrlParams)> {
        split_url_params(url, Dialect::Sqlite)
    }
    fn msg(r: Result<(String, UrlParams)>) -> String {
        r.unwrap_err().to_string()
    }

    #[test]
    fn pool_no_query_string() {
        let (u, p) = pg("postgresql://u:p@h/db").unwrap();
        assert_eq!(u, "postgresql://u:p@h/db");
        assert_eq!(p, UrlParams::default());
    }

    #[test]
    fn pool_only_pool_params_drop_the_question_mark() {
        let (u, p) = pg("postgresql://h/db?connection_limit=2&pool_timeout=7").unwrap();
        assert_eq!(u, "postgresql://h/db");
        assert_eq!(p.connection_limit, Some(2));
        assert_eq!(p.pool_timeout, Some(PoolTimeout::Seconds(7)));
    }

    #[test]
    fn pool_other_params_keep_order_and_encoding() {
        let (u, p) =
            pg("postgresql://h/db?sslmode=require&connection_limit=3&application_name=a%20b&options=-c%20search_path%3Dx&pool_timeout=0")
                .unwrap();
        assert_eq!(u, "postgresql://h/db?sslmode=require&application_name=a%20b&options=-c%20search_path%3Dx");
        assert_eq!(p.connection_limit, Some(3));
        assert_eq!(p.pool_timeout, Some(PoolTimeout::Unlimited));
    }

    #[test]
    fn pool_similar_keys_are_not_removed() {
        let (u, p) = pg("postgresql://h/db?x_connection_limit=1&connection_limits=2&pool_timeouts=3").unwrap();
        assert_eq!(u, "postgresql://h/db?x_connection_limit=1&connection_limits=2&pool_timeouts=3");
        assert_eq!(p, UrlParams::default());
    }

    #[test]
    fn pool_sqlite_urls() {
        let (u, p) = sq("file:./dev.db?connection_limit=1").unwrap();
        assert_eq!((u.as_str(), p.connection_limit), ("file:./dev.db", Some(1)));
        let (u, p) = sq("sqlite://dev.db?connection_limit=1").unwrap();
        assert_eq!((u.as_str(), p.connection_limit), ("sqlite://dev.db", Some(1)));
        let (u, p) = sq("file::memory:?connection_limit=4").unwrap();
        assert_eq!((u.as_str(), p.connection_limit), ("file::memory:", Some(4)));
        let (u, _) = sq("file:./dev.db?mode=memory&connection_limit=4").unwrap();
        assert_eq!(u, "file:./dev.db?mode=memory");
    }

    #[test]
    fn pool_percent_encoded_value() {
        assert_eq!(pg("postgresql://h/db?connection_limit=%32").unwrap().1.connection_limit, Some(2));
        assert_eq!(pg("postgresql://h/db?connection_limit=1%30").unwrap().1.connection_limit, Some(10));
    }

    #[test]
    fn pool_repeated_key_last_wins() {
        let p = pg("postgresql://h/db?connection_limit=2&connection_limit=5").unwrap().1;
        assert_eq!(p.connection_limit, Some(5));
        let p = pg("postgresql://h/db?connection_limit=5&connection_limit=2").unwrap().1;
        assert_eq!(p.connection_limit, Some(2));
        let p = pg("postgresql://h/db?pool_timeout=5&pool_timeout=1").unwrap().1;
        assert_eq!(p.pool_timeout, Some(PoolTimeout::Seconds(1)));
    }

    #[test]
    fn pool_timeout_zero_is_unlimited() {
        assert_eq!(pg("postgresql://h/db?pool_timeout=0").unwrap().1.pool_timeout, Some(PoolTimeout::Unlimited));
    }

    #[test]
    fn pool_invalid_connection_limit() {
        for v in ["0", "-1", "abc", "1.5", "", "4294967296", "+1", " 1"] {
            let m = msg(pg(&format!("postgresql://h/db?connection_limit={v}")));
            assert!(m.contains("`connection_limit`") && m.contains(&format!("got `{v}`")), "{v}: {m}");
            assert!(m.starts_with("The provided database string is invalid."), "{m}");
            assert!(sq(&format!("file:./a.db?connection_limit={v}")).is_err(), "{v}");
        }
        let e = pg("postgresql://h/db?connection_limit=0").unwrap_err();
        assert_eq!(e.code(), Some("P1013"));
    }

    #[test]
    fn pool_invalid_pool_timeout() {
        for v in ["-1", "1.5", "abc", ""] {
            let m = msg(pg(&format!("postgresql://h/db?pool_timeout={v}")));
            assert!(m.contains("`pool_timeout`") && m.contains(&format!("got `{v}`")), "{v}: {m}");
            // The original client ignores any pool_timeout on SQLite.
            let (u, p) = sq(&format!("file:./a.db?pool_timeout={v}")).unwrap();
            assert_eq!((u.as_str(), p.pool_timeout), ("file:./a.db", None));
        }
    }

    #[test]
    fn pool_error_never_echoes_the_url() {
        let m = msg(pg("postgresql://app:secret@host/db?connection_limit=abc"));
        assert!(m.contains("connection_limit") && m.contains("abc"), "{m}");
        for leak in ["app", "secret", "host"] {
            assert!(!m.contains(leak), "{leak} leaked in {m}");
        }
        let m = msg(pg("postgresql://app:secret@host/db?pool_timeout=oops"));
        assert!(!m.contains("secret") && !m.contains("app:"), "{m}");
    }

    #[test]
    fn pool_error_value_is_bounded() {
        let long = "x".repeat(500);
        let m = msg(pg(&format!("postgresql://h/db?connection_limit={long}")));
        assert!(m.len() < 200, "{m}");
    }

    #[test]
    fn pool_huge_pool_timeout_is_unlimited_not_a_panic() {
        let p = pg("postgresql://h/db?pool_timeout=99999999999999999999999").unwrap().1;
        assert_eq!(p.pool_timeout, Some(PoolTimeout::Unlimited));
    }
}
