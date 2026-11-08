//! Port of `jsYaml.dump(value, { skipInvalid: true, noRefs: true })` (js-yaml 5.2, the YAML
//! serializer used for `<path>-yaml`) for JSON values: default schema (YAML 1.1 implicit types), indent 2,
//! line width 80, single quotes.
//!
//! Strings are processed as UTF-16 code units, like JavaScript strings, so line widths and
//! folding match exactly.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

const INDENT: usize = 2;
const LINE_WIDTH: i64 = 80;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tag {
    Str,
    Null,
    Bool,
    Int,
    Float,
    Timestamp,
    Merge,
}

enum Node {
    Scalar { tag: Tag, value: String },
    Seq(Vec<Node>),
    Map(Vec<(Node, Node)>),
}

/// `jsYaml.dump(value, { skipInvalid: true, noRefs: true })`.
pub fn dump(v: &Value) -> String {
    let node = build(v);
    write_node(&node, 0, Ctx { block: true, compact: true, iskey: false }) + "\n"
}

fn build(v: &Value) -> Node {
    match v {
        Value::Null => Node::Scalar { tag: Tag::Null, value: "null".into() },
        Value::Bool(b) => Node::Scalar { tag: Tag::Bool, value: if *b { "true" } else { "false" }.into() },
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Node::Scalar { tag: Tag::Int, value: i.to_string() }
            } else if let Some(u) = n.as_u64() {
                Node::Scalar { tag: Tag::Int, value: u.to_string() }
            } else {
                let f = n.as_f64().unwrap_or(0.0);
                let s = js_number_string(f);
                if f.fract() == 0.0 && !s.contains('e') && !(f == 0.0 && f.is_sign_negative()) {
                    Node::Scalar { tag: Tag::Int, value: s }
                } else {
                    Node::Scalar { tag: Tag::Float, value: represent_float(f) }
                }
            }
        }
        Value::String(s) => Node::Scalar { tag: Tag::Str, value: s.clone() },
        Value::Array(a) => Node::Seq(a.iter().map(build).collect()),
        Value::Object(o) => Node::Map(o.iter().map(|(k, v)| (Node::Scalar { tag: Tag::Str, value: k.clone() }, build(v))).collect()),
    }
}

/// `Number.prototype.toString(10)`.
pub fn js_number_string(f: f64) -> String {
    if f.is_nan() {
        return "NaN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    if f == 0.0 {
        return "0".into();
    }
    // Shortest round-trip digits and exponent.
    let e = format!("{:e}", f.abs());
    let (mant, exp) = e.split_once('e').unwrap_or((&e, "0"));
    let exp: i64 = exp.parse().unwrap_or(0);
    let digits: String = mant.chars().filter(|c| c.is_ascii_digit()).collect();
    let k = digits.len() as i64;
    let n = exp + 1; // position of the decimal point
    let sign = if f < 0.0 { "-" } else { "" };
    let body = if k <= n && n <= 21 {
        format!("{digits}{}", "0".repeat((n - k) as usize))
    } else if 0 < n && n <= 21 {
        format!("{}.{}", &digits[..n as usize], &digits[n as usize..])
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat((-n) as usize))
    } else {
        let e = n - 1;
        let es = if e >= 0 { format!("+{e}") } else { e.to_string() };
        if k == 1 { format!("{digits}e{es}") } else { format!("{}.{}e{es}", &digits[..1], &digits[1..]) }
    };
    format!("{sign}{body}")
}

fn represent_float(f: f64) -> String {
    if f.is_nan() {
        return ".nan".into();
    }
    if f == f64::INFINITY {
        return ".inf".into();
    }
    if f == f64::NEG_INFINITY {
        return "-.inf".into();
    }
    if f == 0.0 && f.is_sign_negative() {
        return "-0.0".into();
    }
    let s = js_number_string(f);
    static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[-+]?[0-9]+e").unwrap());
    if RE.is_match(&s) { s.replacen('e', ".e", 1) } else { s }
}

