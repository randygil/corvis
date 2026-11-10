//! Redaction of query parameters that carry credentials, for request logs.

use super::body_parser::QueryParser;
use super::request::{Request, parse_query_with};

/// What replaces the value of a marked parameter.
const REDACTED: &str = "REDACTED";

/// Names marked per request before every value of the query string is hidden instead.
pub const MAX_REDACTED_PARAMS: usize = 16;

/// Names registered for the whole application before every value of every query string is hidden instead.
const MAX_APP_PARAMS: usize = 64;

/// The longest name that can be marked; a longer one hides every value.
const MAX_NAME_BYTES: usize = 256;

/// The parser's parameter limit (`maxKeys` of `node:querystring`, `parameterLimit` of `qs`). A query string with more
/// pairs has every value hidden without being parsed.
const MAX_PAIRS: usize = 1000;

/// The parameters of one request whose values must not be logged, shared by every clone of the request.
#[derive(Clone, Default)]
struct RedactedParams {
    names: Vec<String>,
    /// More names than [`MAX_REDACTED_PARAMS`] (or a name too long) were marked: hide every value.
    all: bool,
}

/// The query parameters whose values are hidden in the log line of every request: the names the strategies of the
/// application read credentials from, registered once at startup. Built before the first request and never changed,
/// so reading it needs no lock.
#[derive(Debug, Clone, Default)]
pub(crate) struct AppRedaction {
    names: Vec<String>,
    all: bool,
}

impl AppRedaction {
    /// The set of `names`. Past 64 names, or for a name over 256 bytes, every value is hidden.
    pub(crate) fn new(names: impl IntoIterator<Item = String>) -> Self {
        let mut set = AppRedaction::default();
        for name in names {
            if name.len() > MAX_NAME_BYTES || set.names.len() >= MAX_APP_PARAMS {
                set.all = true;
            } else if !set.names.contains(&name) {
                set.names.push(name);
            }
        }
        set
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.names.is_empty() && !self.all
    }
}

impl Request {
    /// Marks the query parameter `name` of this request as sensitive: [`Request::redacted_url`] replaces its values
    /// with `REDACTED`. A guard of your own that reads a credential from the query string calls this before it reads
    /// anything, so the mark is there whatever the outcome (the strategies of the framework register their names for
    /// the whole application at startup instead). At most 16 names are kept; past that (or for a name over 256
    /// bytes) `redacted_url` hides the value of every parameter rather than leave one visible.
    pub fn redact_query_param(&self, name: &str) {
        let mut extensions = self.extensions.write().unwrap_or_else(|e| e.into_inner());
        if extensions.get::<RedactedParams>().is_none() {
            extensions.insert(RedactedParams::default());
        }
        let marks = extensions.get_mut::<RedactedParams>().expect("just inserted");
        if name.len() > MAX_NAME_BYTES {
            marks.all = true;
        } else if !marks.names.iter().any(|n| n == name) {
            if marks.names.len() >= MAX_REDACTED_PARAMS {
                marks.all = true;
            } else {
                marks.names.push(name.to_string());
            }
        }
    }

    /// Sets the names registered for the whole application (see [`AppRedaction`]).
    #[cfg(test)]
    pub(crate) fn with_app_redaction(mut self, app: Option<std::sync::Arc<AppRedaction>>) -> Self {
        self.app_redaction = app.filter(|a| !a.is_empty());
        self
    }

    /// [`Request::url`] with the value of every marked parameter replaced by `REDACTED` (see
    /// [`Request::redact_query_param`]), the form to write in logs. A parameter counts as marked when the query
    /// parser of the application files it under that name, so percent-encoded names, repeated parameters and
    /// bracketed forms such as `name[x]=v` are covered. The names the strategies of the application read credentials
    /// from count as marked on every request. With nothing marked this is exactly `url()`. A query string with more
    /// than 1000 pairs has every value hidden.
    ///
    /// Only the value of the parameter is hidden, and only here: a middleware that logs `url()`, a proxy, a browser
    /// history or a `Referer` header still sees it.
    pub fn redacted_url(&self) -> String {
        let Some(query) = self.uri.query() else { return self.url() };
        let guard = self.extensions.read().unwrap_or_else(|e| e.into_inner());
        let marks = guard.get::<RedactedParams>();
        let app = self.app_redaction.as_deref();
        let request_names: &[String] = marks.map_or(&[], |m| &m.names);
        let app_names: &[String] = app.map_or(&[], |a| &a.names);
        let all = marks.is_some_and(|m| m.all) || app.is_some_and(|a| a.all);
        if request_names.is_empty() && app_names.is_empty() && !all {
            return self.url();
        }
        // `Disabled` leaves `query()` empty; the URL is still split like the default parser.
        let mode = match self.query_parser {
            QueryParser::Disabled => QueryParser::Simple,
            mode => mode,
        };
        let names = Names { request: request_names, app: app_names };
        // More pairs than the parser reads: hide everything without parsing anything.
        let all = all || query.bytes().filter(|&b| b == b'&').count() >= MAX_PAIRS;
        let path = self.uri.path();
        let mut out = String::with_capacity(path.len() + 1 + query.len() + 64);
        out.push_str(path);
        out.push('?');
        for (i, pair) in query.split('&').enumerate() {
            if i > 0 {
                out.push('&');
            }
            redact_pair(pair, &names, all, mode, &mut out);
        }
        out
    }
}

