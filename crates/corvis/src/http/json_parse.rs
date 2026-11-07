//! `JSON.parse` exactly as V8 (Node.js 24) implements it, including its error messages.
//!
//! `body-parser` (and therefore the Express platform) answers a malformed JSON body with
//! `400 { "message": <V8 SyntaxError message>, ... }`, so reproducing an Express API byte for byte
//! requires the very same messages:
//!
//! - `Unexpected token 'x', "x" is not valid JSON`
//! - `Unexpected token '}', ..."e": 1, }"... is not valid JSON` (context of 10 chars around the error)
//! - `Expected property name or '}' in JSON at position 1 (line 1 column 2)`
//! - `Unexpected end of JSON input`, `Unterminated string in JSON at position 5 (line 1 column 6)`, ...
//!
//! The parser is a port of `src/json/json-parser.cc` (V8 13.6): same tokenizer, same error sites
//! and same positions (UTF-16 code units, like JavaScript strings). It is iterative, so deeply
//! nested input does not overflow the stack.
//!
//! The parsed value follows JavaScript semantics where `serde_json::Value` can express them:
//! object keys are ordered like a JS object (array-index keys first, ascending, then insertion
//! order), duplicated keys keep the first position and the last value, and numbers are IEEE-754
//! doubles (`1.0` → `1`, `-0` → `0`, `12345678901234567890` → `12345678901234567000`).
//! Non-finite numbers (`1e400`) become `null` (what `JSON.stringify` prints for them).

use serde_json::{Map, Number, Value};

/// A `SyntaxError` thrown by [`parse`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonParseError {
    /// The exact V8 message.
    pub message: String,
    /// Position (UTF-16 code units) where the error was detected.
    pub position: usize,
}

impl std::fmt::Display for JsonParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for JsonParseError {}

/// `JSON.parse(text)`.
pub fn parse(text: &str) -> Result<Value, JsonParseError> {
    let units: Vec<u16> = text.encode_utf16().collect();
    parse_utf16(&units)
}

/// `JSON.parse(text)` over a JavaScript string given as UTF-16 code units (it may contain
/// lone surrogates, like strings decoded by `iconv-lite`).
pub fn parse_utf16(src: &[u16]) -> Result<Value, JsonParseError> {
    Parser { src, pos: 0, next: Token::Eos }.parse_json()
}

const K_END: i32 = -1;
const MAX_CONTEXT: usize = 10;
const MIN_LEN_FOR_CONTEXT: usize = MAX_CONTEXT * 2 + 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Token {
    Number,
    String,
    LBrace,
    RBrace,
    LBrack,
    RBrack,
    True,
    False,
    Null,
    Whitespace,
    Colon,
    Comma,
    Illegal,
    Eos,
}

fn one_char_token(c: u16) -> Token {
    if c > 0xFF {
        return Token::Illegal;
    }
    match c as u8 {
        b'"' => Token::String,
        b'0'..=b'9' | b'-' => Token::Number,
        b'[' => Token::LBrack,
        b'{' => Token::LBrace,
        b']' => Token::RBrack,
        b'}' => Token::RBrace,
        b't' => Token::True,
        b'f' => Token::False,
        b'n' => Token::Null,
        b' ' | b'\t' | b'\r' | b'\n' => Token::Whitespace,
        b':' => Token::Colon,
        b',' => Token::Comma,
        _ => Token::Illegal,
    }
}

fn is_number_part(c: i32) -> bool {
    matches!(c, 0x30..=0x39) || c == '.' as i32 || c == 'e' as i32 || c == 'E' as i32 || c == '-' as i32 || c == '+' as i32
}

fn is_digit(c: i32) -> bool {
    (0x30..=0x39).contains(&c)
}

fn hex_value(c: i32) -> i32 {
    match c {
        0x30..=0x39 => c - 0x30,
        0x41..=0x46 => c - 0x41 + 10,
        0x61..=0x66 => c - 0x61 + 10,
        _ => -1,
    }
}

