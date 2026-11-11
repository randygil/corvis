//! `Decimal`: an arbitrary-precision decimal compatible with [decimal.js] (the decimal
//! type of the JavaScript ORM clients).
//!
//! The semantics follow decimal.js 10.x:
//!
//! - Values are exact; the constructor never rounds. Trailing zeros are not kept
//!   (`"1.50"` prints as `"1.5"`).
//! - Arithmetic (`plus`, `minus`, `times`, `div`, `modulo`, `pow`, `sqrt`) rounds the
//!   result to `precision` significant digits (default 20) with the `rounding` mode of
//!   the configuration (default `ROUND_HALF_UP`).
//! - `abs`, `neg`, `round`, `floor`, `ceil`, `trunc`, `to_decimal_places` never round
//!   to `precision`.
//! - `to_string()` uses exponential notation when the exponent is `<= to_exp_neg` (-7)
//!   or `>= to_exp_pos` (21). JSON (`serde`) uses `valueOf()`/`toJSON()`, which is
//!   `toString()` but keeps the sign of negative zero (`"-0"`).
//! - Division by zero gives `Infinity` / `NaN` instead of panicking, like decimal.js.
//!
//! The configuration travels with each value (like `x.constructor` in decimal.js):
//! `Decimal::clone_with(...)` is the equivalent of `Decimal.clone({...})`.
//!
//! ```
//! use corvis_orm::{Decimal, decimal};
//! let a = decimal!("0.1");
//! assert_eq!((a.clone() + decimal!("0.2")).to_string(), "0.3");
//! assert_eq!(Decimal::from(1).div(3).to_string(), "0.33333333333333333333");
//! assert_eq!(decimal!("2.345").to_fixed(2), "2.35");
//! assert_eq!(decimal!("-2.345").to_decimal_places_with(2, Decimal::ROUND_HALF_EVEN).to_string(), "-2.34");
//! ```
//!
//! Text becomes a `Decimal` only through conversions that report invalid input: [`Decimal::parse`], `FromStr`
//! (`"12.5".parse::<Decimal>()`) and `TryFrom<&str>` / `TryFrom<String>`, for text read at run time, and the
//! [`decimal!`](crate::decimal!) literal, checked when the program is compiled:
//!
//! ```
//! use corvis_orm::{Decimal, decimal};
//! assert_eq!(decimal!("1.50"), Decimal::from(1.5));
//! assert!("12,5".parse::<Decimal>().is_err());
//! assert!(Decimal::try_from("12,5").is_err());
//! ```
//!
//! An invalid literal does not compile:
//!
//! ```compile_fail
//! let price = corvis_orm::decimal!("1.2.3");
//! ```
//!
//! [decimal.js]: https://mikemcl.github.io/decimal.js/

use std::cmp::Ordering;
use std::fmt;
use std::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Rem, RemAssign, Sub, SubAssign};
use std::str::FromStr;

use num_bigint::BigUint;
use num_integer::Integer;
use num_traits::{One, ToPrimitive, Zero};

/// Rounding modes (`Decimal.ROUND_UP` ... `Decimal.EUCLID`), with the same numeric values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Rounding {
    /// 0: away from zero.
    Up = 0,
    /// 1: towards zero.
    Down = 1,
    /// 2: towards +Infinity.
    Ceil = 2,
    /// 3: towards -Infinity.
    Floor = 3,
    /// 4: to nearest, ties away from zero.
    HalfUp = 4,
    /// 5: to nearest, ties towards zero.
    HalfDown = 5,
    /// 6: to nearest, ties to even.
    HalfEven = 6,
    /// 7: to nearest, ties towards +Infinity.
    HalfCeil = 7,
    /// 8: to nearest, ties towards -Infinity.
    HalfFloor = 8,
    /// 9: only meaningful as `modulo` mode (Euclidean modulo, always positive).
    Euclid = 9,
}

impl Rounding {
    /// From the decimal.js numeric constant (`0..=9`).
    pub fn from_u8(n: u8) -> Option<Rounding> {
        Some(match n {
            0 => Rounding::Up,
            1 => Rounding::Down,
            2 => Rounding::Ceil,
            3 => Rounding::Floor,
            4 => Rounding::HalfUp,
            5 => Rounding::HalfDown,
            6 => Rounding::HalfEven,
            7 => Rounding::HalfCeil,
            8 => Rounding::HalfFloor,
            9 => Rounding::Euclid,
            _ => return None,
        })
    }
}

/// Configuration of a Decimal "constructor" (`Decimal.set` / `Decimal.clone` options).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecimalConfig {
    /// Significant digits of the result of an operation (1..=1e9). Default 20.
    pub precision: u32,
    /// Default rounding mode. Default `ROUND_HALF_UP`.
    pub rounding: Rounding,
    /// Exponent at and below which `to_string` uses exponential notation. Default -7.
    pub to_exp_neg: i64,
    /// Exponent at and above which `to_string` uses exponential notation. Default 21.
    pub to_exp_pos: i64,
    /// Modulo mode. Default `ROUND_DOWN` (like JavaScript's `%`).
    pub modulo: Rounding,
}

impl DecimalConfig {
    /// decimal.js defaults.
    pub const DEFAULT: DecimalConfig =
        DecimalConfig { precision: 20, rounding: Rounding::HalfUp, to_exp_neg: -7, to_exp_pos: 21, modulo: Rounding::Down };
}

impl Default for DecimalConfig {
    fn default() -> Self {
        DecimalConfig::DEFAULT
    }
}

static DEFAULT_CONFIG: DecimalConfig = DecimalConfig::DEFAULT;

/// A Decimal "constructor" bound to a configuration: the result of `Decimal.clone({...})`.
///
/// ```
/// use corvis_orm::{Decimal, DecimalConfig, DecimalCtor};
/// static CFG: DecimalConfig = DecimalConfig { precision: 40, to_exp_neg: -25, to_exp_pos: 30, ..DecimalConfig::DEFAULT };
/// pub static D: DecimalCtor = DecimalCtor::new(&CFG);
/// assert_eq!(D.of(1).div(3).to_string(), "0.3333333333333333333333333333333333333333");
/// ```
#[derive(Debug, Clone, Copy)]
pub struct DecimalCtor {
    cfg: &'static DecimalConfig,
}

impl DecimalCtor {
    pub const fn new(cfg: &'static DecimalConfig) -> Self {
        DecimalCtor { cfg }
    }
    /// The default constructor (`Decimal`).
    pub const fn default_ctor() -> Self {
        DecimalCtor { cfg: &DEFAULT_CONFIG }
    }
    pub fn config(&self) -> &'static DecimalConfig {
        self.cfg
    }
    /// `new D(v)`: the value of `v` (exact, not rounded) with this configuration.
    pub fn of(&self, v: impl Into<Decimal>) -> Decimal {
        let mut d = v.into();
        d.cfg = self.cfg;
        d
    }
    /// `new D(str)` that reports invalid input instead of panicking.
    pub fn parse(&self, s: &str) -> Result<Decimal, DecimalError> {
        let mut d = Decimal::parse(s)?;
        d.cfg = self.cfg;
        Ok(d)
    }
    /// `D.max(...)`
    pub fn max<I: IntoIterator<Item = T>, T: Into<Decimal>>(&self, items: I) -> Decimal {
        self.of(Decimal::max(items))
    }
    /// `D.min(...)`
    pub fn min<I: IntoIterator<Item = T>, T: Into<Decimal>>(&self, items: I) -> Decimal {
        self.of(Decimal::min(items))
    }
    /// `D.sum(...)` (rounded to this constructor's precision).
    pub fn sum<I: IntoIterator<Item = T>, T: Into<Decimal>>(&self, items: I) -> Decimal {
        let mut acc = self.of(0);
        for x in items {
            acc = acc.plus(x);
        }
        acc
    }
}

/// `[DecimalError] Invalid argument: ...`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecimalError {
    pub message: String,
}

impl fmt::Display for DecimalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for DecimalError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Finite,
    Infinite,
    NaN,
}

/// Arbitrary-precision decimal with decimal.js semantics. See the [module docs](self).
#[derive(Clone)]
pub struct Decimal {
    kind: Kind,
    /// Sign (also for zero and infinities).
    neg: bool,
    /// Coefficient without trailing zeros (zero is `0` with `exp == 0`).
    coeff: BigUint,
    /// value = coeff * 10^exp
    exp: i64,
    cfg: &'static DecimalConfig,
}

/// decimal.js `Decimal.maxE`: a result whose exponent (of its most significant digit) is above
/// this is `Infinity`.
const MAX_E: i64 = 9_000_000_000_000_000;
/// decimal.js `Decimal.minE`: a result whose exponent is below this is zero.
const MIN_E: i64 = -MAX_E;

/// Longest string a conversion may produce, like the maximum string length of V8
/// (`2^29 - 24`): beyond it JavaScript throws `RangeError: Invalid string length`.
const MAX_STRING_LEN: i64 = (1 << 29) - 24;

fn pow10(n: u64) -> BigUint {
    // Never truncate the exponent: a wrong power of ten would give a wrong result silently.
    let n = u32::try_from(n).expect("[DecimalError] Exponent out of range");
    BigUint::from(10u32).pow(n)
}

/// Removes each `_` that sits between two decimal digits, like decimal.js (`str.replace(/(\d)_(?=\d)/g, '$1')`).
fn strip_separators(s: &str) -> String {
    let b = s.as_bytes();
    s.char_indices()
        .filter(|&(i, c)| !(c == '_' && i > 0 && b[i - 1].is_ascii_digit() && b.get(i + 1).is_some_and(u8::is_ascii_digit)))
        .map(|(_, c)| c)
        .collect()
}