/// The registered names of a request: its own marks and the application's.
struct Names<'a> {
    request: &'a [String],
    app: &'a [String],
}

impl Names<'_> {
    fn all(&self) -> impl Iterator<Item = &String> {
        self.request.iter().chain(self.app)
    }

    /// The registered name a raw name (as written in the URL) decodes to.
    fn find(&self, raw: &str) -> Option<&str> {
        self.all().find(|n| decode(raw.as_bytes()).eq(n.bytes())).map(String::as_str)
    }
}

/// Percent-decoding of a form-encoded name, byte by byte and without allocating (`+` is a space, a `%` that is not
/// followed by two hex digits stays as it is).
fn decode(raw: &[u8]) -> impl Iterator<Item = u8> + '_ {
    let mut i = 0;
    std::iter::from_fn(move || {
        let b = *raw.get(i)?;
        i += 1;
        Some(match b {
            b'+' => b' ',
            b'%' => match (raw.get(i).and_then(|c| hex(*c)), raw.get(i + 1).and_then(|c| hex(*c))) {
                (Some(high), Some(low)) => {
                    i += 2;
                    high * 16 + low
                }
                _ => b'%',
            },
            b => b,
        })
    })
}

fn hex(b: u8) -> Option<u8> {
    (b as char).to_digit(16).map(|d| d as u8)
}

/// `true` when the raw name can be printed as it is: no brackets, plain or percent-encoded, which can carry data.
fn plain_name(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    !bytes.iter().any(|b| matches!(b, b'[' | b']'))
        && !bytes.windows(3).any(|w| w[0] == b'%' && w[1] == b'5' && matches!(w[2], b'b' | b'B' | b'd' | b'D'))
}

