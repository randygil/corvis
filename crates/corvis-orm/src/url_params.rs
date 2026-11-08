//! The one place that reads the corvis-specific query parameters of the database URL
//! (`connection_limit`, `pool_timeout`, `connect_timeout`, `schema`). They are removed before the URL reaches the driver,
//! which would ignore them (Postgres) or reject them (SQLite); every other pair is kept as is.

use crate::db::Dialect;
use crate::error::{InitializationError, OrmError, Result};

/// `pool_timeout` and `connect_timeout`: whole seconds, `0` meaning no limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Timeout {
    Seconds(u64),
    Unlimited,
}

/// Parameters read from the query string of the database URL.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct UrlParams {
    pub(crate) connection_limit: Option<u32>,
    pub(crate) pool_timeout: Option<Timeout>,
    /// Postgres only; ignored (and never invalid) on SQLite.
    pub(crate) connect_timeout: Option<Timeout>,
    /// Postgres schema, percent-decoded (`?schema=tenant_a`). Ignored on SQLite.
    pub(crate) schema: Option<String>,
}

/// Longest schema name: `NAMEDATALEN - 1` bytes on a default Postgres server.
const MAX_SCHEMA_BYTES: usize = 63;

/// The value is not echoed: unlike a number, a schema name can carry arbitrary text.
fn invalid_schema() -> OrmError {
    OrmError::Initialization(InitializationError {
        message: "The provided database string is invalid. `schema` must be a non-empty schema name of at most 63 bytes, without NUL \
                  characters, other than `$user` and not starting with `pg_`."
            .into(),
        error_code: Some("P1013".into()),
    })
}

fn parse_schema(raw: &str) -> Result<String> {
    let v = percent_decode(raw);
    // `"$user"` is the server's special token in a search path, and `pg_` names are reserved
    // (`pg_temp` would send every object to temporary tables).
    if v.is_empty() || v.len() > MAX_SCHEMA_BYTES || v.contains('\0') || v == "$user" || v.starts_with("pg_") {
        return Err(invalid_schema());
    }
    Ok(v)
}

fn invalid(what: &str, value: &str) -> OrmError {
    // The value is echoed only when it is short and made of plain characters: a parameter key can
    // appear inside an unencoded password (`...:S3cr?connection_limit=etPw@host/db`), and the rest of
    // the URL is never shown.
    let plain = value.len() <= 40 && value.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'.' | b'_' | b'-'));
    let message = if plain {
        format!("The provided database string is invalid. {what}, got `{value}`.")
    } else {
        format!("The provided database string is invalid. {what}, got an invalid value.")
    };
    OrmError::Initialization(InitializationError { message, error_code: Some("P1013".into()) })
}