/// A lowercase `0x` / `0b` / `0o` number: `digits[.digits]` or `.digits`, then an optional `p[+-]n` (decimal.js
/// `isHex` / `isBinary` / `isOctal`). `None` when `s` is not one.
fn parse_radix(neg: bool, s: &str) -> Option<Decimal> {
    let (radix, bits, rest) = match s.get(..2)? {
        "0x" => (16, 4, &s[2..]),
        "0b" => (2, 1, &s[2..]),
        "0o" => (8, 3, &s[2..]),
        _ => return None,
    };
    let (mantissa, p) = match rest.split_once('p') {
        Some((m, e)) => {
            let digits = e.strip_prefix(['+', '-']).unwrap_or(e);
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let p = digits.bytes().fold(0i64, |p, d| p.saturating_mul(10).saturating_add(i64::from(d - b'0')));
            (m, if e.starts_with('-') { -p } else { p })
        }
        None => (rest, 0),
    };
    let (int, frac) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if (int.is_empty() && frac.is_empty()) || !int.chars().chain(frac.chars()).all(|c| c.is_digit(radix)) {
        return None;
    }
    let n = BigUint::parse_bytes(format!("{int}{frac}").as_bytes(), radix)?;
    Some(binary_scaled(neg, n, bits * frac.len() as u64, p))
}

/// `n / 2^frac_bits * 2^p`, like decimal.js `parseOther`: the fraction is exact, and the binary exponent multiplies by
/// the JavaScript number `2 ** p` when `|p| < 54` and by `Decimal.pow(2, p)` (rounded to the default precision)
/// otherwise; the product itself is exact.
fn binary_scaled(neg: bool, n: BigUint, frac_bits: u64, p: i64) -> Decimal {
    if n.is_zero() {
        return Decimal { kind: Kind::Finite, neg, coeff: BigUint::zero(), exp: 0, cfg: &DEFAULT_CONFIG };
    }
    let five = BigUint::from(5u32).pow(u32::try_from(frac_bits).unwrap_or(u32::MAX));
    let x = Decimal::finite(neg, n * five, -i64::try_from(frac_bits).unwrap_or(i64::MAX), &DEFAULT_CONFIG);
    match p {
        0 => x,
        p if p.abs() < 54 => x.times_exact(&Decimal::from_f64(2f64.powi(p as i32))),
        p => x.times_exact(&Decimal::from(2).pow(p)),
    }
}

/// Number of decimal digits of `n` (1 for zero).
fn ndigits(n: &BigUint) -> i64 {
    if let Some(v) = n.to_u128() {
        if v == 0 {
            return 1;
        }
        return v.ilog10() as i64 + 1;
    }
    // Estimate from the bit length and correct.
    let bits = n.bits();
    let est = ((bits - 1) as f64 * std::f64::consts::LOG10_2).floor() as u64;
    let mut d = est as i64 + 1;
    if *n >= pow10(d as u64) {
        d += 1;
    }
    d
}

/// Removes trailing zeros, adjusting the exponent.
fn normalize(mut c: BigUint, mut e: i64) -> (BigUint, i64) {
    if c.is_zero() {
        return (c, 0);
    }
    // Fast path for small values.
    if let Some(mut v) = c.to_u128() {
        while v % 10 == 0 {
            v /= 10;
            e += 1;
        }
        return (BigUint::from(v), e);
    }
    // A number divisible by 10^k is divisible by 2^k, so the binary trailing zeros bound the
    // decimal ones. Strip them in chunks of 10^(2^j), largest first: O(log k) divisions instead
    // of one division per zero (which is quadratic for long runs of zeros).
    let bound = c.trailing_zeros().unwrap_or(0);
    if bound == 0 {
        return (c, e);
    }
    let mut k = 1u64 << (63 - bound.leading_zeros());
    while k > 0 {
        let (q, r) = c.div_rem(&pow10(k));
        if r.is_zero() {
            c = q;
            e += k as i64;
        }
        k >>= 1;
    }
    (c, e)
}

/// Exponent of the most significant digit of `coeff * 10^exp` (`coeff` non-zero).
fn sci_exp_of(coeff: &BigUint, exp: i64) -> i64 {
    exp.saturating_add(ndigits(coeff) - 1)
}

/// Whether to round the magnitude up when dropping digits.
/// `cmp_half`: the dropped part compared with one half of the last kept unit.
fn round_up(rm: Rounding, neg: bool, nonzero: bool, cmp_half: Ordering, odd: bool) -> bool {
    if !nonzero {
        return false;
    }
    match rm {
        Rounding::Up => true,
        Rounding::Down => false,
        Rounding::Ceil => !neg,
        Rounding::Floor => neg,
        Rounding::HalfUp => cmp_half != Ordering::Less,
        Rounding::HalfDown => cmp_half == Ordering::Greater,
        Rounding::HalfEven | Rounding::Euclid => cmp_half == Ordering::Greater || (cmp_half == Ordering::Equal && odd),
        Rounding::HalfCeil => cmp_half == Ordering::Greater || (cmp_half == Ordering::Equal && !neg),
        Rounding::HalfFloor => cmp_half == Ordering::Greater || (cmp_half == Ordering::Equal && neg),
    }
}

/// Rounds `coeff * 10^exp` (+ a "sticky" non-zero tail below it) so that no digit
/// remains below `10^target`. Returns a normalized coefficient/exponent.
fn round_at(neg: bool, coeff: BigUint, exp: i64, sticky: bool, target: i64, rm: Rounding) -> (BigUint, i64) {
    if exp >= target {
        if sticky {
            // The tail lies below the last digit of `coeff`, which is at or above `target`.
            // Treat it as a digit strictly between 0 and half of the unit at `exp`.
            // (Only reachable when the caller provided extra digits; be correct anyway.)
            let c = coeff * pow10((exp - target) as u64);
            let up = round_up(rm, neg, true, Ordering::Less, false);
            let c = if up { c + 1u32 } else { c };
            return normalize(c, target);
        }
        return normalize(coeff, exp);
    }
    let k = (target - exp) as u64;
    let digits = ndigits(&coeff);
    if k as i64 > digits + 1 {
        // Everything is dropped and the value is far below half a unit.
        let up = round_up(rm, neg, !coeff.is_zero() || sticky, Ordering::Less, false);
        return if up { (BigUint::one(), target) } else { (BigUint::zero(), 0) };
    }
    let p = pow10(k);
    let (q, r) = coeff.div_rem(&p);
    let half = &p / 2u32;
    let mut cmp = r.cmp(&half);
    if cmp == Ordering::Equal && sticky {
        cmp = Ordering::Greater;
    }
    let nonzero = !r.is_zero() || sticky;
    let odd = q.is_odd();
    let q = if round_up(rm, neg, nonzero, cmp, odd) { q + 1u32 } else { q };
    normalize(q, target)
}

impl Decimal {
    // decimal.js constants.
    pub const ROUND_UP: Rounding = Rounding::Up;
    pub const ROUND_DOWN: Rounding = Rounding::Down;
    pub const ROUND_CEIL: Rounding = Rounding::Ceil;
    pub const ROUND_FLOOR: Rounding = Rounding::Floor;
    pub const ROUND_HALF_UP: Rounding = Rounding::HalfUp;
    pub const ROUND_HALF_DOWN: Rounding = Rounding::HalfDown;
    pub const ROUND_HALF_EVEN: Rounding = Rounding::HalfEven;
    pub const ROUND_HALF_CEIL: Rounding = Rounding::HalfCeil;
    pub const ROUND_HALF_FLOOR: Rounding = Rounding::HalfFloor;
    pub const EUCLID: Rounding = Rounding::Euclid;

    fn finite(neg: bool, coeff: BigUint, exp: i64, cfg: &'static DecimalConfig) -> Decimal {
        let (coeff, exp) = normalize(coeff, exp);
        Decimal { kind: Kind::Finite, neg, coeff, exp, cfg }.check_range()
    }

    /// decimal.js `finalise`: an exponent above `maxE` overflows to `Infinity` and one below
    /// `minE` underflows to zero (keeping the sign).
    fn check_range(self) -> Decimal {
        if self.kind != Kind::Finite || self.coeff.is_zero() {
            return self;
        }
        // Cheap test first: the number of digits is at most bits * log10(2) + 1.
        let digits_hi = (self.coeff.bits() as f64 * std::f64::consts::LOG10_2) as i64 + 1;
        if self.exp >= MIN_E && self.exp.saturating_add(digits_hi) <= MAX_E {
            return self;
        }
        let e = sci_exp_of(&self.coeff, self.exp);
        if e > MAX_E {
            Decimal::special(Kind::Infinite, self.neg, self.cfg)
        } else if e < MIN_E {
            Decimal { kind: Kind::Finite, neg: self.neg, coeff: BigUint::zero(), exp: 0, cfg: self.cfg }
        } else {
            self
        }
    }

    fn special(kind: Kind, neg: bool, cfg: &'static DecimalConfig) -> Decimal {
        Decimal { kind, neg, coeff: BigUint::zero(), exp: 0, cfg }
    }

    /// `new Decimal(v)` for a number or another `Decimal`. Text goes through [`Decimal::parse`] (or `TryFrom`,
    /// `FromStr`), which returns the error decimal.js throws, or the [`decimal!`](crate::decimal!) literal.
    pub fn new(v: impl Into<Decimal>) -> Decimal {
        v.into()
    }

    /// The value `(-1)^negative * limbs * 10^exp`, where `limbs` are the little-endian 32-bit digits of the
    /// coefficient. What [`decimal!`](crate::decimal!) expands to: the literal was parsed when the program was
    /// compiled. Out of decimal.js' exponent range the value is `Infinity` or zero, like [`Decimal::parse`].
    #[doc(hidden)]
    pub fn from_literal_parts(negative: bool, limbs: &[u32], exp: i64) -> Decimal {
        Decimal::finite(negative, BigUint::from_slice(limbs), exp, &DEFAULT_CONFIG)
    }

    /// The value of a `0x` / `0b` / `0o` literal of [`decimal!`](crate::decimal!): its digits as little-endian 32-bit
    /// limbs, the bits of its fraction and its binary exponent (`0x1.8p3`: `0x18`, 4, 3), as [`Decimal::parse`]
    /// computes it.
    #[doc(hidden)]
    pub fn from_radix_literal_parts(negative: bool, limbs: &[u32], frac_bits: u64, p: i64) -> Decimal {
        binary_scaled(negative, BigUint::from_slice(limbs), frac_bits, p)
    }