/// Message templates (`src/common/message-template.h`).
#[derive(Debug, Clone, Copy)]
enum Msg {
    UnexpectedEos,
    UnexpectedTokenNumber,
    UnexpectedTokenString,
    UnterminatedString,
    ExpectedPropNameOrRBrace,
    ExpectedCommaOrRBrack,
    ExpectedCommaOrRBrace,
    ExpectedDoubleQuotedPropertyName,
    ExponentPartMissingNumber,
    ExpectedColonAfterPropertyName,
    UnterminatedFractionalNumber,
    UnexpectedNonWhiteSpaceCharacter,
    BadEscapedCharacter,
    BadControlCharacter,
    BadUnicodeEscape,
    NoNumberAfterMinusSign,
}

impl Msg {
    fn positional(self) -> Option<&'static str> {
        Some(match self {
            Msg::UnexpectedEos => return None,
            Msg::UnexpectedTokenNumber => "Unexpected number in JSON",
            Msg::UnexpectedTokenString => "Unexpected string in JSON",
            Msg::UnterminatedString => "Unterminated string in JSON",
            Msg::ExpectedPropNameOrRBrace => "Expected property name or '}' in JSON",
            Msg::ExpectedCommaOrRBrack => "Expected ',' or ']' after array element in JSON",
            Msg::ExpectedCommaOrRBrace => "Expected ',' or '}' after property value in JSON",
            Msg::ExpectedDoubleQuotedPropertyName => "Expected double-quoted property name in JSON",
            Msg::ExponentPartMissingNumber => "Exponent part is missing a number in JSON",
            Msg::ExpectedColonAfterPropertyName => "Expected ':' after property name in JSON",
            Msg::UnterminatedFractionalNumber => "Unterminated fractional number in JSON",
            Msg::UnexpectedNonWhiteSpaceCharacter => "Unexpected non-whitespace character after JSON",
            Msg::BadEscapedCharacter => "Bad escaped character in JSON",
            Msg::BadControlCharacter => "Bad control character in string literal in JSON",
            Msg::BadUnicodeEscape => "Bad Unicode escape in JSON",
            Msg::NoNumberAfterMinusSign => "No number after minus sign in JSON",
        })
    }
}

/// Converts UTF-16 code units to a Rust string (lone surrogates become U+FFFD).
pub(crate) fn utf16_to_string(units: &[u16]) -> String {
    char::decode_utf16(units.iter().copied()).map(|r| r.unwrap_or('\u{FFFD}')).collect()
}

/// A JS number as a JSON value: integral doubles become integers (what `JSON.stringify`
/// prints), non-finite ones `null`.
pub(crate) fn number_value(d: f64) -> Value {
    if !d.is_finite() {
        return Value::Null;
    }
    if d == d.trunc() && d.abs() < 9_007_199_254_740_992.0 {
        return Value::Number(Number::from(d as i64));
    }
    if d == d.trunc() && d.abs() < 1e21 {
        // JS prints integral doubles below 1e21 with their shortest digits padded with zeros
        // (12345678901234567000, not ...7168): Rust's `Display` does the same.
        let s = format!("{d}");
        if let Ok(i) = s.parse::<i64>() {
            return Value::Number(Number::from(i));
        }
        if let Ok(u) = s.parse::<u64>() {
            return Value::Number(Number::from(u));
        }
    }
    Number::from_f64(d).map(Value::Number).unwrap_or(Value::Null)
}

/// `true` for canonical array-index keys (`"0"`, `"42"`, up to 2^32 - 2), which JavaScript
/// objects enumerate first, in ascending numeric order.
pub(crate) fn is_array_index(key: &str) -> bool {
    let b = key.as_bytes();
    if b.is_empty() || b.len() > 10 || !b.iter().all(u8::is_ascii_digit) || (b.len() > 1 && b[0] == b'0') {
        return false;
    }
    key.parse::<u64>().map(|n| n < 4_294_967_295).unwrap_or(false)
}

/// An object under construction with JavaScript property-order semantics.
#[derive(Default, Debug, Clone)]
pub(crate) struct JsObject {
    entries: Vec<(String, Value)>,
    index: std::collections::HashMap<String, usize>,
}

impl JsObject {
    pub(crate) fn new() -> Self {
        Self::default()
    }
    /// `obj[key] = value`: an existing key keeps its position.
    pub(crate) fn set(&mut self, key: String, value: Value) {
        if let Some(&i) = self.index.get(&key) {
            self.entries[i].1 = value;
        } else {
            self.index.insert(key.clone(), self.entries.len());
            self.entries.push((key, value));
        }
    }
    pub(crate) fn into_map(self) -> Map<String, Value> {
        js_order(self.entries)
    }
}