/// Form decoding of a key or a value: `%XX` with two ASCII hex digits, and `+` as a space.
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() && b[i + 1].is_ascii_hexdigit() && b[i + 2].is_ascii_hexdigit() => {
                let hex = std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("00");
                out.push(u8::from_str_radix(hex, 16).unwrap_or(0));
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Unsigned decimal digits only (no sign, no fraction, no spaces).
fn digits(v: &str) -> Option<&str> {
    (!v.is_empty() && v.bytes().all(|b| b.is_ascii_digit())).then_some(v)
}

/// Largest pool size, the largest value Postgres accepts for its own `max_connections`. sqlx
/// preallocates its idle queue with one slot per connection, so an unbounded size would abort the
/// process while allocating.
pub(crate) const MAX_POOL_SIZE: u32 = 262_143;

fn parse_limit(raw: &str) -> Result<u32> {
    let v = percent_decode(raw);
    match digits(&v).and_then(|d| d.parse::<u32>().ok()) {
        Some(n) if (1..=MAX_POOL_SIZE).contains(&n) => Ok(n),
        _ => Err(invalid("`connection_limit` must be a positive integer no greater than 262143", &v)),
    }
}

fn parse_timeout(raw: &str) -> Option<Timeout> {
    let v = percent_decode(raw);
    let d = digits(&v)?;
    // Digits that overflow `u64` are a wait longer than any clock: no limit.
    Some(match d.parse::<u64>() {
        Ok(0) | Err(_) => Timeout::Unlimited,
        Ok(n) => Timeout::Seconds(n),
    })
}

/// Splits `url` into the URL for the driver and the parameters read from it.
///
/// The query string starts at the first `?`, ends at the first `#` (the fragment is kept and never read
/// as parameters) and is cut on `&`. A pair is removed when its percent-decoded key is exactly
/// `connection_limit`, `pool_timeout`, `connect_timeout` or `schema`; the others are copied byte for byte, in order, and the
/// `?` disappears when nothing remains. Values are decoded as form data (`%XX` with two hex digits,
/// `+` for a space); a repeated key keeps its last value (what the original client does).
///
/// `connection_limit` must be an integer of at least 1. An invalid `pool_timeout` or `connect_timeout` fails too,
/// except for SQLite, where the original client accepts and ignores any value. `schema` must be 1 to
/// 63 bytes without NUL characters on both providers (SQLite then ignores it).
pub(crate) fn split_url_params(url: &str, dialect: Dialect) -> Result<(String, UrlParams)> {
    let url = url.trim();
    // The query ends at the first `#`; the fragment is never read as parameters and is kept as given.
    let (url_no_fragment, fragment) = match url.find('#') {
        Some(i) => url.split_at(i),
        None => (url, ""),
    };
    let Some((head, query)) = url_no_fragment.split_once('?') else {
        return Ok((url.to_string(), UrlParams::default()));
    };
    let mut params = UrlParams::default();
    let mut kept: Vec<&str> = Vec::new();
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        // Keys are decoded before matching (`schem%61` is `schema`), as the original client does.
        let key = percent_decode(key);
        match key.as_str() {
            "connection_limit" => params.connection_limit = Some(parse_limit(value)?),
            "schema" => params.schema = Some(parse_schema(value)?),
            "pool_timeout" => match parse_timeout(value) {
                Some(t) => params.pool_timeout = Some(t),
                None if dialect == Dialect::Sqlite => params.pool_timeout = None,
                None => return Err(invalid("`pool_timeout` must be a whole number of seconds", &percent_decode(value))),
            },
            "connect_timeout" => match parse_timeout(value) {
                Some(t) => params.connect_timeout = Some(t),
                None if dialect == Dialect::Sqlite => params.connect_timeout = None,
                None => return Err(invalid("`connect_timeout` must be a whole number of seconds", &percent_decode(value))),
            },
            _ => kept.push(pair),
        }
    }
    let driver = if kept.is_empty() { format!("{head}{fragment}") } else { format!("{head}?{}{fragment}", kept.join("&")) };
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
        assert_eq!(p.pool_timeout, Some(Timeout::Seconds(7)));
    }

    #[test]
    fn pool_other_params_keep_order_and_encoding() {
        let (u, p) =
            pg("postgresql://h/db?sslmode=require&connection_limit=3&application_name=a%20b&options=-c%20search_path%3Dx&pool_timeout=0")
                .unwrap();
        assert_eq!(u, "postgresql://h/db?sslmode=require&application_name=a%20b&options=-c%20search_path%3Dx");
        assert_eq!(p.connection_limit, Some(3));
        assert_eq!(p.pool_timeout, Some(Timeout::Unlimited));
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
        assert_eq!(p.pool_timeout, Some(Timeout::Seconds(1)));
    }

    #[test]
    fn pool_timeout_zero_is_unlimited() {
        assert_eq!(pg("postgresql://h/db?pool_timeout=0").unwrap().1.pool_timeout, Some(Timeout::Unlimited));
    }

    #[test]
    fn pool_invalid_connection_limit() {
        for v in ["0", "-1", "abc", "1.5", "", "4294967296", "+1", " 1"] {
            let m = msg(pg(&format!("postgresql://h/db?connection_limit={v}")));
            // `+1` decodes to ` 1` (a space), which is not shown.
            let shown = if v.starts_with('+') || v.starts_with(' ') { "got an invalid value".to_string() } else { format!("got `{v}`") };
            assert!(m.contains("`connection_limit`") && m.contains(&shown), "{v}: {m}");
            assert!(m.starts_with("The provided database string is invalid."), "{m}");
            assert!(sq(&format!("file:./a.db?connection_limit={v}")).is_err(), "{v}");
        }
        let e = pg("postgresql://h/db?connection_limit=0").unwrap_err();
        assert_eq!(e.code(), Some("P1013"));
    }

    #[test]
    fn pool_connection_limit_upper_bound() {
        assert_eq!(pg("postgresql://h/db?connection_limit=262143").unwrap().1.connection_limit, Some(MAX_POOL_SIZE));
        for v in ["262144", "4294967295", "4294967296"] {
            let e = pg(&format!("postgresql://h/db?connection_limit={v}")).unwrap_err();
            assert_eq!(e.code(), Some("P1013"), "{v}");
            let m = e.to_string();
            assert!(m.contains("no greater than 262143") && m.contains(&format!("got `{v}`")), "{v}: {m}");
            assert!(sq(&format!("file:./a.db?connection_limit={v}")).is_err(), "{v}");
        }
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
    fn schema_only_drops_the_question_mark() {
        let (u, p) = pg("postgresql://h/db?schema=other").unwrap();
        assert_eq!(u, "postgresql://h/db");
        assert_eq!(p.schema.as_deref(), Some("other"));
    }

    #[test]
    fn schema_mixed_with_other_params_keeps_them() {
        let (u, p) = pg("postgresql://h/db?sslmode=require&schema=s&application_name=a%20b&options=-c%20x%3Dy&connection_limit=2").unwrap();
        assert_eq!(u, "postgresql://h/db?sslmode=require&application_name=a%20b&options=-c%20x%3Dy");
        assert_eq!(p.schema.as_deref(), Some("s"));
        assert_eq!(p.connection_limit, Some(2));
    }

    #[test]
    fn schema_is_percent_decoded() {
        assert_eq!(pg("postgresql://h/db?schema=My%20Schema").unwrap().1.schema.as_deref(), Some("My Schema"));
        assert_eq!(pg("postgresql://h/db?schema=a%22b").unwrap().1.schema.as_deref(), Some("a\"b"));
    }

    #[test]
    fn schema_repeated_key_last_wins() {
        assert_eq!(pg("postgresql://h/db?schema=a&schema=b").unwrap().1.schema.as_deref(), Some("b"));
    }

    #[test]
    fn schema_similar_keys_are_not_removed() {
        let (u, p) = pg("postgresql://h/db?schemas=a&x_schema=b").unwrap();
        assert_eq!(u, "postgresql://h/db?schemas=a&x_schema=b");
        assert_eq!(p.schema, None);
    }

    #[test]
    fn schema_is_stripped_from_sqlite_urls() {
        let (u, p) = sq("sqlite://dev.db?schema=main").unwrap();
        assert_eq!((u.as_str(), p.schema.as_deref()), ("sqlite://dev.db", Some("main")));
        let (u, _) = sq("file:./dev.db?schema=main").unwrap();
        assert_eq!(u, "file:./dev.db");
        let (u, _) = sq("file:./dev.db?mode=memory&schema=main").unwrap();
        assert_eq!(u, "file:./dev.db?mode=memory");
    }

    #[test]
    fn schema_length_limits() {
        let ok = "a".repeat(63);
        assert_eq!(pg(&format!("postgresql://h/db?schema={ok}")).unwrap().1.schema.as_deref(), Some(ok.as_str()));
        let long = "a".repeat(64);
        let e = pg(&format!("postgresql://h/db?schema={long}")).unwrap_err();
        assert_eq!(e.code(), Some("P1013"));
        assert!(e.to_string().contains("`schema`"), "{e}");
        // The 63 bytes are counted after decoding: 32 two-byte characters are 64 bytes.
        let wide = "%C3%A9".repeat(32);
        assert!(pg(&format!("postgresql://h/db?schema={wide}")).is_err());
        let wide = "%C3%A9".repeat(31);
        assert!(pg(&format!("postgresql://h/db?schema={wide}")).is_ok());
    }

    #[test]
    fn schema_empty_and_nul_are_rejected() {
        for v in ["", "a%00b", "%00"] {
            let e = pg(&format!("postgresql://h/db?schema={v}")).unwrap_err();
            assert_eq!(e.code(), Some("P1013"), "{v}");
            assert!(e.to_string().contains("`schema`"), "{v}: {e}");
            assert!(sq(&format!("file:./a.db?schema={v}")).is_err(), "{v}");
        }
        assert!(pg("postgresql://h/db?schema").is_err());
    }

    #[test]
    fn schema_error_never_echoes_the_url_or_the_value() {
        let m = msg(pg("postgresql://app:secret@host/db?schema="));
        for leak in ["app", "secret", "host"] {
            assert!(!m.contains(leak), "{leak} leaked in {m}");
        }
        let long = format!("topsecretname{}", "z".repeat(60));
        let m = msg(pg(&format!("postgresql://app:secret@host/db?schema={long}")));
        assert!(!m.contains("topsecretname") && !m.contains("secret@"), "{m}");
    }

    #[test]
    fn error_does_not_echo_part_of_a_password() {
        // An unencoded password that contains a parameter key.
        let m = msg(pg("postgres://app:S3cr?connection_limit=etPw%40host@127.0.0.1:55432/postgres"));
        assert!(m.contains("`connection_limit`") && m.contains("got an invalid value"), "{m}");
        for leak in ["etPw", "S3cr", "host", "127.0.0.1"] {
            assert!(!m.contains(leak), "{leak} leaked in {m}");
        }
        let m = msg(pg("postgres://app:S3cr?pool_timeout=etPw%40host@127.0.0.1:55432/postgres"));
        assert!(m.contains("`pool_timeout`") && !m.contains("etPw"), "{m}");
        // Plain short values are still shown; long ones are not.
        assert!(msg(pg("postgresql://h/db?connection_limit=abc")).contains("got `abc`"));
        let long = "a".repeat(41);
        assert!(msg(pg(&format!("postgresql://h/db?connection_limit={long}"))).contains("got an invalid value"));
    }

    #[test]
    fn query_ends_at_the_fragment() {
        let (u, p) = pg("postgresql://h/db?sslmode=disable#schema=x&connection_limit=3").unwrap();
        assert_eq!(u, "postgresql://h/db?sslmode=disable#schema=x&connection_limit=3");
        assert_eq!(p, UrlParams::default());
        let (u, p) = pg("postgresql://h/db?schema=a#frag").unwrap();
        assert_eq!((u.as_str(), p.schema.as_deref()), ("postgresql://h/db#frag", Some("a")));
        // A `?` inside the fragment starts no query.
        let (u, p) = pg("postgresql://h/db#a?schema=x").unwrap();
        assert_eq!((u.as_str(), p.schema), ("postgresql://h/db#a?schema=x", None));
        // An invalid value in the fragment is not validated.
        assert!(pg("postgresql://h/db?x=1#connection_limit=0").is_ok());
    }

    #[test]
    fn keys_are_percent_decoded_before_matching() {
        let (u, p) = pg("postgresql://h/db?schem%61=t&connection%5Flimit=5&pool%5ftimeout=7&sslmode=disable").unwrap();
        assert_eq!(u, "postgresql://h/db?sslmode=disable");
        assert_eq!(p.schema.as_deref(), Some("t"));
        assert_eq!(p.connection_limit, Some(5));
        assert_eq!(p.pool_timeout, Some(Timeout::Seconds(7)));
        // A decoded key that is not one of ours is copied as written.
        let (u, _) = pg("postgresql://h/db?a%20b=1").unwrap();
        assert_eq!(u, "postgresql://h/db?a%20b=1");
    }

    #[test]
    fn plus_is_a_space_in_values() {
        assert_eq!(pg("postgresql://h/db?schema=My+Schema").unwrap().1.schema.as_deref(), Some("My Schema"));
        assert_eq!(pg("postgresql://h/db?schema=a%2Bb").unwrap().1.schema.as_deref(), Some("a+b"));
        assert!(pg("postgresql://h/db?connection_limit=+1").is_err());
    }

    #[test]
    fn percent_needs_two_ascii_hex_digits() {
        // `%+1` is not an escape (from_str_radix would accept the sign): the `%` stays, the `+` is a space.
        assert_eq!(percent_decode("%+1"), "% 1");
        assert_eq!(percent_decode("%zz"), "%zz");
        assert_eq!(percent_decode("%4"), "%4");
        assert_eq!(percent_decode("%-1"), "%-1");
        assert_eq!(percent_decode("%41"), "A");
        assert_eq!(pg("postgresql://h/db?schema=a%+1").unwrap().1.schema.as_deref(), Some("a% 1"));
    }

    #[test]
    fn special_schema_names_are_rejected() {
        for v in ["%24user", "$user", "pg_temp", "pg_catalog", "pg_x", "pg_toast_temp_1"] {
            let e = pg(&format!("postgresql://h/db?schema={v}")).unwrap_err();
            assert_eq!(e.code(), Some("P1013"), "{v}");
            assert!(e.to_string().contains("`schema`"), "{v}: {e}");
        }
        // Only the exact token and the lowercase prefix are reserved.
        for v in ["user", "$users", "PG_x", "pgx", "mypg_x"] {
            assert!(pg(&format!("postgresql://h/db?schema={v}")).is_ok(), "{v}");
        }
    }

    #[test]
    fn connect_timeout_is_removed_with_other_params() {
        let (u, p) = pg("postgresql://h/db?sslmode=disable&connect_timeout=3&schema=s&application_name=a%20b").unwrap();
        assert_eq!(u, "postgresql://h/db?sslmode=disable&application_name=a%20b");
        assert_eq!(p.connect_timeout, Some(Timeout::Seconds(3)));
        let (u, p) = pg("postgresql://h/db?connect_timeout=2").unwrap();
        assert_eq!((u.as_str(), p.connect_timeout), ("postgresql://h/db", Some(Timeout::Seconds(2))));
        assert_eq!(pg("postgresql://h/db").unwrap().1.connect_timeout, None);
    }

    #[test]
    fn connect_timeout_zero_overflow_and_repeats() {
        assert_eq!(pg("postgresql://h/db?connect_timeout=0").unwrap().1.connect_timeout, Some(Timeout::Unlimited));
        assert_eq!(pg("postgresql://h/db?connect_timeout=99999999999999999999").unwrap().1.connect_timeout, Some(Timeout::Unlimited));
        assert_eq!(pg("postgresql://h/db?connect_timeout=1&connect_timeout=3").unwrap().1.connect_timeout, Some(Timeout::Seconds(3)));
        assert_eq!(pg("postgresql://h/db?connect_timeout=3&connect_timeout=1").unwrap().1.connect_timeout, Some(Timeout::Seconds(1)));
        let (u, p) = pg("postgresql://h/db?connect%5Ftimeout=4&pool%5Ftimeout=7").unwrap();
        assert_eq!(
            (u.as_str(), p.connect_timeout, p.pool_timeout),
            ("postgresql://h/db", Some(Timeout::Seconds(4)), Some(Timeout::Seconds(7)))
        );
    }

    #[test]
    fn connect_timeout_similar_keys_are_not_removed() {
        let (u, p) = pg("postgresql://h/db?x_connect_timeout=1&connect_timeouts=2").unwrap();
        assert_eq!(u, "postgresql://h/db?x_connect_timeout=1&connect_timeouts=2");
        assert_eq!(p, UrlParams::default());
    }

    #[test]
    fn connect_timeout_invalid_values_fail_on_postgres() {
        for v in ["-1", "1.5", "abc", ""] {
            let e = pg(&format!("postgresql://h/db?connect_timeout={v}")).unwrap_err();
            assert_eq!(e.code(), Some("P1013"), "{v}");
            let m = e.to_string();
            assert!(m.starts_with("The provided database string is invalid."), "{m}");
            assert!(m.contains("`connect_timeout`") && m.contains(&format!("got `{v}`")), "{v}: {m}");
        }
        let long = "a".repeat(41);
        assert!(msg(pg(&format!("postgresql://h/db?connect_timeout={long}"))).contains("got an invalid value"));
    }

    #[test]
    fn connect_timeout_error_never_echoes_the_url() {
        let m = msg(pg("postgresql://app:secret@host/db?connect_timeout=abc"));
        assert!(m.contains("connect_timeout") && m.contains("abc"), "{m}");
        for leak in ["app", "secret", "host"] {
            assert!(!m.contains(leak), "{leak} leaked in {m}");
        }
        let m = msg(pg("postgres://app:S3cr?connect_timeout=etPw%40host@127.0.0.1:55432/postgres"));
        assert!(m.contains("`connect_timeout`") && !m.contains("etPw") && !m.contains("S3cr"), "{m}");
    }

    #[test]
    fn connect_timeout_is_accepted_and_ignored_on_sqlite() {
        for v in ["1", "0", "abc", "-1", "", "99999999999999999999"] {
            let (u, p) = sq(&format!("file:./a.db?connect_timeout={v}")).unwrap();
            assert_eq!(u, "file:./a.db", "{v}");
            assert!(p.connect_timeout.is_none() || v.chars().all(|c| c.is_ascii_digit()), "{v}");
        }
        let (u, _) = sq("sqlite://dev.db?connect_timeout=abc&mode=rwc").unwrap();
        assert_eq!(u, "sqlite://dev.db?mode=rwc");
    }

    #[test]
    fn pool_huge_pool_timeout_is_unlimited_not_a_panic() {
        let p = pg("postgresql://h/db?pool_timeout=99999999999999999999999").unwrap().1;
        assert_eq!(p.pool_timeout, Some(Timeout::Unlimited));
    }
}
