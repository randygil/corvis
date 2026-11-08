//! JavaScript `RegExp` emulation with JS semantics for `\d`, `\w`, `\s`, `\b`, `.`, flags and
//! `RegExp.prototype.source`.
//!
//! Patterns without lookarounds or backreferences run on the `regex` crate (finite automata,
//! linear time in the input). Only patterns that need them use the backtracking engine of
//! `fancy-regex`, bounded by [`BACKTRACK_LIMIT`].

use std::fmt;
use std::sync::Arc;

/// A JavaScript regular expression: `/source/flags`.
///
/// ```ignore
/// let re = z::JsRegex::new(r"^[a-z]+$", "i");
/// let re: z::JsRegex = "/^[a-z]+$/i".parse()?;   // literal syntax
/// ```
#[derive(Clone)]
pub struct JsRegex {
    source: Arc<str>,
    flags: Arc<str>,
    re: Engine,
}

/// Backtracking steps allowed per `test` on the `fancy-regex` engine.
pub const BACKTRACK_LIMIT: usize = 1_000_000;

#[derive(Clone)]
enum Engine {
    /// No lookarounds or backreferences: `regex` (linear time).
    Linear(Arc<regex::Regex>),
    /// `fancy-regex` (backtracking).
    Backtracking(Arc<fancy_regex::Regex>),
}

impl JsRegex {
    /// Compiles `source` with `flags` (`i`, `m`, `s`, `u`, `v`, `g`, `y`, `d`).
    /// Panics if the pattern is invalid (schemas are static definitions); see [`JsRegex::try_new`].
    pub fn new(source: &str, flags: &str) -> Self {
        Self::try_new(source, flags).unwrap_or_else(|e| panic!("invalid regular expression /{source}/{flags}: {e}"))
    }

    pub fn try_new(source: &str, flags: &str) -> Result<Self, String> {
        // `\b`/`\B` as the ASCII word boundaries of `regex`. The crate rejects lookarounds and
        // backreferences; those patterns go to the backtracking engine.
        let linear = translate_for(source, flags, true).ok().and_then(|t| regex::Regex::new(&t).ok());
        let re = match linear {
            Some(re) => Engine::Linear(Arc::new(re)),
            None => {
                let translated = translate(source, flags)?;
                let re = fancy_regex::RegexBuilder::new(&translated).backtrack_limit(BACKTRACK_LIMIT).build().map_err(|e| e.to_string())?;
                Engine::Backtracking(Arc::new(re))
            }
        };
        Ok(JsRegex { source: escape_source(source).into(), flags: canonical_flags(flags).into(), re })
    }

    /// `regex.test(input)`. On the backtracking engine an input that exceeds
    /// [`BACKTRACK_LIMIT`] does not match (V8 would keep going); that is logged as a warning.
    pub fn test(&self, input: &str) -> bool {
        match &self.re {
            Engine::Linear(re) => re.is_match(input),
            Engine::Backtracking(re) => re.is_match(input).unwrap_or_else(|e| {
                crate::logger::Logger::new("JsRegex").warn(format!(
                    "/{}/{}: {e} on an input of {} bytes; treated as no match",
                    self.source,
                    self.flags,
                    input.len()
                ));
                false
            }),
        }
    }

    /// `true` when the pattern runs on the linear-time engine (no lookarounds/backreferences).
    pub fn is_linear(&self) -> bool {
        matches!(self.re, Engine::Linear(_))
    }

    /// `regex.source`.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// `regex.flags`.
    pub fn flags(&self) -> &str {
        &self.flags
    }
}

impl fmt::Display for JsRegex {
    /// `regex.toString()` → `/source/flags`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "/{}/{}", self.source, self.flags)
    }
}

impl fmt::Debug for JsRegex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "/{}/{}", self.source, self.flags)
    }
}

impl std::str::FromStr for JsRegex {
    type Err = String;
    /// Parses the literal syntax `/source/flags`.
    fn from_str(s: &str) -> Result<Self, String> {
        let body = s.strip_prefix('/').ok_or("a regex literal must start with '/'")?;
        let end = body.rfind('/').ok_or("a regex literal must end with '/flags'")?;
        JsRegex::try_new(&body[..end], &body[end + 1..])
    }
}