    /// `0`
    pub fn zero() -> Decimal {
        Decimal::finite(false, BigUint::zero(), 0, &DEFAULT_CONFIG)
    }

    /// `1`
    pub fn one() -> Decimal {
        Decimal::finite(false, BigUint::one(), 0, &DEFAULT_CONFIG)
    }

    /// `NaN`
    pub fn nan() -> Decimal {
        Decimal::special(Kind::NaN, false, &DEFAULT_CONFIG)
    }

    /// `Infinity` / `-Infinity`
    pub fn infinity(negative: bool) -> Decimal {
        Decimal::special(Kind::Infinite, negative, &DEFAULT_CONFIG)
    }

    /// `Decimal.clone({...})`: a constructor with its own configuration. The
    /// configuration is leaked (constructors are meant to be created once).
    pub fn clone_with(cfg: DecimalConfig) -> DecimalCtor {
        DecimalCtor { cfg: Box::leak(Box::new(cfg)) }
    }

    /// The configuration of this value (`x.constructor`).
    pub fn config(&self) -> &'static DecimalConfig {
        self.cfg
    }

    /// The same value with another configuration (`new D(x)`).
    pub fn with_config(mut self, cfg: &'static DecimalConfig) -> Decimal {
        self.cfg = cfg;
        self
    }

    /// Parses a decimal.js string: `[+-]digits[.digits][e[+-]n]`, `Infinity`, `NaN`, and `0x` / `0b` / `0o` numbers
    /// with an optional fraction and binary exponent (`0x1.8p3`); one `_` between two decimal digits is a separator
    /// (`1_000`). Checked input by input against decimal.js in `tests/fixtures/decimal_parse.json`.
    pub fn parse(s: &str) -> Result<Decimal, DecimalError> {
        let (neg, raw) = match s.as_bytes().first() {
            Some(b'-') => (true, &s[1..]),
            Some(b'+') => (false, &s[1..]),
            _ => (false, s),
        };
        let stripped;
        let body = if raw.contains('_') {
            stripped = strip_separators(raw);
            stripped.as_str()
        } else {
            if raw == "Infinity" {
                return Ok(Decimal::special(Kind::Infinite, neg, &DEFAULT_CONFIG));
            }
            if raw == "NaN" {
                return Ok(Decimal::nan());
            }
            raw
        };
        let invalid = || DecimalError { message: format!("[DecimalError] Invalid argument: {body}") };
        if let Some(d) = parse_radix(neg, &body.to_ascii_lowercase()) {
            return Ok(d);
        }
        // /^(\d+(\.\d*)?|\.\d+)(e[+-]?\d+)?$/i
        let bytes = body.as_bytes();
        let mut i = 0;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        let int_part = &bytes[..i];
        let mut frac_part: &[u8] = &[];
        if i < bytes.len() && bytes[i] == b'.' {
            i += 1;
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            frac_part = &bytes[start..i];
        }
        if int_part.is_empty() && frac_part.is_empty() {
            return Err(invalid());
        }
        // The exponent saturates: anything beyond `maxE` / `minE` is out of range anyway.
        let mut exp: i64 = 0;
        if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
            i += 1;
            let mut eneg = false;
            if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
                eneg = bytes[i] == b'-';
                i += 1;
            }
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                exp = exp.saturating_mul(10).saturating_add((bytes[i] - b'0') as i64);
                i += 1;
            }
            if start == i {
                return Err(invalid());
            }
            if eneg {
                exp = -exp;
            }
        }
        if i != bytes.len() {
            return Err(invalid());
        }
        // Leading and trailing zeros are dropped on the text (linear) instead of dividing the
        // coefficient by ten once per zero.
        let mut digits: Vec<u8> = Vec::with_capacity(int_part.len() + frac_part.len());
        digits.extend_from_slice(int_part);
        digits.extend_from_slice(frac_part);
        let lead = digits.iter().take_while(|d| **d == b'0').count();
        if lead == digits.len() {
            return Ok(Decimal { kind: Kind::Finite, neg, coeff: BigUint::zero(), exp: 0, cfg: &DEFAULT_CONFIG });
        }
        let trail = digits.iter().rev().take_while(|d| **d == b'0').count();
        let significant = &digits[lead..digits.len() - trail];
        // Exponent of the last kept digit, and of the first one (decimal.js `x.e`).
        let exp = exp.saturating_sub(frac_part.len() as i64).saturating_add(trail as i64);
        let sci = exp.saturating_add(significant.len() as i64 - 1);
        if sci > MAX_E {
            return Ok(Decimal::special(Kind::Infinite, neg, &DEFAULT_CONFIG));
        }
        if sci < MIN_E {
            return Ok(Decimal { kind: Kind::Finite, neg, coeff: BigUint::zero(), exp: 0, cfg: &DEFAULT_CONFIG });
        }
        let coeff = BigUint::parse_bytes(significant, 10).ok_or_else(invalid)?;
        Ok(Decimal { kind: Kind::Finite, neg, coeff, exp, cfg: &DEFAULT_CONFIG })
    }

    /// From an `f64`, like `new Decimal(number)` (uses the shortest round-trip digits).
    pub fn from_f64(v: f64) -> Decimal {
        if v.is_nan() {
            return Decimal::nan();
        }
        if v.is_infinite() {
            return Decimal::infinity(v < 0.0);
        }
        if v == 0.0 {
            return Decimal::finite(v.is_sign_negative(), BigUint::zero(), 0, &DEFAULT_CONFIG);
        }
        // `{:e}` gives the shortest round-trip digits in scientific form. When two
        // shortest candidates are equally close to the exact value, JavaScript picks the
        // even one (Number::toString) while Rust picks the upper one: fix that up.
        let d = Decimal::parse(&format!("{v:e}")).expect("f64 formats as a valid decimal");
        if d.coeff.is_odd() {
            let exact = Decimal::exact_f64(v.abs());
            let lower = Decimal::finite(false, &d.coeff - 1u32, d.exp, &DEFAULT_CONFIG);
            let upper = Decimal::finite(false, d.coeff.clone(), d.exp, &DEFAULT_CONFIG);
            let du = upper.minus_exact(&exact);
            let dl = exact.minus_exact(&lower);
            if du.cmp_abs(&dl) == Ordering::Equal && lower.to_number() == v.abs() {
                return Decimal::finite(v < 0.0, lower.coeff, lower.exp, &DEFAULT_CONFIG);
            }
        }
        d
    }

    /// The exact decimal value of a finite, non-negative `f64`.
    fn exact_f64(v: f64) -> Decimal {
        let bits = v.to_bits();
        let e = ((bits >> 52) & 0x7ff) as i64;
        let frac = bits & ((1u64 << 52) - 1);
        let (m, e2) = if e == 0 { (frac, -1074) } else { (frac | (1u64 << 52), e - 1075) };
        let m = BigUint::from(m);
        if e2 >= 0 {
            Decimal::finite(false, m << (e2 as usize), 0, &DEFAULT_CONFIG)
        } else {
            let k = (-e2) as u32;
            Decimal::finite(false, m * BigUint::from(5u32).pow(k), e2, &DEFAULT_CONFIG)
        }
    }

    /// Exact `self - y` for finite values (no rounding).
    fn minus_exact(&self, y: &Decimal) -> Decimal {
        let (neg, c, e, _) = self.add_exact(&Decimal::neg(y), u32::MAX / 4);
        Decimal::finite(neg, c, e, self.cfg)
    }

    fn from_i128(v: i128) -> Decimal {
        Decimal::finite(v < 0, BigUint::from(v.unsigned_abs()), 0, &DEFAULT_CONFIG)
    }

    // ---------------------------------------------------------------------
    // Inspection
    // ---------------------------------------------------------------------

    /// Exponent of the most significant digit (decimal.js `x.e`).
    fn sci_exp(&self) -> i64 {
        if self.coeff.is_zero() { 0 } else { ndigits(&self.coeff) - 1 + self.exp }
    }

    pub fn is_nan(&self) -> bool {
        self.kind == Kind::NaN
    }
    pub fn is_finite(&self) -> bool {
        self.kind == Kind::Finite
    }
    pub fn is_zero(&self) -> bool {
        self.kind == Kind::Finite && self.coeff.is_zero()
    }
    /// `isNeg()`: true for negative values, including `-0` and `-Infinity`.
    pub fn is_neg(&self) -> bool {
        self.kind != Kind::NaN && self.neg
    }
    pub fn is_negative(&self) -> bool {
        self.is_neg()
    }
    /// `isPos()`: true for positive values, including `0` and `Infinity`.
    pub fn is_pos(&self) -> bool {
        self.kind != Kind::NaN && !self.neg
    }
    pub fn is_positive(&self) -> bool {
        self.is_pos()
    }
    /// `isInteger()` / `isInt()`.
    pub fn is_integer(&self) -> bool {
        self.kind == Kind::Finite && (self.coeff.is_zero() || self.exp >= 0)
    }
    pub fn is_int(&self) -> bool {
        self.is_integer()
    }
    /// `decimalPlaces()` / `dp()`: number of decimal places (`NaN` → `None`).
    pub fn decimal_places(&self) -> Option<u64> {
        match self.kind {
            Kind::NaN => None,
            Kind::Infinite => None,
            Kind::Finite => Some(if self.exp < 0 { (-self.exp) as u64 } else { 0 }),
        }
    }
    /// `precision()` / `sd()`: number of significant digits.
    pub fn precision(&self, include_zeros: bool) -> Option<u64> {
        if self.kind != Kind::Finite {
            return None;
        }
        if self.coeff.is_zero() {
            return Some(1);
        }
        let d = ndigits(&self.coeff) as u64;
        if include_zeros && self.exp > 0 { Some(d + self.exp as u64) } else { Some(d) }
    }
    /// `Decimal.sign(x)`: 1, -1, 0 (also for -0) or `None` for NaN.
    pub fn signum(&self) -> Option<i32> {
        match self.kind {
            Kind::NaN => None,
            _ if self.is_zero() => Some(0),
            _ => Some(if self.neg { -1 } else { 1 }),
        }
    }

    // ---------------------------------------------------------------------
    // Comparison
    // ---------------------------------------------------------------------

    fn cmp_abs(&self, y: &Decimal) -> Ordering {
        match (self.kind, y.kind) {
            (Kind::Infinite, Kind::Infinite) => return Ordering::Equal,
            (Kind::Infinite, _) => return Ordering::Greater,
            (_, Kind::Infinite) => return Ordering::Less,
            _ => {}
        }
        match (self.coeff.is_zero(), y.coeff.is_zero()) {
            (true, true) => return Ordering::Equal,
            (true, false) => return Ordering::Less,
            (false, true) => return Ordering::Greater,
            _ => {}
        }
        let (ex, ey) = (self.sci_exp(), y.sci_exp());
        if ex != ey {
            return ex.cmp(&ey);
        }
        // Same magnitude: align on the smaller exponent (difference bounded by digit counts).
        let e = self.exp.min(y.exp);
        let a = &self.coeff * pow10((self.exp - e) as u64);
        let b = &y.coeff * pow10((y.exp - e) as u64);
        a.cmp(&b)
    }

    /// `comparedTo` / `cmp`: `None` when either is NaN.
    pub fn compared_to(&self, y: impl Into<Decimal>) -> Option<Ordering> {
        let y = y.into();
        self.partial_cmp_dec(&y)
    }

    fn partial_cmp_dec(&self, y: &Decimal) -> Option<Ordering> {
        if self.is_nan() || y.is_nan() {
            return None;
        }
        let xz = self.is_zero();
        let yz = y.is_zero();
        if xz && yz {
            return Some(Ordering::Equal);
        }
        let xs = if xz {
            0
        } else if self.neg {
            -1
        } else {
            1
        };
        let ys = if yz {
            0
        } else if y.neg {
            -1
        } else {
            1
        };
        if xs != ys {
            return Some(xs.cmp(&ys));
        }
        let c = self.cmp_abs(y);
        Some(if xs < 0 { c.reverse() } else { c })
    }

    /// `cmp()` as a number: -1, 0, 1 (`NaN` → `None`).
    #[allow(clippy::should_implement_trait)] // decimal.js `cmp`: returns -1/0/1/NaN, not an `Ordering`
    pub fn cmp(&self, y: impl Into<Decimal>) -> Option<i32> {
        self.compared_to(y).map(|o| o as i32)
    }
    /// `eq` / `equals`
    #[allow(clippy::should_implement_trait)] // decimal.js `eq`: accepts any `Into<Decimal>`, not `&Self`
    pub fn eq(&self, y: impl Into<Decimal>) -> bool {
        self.compared_to(y) == Some(Ordering::Equal)
    }
    pub fn equals(&self, y: impl Into<Decimal>) -> bool {
        self.eq(y)
    }
    /// `gt` / `greaterThan`
    pub fn gt(&self, y: impl Into<Decimal>) -> bool {
        self.compared_to(y) == Some(Ordering::Greater)
    }
    pub fn greater_than(&self, y: impl Into<Decimal>) -> bool {
        self.gt(y)
    }
    /// `gte` / `greaterThanOrEqualTo`
    pub fn gte(&self, y: impl Into<Decimal>) -> bool {
        matches!(self.compared_to(y), Some(Ordering::Greater | Ordering::Equal))
    }
    pub fn greater_than_or_equal_to(&self, y: impl Into<Decimal>) -> bool {
        self.gte(y)
    }
    /// `lt` / `lessThan`
    pub fn lt(&self, y: impl Into<Decimal>) -> bool {
        self.compared_to(y) == Some(Ordering::Less)
    }
    pub fn less_than(&self, y: impl Into<Decimal>) -> bool {
        self.lt(y)
    }
    /// `lte` / `lessThanOrEqualTo`
    pub fn lte(&self, y: impl Into<Decimal>) -> bool {
        matches!(self.compared_to(y), Some(Ordering::Less | Ordering::Equal))
    }
    pub fn less_than_or_equal_to(&self, y: impl Into<Decimal>) -> bool {
        self.lte(y)
    }

    /// `Decimal.max(...)` (NaN if any argument is NaN).
    pub fn max<I: IntoIterator<Item = T>, T: Into<Decimal>>(items: I) -> Decimal {
        Self::extreme(items, Ordering::Greater)
    }
    /// `Decimal.min(...)` (NaN if any argument is NaN).
    pub fn min<I: IntoIterator<Item = T>, T: Into<Decimal>>(items: I) -> Decimal {
        Self::extreme(items, Ordering::Less)
    }
    fn extreme<I: IntoIterator<Item = T>, T: Into<Decimal>>(items: I, want: Ordering) -> Decimal {
        let mut best: Option<Decimal> = None;
        for it in items {
            let d: Decimal = it.into();
            if d.is_nan() {
                return Decimal::nan();
            }
            best = Some(match best {
                None => d,
                Some(b) => {
                    if d.partial_cmp_dec(&b) == Some(want) {
                        d
                    } else {
                        b
                    }
                }
            });
        }
        best.unwrap_or_else(Decimal::nan)
    }
    /// `Decimal.sum(...)` with the default precision.
    pub fn sum<I: IntoIterator<Item = T>, T: Into<Decimal>>(items: I) -> Decimal {
        DecimalCtor::default_ctor().sum(items)
    }

    // ---------------------------------------------------------------------
    // Rounding
    // ---------------------------------------------------------------------

    /// Rounds to `sd` significant digits (`finalise(x, sd, rm)`).
    fn round_sd(mut self, sd: u32, rm: Rounding) -> Decimal {
        if self.kind != Kind::Finite || self.coeff.is_zero() {
            return self;
        }
        let target = self.sci_exp() - sd as i64 + 1;
        let (c, e) = round_at(self.neg, std::mem::take(&mut self.coeff), self.exp, false, target, rm);
        self.coeff = c;
        self.exp = e;
        self
    }

    /// Rounds so that no digit remains below `10^target`.
    fn round_pos(mut self, target: i64, rm: Rounding) -> Decimal {
        if self.kind != Kind::Finite {
            return self;
        }
        let (c, e) = round_at(self.neg, std::mem::take(&mut self.coeff), self.exp, false, target, rm);
        self.coeff = c;
        self.exp = e;
        self
    }

    /// `toDecimalPlaces(dp)` / `toDP(dp)` with the configured rounding mode.
    pub fn to_decimal_places(&self, dp: u32) -> Decimal {
        self.clone().round_pos(-(dp as i64), self.cfg.rounding)
    }
    /// `toDecimalPlaces(dp, rm)` / `toDP(dp, rm)`.
    pub fn to_decimal_places_with(&self, dp: u32, rm: Rounding) -> Decimal {
        self.clone().round_pos(-(dp as i64), rm)
    }
    /// Alias of [`Decimal::to_decimal_places`].
    pub fn to_dp(&self, dp: u32) -> Decimal {
        self.to_decimal_places(dp)
    }
    /// Alias of [`Decimal::to_decimal_places_with`].
    pub fn to_dp_with(&self, dp: u32, rm: Rounding) -> Decimal {
        self.to_decimal_places_with(dp, rm)
    }
    /// `toSignificantDigits(sd)` / `toSD(sd)`.
    pub fn to_significant_digits(&self, sd: u32) -> Decimal {
        self.clone().round_sd(sd.max(1), self.cfg.rounding)
    }
    /// `toSignificantDigits(sd, rm)`.
    pub fn to_significant_digits_with(&self, sd: u32, rm: Rounding) -> Decimal {
        self.clone().round_sd(sd.max(1), rm)
    }
    pub fn to_sd(&self, sd: u32) -> Decimal {
        self.to_significant_digits(sd)
    }
    pub fn to_sd_with(&self, sd: u32, rm: Rounding) -> Decimal {
        self.to_significant_digits_with(sd, rm)
    }
    /// `round()`: to an integer with the configured rounding mode.
    pub fn round(&self) -> Decimal {
        self.clone().round_pos(0, self.cfg.rounding)
    }
    /// `floor()`
    pub fn floor(&self) -> Decimal {
        self.clone().round_pos(0, Rounding::Floor)
    }
    /// `ceil()`
    pub fn ceil(&self) -> Decimal {
        self.clone().round_pos(0, Rounding::Ceil)
    }
    /// `trunc()` / `truncated()`
    pub fn trunc(&self) -> Decimal {
        self.clone().round_pos(0, Rounding::Down)
    }
    pub fn truncated(&self) -> Decimal {
        self.trunc()
    }
    /// `toNearest(y, rm)`: the nearest multiple of `y`.
    pub fn to_nearest(&self, y: impl Into<Decimal>, rm: Option<Rounding>) -> Decimal {
        let y = y.into();
        let rm = rm.unwrap_or(self.cfg.rounding);
        if self.kind != Kind::Finite {
            return self.clone();
        }
        if y.is_nan() {
            return Decimal::nan().with_config(self.cfg);
        }
        if y.is_zero() || y.kind == Kind::Infinite {
            let mut z = self.clone();
            if y.is_zero() {
                z = Decimal::finite(self.neg, BigUint::zero(), 0, self.cfg);
            }
            return z;
        }
        // q = round(x / y) (to integer, mode rm), result = q * |y|
        let (q, sticky_neg) = self.div_exact_integer(&y, rm);
        let mut r = q.times_exact(&y.abs());
        r.neg = sticky_neg;
        r.cfg = self.cfg;
        r
    }

    /// round(x / y) to an integer with mode `rm` (exactly). Returns (|q| with sign, sign of x).
    fn div_exact_integer(&self, y: &Decimal, rm: Rounding) -> (Decimal, bool) {
        // x / y = (cx * 10^ex) / (cy * 10^ey)
        let neg = self.neg != y.neg;
        let (num, den) = scaled_ratio(&self.coeff, self.exp, &y.coeff, y.exp);
        let (q, r) = num.div_rem(&den);
        let twice = &r * 2u32;
        let cmp = twice.cmp(&den);
        let up = round_up(rm, neg, !r.is_zero(), cmp, q.is_odd());
        let q = if up { q + 1u32 } else { q };
        (Decimal::finite(neg, q, 0, self.cfg), self.neg)
    }

    // ---------------------------------------------------------------------
    // Arithmetic
    // ---------------------------------------------------------------------

    fn arg(&self, y: impl Into<Decimal>) -> Decimal {
        let mut y = y.into();
        y.cfg = self.cfg;
        y
    }

    fn nan_cfg(&self) -> Decimal {
        Decimal::special(Kind::NaN, false, self.cfg)
    }

    /// `abs()` / `absoluteValue()` (not rounded).
    pub fn abs(&self) -> Decimal {
        let mut x = self.clone();
        if x.kind != Kind::NaN {
            x.neg = false;
        }
        x
    }
    pub fn absolute_value(&self) -> Decimal {
        self.abs()
    }
    /// `neg()` / `negated()` (not rounded).
    pub fn neg(&self) -> Decimal {
        let mut x = self.clone();
        if x.kind != Kind::NaN {
            x.neg = !x.neg;
        }
        x
    }
    pub fn negated(&self) -> Decimal {
        Decimal::neg(self)
    }

    /// Exact sum (no rounding), with the same far-operand treatment as decimal.js.
    fn add_exact(&self, y: &Decimal, pr: u32) -> (bool, BigUint, i64, bool) {
        // Returns (neg, coeff, exp, sticky)
        if self.coeff.is_zero() {
            return (y.neg && (self.neg || !y.coeff.is_zero()), y.coeff.clone(), y.exp, false);
        }
        if y.coeff.is_zero() {
            return (self.neg, self.coeff.clone(), self.exp, false);
        }
        // If one operand is entirely far below the precision of the other, replace it by a
        // tiny sticky contribution to avoid huge shifts.
        let (big, small) = if self.sci_exp() >= y.sci_exp() { (self, y) } else { (y, self) };
        let limit = big.sci_exp() - pr as i64 - 3;
        let (sc, se) = if small.sci_exp() < limit { (BigUint::one(), limit - 1) } else { (small.coeff.clone(), small.exp) };
        let e = big.exp.min(se);
        let a = &big.coeff * pow10((big.exp - e) as u64);
        let b = sc * pow10((se - e) as u64);
        if big.neg == small.neg {
            (big.neg, a + b, e, false)
        } else {
            match a.cmp(&b) {
                Ordering::Greater => (big.neg, a - b, e, false),
                Ordering::Less => (small.neg, b - a, e, false),
                Ordering::Equal => (false, BigUint::zero(), 0, false),
            }
        }
    }

    /// `plus` / `add` (rounded to `precision`).
    pub fn plus(&self, y: impl Into<Decimal>) -> Decimal {
        let y = self.arg(y);
        let cfg = self.cfg;
        match (self.kind, y.kind) {
            (Kind::NaN, _) | (_, Kind::NaN) => return self.nan_cfg(),
            (Kind::Infinite, Kind::Infinite) => {
                return if self.neg == y.neg { self.clone() } else { self.nan_cfg() };
            }
            (Kind::Infinite, _) => return self.clone(),
            (_, Kind::Infinite) => return y,
            _ => {}
        }
        let (mut neg, c, e, _) = self.add_exact(&y, cfg.precision);
        if c.is_zero() {
            // x + (-x) = 0 (or -0 when rounding to -Infinity); -0 + -0 = -0.
            neg = if self.neg && y.neg { true } else { cfg.rounding == Rounding::Floor && self.neg != y.neg };
            return Decimal::finite(neg, c, 0, cfg);
        }
        Decimal::finite(neg, c, e, cfg).round_sd(cfg.precision, cfg.rounding)
    }
    pub fn add(&self, y: impl Into<Decimal>) -> Decimal {
        self.plus(y)
    }

    /// `minus` / `sub` (rounded to `precision`).
    pub fn minus(&self, y: impl Into<Decimal>) -> Decimal {
        let y = self.arg(y);
        self.plus(Decimal::neg(&y))
    }
    pub fn sub(&self, y: impl Into<Decimal>) -> Decimal {
        self.minus(y)
    }

    fn times_exact(&self, y: &Decimal) -> Decimal {
        Decimal::finite(self.neg != y.neg, &self.coeff * &y.coeff, self.exp + y.exp, self.cfg)
    }

    /// `times` / `mul` (rounded to `precision`).
    pub fn times(&self, y: impl Into<Decimal>) -> Decimal {
        let y = self.arg(y);
        let cfg = self.cfg;
        let neg = self.neg != y.neg;
        match (self.kind, y.kind) {
            (Kind::NaN, _) | (_, Kind::NaN) => return self.nan_cfg(),
            (Kind::Infinite, _) | (_, Kind::Infinite) => {
                if self.is_zero() || y.is_zero() {
                    return self.nan_cfg();
                }
                return Decimal::special(Kind::Infinite, neg, cfg);
            }
            _ => {}
        }
        self.times_exact(&y).round_sd(cfg.precision, cfg.rounding)
    }
    pub fn mul(&self, y: impl Into<Decimal>) -> Decimal {
        self.times(y)
    }

    /// Correctly rounded quotient to `sd` significant digits (or `dp` decimal places
    /// when `dp_mode`).
    fn div_round(&self, y: &Decimal, digits: i64, dp_mode: bool, rm: Rounding) -> Decimal {
        let cfg = self.cfg;
        let neg = self.neg != y.neg;
        if self.coeff.is_zero() {
            return Decimal::finite(neg, BigUint::zero(), 0, cfg);
        }
        // q = cx * 10^s / cy, value = q * 10^(ex - ey - s)
        let target_digits = if dp_mode {
            // enough digits to reach 10^-digits
            let e_est = self.sci_exp() - y.sci_exp() + 1;
            (e_est + digits + 2).max(1)
        } else {
            digits + 2
        };
        let s = (target_digits + ndigits(&y.coeff) - ndigits(&self.coeff)).max(0);
        let num = &self.coeff * pow10(s as u64);
        let (q, r) = num.div_rem(&y.coeff);
        let exp = self.exp - y.exp - s;
        let sticky = !r.is_zero();
        if q.is_zero() {
            // Only possible in dp mode with a tiny quotient.
            let target = -digits;
            let up = round_up(rm, neg, sticky, Ordering::Less, false);
            return if up { Decimal::finite(neg, BigUint::one(), target, cfg) } else { Decimal::finite(neg, BigUint::zero(), 0, cfg) };
        }
        let target = if dp_mode { -digits } else { ndigits(&q) - 1 + exp - digits + 1 };
        let (c, e) = round_at(neg, q, exp, sticky, target, rm);
        Decimal::finite(neg, c, e, cfg)
    }

    /// `div` / `dividedBy` (rounded to `precision`).
    pub fn div(&self, y: impl Into<Decimal>) -> Decimal {
        let y = self.arg(y);
        let cfg = self.cfg;
        let neg = self.neg != y.neg;
        match (self.kind, y.kind) {
            (Kind::NaN, _) | (_, Kind::NaN) => return self.nan_cfg(),
            (Kind::Infinite, Kind::Infinite) => return self.nan_cfg(),
            (Kind::Infinite, _) => return Decimal::special(Kind::Infinite, neg, cfg),
            (_, Kind::Infinite) => return Decimal::finite(neg, BigUint::zero(), 0, cfg),
            _ => {}
        }
        if y.is_zero() {
            if self.is_zero() {
                return self.nan_cfg();
            }
            return Decimal::special(Kind::Infinite, neg, cfg);
        }
        self.div_round(&y, cfg.precision as i64, false, cfg.rounding)
    }
    pub fn divided_by(&self, y: impl Into<Decimal>) -> Decimal {
        self.div(y)
    }

    /// `dividedToIntegerBy` / `divToInt`: the integer part of `x / y`, rounded to `precision`.
    pub fn divided_to_integer_by(&self, y: impl Into<Decimal>) -> Decimal {
        let y = self.arg(y);
        if !self.is_finite() || !y.is_finite() || y.is_zero() {
            return self.div(y).trunc();
        }
        let q = self.div_round(&y, 0, true, Rounding::Down);
        q.round_sd(self.cfg.precision, self.cfg.rounding)
    }
    pub fn div_to_int(&self, y: impl Into<Decimal>) -> Decimal {
        self.divided_to_integer_by(y)
    }

    /// `mod` / `modulo` (with the configured `modulo` mode, default `ROUND_DOWN`
    /// = JavaScript `%`), rounded to `precision`.
    pub fn modulo(&self, y: impl Into<Decimal>) -> Decimal {
        let y = self.arg(y);
        let cfg = self.cfg;
        if self.is_nan() || y.is_nan() || self.kind == Kind::Infinite || y.is_zero() {
            return self.nan_cfg();
        }
        if y.kind == Kind::Infinite || self.is_zero() {
            return self.clone().round_sd(cfg.precision, cfg.rounding);
        }
        // q = integer quotient per the modulo mode; r = x - q * y (exact).
        let q = match cfg.modulo {
            Rounding::Euclid => {
                let t = self.div_round(&y.abs(), 0, true, Rounding::Floor);
                let mut q = t;
                q.neg = q.neg != y.neg;
                if q.coeff.is_zero() {
                    q.neg = false;
                }
                q
            }
            rm => self.div_round(&y, 0, true, rm),
        };
        let prod = q.times_exact(&y);
        let (neg, c, e, _) = self.add_exact(&Decimal::neg(&prod), u32::MAX / 4);
        if c.is_zero() {
            return Decimal::finite(self.neg, c, 0, cfg);
        }
        Decimal::finite(neg, c, e, cfg).round_sd(cfg.precision, cfg.rounding)
    }
    /// Alias of [`Decimal::modulo`] (`mod` is a Rust keyword).
    pub fn r#mod(&self, y: impl Into<Decimal>) -> Decimal {
        self.modulo(y)
    }

    /// `pow` / `toPower`. Integer exponents are computed exactly and rounded to
    /// `precision`; other exponents go through `f64` (not bit-for-bit decimal.js).
    pub fn pow(&self, y: impl Into<Decimal>) -> Decimal {
        let y = self.arg(y);
        let cfg = self.cfg;
        if self.is_nan() || y.is_nan() {
            return self.nan_cfg();
        }
        if y.is_zero() {
            return Decimal::finite(false, BigUint::one(), 0, cfg);
        }
        if y.is_integer()
            && self.is_finite()
            && let Some(n) = y.to_i64_exact()
            && n.unsigned_abs() <= 100_000
        {
            let k = n.unsigned_abs();
            if self.is_zero() {
                return if n < 0 {
                    Decimal::special(Kind::Infinite, self.neg && k % 2 == 1, cfg)
                } else {
                    Decimal::finite(self.neg && k % 2 == 1, BigUint::zero(), 0, cfg)
                };
            }
            let neg = self.neg && k % 2 == 1;
            // The exponent of the result is outside the range when it does not even fit an i64.
            let Some(exp) = self.exp.checked_mul(k as i64) else {
                let big = (self.exp > 0) == (n > 0);
                return if big { Decimal::special(Kind::Infinite, neg, cfg) } else { Decimal::finite(neg, BigUint::zero(), 0, cfg) };
            };
            let c = self.coeff.pow(k as u32);
            let r = Decimal::finite(neg, c, exp, cfg);
            if n < 0 {
                return Decimal::finite(false, BigUint::one(), 0, cfg).div(r);
            }
            return r.round_sd(cfg.precision, cfg.rounding);
        }
        let v = self.to_number().powf(y.to_number());
        Decimal::from_f64(v).with_config(cfg).round_sd(cfg.precision, cfg.rounding)
    }
    pub fn to_power(&self, y: impl Into<Decimal>) -> Decimal {
        self.pow(y)
    }

    /// `sqrt` / `squareRoot` (correctly rounded to `precision`).
    pub fn sqrt(&self) -> Decimal {
        let cfg = self.cfg;
        if self.is_nan() || (self.neg && !self.is_zero()) {
            return self.nan_cfg();
        }
        if self.kind == Kind::Infinite || self.is_zero() {
            return self.clone();
        }
        let pr = cfg.precision as i64;
        // Make the exponent even and scale so the root has >= pr + 2 digits.
        let mut c = self.coeff.clone();
        let mut e = self.exp;
        let want = 2 * (pr + 2) - ndigits(&c);
        let mut shift = want.max(0);
        if (e - shift) % 2 != 0 {
            shift += 1;
        }
        c *= pow10(shift as u64);
        e -= shift;
        let root = c.sqrt();
        let sticky = &root * &root != c;
        let exp = e / 2;
        let target = ndigits(&root) - 1 + exp - pr + 1;
        let (rc, re) = round_at(false, root, exp, sticky, target, cfg.rounding);
        Decimal::finite(false, rc, re, cfg)
    }
    pub fn square_root(&self) -> Decimal {
        self.sqrt()
    }

    /// `clampedTo(min, max)` / `clamp`.
    pub fn clamped_to(&self, min: impl Into<Decimal>, max: impl Into<Decimal>) -> Decimal {
        let (min, max) = (self.arg(min), self.arg(max));
        if self.lt(&min) {
            min
        } else if self.gt(&max) {
            max
        } else {
            self.clone()
        }
    }

    // ---------------------------------------------------------------------
    // Conversion
    // ---------------------------------------------------------------------

    /// `toNumber()`: the nearest `f64`.
    pub fn to_number(&self) -> f64 {
        match self.kind {
            Kind::NaN => f64::NAN,
            Kind::Infinite => {
                if self.neg {
                    f64::NEG_INFINITY
                } else {
                    f64::INFINITY
                }
            }
            Kind::Finite => {
                let s = self.to_exponential_raw();
                let v: f64 = s.parse().unwrap_or(f64::NAN);
                if self.neg && v == 0.0 { -0.0 } else { v }
            }
        }
    }

    fn to_exponential_raw(&self) -> String {
        let digits = self.coeff.to_string();
        let sign = if self.neg { "-" } else { "" };
        format!("{sign}{digits}e{}", self.exp)
    }

    /// The value as `i64` when it is an integer that fits.
    pub fn to_i64_exact(&self) -> Option<i64> {
        if !self.is_integer() || self.sci_exp() > 19 {
            return None;
        }
        let m = (&self.coeff * pow10(self.exp.max(0) as u64)).to_i128()?;
        let v = if self.neg { -m } else { m };
        i64::try_from(v).ok()
    }

    fn digits_string(&self) -> String {
        self.coeff.to_string()
    }

    /// decimal.js `finiteToString(x, isExp, sd)` (no sign).
    fn finite_to_string(&self, is_exp: bool, sd: Option<i64>) -> String {
        let e = self.sci_exp();
        let mut s = self.digits_string();
        let len = s.len() as i64;
        let zeros = |n: i64| {
            // Like JavaScript (`RangeError: Invalid string length`): never try to allocate a
            // string longer than a JS engine can hold.
            if n > MAX_STRING_LEN {
                panic!("RangeError: Invalid string length");
            }
            "0".repeat(n.max(0) as usize)
        };
        if is_exp {
            if let Some(sd) = sd
                && sd - len > 0
            {
                s = format!("{}.{}{}", &s[..1], &s[1..], zeros(sd - len));
            } else if len > 1 {
                s = format!("{}.{}", &s[..1], &s[1..]);
            }
            s = format!("{s}{}{e}", if e < 0 { "e" } else { "e+" });
        } else if e < 0 {
            s = format!("0.{}{s}", zeros(-e - 1));
            if let Some(sd) = sd
                && sd - len > 0
            {
                s.push_str(&zeros(sd - len));
            }
        } else if e >= len {
            s.push_str(&zeros(e + 1 - len));
            if let Some(sd) = sd
                && sd - e - 1 > 0
            {
                s = format!("{s}.{}", zeros(sd - e - 1));
            }
        } else {
            let k = e + 1;
            if k < len {
                s = format!("{}.{}", &s[..k as usize], &s[k as usize..]);
            }
            if let Some(sd) = sd
                && sd - len > 0
            {
                if e + 1 == len {
                    s.push('.');
                }
                s.push_str(&zeros(sd - len));
            }
        }
        s
    }

    fn non_finite_string(&self) -> &'static str {
        match self.kind {
            Kind::NaN => "NaN",
            _ => {
                if self.neg {
                    "-Infinity"
                } else {
                    "Infinity"
                }
            }
        }
    }

    /// `toString()` (exponential notation beyond `to_exp_neg` / `to_exp_pos`).
    pub fn to_js_string(&self) -> String {
        if self.kind != Kind::Finite {
            return self.non_finite_string().to_string();
        }
        let e = self.sci_exp();
        let s = self.finite_to_string(e <= self.cfg.to_exp_neg || e >= self.cfg.to_exp_pos, None);
        if self.neg && !self.is_zero() { format!("-{s}") } else { s }
    }

    /// `valueOf()` / `toJSON()`: like `toString()` but `-0` keeps its sign. This is
    /// what `JSON.stringify` writes for a Decimal (and what serde serializes).
    pub fn value_of(&self) -> String {
        if self.kind != Kind::Finite {
            return self.non_finite_string().to_string();
        }
        let e = self.sci_exp();
        let s = self.finite_to_string(e <= self.cfg.to_exp_neg || e >= self.cfg.to_exp_pos, None);
        if self.neg { format!("-{s}") } else { s }
    }
    pub fn to_json(&self) -> String {
        self.value_of()
    }

    /// `toFixed()` without arguments: normal notation, never exponential.
    pub fn to_fixed_plain(&self) -> String {
        if self.kind != Kind::Finite {
            return self.non_finite_string().to_string();
        }
        let s = self.finite_to_string(false, None);
        if self.neg && !self.is_zero() { format!("-{s}") } else { s }
    }

    /// `toFixed(dp)` with the configured rounding mode.
    pub fn to_fixed(&self, dp: u32) -> String {
        self.to_fixed_with(dp, self.cfg.rounding)
    }

    /// `toFixed(dp, rm)`.
    pub fn to_fixed_with(&self, dp: u32, rm: Rounding) -> String {
        if self.kind != Kind::Finite {
            return self.non_finite_string().to_string();
        }
        let y = self.clone().round_pos(-(dp as i64), rm);
        let s = y.finite_to_string(false, Some(dp as i64 + y.sci_exp() + 1));
        // The sign comes from the value before rounding.
        if self.neg && !self.is_zero() { format!("-{s}") } else { s }
    }

    /// `toExponential(dp)` (`None`: as many digits as needed).
    pub fn to_exponential(&self, dp: Option<u32>, rm: Option<Rounding>) -> String {
        if self.kind != Kind::Finite {
            return self.non_finite_string().to_string();
        }
        let s = match dp {
            None => self.finite_to_string(true, None),
            Some(dp) => {
                let y = self.clone().round_sd(dp + 1, rm.unwrap_or(self.cfg.rounding));
                y.finite_to_string(true, Some(dp as i64 + 1))
            }
        };
        if self.neg && !self.is_zero() { format!("-{s}") } else { s }
    }

    /// `toPrecision(sd)` (`None`: like `toString()`).
    pub fn to_precision(&self, sd: Option<u32>, rm: Option<Rounding>) -> String {
        if self.kind != Kind::Finite {
            return self.non_finite_string().to_string();
        }
        let s = match sd {
            None => {
                let e = self.sci_exp();
                self.finite_to_string(e <= self.cfg.to_exp_neg || e >= self.cfg.to_exp_pos, None)
            }
            Some(sd) => {
                let y = self.clone().round_sd(sd.max(1), rm.unwrap_or(self.cfg.rounding));
                let e = y.sci_exp();
                y.finite_to_string(sd as i64 <= e || e <= self.cfg.to_exp_neg, Some(sd as i64))
            }
        };
        if self.neg && !self.is_zero() { format!("-{s}") } else { s }
    }
}

