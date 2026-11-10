//! Redaction of query parameters that carry credentials, for request logs.

use super::body_parser::QueryParser;
use super::request::{Request, parse_query_with};

/// What replaces the value of a marked parameter.
const REDACTED: &str = "REDACTED";

/// Names marked per request before every value of the query string is hidden instead.
pub const MAX_REDACTED_PARAMS: usize = 16;

/// The longest name that can be marked; a longer one hides every value.
const MAX_NAME_BYTES: usize = 256;

/// The parameters of one request whose values must not be logged, shared by every clone of the request.
#[derive(Clone, Default)]
struct RedactedParams {
    names: Vec<String>,
    /// More names than [`MAX_REDACTED_PARAMS`] (or a name too long) were marked: hide every value.
    all: bool,
}

impl Request {
    /// Marks the query parameter `name` of this request as sensitive: [`Request::redacted_url`] replaces its values
    /// with `REDACTED`. A strategy that reads a credential from the query string calls this before it reads
    /// anything, so the mark is there whatever the outcome. At most 16 names are kept; past that (or for a name over
    /// 256 bytes) `redacted_url` hides the value of every parameter rather than leave one visible.
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

    /// [`Request::url`] with the value of every marked parameter replaced by `REDACTED` (see
    /// [`Request::redact_query_param`]), the form to write in logs. A parameter counts as marked when the query
    /// parser of the application files it under that name, so percent-encoded names, repeated parameters and
    /// bracketed forms such as `name[x]=v` are covered. With nothing marked this is exactly `url()`.
    ///
    /// Only the value of the parameter is hidden; a middleware that logs `url()` before the strategy ran, a proxy or
    /// a browser still sees it.
    pub fn redacted_url(&self) -> String {
        let marks = self.extensions.read().unwrap_or_else(|e| e.into_inner()).get::<RedactedParams>().cloned();
        let (Some(marks), Some(query)) = (marks, self.uri.query()) else { return self.url() };
        if marks.names.is_empty() && !marks.all {
            return self.url();
        }
        // `Disabled` leaves `query()` empty; the URL is still split like the default parser.
        let mode = match self.query_parser {
            QueryParser::Disabled => QueryParser::Simple,
            mode => mode,
        };
        let redacted: Vec<String> = query.split('&').map(|pair| redact_pair(pair, &marks, mode)).collect();
        format!("{}?{}", self.uri.path(), redacted.join("&"))
    }
}

/// `true` when the raw name can be printed as it is: no brackets, plain or percent-encoded, which can carry data.
fn plain_name(raw: &str) -> bool {
    !raw.bytes().any(|b| matches!(b, b'[' | b']')) && !raw.to_ascii_lowercase().contains("%5b") && !raw.to_ascii_lowercase().contains("%5d")
}

fn redact_pair(pair: &str, marks: &RedactedParams, mode: QueryParser) -> String {
    let Some((raw_name, _value)) = pair.split_once('=') else { return pair.to_string() };
    let parsed = parse_query_with(pair, mode);
    // The name the parser filed the pair under, when it is a marked one (every pair when the marks overflowed).
    let hit = if marks.all {
        Some(parsed.keys().next().map(String::as_str).unwrap_or("param"))
    } else {
        marks.names.iter().find(|n| parsed.contains_key(n.as_str())).map(String::as_str)
    };
    match hit {
        None => pair.to_string(),
        Some(_) if plain_name(raw_name) => format!("{raw_name}={REDACTED}"),
        // A bracketed name can carry data of its own: print the name the parser used.
        Some(name) => format!("{name}={REDACTED}"),
    }
}

#[cfg(test)]
mod tests {
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
}