// --- implicit resolution (YAML 1.1 schema + core int/float) --------------------------------

static INT_YAML11: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:[-+]?0b[0-1_]+|[-+]?0[0-7_]+|[-+]?0x[0-9a-fA-F_]+|[-+]?[0-9][0-9_]*(?::[0-5]?[0-9])+|[-+]?(?:0|[1-9][0-9_]*))$")
        .unwrap()
});
static INT_CORE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:0o[0-7]+|0x[0-9a-fA-F]+|[-+]?[0-9]+)$").unwrap());
static FLOAT_YAML11: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:[-+]?(?:(?:[0-9][0-9_]*)?\.[0-9_]*)(?:[eE][-+][0-9]+)?|[-+]?[0-9][0-9_]*(?::[0-5]?[0-9])+\.[0-9_]*|[-+]?\.(?:inf|Inf|INF)|\.(?:nan|NaN|NAN))$").unwrap()
});
static FLOAT_CORE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?:[-+]?[0-9]+(?:\.[0-9]*)?(?:[eE][-+]?[0-9]+)?|[-+]?\.[0-9]+(?:[eE][-+]?[0-9]+)?|[-+]?\.(?:inf|Inf|INF)|\.(?:nan|NaN|NAN))$",
    )
    .unwrap()
});
static FLOAT_SPECIAL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:[-+]?\.(?:inf|Inf|INF)|\.(?:nan|NaN|NAN))$").unwrap());
static DATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([0-9][0-9][0-9][0-9])-([0-9][0-9])-([0-9][0-9])$").unwrap());
static TIMESTAMP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^([0-9][0-9][0-9][0-9])-([0-9][0-9]?)-([0-9][0-9]?)(?:[Tt]|[ \t]+)([0-9][0-9]?):([0-9][0-9]):([0-9][0-9])(?:\.([0-9]*))?(?:[ \t]*(Z|([-+])([0-9][0-9]?)(?::([0-9][0-9]))?))?$").unwrap()
});

fn finite_number(s: &str) -> bool {
    s.parse::<f64>().map(|f| f.is_finite()).unwrap_or(false)
}

fn int_yaml11(s: &str) -> bool {
    if !INT_YAML11.is_match(s) {
        return false;
    }
    let v = s.replace('_', "");
    let v = v.trim_start_matches(['-', '+']);
    if let Some(d) = v.strip_prefix("0b") {
        return !d.is_empty();
    }
    if let Some(d) = v.strip_prefix("0x") {
        return !d.is_empty();
    }
    if v.contains(':') {
        return true;
    }
    !v.is_empty() && (v.len() < 309 || finite_number(v))
}

fn int_core(s: &str) -> bool {
    INT_CORE.is_match(s) && {
        let v = s.trim_start_matches(['-', '+']);
        v.starts_with("0o") || v.starts_with("0x") || finite_number(v)
    }
}

fn float_yaml11(s: &str) -> bool {
    if !FLOAT_YAML11.is_match(s) {
        return false;
    }
    if FLOAT_SPECIAL.is_match(s) {
        return true;
    }
    let v = s.to_lowercase().replace('_', "");
    let v = v.trim_start_matches(['-', '+']);
    if v.contains(':') {
        return true;
    }
    // parseFloat of a string the pattern accepted: finite unless it has no digit or overflows
    finite_number(v)
}

fn float_core(s: &str) -> bool {
    if !FLOAT_CORE.is_match(s) {
        return false;
    }
    if FLOAT_SPECIAL.is_match(s) {
        return true;
    }
    let v = s.to_lowercase();
    let v = v.trim_start_matches(['-', '+']);
    finite_number(v)
}