/// `(num, den)` such that `(cx*10^ex)/(cy*10^ey) = num/den` with integers.
fn scaled_ratio(cx: &BigUint, ex: i64, cy: &BigUint, ey: i64) -> (BigUint, BigUint) {
    let d = ex - ey;
    if d >= 0 { (cx * pow10(d as u64), cy.clone()) } else { (cx.clone(), cy * pow10((-d) as u64)) }
}

impl Default for Decimal {
    fn default() -> Self {
        Decimal::zero()
    }
}

impl fmt::Display for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_js_string())
    }
}

impl fmt::Debug for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Decimal({})", self.value_of())
    }
}

impl FromStr for Decimal {
    type Err = DecimalError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Decimal::parse(s)
    }
}

impl PartialEq for Decimal {
    fn eq(&self, other: &Self) -> bool {
        self.partial_cmp_dec(other) == Some(Ordering::Equal)
    }
}

impl PartialOrd for Decimal {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.partial_cmp_dec(other)
    }
}

macro_rules! cmp_prim {
    ($($t:ty),*) => {$(
        impl PartialEq<$t> for Decimal {
            fn eq(&self, other: &$t) -> bool {
                self.partial_cmp_dec(&Decimal::from(*other)) == Some(Ordering::Equal)
            }
        }
        impl PartialOrd<$t> for Decimal {
            fn partial_cmp(&self, other: &$t) -> Option<Ordering> {
                self.partial_cmp_dec(&Decimal::from(*other))
            }
        }
    )*};
}
cmp_prim!(i32, i64, u32, u64, f64);