/// Orders `entries` like `Object.keys` (array indices ascending first, then insertion order).
pub(crate) fn js_order(entries: Vec<(String, Value)>) -> Map<String, Value> {
    let mut map = Map::new();
    if entries.iter().any(|(k, _)| is_array_index(k)) {
        let (mut idx, rest): (Vec<_>, Vec<_>) = entries.into_iter().partition(|(k, _)| is_array_index(k));
        idx.sort_by_key(|(k, _)| k.parse::<u64>().unwrap_or(0));
        for (k, v) in idx.into_iter().chain(rest) {
            map.insert(k, v);
        }
    } else {
        for (k, v) in entries {
            map.insert(k, v);
        }
    }
    map
}

enum Frame {
    Object { obj: JsObject, key: String },
    Array(Vec<Value>),
}

struct Parser<'a> {
    src: &'a [u16],
    pos: usize,
    next: Token,
}

type PResult<T> = Result<T, JsonParseError>;

impl<'a> Parser<'a> {
    fn at_end(&self) -> bool {
        self.pos >= self.src.len()
    }
    fn current(&self) -> i32 {
        if self.at_end() { K_END } else { self.src[self.pos] as i32 }
    }
    fn advance(&mut self) {
        self.pos += 1;
    }
    fn next_char(&mut self) -> i32 {
        self.advance();
        self.current()
    }

    fn skip_whitespace(&mut self) {
        let mut next = Token::Eos;
        while self.pos < self.src.len() {
            let t = one_char_token(self.src[self.pos]);
            if t != Token::Whitespace {
                next = t;
                break;
            }
            self.pos += 1;
        }
        self.next = next;
    }

    fn check(&mut self, t: Token) -> bool {
        self.skip_whitespace();
        if self.next != t {
            return false;
        }
        self.advance();
        true
    }

    fn expect(&mut self, t: Token, msg: Msg) -> PResult<()> {
        if self.next == t {
            self.advance();
            Ok(())
        } else {
            Err(self.report(self.next, Some(msg)))
        }
    }

    fn expect_next(&mut self, t: Token, msg: Msg) -> PResult<()> {
        self.skip_whitespace();
        self.expect(t, msg)
    }

    fn report_char(&self, c: i32) -> JsonParseError {
        let token = if c == K_END {
            Token::Eos
        } else if c <= 0xFF {
            one_char_token(c as u16)
        } else {
            Token::Illegal
        };
        self.report(token, None)
    }

    /// `JsonParser::ReportUnexpectedToken`.
    fn report(&self, token: Token, msg: Option<Msg>) -> JsonParseError {
        let pos = self.pos.min(self.src.len());
        let (line, column) = self.file_location(pos);
        let msg = match (msg, token) {
            (Some(m), _) => m,
            (None, Token::Eos) => Msg::UnexpectedEos,
            (None, Token::Number) => Msg::UnexpectedTokenNumber,
            (None, Token::String) => Msg::UnexpectedTokenString,
            (None, _) => return JsonParseError { message: self.unexpected_token_message(pos), position: pos },
        };
        let message = match msg.positional() {
            None => "Unexpected end of JSON input".to_string(),
            Some(text) => format!("{text} at position {pos} (line {line} column {column})"),
        };
        JsonParseError { message, position: pos }
    }

    /// `IsSpecialString` + `GetErrorMessageWithEllipses`.
    fn unexpected_token_message(&self, pos: usize) -> String {
        let src = self.src;
        let whole = utf16_to_string(src);
        if matches!(whole.as_str(), "[object Object]" | "undefined" | "Infinity" | "NaN") {
            return format!("\"{whole}\" is not valid JSON");
        }
        let token = utf16_to_string(&src[pos..(pos + 1).min(src.len())]);
        let len = src.len();
        if len < MIN_LEN_FOR_CONTEXT {
            return format!("Unexpected token '{token}', \"{whole}\" is not valid JSON");
        }
        if pos < MAX_CONTEXT {
            let ctx = utf16_to_string(&src[..(pos + MAX_CONTEXT).min(len)]);
            format!("Unexpected token '{token}', \"{ctx}\"... is not valid JSON")
        } else if pos < len - MAX_CONTEXT {
            let ctx = utf16_to_string(&src[pos - MAX_CONTEXT..pos + MAX_CONTEXT]);
            format!("Unexpected token '{token}', ...\"{ctx}\"... is not valid JSON")
        } else {
            let ctx = utf16_to_string(&src[pos - MAX_CONTEXT..]);
            format!("Unexpected token '{token}', ...\"{ctx}\" is not valid JSON")
        }
    }