impl From<&str> for JsRegex {
    fn from(s: &str) -> Self {
        JsRegex::new(s, "")
    }
}

impl From<String> for JsRegex {
    fn from(s: String) -> Self {
        JsRegex::new(&s, "")
    }
}

impl From<&String> for JsRegex {
    fn from(s: &String) -> Self {
        JsRegex::new(s, "")
    }
}

impl From<(&str, &str)> for JsRegex {
    fn from((s, f): (&str, &str)) -> Self {
        JsRegex::new(s, f)
    }
}

impl From<regex::Regex> for JsRegex {
    fn from(r: regex::Regex) -> Self {
        JsRegex::new(r.as_str(), "")
    }
}

impl From<&regex::Regex> for JsRegex {
    fn from(r: &regex::Regex) -> Self {
        JsRegex::new(r.as_str(), "")
    }
}

fn canonical_flags(flags: &str) -> String {
    // V8 reports flags in the order "dgimsuvy".
    "dgimsuvy".chars().filter(|c| flags.contains(*c)).collect()
}

/// `EscapeRegExpPattern`: what `RegExp.prototype.source` returns.
fn escape_source(src: &str) -> String {
    if src.is_empty() {
        return "(?:)".into();
    }
    let mut out = String::with_capacity(src.len());
    let mut in_class = false;
    let mut chars = src.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                out.push('\\');
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            }
            '[' => {
                in_class = true;
                out.push(c);
            }
            ']' => {
                in_class = false;
                out.push(c);
            }
            '/' if !in_class => out.push_str("\\/"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            _ => out.push(c),
        }
    }
    out
}

const WS: &str = r"\t\n\x0B\x0C\r \x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}";
const NOT_WS: &str = r"\x00-\x08\x0E-\x1F\x21-\x{9F}\x{A1}-\x{167F}\x{1681}-\x{1FFF}\x{200B}-\x{2027}\x{202A}-\x{202E}\x{2030}-\x{205E}\x{2060}-\x{2FFF}\x{3001}-\x{FEFE}\x{FF00}-\x{10FFFF}";
const DIGIT: &str = "0-9";
const NOT_DIGIT: &str = r"\x00-\x2F\x3A-\x{10FFFF}";
const WORD: &str = "A-Za-z0-9_";
const NOT_WORD: &str = r"\x00-\x2F\x3A-\x40\x5B-\x5E\x60\x7B-\x{10FFFF}";
const WORD_BOUNDARY: &str = "(?:(?<=[A-Za-z0-9_])(?![A-Za-z0-9_])|(?<![A-Za-z0-9_])(?=[A-Za-z0-9_]))";
const NOT_WORD_BOUNDARY: &str = "(?:(?<=[A-Za-z0-9_])(?=[A-Za-z0-9_])|(?<![A-Za-z0-9_])(?![A-Za-z0-9_]))";

fn push_literal(out: &mut String, c: char, in_class: bool) {
    let special = if in_class {
        matches!(c, '[' | ']' | '\\' | '^' | '-' | '&' | '~')
    } else {
        matches!(c, '\\' | '.' | '+' | '*' | '?' | '(' | ')' | '|' | '[' | ']' | '{' | '}' | '^' | '$' | '#' | '&' | '-' | '~')
    };
    if special {
        out.push('\\');
        out.push(c);
    } else if c.is_whitespace() || c.is_control() {
        out.push_str(&format!("\\x{{{:X}}}", c as u32));
    } else {
        out.push(c);
    }
}

fn read_hex(chars: &[char], i: usize, n: usize) -> Option<u32> {
    if i + n > chars.len() {
        return None;
    }
    let s: String = chars[i..i + n].iter().collect();
    if s.chars().all(|c| c.is_ascii_hexdigit()) { u32::from_str_radix(&s, 16).ok() } else { None }
}