impl PartialEq<&str> for Decimal {
    fn eq(&self, other: &&str) -> bool {
        Decimal::parse(other).is_ok_and(|o| self.partial_cmp_dec(&o) == Some(Ordering::Equal))
    }
}

// ---------------------------------------------------------------------------
// Conversions
// ---------------------------------------------------------------------------

macro_rules! from_int {
    ($($t:ty),*) => {$(
        impl From<$t> for Decimal {
            fn from(v: $t) -> Self {
                Decimal::from_i128(v as i128)
            }
        }
    )*};
}
from_int!(i8, i16, i32, i64, isize, u8, u16, u32, u64, usize);

impl From<i128> for Decimal {
    fn from(v: i128) -> Self {
        Decimal::from_i128(v)
    }
}
impl From<u128> for Decimal {
    fn from(v: u128) -> Self {
        Decimal::finite(false, BigUint::from(v), 0, &DEFAULT_CONFIG)
    }
}
impl From<f64> for Decimal {
    fn from(v: f64) -> Self {
        Decimal::from_f64(v)
    }
}
impl From<f32> for Decimal {
    /// Like JavaScript, an `f32` is first widened to a double (`0.1f32` is `0.10000000149011612`).
    fn from(v: f32) -> Self {
        Decimal::from_f64(v as f64)
    }
}
impl From<&Decimal> for Decimal {
    fn from(v: &Decimal) -> Self {
        v.clone()
    }
}
impl From<&mut Decimal> for Decimal {
    fn from(v: &mut Decimal) -> Self {
        v.clone()
    }
}
// Text becomes a `Decimal` only through conversions that can fail (`parse`, `FromStr`, `TryFrom`) or through the
// compile-time checked `decimal!` literal: a `From<&str>` would have to panic on invalid text, and typed helpers
// take `impl Into<Decimal>`, so text from a request could crash the handler.
impl TryFrom<&str> for Decimal {
    type Error = DecimalError;
    fn try_from(s: &str) -> Result<Self, DecimalError> {
        Decimal::parse(s)
    }
}
impl TryFrom<String> for Decimal {
    type Error = DecimalError;
    fn try_from(s: String) -> Result<Self, DecimalError> {
        Decimal::parse(&s)
    }
}

