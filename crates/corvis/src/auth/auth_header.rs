//! The `Authorization` header as `passport-jwt` parses it: its first value, matched with `/(\S+)\s+(\S+)/` using
//! JavaScript's whitespace, at its first match. Shared by the JWT extractor and the API key `bearer()` source, each
//! with its own checks on the result.

use http::HeaderMap;
use http::header::AUTHORIZATION;

/// JavaScript's `\s` inside the latin1 range (headers are latin1 in Node), as bytes: tab, line feeds, form feed, space
/// and the no-break space.
pub(crate) fn is_js_space(b: u8) -> bool {
    matches!(b, b'\t' | b'\n' | 0x0b | 0x0c | b'\r' | b' ' | 0xa0)
}

/// The first value of the `Authorization` header: Node keeps the first of a repeated one and drops the others.
pub(crate) fn first_value(headers: &HeaderMap) -> Option<&[u8]> {
    headers.get_all(AUTHORIZATION).iter().next().map(|v| v.as_bytes())
}

/// The scheme of a header value and where its token starts.
pub(crate) struct Credentials<'a> {
    /// The first run of non-space bytes.
    pub(crate) scheme: &'a [u8],
    value: &'a [u8],
    token_start: usize,
}

/// The token is longer than the limit given to [`Credentials::token`].
pub(crate) struct TooLong;

impl<'a> Credentials<'a> {
    /// The token: the run of non-space bytes after the separator, empty when there is none. With `max`, the scan stops
    /// once the run passes `max` bytes and the token is [`TooLong`].
    pub(crate) fn token(&self, max: Option<usize>) -> Result<&'a [u8], TooLong> {
        let rest = &self.value[self.token_start..];
        let len = rest.iter().position(|b| is_js_space(*b)).unwrap_or(rest.len());
        match max {
            Some(max) if len > max => Err(TooLong),
            _ => Ok(&rest[..len]),
        }
    }
}

/// The scheme and the token position of `value`, as `/(\S+)\s+(\S+)/` finds its first match: leading space is skipped,
/// the scheme is the first run of non-space bytes, and at least one space separates it from the token. `None` when
/// there is no scheme, or nothing but the scheme.
///
/// With `window`, the leading space, the scheme and the separator are looked for in the first `window` bytes only (the
/// token itself is not bounded by it): a value whose scheme does not end within the window has none, so the work spent
/// on what precedes the token is bounded whatever the header holds.
pub(crate) fn parse(value: &[u8], window: Option<usize>) -> Option<Credentials<'_>> {
    let end = window.map_or(value.len(), |w| w.min(value.len()));
    let mut i = 0;
    while i < end && is_js_space(value[i]) {
        i += 1;
    }
    let scheme_start = i;
    while i < end && !is_js_space(value[i]) {
        i += 1;
    }
    let scheme = &value[scheme_start..i];
    // A scheme that runs to the end of the window (or of the value) has no separator after it within reach.
    if scheme.is_empty() || i >= end {
        return None;
    }
    while i < end && is_js_space(value[i]) {
        i += 1;
    }
    Some(Credentials { scheme, value, token_start: i })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn groups(value: &[u8]) -> Option<(&[u8], &[u8])> {
        let c = parse(value, None)?;
        let token = c.token(None).ok()?;
        (!token.is_empty()).then_some((c.scheme, token))
    }

    #[test]
    fn the_first_two_runs_like_the_regular_expression() {
        assert_eq!(groups(b"Bearer abc"), Some((&b"Bearer"[..], &b"abc"[..])));
        assert_eq!(groups(b"  Bearer \t\xa0abc extra"), Some((&b"Bearer"[..], &b"abc"[..])));
        assert_eq!(groups(b"Bearer"), None);
        assert_eq!(groups(b"Bearer "), None);
        assert_eq!(groups(b""), None);
        assert_eq!(groups(b"   "), None);
        assert_eq!(groups(b"Bearer\x85abc"), None, "0x85 is not whitespace in JavaScript");
    }

    #[test]
    fn the_window_bounds_what_precedes_the_token_but_not_the_token() {
        let mut value = vec![b' '; 100];
        value.extend_from_slice(b"Bearer ");
        value.extend(std::iter::repeat_n(b'k', 500));
        let c = parse(&value, Some(110)).unwrap();
        assert_eq!(c.scheme, b"Bearer");
        assert_eq!(c.token(None).ok().unwrap().len(), 500);
        assert!(c.token(Some(499)).is_err());
        assert!(c.token(Some(500)).is_ok());
        // Past the window the scheme is not found.
        assert!(parse(&value, Some(100)).is_none());
        // A scheme cut by the window has no separator within it.
        assert!(parse(&value, Some(104)).is_none());
    }
}