/// Writes `pair` with its value hidden when its name is registered (or `all` is set).
fn redact_pair(pair: &str, names: &Names, all: bool, mode: QueryParser, out: &mut String) {
    let Some((raw_name, _value)) = pair.split_once('=') else {
        out.push_str(pair);
        return;
    };
    let plain = plain_name(raw_name);
    let hit: Option<&str> = if all {
        Some("param")
    } else if plain || mode != QueryParser::Extended {
        // Without brackets, or with the parser that keeps them literal, the name is what the URL spells.
        names.find(raw_name)
    } else {
        // Only here is the parser of the application asked: it files `a[x]` and `[a]` under `a`.
        let parsed = parse_query_with(pair, mode);
        names.all().find(|n| parsed.contains_key(n.as_str())).map(String::as_str)
    };
    match hit {
        None => out.push_str(pair),
        Some(_) if plain => {
            out.push_str(raw_name);
            out.push('=');
            out.push_str(REDACTED);
        }
        // A bracketed name can carry data of its own: print the registered name it was filed under.
        Some(name) => {
            out.push_str(name);
            out.push('=');
            out.push_str(REDACTED);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bytes::Bytes;
    use http::{HeaderMap, Method};

    use super::*;
    use crate::http::body_parser::BodyParserOptions;

    fn request(url: &str, mode: QueryParser) -> Request {
        let parser = BodyParserOptions { query_parser: mode, ..BodyParserOptions::default() };
        Request::with_parser(Method::GET, url.parse().unwrap(), HeaderMap::new(), Bytes::new(), &parser)
    }

    fn redacted(url: &str, names: &[&str]) -> String {
        redacted_with(url, names, QueryParser::Simple)
    }

    fn redacted_with(url: &str, names: &[&str], mode: QueryParser) -> String {
        let req = request(url, mode);
        for n in names {
            req.redact_query_param(n);
        }
        req.redacted_url()
    }

    #[test]
    fn nothing_marked_is_the_plain_url() {
        for url in ["/data", "/data?a=1&api_key=secret", "/data?", "/"] {
            let req = request(url, QueryParser::Simple);
            assert_eq!(req.redacted_url(), req.url(), "{url}");
        }
    }

    #[test]
    fn the_value_of_a_marked_parameter_is_hidden() {
        assert_eq!(redacted("/data?page=2&api_key=secret", &["api_key"]), "/data?page=2&api_key=REDACTED");
        assert_eq!(redacted("/data?api_key=secret&page=2", &["api_key"]), "/data?api_key=REDACTED&page=2");
        assert_eq!(redacted("/data?api_key=", &["api_key"]), "/data?api_key=REDACTED");
        assert_eq!(redacted("/data?api_key", &["api_key"]), "/data?api_key");
        assert_eq!(redacted("/data?page=2", &["api_key"]), "/data?page=2");
        assert_eq!(redacted("/data", &["api_key"]), "/data");
        assert_eq!(redacted("/data?a=1&b=2", &["a", "b"]), "/data?a=REDACTED&b=REDACTED");
    }

    #[test]
    fn encoded_names_repeats_and_plus_signs_are_covered() {
        assert_eq!(redacted("/d?api%5Fkey=a&api_key=b", &["api_key"]), "/d?api%5Fkey=REDACTED&api_key=REDACTED");
        assert_eq!(redacted("/d?%61pi_key=a&%41PI_KEY=b", &["api_key"]), "/d?%61pi_key=REDACTED&%41PI_KEY=b");
        assert_eq!(redacted("/d?api_key=a&api_key=b&api_key=c", &["api_key"]), "/d?api_key=REDACTED&api_key=REDACTED&api_key=REDACTED");
        assert_eq!(redacted("/d?api+key=a", &["api key"]), "/d?api+key=REDACTED");
        assert_eq!(redacted("/d?api_key=a=b&x=1", &["api_key"]), "/d?api_key=REDACTED&x=1");
    }

    #[test]
    fn bracketed_forms_follow_the_query_parser() {
        // The extended parser files `api_key[x]` and `[api_key]` under `api_key`; the name is not echoed.
        assert_eq!(redacted_with("/d?api_key[x]=a&p=1", &["api_key"], QueryParser::Extended), "/d?api_key=REDACTED&p=1");
        assert_eq!(redacted_with("/d?api_key%5Bx%5D=a", &["api_key"], QueryParser::Extended), "/d?api_key=REDACTED");
        assert_eq!(redacted_with("/d?[api_key]=a", &["api_key"], QueryParser::Extended), "/d?api_key=REDACTED");
        // The simple parser keeps the key literal: `api_key[x]` is another parameter.
        assert_eq!(redacted_with("/d?api_key[x]=a", &["api_key"], QueryParser::Simple), "/d?api_key[x]=a");
        // With the query parser disabled the URL is still split like the default parser.
        assert_eq!(redacted_with("/d?api_key=a", &["api_key"], QueryParser::Disabled), "/d?api_key=REDACTED");
    }

    #[test]
    fn marks_are_shared_by_clones_and_deduplicated() {
        let req = request("/d?k=secret", QueryParser::Simple);
        let clone = req.clone();
        clone.redact_query_param("k");
        clone.redact_query_param("k");
        assert_eq!(req.redacted_url(), "/d?k=REDACTED");
        assert_eq!(req.get::<RedactedParams>().unwrap().names.len(), 1);
    }

    #[test]
    fn the_seventeenth_name_hides_every_value() {
        let req = request("/d?a=1&p17=secret&z=2", QueryParser::Simple);
        for i in 1..=16 {
            req.redact_query_param(&format!("p{i}"));
        }
        assert_eq!(req.redacted_url(), "/d?a=1&p17=secret&z=2", "sixteen names fit and none matches");
        req.redact_query_param("p17");
        assert_eq!(req.redacted_url(), "/d?a=REDACTED&p17=REDACTED&z=REDACTED");
        let long = request("/d?a=1", QueryParser::Simple);
        long.redact_query_param(&"n".repeat(300));
        assert_eq!(long.redacted_url(), "/d?a=REDACTED");
    }

    #[test]
    fn application_names_apply_to_every_request_without_a_mark() {
        let app = Some(Arc::new(AppRedaction::new(["api_key".to_string(), "token".to_string()])));
        let req = request("/d?page=1&api_key=SECRET&token=T", QueryParser::Simple).with_app_redaction(app.clone());
        assert_eq!(req.redacted_url(), "/d?page=1&api_key=REDACTED&token=REDACTED");
        // Together with a per-request mark.
        req.redact_query_param("page");
        assert_eq!(req.redacted_url(), "/d?page=REDACTED&api_key=REDACTED&token=REDACTED");
        // An empty set is the same as none: the plain URL.
        let empty = request("/d?api_key=SECRET", QueryParser::Simple).with_app_redaction(Some(Arc::new(AppRedaction::new([]))));
        assert_eq!(empty.redacted_url(), empty.url());
        assert!(empty.app_redaction.is_none());
        // A set that overflowed hides everything.
        let many = AppRedaction::new((0..65).map(|i| format!("n{i}")));
        let req = request("/d?a=1&b=2", QueryParser::Simple).with_app_redaction(Some(Arc::new(many)));
        assert_eq!(req.redacted_url(), "/d?a=REDACTED&b=REDACTED");
    }

    #[test]
    fn more_pairs_than_the_parser_reads_hide_every_value() {
        let mut query = "api_key=SECRET1&".to_string();
        query.push_str(&"a=SECRET2&".repeat(1100));
        query.push_str("b[x]=SECRET3");
        for mode in [QueryParser::Simple, QueryParser::Extended] {
            let out = redacted_with(&format!("/d?{query}"), &["api_key"], mode);
            assert!(!out.contains("SECRET"), "{mode:?}");
            assert!(out.ends_with("&param=REDACTED"), "a name that can carry data is not echoed: {mode:?}");
        }
        // Exactly 1000 pairs are still parsed pair by pair.
        let thousand = vec!["a=1"; 999].join("&") + "&api_key=SECRET";
        assert!(redacted(&format!("/d?{thousand}"), &["api_key"]).ends_with("&api_key=REDACTED"));
        assert!(redacted(&format!("/d?{thousand}"), &["api_key"]).starts_with("/d?a=1&a=1"));
    }

    /// The implementation this one replaced: one call of the query parser per pair. Kept as the reference.
    fn reference(req: &Request, names: &[&str]) -> String {
        let Some(query) = req.uri().query() else { return req.url() };
        if names.is_empty() {
            return req.url();
        }
        let mode = match req.query_parser {
            QueryParser::Disabled => QueryParser::Simple,
            mode => mode,
        };
        let plain = |raw: &str| {
            !raw.bytes().any(|b| matches!(b, b'[' | b']'))
                && !raw.to_ascii_lowercase().contains("%5b")
                && !raw.to_ascii_lowercase().contains("%5d")
        };
        let pairs: Vec<String> = query
            .split('&')
            .map(|pair| {
                let Some((raw_name, _)) = pair.split_once('=') else { return pair.to_string() };
                let parsed = parse_query_with(pair, mode);
                match names.iter().find(|n| parsed.contains_key(**n)) {
                    None => pair.to_string(),
                    Some(_) if plain(raw_name) => format!("{raw_name}={REDACTED}"),
                    Some(name) => format!("{name}={REDACTED}"),
                }
            })
            .collect();
        format!("{}?{}", req.uri().path(), pairs.join("&"))
    }

    /// A small deterministic generator, so the corpus is the same on every run.
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> usize {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (self.0 >> 33) as usize
        }
        fn pick<'a>(&mut self, list: &[&'a str]) -> &'a str {
            list[self.next() % list.len()]
        }
    }

    #[test]
    fn the_output_equals_the_reference_on_a_corpus() {
        let name_pool = [
            "api_key",
            "api%5Fkey",
            "api%5fkey",
            "%61pi_key",
            "%41PI_KEY",
            "[api_key]",
            "api_key[x]",
            "api_key[x][y]",
            "api_key%5Bx%5D",
            "api_key%5bx%5d",
            "%5Bapi_key%5D",
            "api+key",
            "api%20key",
            "other",
            "other[0]",
            "x",
            "",
            "a%2",
            "a%zz",
            "%",
            "caf%C3%A9",
            "api_key.x",
            "__proto__",
        ];
        let value_pool = ["v", "SECRET", "", "a%20b", "x+y", "a=b", "%", "[1]", "%C3%BC"];
        let sets: [&[&str]; 5] = [&["api_key"], &["api_key", "other"], &["api key"], &["café", "x"], &[]];
        let mut rng = Lcg(42);
        for case in 0..3000 {
            let pairs = rng.next() % 9;
            let query: Vec<String> = (0..pairs)
                .map(|_| {
                    let name = rng.pick(&name_pool);
                    if rng.next().is_multiple_of(7) { name.to_string() } else { format!("{name}={}", rng.pick(&value_pool)) }
                })
                .collect();
            let url = format!("/p/q?{}", query.join("&"));
            let names = sets[case % sets.len()];
            for mode in [QueryParser::Simple, QueryParser::Extended, QueryParser::Disabled] {
                let req = request(&url, mode);
                for n in names {
                    req.redact_query_param(n);
                }
                assert_eq!(req.redacted_url(), reference(&req, names), "{mode:?} {names:?} {url}");
            }
        }
    }
}