impl From<Decimal> for f64 {
    fn from(d: Decimal) -> f64 {
        d.to_number()
    }
}

impl From<Decimal> for String {
    fn from(d: Decimal) -> String {
        d.to_js_string()
    }
}

// ---------------------------------------------------------------------------
// Operators
// ---------------------------------------------------------------------------

macro_rules! bin_op {
    ($tr:ident, $f:ident, $method:ident, $atr:ident, $af:ident) => {
        impl<T: Into<Decimal>> $tr<T> for Decimal {
            type Output = Decimal;
            fn $f(self, rhs: T) -> Decimal {
                Decimal::$method(&self, rhs)
            }
        }
        impl<T: Into<Decimal>> $tr<T> for &Decimal {
            type Output = Decimal;
            fn $f(self, rhs: T) -> Decimal {
                Decimal::$method(self, rhs)
            }
        }
        impl<T: Into<Decimal>> $atr<T> for Decimal {
            fn $af(&mut self, rhs: T) {
                *self = Decimal::$method(self, rhs);
            }
        }
    };
}
bin_op!(Add, add, plus, AddAssign, add_assign);
bin_op!(Sub, sub, minus, SubAssign, sub_assign);
bin_op!(Mul, mul, times, MulAssign, mul_assign);
bin_op!(Div, div, div, DivAssign, div_assign);
bin_op!(Rem, rem, modulo, RemAssign, rem_assign);