    /// `CalculateFileLocation`: `\r`, `\n` and `\r\n` are line terminators.
    fn file_location(&self, end: usize) -> (usize, usize) {
        let mut line = 1;
        let mut last_break = 0;
        let mut i = 0;
        while i < end {
            if self.src[i] == b'\r' as u16 && i + 1 < end && self.src[i + 1] == b'\n' as u16 {
                i += 1;
            }
            if self.src[i] == b'\r' as u16 || self.src[i] == b'\n' as u16 {
                line += 1;
                last_break = i + 1;
            }
            i += 1;
        }
        (line, 1 + i - last_break)
    }

    fn parse_json(mut self) -> PResult<Value> {
        let value = self.parse_value()?;
        if !self.check(Token::Eos) {
            return Err(self.report(self.next, Some(Msg::UnexpectedNonWhiteSpaceCharacter)));
        }
        Ok(value)
    }

    /// Iterative `ParseJsonValue`.
    fn parse_value(&mut self) -> PResult<Value> {
        let mut stack: Vec<Frame> = Vec::new();
        'value: loop {
            self.skip_whitespace();
            let mut value = match self.next {
                Token::Number => self.parse_number()?,
                Token::String => {
                    self.advance();
                    Value::String(utf16_to_string(&self.scan_string()?))
                }
                Token::True => {
                    self.scan_literal("true")?;
                    Value::Bool(true)
                }
                Token::False => {
                    self.scan_literal("false")?;
                    Value::Bool(false)
                }
                Token::Null => {
                    self.scan_literal("null")?;
                    Value::Null
                }
                Token::LBrace => {
                    self.advance();
                    if self.check(Token::RBrace) {
                        Value::Object(Map::new())
                    } else {
                        self.expect_next(Token::String, Msg::ExpectedPropNameOrRBrace)?;
                        let key = utf16_to_string(&self.scan_string()?);
                        self.expect_next(Token::Colon, Msg::ExpectedColonAfterPropertyName)?;
                        stack.push(Frame::Object { obj: JsObject::new(), key });
                        continue 'value;
                    }
                }
                Token::LBrack => {
                    self.advance();
                    if self.check(Token::RBrack) {
                        Value::Array(Vec::new())
                    } else {
                        stack.push(Frame::Array(Vec::new()));
                        continue 'value;
                    }
                }
                _ => return Err(self.report_char(self.current())),
            };
            // Hand the value to the enclosing containers.
            loop {
                match stack.last_mut() {
                    None => return Ok(value),
                    Some(Frame::Object { obj, key }) => {
                        obj.set(std::mem::take(key), value);
                        if self.check(Token::Comma) {
                            self.expect_next(Token::String, Msg::ExpectedDoubleQuotedPropertyName)?;
                            let k = utf16_to_string(&self.scan_string()?);
                            self.expect_next(Token::Colon, Msg::ExpectedColonAfterPropertyName)?;
                            if let Some(Frame::Object { key, .. }) = stack.last_mut() {
                                *key = k;
                            }
                            continue 'value;
                        }
                        self.expect(Token::RBrace, Msg::ExpectedCommaOrRBrace)?;
                        let Some(Frame::Object { obj, .. }) = stack.pop() else { unreachable!() };
                        value = Value::Object(obj.into_map());
                    }
                    Some(Frame::Array(items)) => {
                        items.push(value);
                        if self.check(Token::Comma) {
                            continue 'value;
                        }
                        self.expect(Token::RBrack, Msg::ExpectedCommaOrRBrack)?;
                        let Some(Frame::Array(items)) = stack.pop() else { unreachable!() };
                        value = Value::Array(items);
                    }
                }
            }
        }
    }

    /// `ScanLiteral`: the first character was already matched.
    fn scan_literal(&mut self, lit: &str) -> PResult<()> {
        let s: Vec<u16> = lit.encode_utf16().collect();
        let n = s.len();
        let remaining = self.src.len() - self.pos;
        if remaining >= n && self.src[self.pos + 1..self.pos + n] == s[1..] {
            self.pos += n;
            return Ok(());
        }
        self.pos += 1;
        for i in 0..(n - 1).min(remaining - 1) {
            if s[1 + i] != self.src[self.pos] {
                return Err(self.report_char(self.src[self.pos] as i32));
            }
            self.pos += 1;
        }
        Err(self.report(Token::Eos, None))
    }

    /// `ParseJsonNumberAsDoubleOrSmi`.
    fn parse_number(&mut self) -> PResult<Value> {
        let start = self.pos;
        let mut c = self.current();
        let mut negative = false;
        if c == '-' as i32 {
            negative = true;
            c = self.next_char();
        }
        if c == '0' as i32 {
            c = self.next_char();
            if (0..=0xFF).contains(&c) && is_number_part(c) {
                if is_digit(c) {
                    return Err(self.report(Token::Number, None));
                }
            } else if !negative {
                return Ok(Value::Number(0.into()));
            }
        } else {
            let smi_start = self.pos;
            let stop = (self.pos + 9).min(self.src.len());
            let mut i: i64 = 0;
            while self.pos < stop && is_digit(self.src[self.pos] as i32) {
                i = i * 10 + (self.src[self.pos] as i64 - '0' as i64);
                self.pos += 1;
            }
            if smi_start == self.pos {
                return Err(self.report(Token::Illegal, Some(Msg::NoNumberAfterMinusSign)));
            }
            let c = self.current();
            if !(0..=0xFF).contains(&c) || !is_number_part(c) {
                return Ok(Value::Number((if negative { -i } else { i }).into()));
            }
            self.advance_to_non_decimal();
        }
        if self.current() == '.' as i32 {
            let c = self.next_char();
            if !is_digit(c) {
                return Err(self.report(Token::Illegal, Some(Msg::UnterminatedFractionalNumber)));
            }
            self.advance_to_non_decimal();
        }
        let c = self.current();
        if c == 'e' as i32 || c == 'E' as i32 {
            let mut c = self.next_char();
            if c == '-' as i32 || c == '+' as i32 {
                c = self.next_char();
            }
            if !is_digit(c) {
                return Err(self.report(Token::Illegal, Some(Msg::ExponentPartMissingNumber)));
            }
            self.advance_to_non_decimal();
        }
        let text: String = self.src[start..self.pos].iter().map(|&u| u as u8 as char).collect();
        let d: f64 = text.parse().unwrap_or(f64::NAN);
        Ok(number_value(d))
    }

    fn advance_to_non_decimal(&mut self) {
        while self.pos < self.src.len() && is_digit(self.src[self.pos] as i32) {
            self.pos += 1;
        }
    }

    /// `ScanUnicodeCharacter`: four hex digits after `\u`.
    fn scan_unicode(&mut self) -> Option<u16> {
        let mut value: u32 = 0;
        for _ in 0..4 {
            let d = hex_value(self.next_char());
            if d < 0 {
                return None;
            }
            value = value * 16 + d as u32;
        }
        Some(value as u16)
    }

    /// `ScanJsonString` (the opening quote was consumed); returns the decoded code units.
    fn scan_string(&mut self) -> PResult<Vec<u16>> {
        let mut out: Vec<u16> = Vec::new();
        loop {
            // Find the next character that may terminate the string.
            let from = self.pos;
            while self.pos < self.src.len() {
                let c = self.src[self.pos];
                if c < 0x20 || c == b'"' as u16 || c == b'\\' as u16 {
                    break;
                }
                self.pos += 1;
            }
            out.extend_from_slice(&self.src[from..self.pos]);
            if self.at_end() {
                return Err(self.report(Token::Illegal, Some(Msg::UnterminatedString)));
            }
            let c = self.src[self.pos];
            if c == b'"' as u16 {
                self.advance();
                return Ok(out);
            }
            if c == b'\\' as u16 {
                let e = self.next_char();
                if !(0..=0xFF).contains(&e) {
                    return Err(self.report_char(e));
                }
                let decoded = match e as u8 {
                    b'"' => b'"' as u16,
                    b'\\' => b'\\' as u16,
                    b'/' => b'/' as u16,
                    b'b' => 0x08,
                    b't' => 0x09,
                    b'n' => 0x0A,
                    b'f' => 0x0C,
                    b'r' => 0x0D,
                    b'u' => match self.scan_unicode() {
                        Some(v) => v,
                        None => return Err(self.report(Token::Illegal, Some(Msg::BadUnicodeEscape))),
                    },
                    _ => return Err(self.report(Token::Illegal, Some(Msg::BadEscapedCharacter))),
                };
                out.push(decoded);
                self.advance();
                continue;
            }
            return Err(self.report(Token::Illegal, Some(Msg::BadControlCharacter)));
        }
    }
}