fn days_in_month(year: i64, month0: i64) -> i64 {
    match month0 {
        0 | 2 | 4 | 6 | 7 | 9 | 11 => 31,
        3 | 5 | 8 | 10 => 30,
        1 => {
            if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

fn timestamp(s: &str) -> bool {
    let caps = DATE.captures(s).or_else(|| TIMESTAMP.captures(s));
    let Some(c) = caps else { return false };
    let num = |i: usize| c.get(i).and_then(|m| m.as_str().parse::<i64>().ok()).unwrap_or(0);
    let year = num(1);
    let month = num(2) - 1;
    let day = num(3);
    // `Date.UTC(year, ...)` maps years 0..=99 to 1900..=1999, so they never round-trip.
    if year < 100 || !(0..12).contains(&month) || day < 1 || day > days_in_month(year, month) {
        return false;
    }
    if c.get(4).is_none() {
        return true;
    }
    if num(4) > 23 || num(5) > 59 || num(6) > 59 {
        return false;
    }
    if c.get(9).is_some() && c.get(10).is_some() && (num(10) > 23 || num(11) > 59) {
        return false;
    }
    true
}

fn resolve_implicit(s: &str) -> Tag {
    if ["", "~", "null", "Null", "NULL"].contains(&s) {
        return Tag::Null;
    }
    const BOOLS: [&str; 22] = [
        "true", "True", "TRUE", "y", "Y", "yes", "Yes", "YES", "on", "On", "ON", "false", "False", "FALSE", "n", "N", "no", "No", "NO",
        "off", "Off", "OFF",
    ];
    if BOOLS.contains(&s) {
        return Tag::Bool;
    }
    if int_yaml11(s) || int_core(s) {
        return Tag::Int;
    }
    if float_yaml11(s) || float_core(s) {
        return Tag::Float;
    }
    if timestamp(s) {
        return Tag::Timestamp;
    }
    if s == "<<" {
        return Tag::Merge;
    }
    Tag::Str
}

// --- presenter -------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Ctx {
    block: bool,
    compact: bool,
    iskey: bool,
}

fn next_line(level: usize) -> String {
    format!("\n{}", " ".repeat(INDENT * level))
}

fn write_node(node: &Node, level: usize, ctx: Ctx) -> String {
    match node {
        Node::Map(items) if ctx.block && !items.is_empty() => write_block_mapping(items, level, ctx.compact),
        Node::Map(items) => write_flow_mapping(items, level),
        Node::Seq(items) if ctx.block && !items.is_empty() => write_block_sequence(items, level, ctx.compact),
        Node::Seq(items) => {
            let parts: Vec<String> =
                items.iter().map(|i| write_node(i, level, Ctx { block: false, compact: false, iskey: false })).collect();
            format!("[{}]", parts.join(", "))
        }
        Node::Scalar { tag, value } => {
            let layout = Layout::new(level);
            let style = resolve_scalar_style(*tag, value, &layout, ctx.iskey, ctx.block);
            render_scalar(value, style, &layout)
        }
    }
}

fn write_flow_mapping(items: &[(Node, Node)], level: usize) -> String {
    let mut result = String::new();
    for (k, v) in items {
        if !result.is_empty() {
            result.push_str(", ");
        }
        let key = write_node(k, level, Ctx { block: false, compact: false, iskey: true });
        let explicit = utf16_len(&key) > 1024;
        if explicit {
            result.push_str("? ");
        }
        let value = write_node(v, level, Ctx { block: false, compact: false, iskey: false });
        let sep = if value.is_empty() { "" } else { " " };
        result.push_str(&format!("{key}:{sep}{value}"));
    }
    format!("{{{result}}}")
}

fn write_block_sequence(items: &[Node], level: usize, compact: bool) -> String {
    let mut result = String::new();
    for item in items {
        let text = write_node(item, level + 1, Ctx { block: true, compact: true, iskey: false });
        if !compact || !result.is_empty() {
            result.push_str(&next_line(level));
        }
        if text.is_empty() || text.starts_with('\n') {
            result.push('-');
        } else {
            result.push_str("- ");
        }
        result.push_str(&text);
    }
    result
}

fn write_block_mapping(items: &[(Node, Node)], level: usize, compact: bool) -> String {
    let mut result = String::new();
    for (k, v) in items {
        let mut pair = String::new();
        if !compact || !result.is_empty() {
            pair.push_str(&next_line(level));
        }
        let key_text = write_node(k, level + 1, Ctx { block: true, compact: true, iskey: true });
        let key_value = match k {
            Node::Scalar { value, .. } => value.as_str(),
            _ => "",
        };
        let explicit = key_value.contains('\n') || utf16_len(&key_text) > 1024;
        if explicit {
            pair.push_str(if key_text.starts_with('\n') { "?" } else { "? " });
        }
        pair.push_str(&key_text);
        if explicit {
            pair.push_str(&next_line(level));
        }
        let value_text = write_node(v, level + 1, Ctx { block: true, compact: explicit, iskey: false });
        let bare = key_value.is_empty() && !key_text.is_empty() && !key_text.ends_with('\'') && !key_text.ends_with('"');
        let sep = if !explicit && bare { " " } else { "" };
        if value_text.is_empty() || value_text.starts_with('\n') {
            pair.push_str(&format!("{sep}:"));
        } else {
            pair.push_str(&format!("{sep}: "));
        }
        pair.push_str(&value_text);
        result.push_str(&pair);
    }
    result
}

struct Layout {
    indent: usize,
    block_indent: usize,
    line_width: i64,
}

impl Layout {
    fn new(level: usize) -> Self {
        let indent = INDENT * level.max(1);
        Layout {
            indent,
            block_indent: if level == 0 { INDENT + 1 } else { INDENT },
            line_width: (LINE_WIDTH.min(40)).max(LINE_WIDTH - indent as i64),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Style {
    Plain,
    Single,
    Literal,
    Folded,
    Double,
}

fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

fn from_utf16(u: &[u16]) -> String {
    String::from_utf16_lossy(u)
}

fn code_point_at(s: &[u16], pos: usize) -> u32 {
    let first = s[pos] as u32;
    if (0xD800..=0xDBFF).contains(&first) && pos + 1 < s.len() {
        let second = s[pos + 1] as u32;
        if (0xDC00..=0xDFFF).contains(&second) {
            return (first - 0xD800) * 1024 + second - 0xDC00 + 0x10000;
        }
    }
    first
}

const BOM: u32 = 0xFEFF;

fn is_whitespace(c: u32) -> bool {
    c == 32 || c == 9
}

fn is_printable(c: u32) -> bool {
    (32..=126).contains(&c)
        || ((161..=0xD7FF).contains(&c) && c != 0x2028 && c != 0x2029)
        || ((0xE000..=0xFFFD).contains(&c) && c != BOM)
        || (0x10000..=0x10FFFF).contains(&c)
}

fn is_ns_char_or_whitespace(c: u32) -> bool {
    is_printable(c) && c != BOM && c != 13 && c != 10
}

fn is_plain_safe(c: u32, prev: i64, inblock: bool) -> bool {
    let c_ns_or_ws = is_ns_char_or_whitespace(c);
    let c_ns = c_ns_or_ws && !is_whitespace(c);
    let prev_colon = prev == ':' as i64;
    let flow_ok = if inblock { c_ns_or_ws } else { c_ns_or_ws && !matches!(char::from_u32(c), Some(',' | '[' | ']' | '{' | '}')) };
    let prev_u = if prev >= 0 { prev as u32 } else { u32::MAX };
    (flow_ok && c != '#' as u32 && !(prev_colon && !c_ns))
        || (prev >= 0 && is_ns_char_or_whitespace(prev_u) && !is_whitespace(prev_u) && c == '#' as u32)
        || (prev_colon && c_ns)
}

fn is_plain_safe_first(c: u32) -> bool {
    is_printable(c)
        && c != BOM
        && !is_whitespace(c)
        && !matches!(
            char::from_u32(c),
            Some('-' | '?' | ':' | ',' | '[' | ']' | '{' | '}' | '#' | '&' | '*' | '!' | '|' | '=' | '>' | '\'' | '"' | '%' | '@' | '`')
        )
}

fn is_plain_safe_at_start(s: &[u16], inblock: bool) -> bool {
    let first = code_point_at(s, 0);
    if is_plain_safe_first(first) {
        return true;
    }
    if s.len() > 1 && matches!(char::from_u32(first), Some('-' | '?' | ':')) {
        let second = code_point_at(s, 1);
        return !is_whitespace(second) && is_plain_safe(second, first as i64, inblock);
    }
    false
}

fn is_plain_safe_last(c: u32) -> bool {
    !is_whitespace(c) && c != ':' as u32
}

fn starts_with_document_separator(s: &[u16]) -> bool {
    if s.len() < 3 {
        return false;
    }
    let m = s[0];
    if (m != '-' as u16 && m != '.' as u16) || s[1] != m || s[2] != m {
        return false;
    }
    if s.len() == 3 {
        return true;
    }
    let f = s[3] as u32;
    is_whitespace(f) || f == 13 || f == 10
}

fn need_indent_indicator(s: &[u16]) -> bool {
    let mut i = 0;
    while i < s.len() && s[i] == '\n' as u16 {
        i += 1;
    }
    i < s.len() && s[i] == ' ' as u16
}

fn choose_scalar_style(s: &[u16], layout: &Layout, single_line_only: bool, inblock: bool) -> Style {
    let line_width = layout.line_width;
    let mut has_line_break = false;
    let mut has_foldable = false;
    let mut prev_break: i64 = -1;
    let mut prev: i64 = -1;
    let mut plain =
        !starts_with_document_separator(s) && is_plain_safe_at_start(s, inblock) && is_plain_safe_last(code_point_at(s, s.len() - 1));
    let mut i = 0usize;
    while i < s.len() {
        let c = code_point_at(s, i);
        if !single_line_only && c == 10 {
            has_line_break = true;
            has_foldable =
                has_foldable || (i as i64 - prev_break - 1 > line_width && s.get((prev_break + 1) as usize) != Some(&(' ' as u16)));
            prev_break = i as i64;
        } else if !is_printable(c) {
            return Style::Double;
        }
        plain = plain && is_plain_safe(c, prev, inblock);
        prev = c as i64;
        i += if c >= 0x10000 { 2 } else { 1 };
    }
    if !single_line_only {
        has_foldable = has_foldable || (i as i64 - prev_break - 1 > line_width && s.get((prev_break + 1) as usize) != Some(&(' ' as u16)));
    }
    if !has_line_break && !has_foldable {
        return if plain { Style::Plain } else { Style::Single };
    }
    if layout.block_indent > 9 && need_indent_indicator(s) {
        return Style::Double;
    }
    if has_foldable { Style::Folded } else { Style::Literal }
}

fn resolve_scalar_style(tag: Tag, value: &str, layout: &Layout, iskey: bool, inblock: bool) -> Style {
    let single_line_only = iskey || !inblock;
    if value.is_empty() {
        return if resolve_implicit(value) == tag { Style::Plain } else { Style::Single };
    }
    let s = utf16(value);
    let style = choose_scalar_style(&s, layout, single_line_only, inblock);
    if style == Style::Plain && resolve_implicit(value) != tag {
        return Style::Single;
    }
    style
}

fn render_scalar(value: &str, style: Style, layout: &Layout) -> String {
    let s = utf16(value);
    match style {
        Style::Plain => from_utf16(&encode_flow_breaks(&s, layout.indent)),
        Style::Single => format!("'{}'", from_utf16(&encode_flow_breaks(&s, layout.indent)).replace('\'', "''")),
        Style::Literal => {
            format!("|{}{}", block_header(&s, layout.block_indent), from_utf16(&drop_ending_newline(&indent_string(&s, layout.indent))))
        }
        Style::Folded => format!(
            ">{}{}",
            block_header(&s, layout.block_indent),
            from_utf16(&drop_ending_newline(&indent_string(&fold_block_scalar(&s, layout.line_width), layout.indent)))
        ),
        Style::Double => format!("\"{}\"", escape_string(&s)),
    }
}

fn block_header(s: &[u16], indent_per_level: usize) -> String {
    let indicator = if need_indent_indicator(s) { indent_per_level.to_string() } else { String::new() };
    let nl = '\n' as u16;
    let clip = s.last() == Some(&nl);
    let keep = clip && ((s.len() >= 2 && s[s.len() - 2] == nl) || s == [nl]);
    format!(
        "{indicator}{}\n",
        if keep {
            "+"
        } else if clip {
            ""
        } else {
            "-"
        }
    )
}

fn encode_flow_breaks(s: &[u16], indent: usize) -> Vec<u16> {
    let nl = '\n' as u16;
    let Some(first) = s.iter().position(|c| *c == nl) else {
        return s.to_vec();
    };
    let pad: Vec<u16> = vec![' ' as u16; indent];
    let mut out = s[..first].to_vec();
    let mut i = first;
    while i < s.len() {
        // (\n+)([^\n]*)
        let start = i;
        while i < s.len() && s[i] == nl {
            i += 1;
        }
        let breaks = i - start;
        let line_start = i;
        while i < s.len() && s[i] != nl {
            i += 1;
        }
        out.extend(std::iter::repeat_n(nl, breaks + 1));
        out.extend(&pad);
        out.extend(&s[line_start..i]);
    }
    out
}

fn indent_string(s: &[u16], spaces: usize) -> Vec<u16> {
    let nl = '\n' as u16;
    let ind: Vec<u16> = vec![' ' as u16; spaces];
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < s.len() {
        let line = match s[pos..].iter().position(|c| *c == nl) {
            Some(n) => {
                let l = &s[pos..pos + n + 1];
                pos += n + 1;
                l
            }
            None => {
                let l = &s[pos..];
                pos = s.len();
                l
            }
        };
        if !line.is_empty() && line != [nl] {
            out.extend(&ind);
        }
        out.extend(line);
    }
    out
}

fn drop_ending_newline(s: &[u16]) -> Vec<u16> {
    if s.last() == Some(&('\n' as u16)) { s[..s.len() - 1].to_vec() } else { s.to_vec() }
}

fn fold_block_scalar(s: &[u16], width: i64) -> Vec<u16> {
    let nl = '\n' as u16;
    let sp = ' ' as u16;
    let first = s.iter().position(|c| *c == nl).unwrap_or(s.len());
    let mut result = fold_line(&s[..first], width);
    let mut prev_more_indented = s.first() == Some(&nl) || s.first() == Some(&sp);
    let mut i = first;
    while i < s.len() {
        let start = i;
        while i < s.len() && s[i] == nl {
            i += 1;
        }
        let prefix = &s[start..i];
        let ls = i;
        while i < s.len() && s[i] != nl {
            i += 1;
        }
        let line = &s[ls..i];
        let more_indented = line.first() == Some(&sp);
        result.extend(prefix);
        if !prev_more_indented && !more_indented && !line.is_empty() {
            result.push(nl);
        }
        result.extend(fold_line(line, width));
        prev_more_indented = more_indented;
    }
    result
}

fn fold_line(line: &[u16], width: i64) -> Vec<u16> {
    let sp = ' ' as u16;
    let nl = '\n' as u16;
    if line.is_empty() || line[0] == sp {
        return line.to_vec();
    }
    let mut start: usize = 0;
    let mut curr: usize = 0;
    let mut result: Vec<u16> = Vec::new();
    // / [^ ]/g
    let mut idx = 0;
    while idx + 1 < line.len() {
        if line[idx] == sp && line[idx + 1] != sp {
            let next = idx;
            if next as i64 - start as i64 > width {
                let end = if curr > start { curr } else { next };
                result.push(nl);
                result.extend(&line[start..end]);
                start = end + 1;
            }
            curr = next;
            idx += 2;
        } else {
            idx += 1;
        }
    }
    result.push(nl);
    if line.len() as i64 - start as i64 > width && curr > start {
        result.extend(&line[start..curr]);
        result.push(nl);
        result.extend(&line[curr + 1..]);
    } else {
        result.extend(&line[start.min(line.len())..]);
    }
    result[1..].to_vec()
}

fn escape_string(s: &[u16]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < s.len() {
        let c = code_point_at(s, i);
        let esc = match c {
            0 => Some("\\0"),
            7 => Some("\\a"),
            8 => Some("\\b"),
            9 => Some("\\t"),
            10 => Some("\\n"),
            11 => Some("\\v"),
            12 => Some("\\f"),
            13 => Some("\\r"),
            27 => Some("\\e"),
            34 => Some("\\\""),
            92 => Some("\\\\"),
            133 => Some("\\N"),
            160 => Some("\\_"),
            8232 => Some("\\L"),
            8233 => Some("\\P"),
            _ => None,
        };
        if let Some(e) = esc {
            out.push_str(e);
        } else if is_printable(c) {
            out.push_str(&from_utf16(&s[i..i + if c >= 0x10000 { 2 } else { 1 }]));
        } else {
            let hex = format!("{c:X}");
            if c <= 255 {
                out.push_str(&format!("\\x{}{hex}", "0".repeat(2usize.saturating_sub(hex.len()))));
            } else {
                out.push_str(&format!("\\u{}{hex}", "0".repeat(4usize.saturating_sub(hex.len()))));
            }
        }
        i += if c >= 0x10000 { 2 } else { 1 };
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn basic_document() {
        let v = json!({
            "openapi": "3.0.0",
            "paths": { "/health": { "get": { "parameters": [], "responses": { "200": { "description": "" } }, "tags": ["health"] } } },
            "info": { "title": "API", "version": "1.0", "contact": {} },
            "n": 1, "f": 1.5, "b": true, "z": null, "s": "1.0", "y": "yes", "colon": "a: b", "hash": "a #b"
        });
        let out = dump(&v);
        let expected = "openapi: 3.0.0\npaths:\n  /health:\n    get:\n      parameters: []\n      responses:\n        '200':\n          description: ''\n      tags:\n        - health\ninfo:\n  title: API\n  version: '1.0'\n  contact: {}\n'n': 1\nf: 1.5\nb: true\nz: null\ns: '1.0'\n'y': 'yes'\ncolon: 'a: b'\nhash: 'a #b'\n";
        assert_eq!(out, expected);
    }

    #[test]
    fn sequences_of_mappings() {
        let v = json!({ "parameters": [{ "name": "id", "in": "path" }] });
        assert_eq!(dump(&v), "parameters:\n  - name: id\n    in: path\n");
    }

    #[test]
    fn long_and_multiline_strings() {
        let long = "word ".repeat(30).trim_end().to_string();
        let out = dump(&json!({ "d": long }));
        assert!(out.starts_with("d: >-\n  word word"), "{out}");
        let out = dump(&json!({ "d": "line1\nline2" }));
        assert_eq!(out, "d: |-\n  line1\n  line2\n");
    }

    #[test]
    fn numbers() {
        assert_eq!(js_number_string(1e21), "1e+21");
        assert_eq!(js_number_string(0.000001), "0.000001");
        assert_eq!(js_number_string(1e-7), "1e-7");
        assert_eq!(js_number_string(123.456), "123.456");
        assert_eq!(js_number_string(9007199254740991.0), "9007199254740991");
        assert_eq!(dump(&json!({ "x": 1e21 })), "x: 1.e+21\n");
    }

    #[test]
    fn implicit_types_are_quoted() {
        for s in ["true", "null", "~", "12", "0x1F", "1_000", "1.5", ".inf", "2024-01-01", "<<", "on"] {
            assert!(dump(&json!(s)).starts_with('\''), "{s}");
        }
        for s in ["hello", "1.2.3", "v1", "0000-01-01"] {
            assert!(!dump(&json!(s)).starts_with('\''), "{s}");
        }
    }
}
