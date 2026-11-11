//! `decimal!("1.50")`: the literal is parsed when the program is compiled, with the grammar of `Decimal::parse`
//! (`[+-]digits[.digits][e[+-]n]`, `Infinity`, `NaN`, `0x` / `0b` / `0o` numbers with an optional fraction and binary
//! exponent, one `_` between two decimal digits), and expands to a constructor that takes the already computed
//! coefficient and exponents, so nothing is parsed at run time.

use proc_macro2::TokenStream;
use quote::quote;

/// The digits of `digits` in base `radix`, as little-endian 32-bit limbs (`None` for a character that is not a digit).
fn limbs(digits: &[u8], radix: u32) -> Option<Vec<u32>> {
    let mut out: Vec<u32> = Vec::new();
    for &c in digits {
        let mut carry = u64::from(char::from(c).to_digit(radix)?);
        for limb in out.iter_mut() {
            let v = u64::from(*limb) * u64::from(radix) + carry;
            *limb = v as u32;
            carry = v >> 32;
        }
        if carry > 0 {
            out.push(carry as u32);
        }
    }
    Some(out)
}

fn finite(neg: bool, limbs: &[u32], exp: i64) -> TokenStream {
    quote!(::corvis_orm::Decimal::from_literal_parts(#neg, &[#(#limbs),*], #exp))
}

/// Removes each `_` that sits between two decimal digits (`Decimal::parse` does the same).
fn strip_separators(s: &str) -> String {
    let b = s.as_bytes();
    s.char_indices()
        .filter(|&(i, c)| !(c == '_' && i > 0 && b[i - 1].is_ascii_digit() && b.get(i + 1).is_some_and(u8::is_ascii_digit)))
        .map(|(_, c)| c)
        .collect()
}

/// The expansion of a lowercase `0x` / `0b` / `0o` literal (`digits[.digits]` or `.digits`, then `p[+-]n`), `None`
/// when `s` has none of those prefixes.
fn radix_literal(neg: bool, s: &str) -> Option<Result<TokenStream, &'static str>> {
    let (radix, bits, rest) = match s.get(..2)? {
        "0x" => (16u32, 4u64, &s[2..]),
        "0b" => (2, 1, &s[2..]),
        "0o" => (8, 3, &s[2..]),
        _ => return None,
    };
    Some(radix_value(neg, radix, bits, rest))
}

/// The expansion of the part of a radix literal after its prefix.
fn radix_value(neg: bool, radix: u32, bits: u64, rest: &str) -> Result<TokenStream, &'static str> {
    let (mantissa, p) = match rest.split_once('p') {
        Some((m, e)) => {
            let digits = e.strip_prefix(['+', '-']).unwrap_or(e);
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return Err("a binary exponent needs digits");
            }
            let p = digits.bytes().fold(0i64, |p, d| p.saturating_mul(10).saturating_add(i64::from(d - b'0')));
            (m, if e.starts_with('-') { -p } else { p })
        }
        None => (rest, 0),
    };
    let (int, frac) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if int.is_empty() && frac.is_empty() {
        return Err("a base prefix needs digits");
    }
    let limbs = limbs(format!("{int}{frac}").as_bytes(), radix).ok_or("a digit is not valid for its base")?;
    let frac_bits = bits * frac.len() as u64;
    Ok(quote!(::corvis_orm::Decimal::from_radix_literal_parts(#neg, &[#(#limbs),*], #frac_bits, #p)))
}

/// The expansion of `decimal!(text)`, or why `text` is not a decimal literal.
pub(crate) fn expand(text: &str) -> Result<TokenStream, &'static str> {
    let (neg, raw) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    if !raw.contains('_') {
        if raw == "Infinity" {
            return Ok(quote!(::corvis_orm::Decimal::infinity(#neg)));
        }
        if raw == "NaN" {
            return Ok(quote!(::corvis_orm::Decimal::nan()));
        }
    }
    let body = strip_separators(raw);
    if let Some(radix) = radix_literal(neg, &body.to_ascii_lowercase()) {
        return radix;
    }
    let bytes = body.as_bytes();
    let digits_from = |mut i: usize| {
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        (start, i)
    };
    let (_, int_end) = digits_from(0);
    let mut i = int_end;
    let mut frac = 0..0;
    if i < bytes.len() && bytes[i] == b'.' {
        let (start, end) = digits_from(i + 1);
        frac = start..end;
        i = end;
    }
    if int_end == 0 && frac.is_empty() {
        return Err("expected digits");
    }
    let mut exp: i64 = 0;
    if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
        i += 1;
        let negative_exp = i < bytes.len() && bytes[i] == b'-';
        if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
            i += 1;
        }
        let (start, end) = digits_from(i);
        if start == end {
            return Err("an exponent needs digits");
        }
        // Saturating, like `Decimal::parse`: beyond `maxE` the value is out of range anyway.
        exp = bytes[start..end].iter().fold(0i64, |e, d| e.saturating_mul(10).saturating_add(i64::from(d - b'0')));
        if negative_exp {
            exp = -exp;
        }
        i = end;
    }
    if i != bytes.len() {
        return Err("unexpected characters");
    }
    let digits: Vec<u8> = bytes[..int_end].iter().chain(&bytes[frac.clone()]).copied().collect();
    let lead = digits.iter().take_while(|d| **d == b'0').count();
    if lead == digits.len() {
        return Ok(finite(neg, &[], 0));
    }
    let trail = digits.iter().rev().take_while(|d| **d == b'0').count();
    let significant = &digits[lead..digits.len() - trail];
    // Out of decimal.js' exponent range, the constructor gives `Infinity` or zero, as `Decimal::parse` does.
    let exp = exp.saturating_sub(frac.len() as i64).saturating_add(trail as i64);
    let limbs = limbs(significant, 10).ok_or("expected digits")?;
    Ok(finite(neg, &limbs, exp))
}