/// The error `body-parser` builds for a strict-mode violation (a top-level value that is not
/// an object or an array): it parses `str` with everything from the first non-blank character
/// replaced by `#`, then puts the original text back into the message.
pub(crate) fn strict_violation(src: &[u16], first: Option<u16>) -> JsonParseError {
    // `str.indexOf(char)`; with no first char body-parser calls `indexOf(undefined)`.
    let index = match first {
        Some(c) => src.iter().position(|&u| u == c),
        None => {
            let needle: Vec<u16> = "undefined".encode_utf16().collect();
            src.windows(needle.len()).position(|w| w == needle.as_slice())
        }
    };
    let mut partial: Vec<u16> = Vec::new();
    if let Some(i) = index {
        partial.extend_from_slice(&src[..i]);
        partial.extend(std::iter::repeat_n(b'#' as u16, src.len() - i));
    }
    let err = match parse_utf16(&partial) {
        Err(e) => e,
        Ok(_) => return JsonParseError { message: "strict violation".into(), position: 0 },
    };
    // message.replace(/#+/g, placeholder => str.substring(index, index + placeholder.length))
    let msg: Vec<u16> = err.message.encode_utf16().collect();
    let start = index.unwrap_or(0);
    let mut out: Vec<u16> = Vec::with_capacity(msg.len());
    let mut i = 0;
    while i < msg.len() {
        if msg[i] == b'#' as u16 {
            let mut j = i;
            while j < msg.len() && msg[j] == b'#' as u16 {
                j += 1;
            }
            let len = j - i;
            let end = (start + len).min(src.len());
            out.extend_from_slice(&src[start.min(src.len())..end]);
            i = j;
        } else {
            out.push(msg[i]);
            i += 1;
        }
    }
    JsonParseError { message: utf16_to_string(&out), position: err.position }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(s: &str) -> String {
        parse(s).unwrap_err().message
    }

    #[test]
    fn values() {
        assert_eq!(parse(r#"{"b":1,"a":[1,2.5,"x",true,false,null],"2":0,"1":-0}"#).unwrap().to_string(), r#"{"1":0,"2":0,"b":1,"a":[1,2.5,"x",true,false,null]}"#);
        assert_eq!(parse("12345678901234567890").unwrap().to_string(), "12345678901234567000");
        assert_eq!(parse("1.0").unwrap().to_string(), "1");
        assert_eq!(parse("1e400").unwrap(), Value::Null);
        assert_eq!(parse(r#""😀""#).unwrap(), Value::String("😀".into()));
        assert_eq!(parse(r#"{"a":1,"b":2,"a":3}"#).unwrap().to_string(), r#"{"a":3,"b":2}"#);
    }

    #[test]
    fn messages() {
        assert_eq!(msg("x"), "Unexpected token 'x', \"x\" is not valid JSON");
        assert_eq!(msg("{a:1}"), "Expected property name or '}' in JSON at position 1 (line 1 column 2)");
        assert_eq!(msg(""), "Unexpected end of JSON input");
        assert_eq!(msg("undefined"), "\"undefined\" is not valid JSON");
        assert_eq!(msg("{\r\n\"a\":1,\r\n\r\n}"), "Expected double-quoted property name in JSON at position 13 (line 4 column 1)");
    }

    #[test]
    fn strict() {
        let s: Vec<u16> = "\"x\"".encode_utf16().collect();
        assert_eq!(strict_violation(&s, Some(b'"' as u16)).message, "Unexpected token '\"', \"\"x\"\" is not valid JSON");
        let s: Vec<u16> = "   ".encode_utf16().collect();
        assert_eq!(strict_violation(&s, None).message, "Unexpected end of JSON input");
    }
}