/// Parses an escape at `chars[i]` (just after the backslash). Returns the Rust translation
/// and the number of chars consumed.
fn translate_escape(chars: &[char], i: usize, in_class: bool, unicode: bool) -> Result<(String, usize), String> {
    let c = *chars.get(i).ok_or("\\ at end of pattern")?;
    let lit = |ch: char| {
        let mut s = String::new();
        push_literal(&mut s, ch, in_class);
        s
    };
    let class = |inner: &str, neg: &str| -> String { if in_class { inner.to_string() } else { format!("[{neg}{inner}]") } };
    Ok(match c {
        'd' => (class(DIGIT, ""), 1),
        'D' => (if in_class { NOT_DIGIT.into() } else { format!("[^{DIGIT}]") }, 1),
        'w' => (class(WORD, ""), 1),
        'W' => (if in_class { NOT_WORD.into() } else { format!("[^{WORD}]") }, 1),
        's' => (class(WS, ""), 1),
        'S' => (if in_class { NOT_WS.into() } else { format!("[^{WS}]") }, 1),
        'b' if in_class => (r"\x08".into(), 1),
        'b' => (WORD_BOUNDARY.into(), 1),
        'B' => (NOT_WORD_BOUNDARY.into(), 1),
        'n' => (r"\n".into(), 1),
        'r' => (r"\r".into(), 1),
        't' => (r"\t".into(), 1),
        'f' => (r"\x0C".into(), 1),
        'v' => (r"\x0B".into(), 1),
        '0' if !chars.get(i + 1).is_some_and(char::is_ascii_digit) => (r"\x00".into(), 1),
        '1'..='9' if !in_class => {
            let mut j = i;
            while j < chars.len() && chars[j].is_ascii_digit() {
                j += 1;
            }
            let n: String = chars[i..j].iter().collect();
            (format!("\\{n}"), j - i)
        }
        'k' if chars.get(i + 1) == Some(&'<') => {
            let end = chars[i..].iter().position(|c| *c == '>').ok_or("bad \\k")? + i;
            let name: String = chars[i + 2..end].iter().collect();
            (format!("\\k<{name}>"), end - i + 1)
        }
        'x' => match read_hex(chars, i + 1, 2) {
            Some(v) => (lit(char::from_u32(v).unwrap_or('x')), 3),
            None => (lit('x'), 1),
        },
        'u' => {
            if unicode && chars.get(i + 1) == Some(&'{') {
                let end = chars[i..].iter().position(|c| *c == '}').ok_or("bad \\u{")? + i;
                let hex: String = chars[i + 2..end].iter().collect();
                let v = u32::from_str_radix(&hex, 16).map_err(|e| e.to_string())?;
                (lit(char::from_u32(v).ok_or("bad code point")?), end - i + 1)
            } else if let Some(hi) = read_hex(chars, i + 1, 4) {
                if (0xD800..0xDC00).contains(&hi)
                    && chars.get(i + 5) == Some(&'\\')
                    && chars.get(i + 6) == Some(&'u')
                    && let Some(lo) = read_hex(chars, i + 7, 4).filter(|lo| (0xDC00..0xE000).contains(lo))
                {
                    let cp = 0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00);
                    return Ok((lit(char::from_u32(cp).unwrap_or('\u{FFFD}')), 11));
                }
                match char::from_u32(hi) {
                    Some(ch) => (lit(ch), 5),
                    // Lone surrogate: cannot appear in a Rust string, so it never matches.
                    None => ("[^\\x00-\\x{10FFFF}]".into(), 5),
                }
            } else {
                (lit('u'), 1)
            }
        }
        'c' => match chars.get(i + 1) {
            Some(l) if l.is_ascii_alphabetic() => (lit(char::from_u32((*l as u32) % 32).unwrap_or('c')), 2),
            _ => (r"\\c".into(), 1),
        },
        'p' | 'P' if unicode && chars.get(i + 1) == Some(&'{') => {
            let end = chars[i..].iter().position(|c| *c == '}').ok_or("bad \\p{")? + i;
            let name: String = chars[i + 2..end].iter().collect();
            (format!("\\{c}{{{name}}}"), end - i + 1)
        }
        other => (lit(other), 1),
    })
}