impl Neg for Decimal {
    type Output = Decimal;
    fn neg(self) -> Decimal {
        Decimal::neg(&self)
    }
}
impl Neg for &Decimal {
    type Output = Decimal;
    fn neg(self) -> Decimal {
        Decimal::neg(self)
    }
}

impl std::iter::Sum for Decimal {
    fn sum<I: Iterator<Item = Decimal>>(iter: I) -> Decimal {
        iter.fold(Decimal::zero(), |a, b| a.plus(b))
    }
}
impl<'a> std::iter::Sum<&'a Decimal> for Decimal {
    fn sum<I: Iterator<Item = &'a Decimal>>(iter: I) -> Decimal {
        iter.fold(Decimal::zero(), |a, b| a.plus(b))
    }
}
impl std::iter::Product for Decimal {
    fn product<I: Iterator<Item = Decimal>>(iter: I) -> Decimal {
        iter.fold(Decimal::one(), |a, b| a.times(b))
    }
}

// ---------------------------------------------------------------------------
// serde: JSON.stringify(decimal) === decimal.toJSON() (a string)
// ---------------------------------------------------------------------------

impl serde::Serialize for Decimal {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.value_of())
    }
}

impl<'de> serde::Deserialize<'de> for Decimal {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl serde::de::Visitor<'_> for V {
            type Value = Decimal;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a decimal string or number")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Decimal, E> {
                Decimal::parse(v).map_err(E::custom)
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Decimal, E> {
                Ok(Decimal::from(v))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Decimal, E> {
                Ok(Decimal::from(v))
            }
            fn visit_i128<E: serde::de::Error>(self, v: i128) -> Result<Decimal, E> {
                Ok(Decimal::from(v))
            }
            fn visit_u128<E: serde::de::Error>(self, v: u128) -> Result<Decimal, E> {
                Ok(Decimal::from(v))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Decimal, E> {
                Ok(Decimal::from(v))
            }
        }
        d.deserialize_any(V)
    }
}

// ---------------------------------------------------------------------------
// Postgres NUMERIC (binary format) and SQLite
// ---------------------------------------------------------------------------

mod pg {
    use super::*;
    use sqlx::encode::IsNull;
    use sqlx::error::BoxDynError;
    use sqlx::postgres::{PgArgumentBuffer, PgHasArrayType, PgTypeInfo, PgValueFormat, PgValueRef, Postgres};

    const SIGN_POS: u16 = 0x0000;
    const SIGN_NEG: u16 = 0x4000;
    const SIGN_NAN: u16 = 0xC000;
    const SIGN_PINF: u16 = 0xD000;
    const SIGN_NINF: u16 = 0xF000;

    impl sqlx::Type<Postgres> for Decimal {
        fn type_info() -> PgTypeInfo {
            PgTypeInfo::with_name("numeric")
        }
        fn compatible(ty: &PgTypeInfo) -> bool {
            use sqlx::TypeInfo;
            matches!(ty.name(), "NUMERIC" | "numeric")
        }
    }

    impl PgHasArrayType for Decimal {
        fn array_type_info() -> PgTypeInfo {
            PgTypeInfo::with_name("_numeric")
        }
    }

