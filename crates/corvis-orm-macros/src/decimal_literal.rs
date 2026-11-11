//! `decimal!("1.50")`: the literal is parsed when the program is compiled, with the grammar of `Decimal::parse`
//! (`[+-]digits[.digits][e[+-]n]`, `Infinity`, `NaN`, and `0x` / `0b` / `0o` integers), and expands to a
//! constructor that takes the already computed coefficient and exponent, so nothing is parsed at run time.

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

/// The expansion of `decimal!(text)`, or why `text` is not a decimal literal.
pub(crate) fn expand(text: &str) -> Result<TokenStream, &'static str> {
    let (neg, body) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    if body == "Infinity" {
        return Ok(quote!(::corvis_orm::Decimal::infinity(#neg)));
    }
    if body == "NaN" {
        return Ok(quote!(::corvis_orm::Decimal::nan()));
    }
    let lower = body.to_ascii_lowercase();
    for (prefix, radix) in [("0x", 16u32), ("0b", 2), ("0o", 8)] {
        if let Some(digits) = lower.strip_prefix(prefix) {
            if digits.is_empty() {
                return Err("an integer prefix needs digits");
            }
            let limbs = limbs(digits.as_bytes(), radix).ok_or("a digit is not valid for its base")?;
            return Ok(finite(neg, &limbs, 0));
        }
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