/// Translates JS regex syntax to fancy-regex syntax.
pub(crate) fn translate(src: &str, flags: &str) -> Result<String, String> {
    translate_for(src, flags, false)
}

/// Translates JS regex syntax. With `native_boundaries`, `\b`/`\B` become the ASCII word
/// boundaries of `regex` (`(?-u:\b)`) instead of lookarounds.
fn translate_for(src: &str, flags: &str, native_boundaries: bool) -> Result<String, String> {
    for f in flags.chars() {
        if !"dgimsuvy".contains(f) {
            return Err(format!("invalid flag '{f}'"));
        }
    }
    let unicode = flags.contains('u') || flags.contains('v');
    let dotall = flags.contains('s');
    let mut out = String::new();
    let mut prefix = String::new();
    if flags.contains('i') {
        prefix.push('i');
    }
    if flags.contains('m') {
        prefix.push('m');
    }
    if !prefix.is_empty() {
        out.push_str(&format!("(?{prefix})"));
    }
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' => {
                let (t, n) = match chars.get(i + 1) {
                    Some('b') if native_boundaries => (r"(?-u:\b)".to_string(), 1),
                    Some('B') if native_boundaries => (r"(?-u:\B)".to_string(), 1),
                    _ => translate_escape(&chars, i + 1, false, unicode)?,
                };
                out.push_str(&t);
                i += 1 + n;
            }
            '[' => {
                i += 1;
                let negated = chars.get(i) == Some(&'^');
                if negated {
                    i += 1;
                }
                let mut body = String::new();
                let mut closed = false;
                while i < chars.len() {
                    let c = chars[i];
                    if c == ']' {
                        closed = true;
                        i += 1;
                        break;
                    }
                    if c == '\\' {
                        let (t, n) = translate_escape(&chars, i + 1, true, unicode)?;
                        body.push_str(&t);
                        i += 1 + n;
                        continue;
                    }
                    if c == '-' {
                        // range operator only between two atoms
                        let prev_atom = !body.is_empty() && !body.ends_with('-');
                        let next_atom = chars.get(i + 1).is_some_and(|n| *n != ']');
                        if prev_atom && next_atom {
                            body.push('-');
                        } else {
                            body.push_str("\\-");
                        }
                        i += 1;
                        continue;
                    }
                    push_literal(&mut body, c, true);
                    i += 1;
                }
                if !closed {
                    return Err("missing /".into());
                }
                if body.is_empty() {
                    out.push_str(if negated { r"[\x00-\x{10FFFF}]" } else { r"[^\x00-\x{10FFFF}]" });
                } else {
                    out.push('[');
                    if negated {
                        out.push('^');
                    }
                    out.push_str(&body);
                    out.push(']');
                }
            }
            '.' => {
                out.push_str(if dotall { r"[\x00-\x{10FFFF}]" } else { r"[^\n\r\x{2028}\x{2029}]" });
                i += 1;
            }
            '{' => {
                // quantifier `{n}`, `{n,}`, `{n,m}`; otherwise a literal brace
                let mut j = i + 1;
                let mut ok = false;
                while j < chars.len() && chars[j].is_ascii_digit() {
                    j += 1;
                }
                if j > i + 1 {
                    if chars.get(j) == Some(&'}') {
                        ok = true;
                    } else if chars.get(j) == Some(&',') {
                        j += 1;
                        while j < chars.len() && chars[j].is_ascii_digit() {
                            j += 1;
                        }
                        ok = chars.get(j) == Some(&'}');
                    }
                }
                if ok && !out.is_empty() {
                    let q: String = chars[i..=j].iter().collect();
                    out.push_str(&q);
                    i = j + 1;
                } else {
                    out.push_str("\\{");
                    i += 1;
                }
            }
            '}' => {
                out.push_str("\\}");
                i += 1;
            }
            ']' => {
                out.push_str("\\]");
                i += 1;
            }
            '(' => {
                if chars.get(i + 1) == Some(&'?') && chars.get(i + 2) == Some(&'<') {
                    let n3 = chars.get(i + 3);
                    if n3 != Some(&'=') && n3 != Some(&'!') {
                        out.push_str("(?P<");
                        i += 3;
                        continue;
                    }
                }
                out.push('(');
                i += 1;
            }
            '#' | '&' | '~' => {
                out.push('\\');
                out.push(c);
                i += 1;
            }
            ' ' => {
                out.push_str("\\x20");
                i += 1;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn js_semantics() {
        let email = JsRegex::new(r"^(?!\.)(?!.*\.\.)([A-Za-z0-9_'+\-\.]*)[A-Za-z0-9_+-]@([A-Za-z0-9][A-Za-z0-9\-]*\.)+[A-Za-z]{2,}$", "");
        assert!(email.test("ana@corvis.dev"));
        assert!(!email.test(".ana@corvis.dev"));
        assert!(!email.test("a..b@corvis.dev"));
        assert!(JsRegex::new(r"^\d+$", "").test("123"));
        assert!(!JsRegex::new(r"^\d+$", "").test("١٢٣"));
        assert!(JsRegex::new(r"^[a-z]+$", "i").test("ABC"));
        assert!(JsRegex::new(r"^a.c$", "").test("abc"));
        assert!(!JsRegex::new(r"^a.c$", "").test("a\nc"));
        assert!(JsRegex::new(r"^a.c$", "s").test("a\nc"));
        assert!(JsRegex::new(r"\bfoo\b", "").test("a foo b"));
        assert!(JsRegex::new(r"^(\w)\1$", "").test("aa"));
        assert!(JsRegex::new(r"^a{$", "").test("a{"));
        assert_eq!(JsRegex::new("a/b", "gi").to_string(), r"/a\/b/gi");
        assert_eq!(JsRegex::new("[/]", "").source(), "[/]");
        assert_eq!(JsRegex::new("", "").to_string(), "/(?:)/");
        assert!(JsRegex::new(r"^(\p{Extended_Pictographic}|\p{Emoji_Component})+$", "u").test("😀"));
        assert!(JsRegex::new(r"^[^]$", "").test("\n"));
        let re: JsRegex = "/^x+$/i".parse().unwrap();
        assert!(re.test("XX"));
    }

    #[test]
    fn engine_selection() {
        // `\b` is the ASCII word boundary of JS on both engines.
        for (src, linear) in [(r"\bfoo\b", true), (r"^a\Bb$", true), (r"^(\w)\1$", false), (r"(?=a)a", false), (r"^\d+$", true)] {
            assert_eq!(JsRegex::new(src, "").is_linear(), linear, "{src}");
        }
        let b = JsRegex::new(r"\bfoo\b", "");
        assert!(b.test("a foo b") && b.test("foo") && !b.test("afoo") && !b.test("foo_"));
        assert!(b.test("éfooé"), "non-ASCII letters are not word characters in JS");
        assert!(JsRegex::new(r"^[a-z]+\b$", "i").test("ABC"));
        assert!(JsRegex::new(r"^a\Bb$", "").test("ab"));
    }

    /// Before: every pattern with `\b` ran on the backtracking engine, and when it reached the
    /// backtrack limit `test` returned `false` even for valid input (here `c$` matches).
    #[test]
    fn word_boundary_patterns_are_linear_and_exact() {
        let re = JsRegex::new(r"\b(\w+\s?)+\b!|c$", "");
        assert!(re.is_linear());
        assert!(re.test(&format!("{}c", "ab ".repeat(30))));
        let re = JsRegex::new(r"\w+\bX", "");
        let t = std::time::Instant::now();
        assert!(!re.test(&"a".repeat(90_000)));
        assert!(t.elapsed() < std::time::Duration::from_secs(1));
        // With a lookahead the pattern stays on the backtracking engine: the limit still applies
        // (logged as a warning) and the result is `false`, as documented.
        let re = JsRegex::new(r"(?=\w)\b(\w+\s?)+\b!|c$", "");
        assert!(!re.is_linear());
        assert!(!re.test(&format!("{}c", "ab ".repeat(30))));
    }
}