    impl sqlx::Encode<'_, Postgres> for Decimal {
        fn encode_by_ref(&self, buf: &mut PgArgumentBuffer) -> Result<IsNull, BoxDynError> {
            buf.extend_from_slice(&encode_numeric(self)?);
            Ok(IsNull::No)
        }
    }

    impl<'r> sqlx::Decode<'r, Postgres> for Decimal {
        fn decode(value: PgValueRef<'r>) -> Result<Self, BoxDynError> {
            match value.format() {
                PgValueFormat::Binary => decode_numeric(value.as_bytes()?),
                PgValueFormat::Text => Ok(Decimal::parse(value.as_str()?)?),
            }
        }
    }

    /// `(ndigits, weight, dscale)` of the binary NUMERIC of a finite, non-zero value, or `None`
    /// when a field does not fit its 16-bit slot (the value cannot be sent to Postgres).
    pub(crate) fn numeric_header(d: &Decimal) -> Option<(i16, i16, u16)> {
        let dscale = if d.exp < 0 { u16::try_from(d.exp.checked_neg()?).ok()? } else { 0 };
        // Base-10000 groups: the least significant one holds the digit at 10^exp.
        let low = d.exp.div_euclid(4);
        let weight = d.sci_exp().div_euclid(4);
        let nd = weight.checked_sub(low)?.checked_add(1)?;
        Some((i16::try_from(nd).ok()?, i16::try_from(weight).ok()?, dscale))
    }

    /// The conversion error of a value that does not fit the binary NUMERIC format (the
    /// wording of the reference query engine).
    pub(crate) const NUMERIC_OVERFLOW: &str = "Couldn't convert an integer (possible overflow).";

    /// Binary NUMERIC: ndigits, weight, sign, dscale, base-10000 digits.
    pub(crate) fn encode_numeric(d: &Decimal) -> Result<Vec<u8>, BoxDynError> {
        let mut out = Vec::new();
        let header = |out: &mut Vec<u8>, nd: i16, weight: i16, sign: u16, dscale: u16| {
            out.extend_from_slice(&nd.to_be_bytes());
            out.extend_from_slice(&weight.to_be_bytes());
            out.extend_from_slice(&sign.to_be_bytes());
            out.extend_from_slice(&dscale.to_be_bytes());
        };
        match d.kind {
            Kind::NaN => {
                header(&mut out, 0, 0, SIGN_NAN, 0);
                return Ok(out);
            }
            Kind::Infinite => {
                header(&mut out, 0, 0, if d.neg { SIGN_NINF } else { SIGN_PINF }, 0);
                return Ok(out);
            }
            Kind::Finite => {}
        }
        if d.coeff.is_zero() {
            header(&mut out, 0, 0, SIGN_POS, 0);
            return Ok(out);
        }
        // Never truncate the 16-bit fields: a truncated weight or scale is another number.
        let (nd, weight, dscale) = numeric_header(d).ok_or(NUMERIC_OVERFLOW)?;
        // Digits of the coefficient followed by the zeros that align its exponent on a group
        // boundary, left-padded to whole groups.
        let rem = d.exp.rem_euclid(4) as usize;
        let mut digits = d.coeff.to_string().into_bytes();
        digits.resize(digits.len() + rem, b'0');
        let pad = nd as usize * 4 - digits.len();
        out.reserve(nd as usize * 2);
        header(&mut out, nd, weight, if d.neg { SIGN_NEG } else { SIGN_POS }, dscale);
        let mut padded = vec![b'0'; pad];
        padded.extend_from_slice(&digits);
        for g in padded.chunks(4) {
            let v = g.iter().fold(0i16, |acc, c| acc * 10 + (c - b'0') as i16);
            out.extend_from_slice(&v.to_be_bytes());
        }
        Ok(out)
    }

    pub(crate) fn decode_numeric(b: &[u8]) -> Result<Decimal, BoxDynError> {
        if b.len() < 8 {
            return Err("invalid NUMERIC value".into());
        }
        let nd = i16::from_be_bytes([b[0], b[1]]) as usize;
        let weight = i16::from_be_bytes([b[2], b[3]]) as i64;
        let sign = u16::from_be_bytes([b[4], b[5]]);
        match sign {
            SIGN_NAN => return Ok(Decimal::nan()),
            SIGN_PINF => return Ok(Decimal::infinity(false)),
            SIGN_NINF => return Ok(Decimal::infinity(true)),
            _ => {}
        }
        if b.len() < 8 + nd * 2 {
            return Err("invalid NUMERIC value".into());
        }
        let mut c = BigUint::zero();
        let base = BigUint::from(10000u32);
        for i in 0..nd {
            let g = i16::from_be_bytes([b[8 + 2 * i], b[9 + 2 * i]]) as u32;
            c = c * &base + BigUint::from(g);
        }
        // The last group has weight (weight - nd + 1).
        let exp = (weight - nd as i64 + 1) * 4;
        Ok(Decimal::finite(sign == SIGN_NEG, c, exp, &DEFAULT_CONFIG))
    }
}

#[allow(unused_imports)]
pub(crate) use pg::encode_numeric;
pub(crate) use pg::{NUMERIC_OVERFLOW, decode_numeric};

impl Decimal {
    /// `true` when the value can be stored in a Postgres `NUMERIC`: its binary representation
    /// needs a weight, a scale and a number of base-10000 digits that fit in 16 bits (at most
    /// 131072 digits before the decimal point and 65535 after it). Values beyond that are
    /// rejected with a conversion error instead of being truncated.
    pub fn fits_numeric(&self) -> bool {
        self.kind != Kind::Finite || self.coeff.is_zero() || pg::numeric_header(self).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decimal;

    #[test]
    fn basics() {
        assert_eq!(decimal!("1.50").to_string(), "1.5");
        assert_eq!(Decimal::from(-0.0).to_string(), "0");
        assert_eq!(Decimal::from(-0.0).value_of(), "-0");
        assert_eq!(decimal!("1e21").to_string(), "1e+21");
        assert_eq!(decimal!("1e-7").to_string(), "1e-7");
        assert_eq!(decimal!("0.000001").to_string(), "0.000001");
        assert_eq!(Decimal::from(1).div(3).to_string(), "0.33333333333333333333");
        assert_eq!(decimal!("123.456").modulo(7).to_string(), "4.456");
        assert_eq!(Decimal::from(-0.001).to_fixed(2), "-0.00");
        assert_eq!(serde_json::to_string(&decimal!("-0.004").to_dp(2)).unwrap(), "\"-0\"");
        assert_eq!(Decimal::from(2).sqrt().to_string(), "1.4142135623730950488");
    }

    #[test]
    fn the_literal_macro_gives_what_parse_gives() {
        macro_rules! same {
            ($($lit:literal),* $(,)?) => {$(
                let parsed = Decimal::parse($lit).unwrap();
                let literal = decimal!($lit);
                assert_eq!((literal.value_of(), literal.is_negative()), (parsed.value_of(), parsed.is_negative()), $lit);
            )*};
        }
        same!(
            "0x1.8",
            "0x1p4",
            "0x1.8p1",
            "0x1P-2",
            "0x.8",
            "0x1p-53",
            "0x3p99999",
            "-0x0",
            "0b0.0001",
            "0o1.4",
            "1_000_000.000_1",
            "0x1p1_0",
            "1e1_0",
            "0",
            "-0",
            "+0.000",
            "1.50",
            "-1.5e-3",
            "000123.4500e2",
            "1e21",
            "1E+5",
            ".5",
            "5.",
            "0x1F",
            "0B101",
            "0o17",
            "-0xff",
            "Infinity",
            "-Infinity",
            "NaN",
            "1e9000000000000000",
            "1e9000000000000001",
            "-1e-9000000000000001",
            "123456789012345678901234567890.123456789",
            "4294967296",
            "18446744073709551616",
            "1e-9223372036854775808",
        );
    }

    #[test]
    fn text_converts_only_through_fallible_conversions() {
        assert_eq!(Decimal::try_from("1.5").unwrap(), decimal!("1.5"));
        assert_eq!(Decimal::try_from(String::from("-2")).unwrap(), decimal!("-2"));
        for bad in ["12,5", "", "1.2.3", "abc", "1e"] {
            assert!(Decimal::try_from(bad).is_err(), "{bad}");
            assert!(bad.parse::<Decimal>().is_err(), "{bad}");
        }
    }

    #[test]
    fn parse_applies_the_exponent_range_of_decimal_js() {
        // maxE / minE = ±9e15 on the exponent of the first digit; no overflow on the way.
        assert_eq!(Decimal::parse("1e9000000000000000").unwrap().sci_exp(), 9_000_000_000_000_000);
        assert!(!Decimal::parse("1e9000000000000001").unwrap().is_finite());
        assert!(Decimal::parse("10e8999999999999999").unwrap().is_finite());
        assert!(!Decimal::parse("100e8999999999999999").unwrap().is_finite());
        assert!(Decimal::parse("-1e-9000000000000001").unwrap().is_zero());
        assert_eq!(Decimal::parse("-1e-9000000000000001").unwrap().value_of(), "-0");
        assert!(Decimal::parse("0.11e-9223372036854775807").unwrap().is_zero());
        assert!(Decimal::parse("1e-9223372036854775808").unwrap().is_zero());
        assert!(!Decimal::parse("1e+99999999999999999999999999999").unwrap().is_finite());
        assert_eq!(Decimal::parse("000123.4500e2").unwrap().to_string(), "12345");
        assert_eq!(Decimal::parse("-0.000").unwrap().value_of(), "-0");
        assert!(Decimal::parse("1e").is_err() && Decimal::parse(".").is_err() && Decimal::parse("1e5.5").is_err());
        // Arithmetic results are clamped the same way (decimal.js `finalise`).
        let big = Decimal::parse("1e8000000000000000").unwrap();
        assert!(!big.times(&big).is_finite());
        assert!(!big.pow(100_000).is_finite());
        assert!(Decimal::parse("1e-8000000000000000").unwrap().pow(100_000).is_zero());
        assert_eq!(big.to_i64_exact(), None);
    }

    #[test]
    fn trailing_zeros_are_removed_quickly() {
        let t = std::time::Instant::now();
        let c = BigUint::parse_bytes(format!("7{}", "0".repeat(60_000)).as_bytes(), 10).unwrap();
        let (c, e) = normalize(c, -5);
        assert_eq!((c, e), (BigUint::from(7u32), 59_995));
        let c = BigUint::from(2u32).pow(70_000);
        let (c2, e) = normalize(c.clone(), 0);
        assert_eq!((c2, e), (c, 0));
        assert!(t.elapsed() < std::time::Duration::from_secs(5), "{:?}", t.elapsed());
    }

    #[test]
    #[should_panic(expected = "RangeError: Invalid string length")]
    fn huge_fixed_notation_panics_instead_of_aborting() {
        Decimal::parse("1e8000000000000000").unwrap().to_fixed_plain();
    }

    #[test]
    fn numeric_encoding_never_truncates() {
        for s in ["9999e-262144", "5e262144", "1e131072", "1e-65536"] {
            let d = Decimal::parse(s).unwrap();
            assert!(!d.fits_numeric(), "{s}");
            assert!(encode_numeric(&d).is_err(), "{s}");
        }
        for s in ["1e131071", "-1e-16383", "1e-65535", "123456789.000000001", "9.9999e131071"] {
            let d = Decimal::parse(s).unwrap();
            assert!(d.fits_numeric(), "{s}");
            let back = decode_numeric(&encode_numeric(&d).unwrap()).unwrap();
            assert!(back == d, "{s}: {back:?}");
        }
        // Long coefficients encode in linear time (one decimal conversion, no per-group division).
        let d = Decimal::parse(&format!("1{}.5", "7".repeat(100_000))).unwrap();
        assert_eq!(decode_numeric(&encode_numeric(&d).unwrap()).unwrap(), d);
    }

    #[test]
    fn numeric_roundtrip() {
        for s in ["0", "1", "-1", "24.5", "0.0001", "12345678.12345678", "1e30", "-0.00000123", "10000", "99999999999999999999.99"] {
            let d = Decimal::parse(s).unwrap();
            let back = decode_numeric(&encode_numeric(&d).unwrap()).unwrap();
            assert_eq!(back.to_string(), d.to_string(), "{s}");
        }
    }
}
