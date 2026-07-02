//! `PlotNum` — the numeric interface the formula evaluator and plot renderer share, so both
//! can run over a runtime-selected precision: the 25 Spirix `ScalarFxEy` types plus native
//! `f32` / `f64`.
//!
//! The `Scalar` impls are macro-generated over the concrete aliases — one impl per type, no
//! generic `where`-clauses (which would otherwise be an enormous union of every op's bounds).
//! `f32` / `f64` are hand-written, mapping IEEE states onto the Spirix class vocabulary:
//! `inf → infinite`, `NaN → undefined`, `subnormal → vanished`. IEEE has no "exploded"
//! class (overflow goes to infinity), so `is_exploded` is always false there.
//!
//! Method bodies call the *inherent* Scalar/f32 methods (inherent resolution wins over the
//! trait), so `self.sin()` etc. never recurse into the trait.

use spirix::{ScalarConstants, *};

pub trait PlotNum: Copy {
    // Construction / constants.
    fn from_f64(v: f64) -> Self;
    fn to_f64(self) -> f64;
    fn infinity() -> Self;
    fn pi() -> Self;
    fn e() -> Self;
    fn phi() -> Self;
    fn gamma() -> Self;
    fn catalan() -> Self;
    fn random() -> Self;
    fn random_gauss() -> Self;

    // Binary operators.
    fn add(self, o: Self) -> Self;
    fn sub(self, o: Self) -> Self;
    fn mul(self, o: Self) -> Self;
    fn div(self, o: Self) -> Self;
    fn rem(self, o: Self) -> Self;
    fn pow(self, o: Self) -> Self;
    fn log(self, base: Self) -> Self;
    fn bitand(self, o: Self) -> Self;
    fn bitor(self, o: Self) -> Self;
    fn bitxor(self, o: Self) -> Self;
    fn shl(self, n: i32) -> Self;
    fn shr(self, n: i32) -> Self;

    // Multi-argument functions (parsed as `fn(a, b[, c])`).
    fn min(self, o: Self) -> Self;
    fn max(self, o: Self) -> Self;
    fn atan2(self, x: Self) -> Self;
    fn clamp(self, lo: Self, hi: Self) -> Self;

    // Unary operators / functions.
    fn neg(self) -> Self;
    fn not(self) -> Self;
    fn sqrt(self) -> Self;
    fn abs(self) -> Self;
    fn ln(self) -> Self;
    fn lb(self) -> Self;
    fn exp(self) -> Self;
    fn powb(self) -> Self;
    fn square(self) -> Self;
    fn recip(self) -> Self;
    fn sin(self) -> Self;
    fn cos(self) -> Self;
    fn tan(self) -> Self;
    fn asin(self) -> Self;
    fn acos(self) -> Self;
    fn atan(self) -> Self;
    fn sinh(self) -> Self;
    fn cosh(self) -> Self;
    fn tanh(self) -> Self;
    fn ceil(self) -> Self;
    fn floor(self) -> Self;
    fn frac(self) -> Self;
    fn round(self) -> Self;
    fn sign(self) -> Self;

    // Class queries — drive the plot's per-column coloring.
    fn is_undefined(self) -> bool;
    fn is_infinite(self) -> bool;
    fn is_zero(self) -> bool;
    fn is_exploded(self) -> bool;
    fn is_vanished(self) -> bool;
    fn is_positive(self) -> bool;

    /// Representative significand byte, used only to pick a phase color within a class.
    fn phase_byte(self) -> u8;
    /// Human-readable value in `base` (Spirix Display for Scalar; base ignored for IEEE).
    fn display(self, base: u8) -> String;
}

macro_rules! impl_plotnum_scalar {
    ($($t:ty),* $(,)?) => { $(
        impl PlotNum for $t {
            fn from_f64(v: f64) -> Self { <$t>::from(v) }
            fn to_f64(self) -> f64 { Self::to_f64(&self) }
            fn infinity() -> Self { <$t>::INFINITY }
            fn pi() -> Self { <$t>::PI }
            fn e() -> Self { <$t>::E }
            fn phi() -> Self { <$t>::PHI }
            fn gamma() -> Self { <$t>::EULER_GAMMA }
            fn catalan() -> Self { <$t>::CATALAN }
            fn random() -> Self { <$t>::random() }
            fn random_gauss() -> Self { <$t>::random_gauss() }

            fn add(self, o: Self) -> Self { self + o }
            fn sub(self, o: Self) -> Self { self - o }
            fn mul(self, o: Self) -> Self { self * o }
            fn div(self, o: Self) -> Self { self / o }
            fn rem(self, o: Self) -> Self { self % o }
            fn pow(self, o: Self) -> Self { Self::pow(&self, o) }
            fn log(self, base: Self) -> Self { Self::log(&self, base) }
            fn bitand(self, o: Self) -> Self { self & o }
            fn bitor(self, o: Self) -> Self { self | o }
            fn bitxor(self, o: Self) -> Self { self ^ o }
            fn shl(self, n: i32) -> Self { self << n }
            fn shr(self, n: i32) -> Self { self >> n }

            fn min(self, o: Self) -> Self { Self::min(&self, o) }
            fn max(self, o: Self) -> Self { Self::max(&self, o) }
            fn atan2(self, x: Self) -> Self { Self::atan2(&self, x) }
            fn clamp(self, lo: Self, hi: Self) -> Self { Self::clamp(&self, lo, hi) }

            fn neg(self) -> Self { -self }
            fn not(self) -> Self { !self }
            fn sqrt(self) -> Self { Self::sqrt(&self) }
            fn abs(self) -> Self { self.magnitude() }
            fn ln(self) -> Self { Self::ln(&self) }
            fn lb(self) -> Self { Self::lb(&self) }
            fn exp(self) -> Self { Self::exp(&self) }
            fn powb(self) -> Self { Self::powb(&self) }
            fn square(self) -> Self { Self::square(&self) }
            fn recip(self) -> Self { self.reciprocal() }
            fn sin(self) -> Self { Self::sin(&self) }
            fn cos(self) -> Self { Self::cos(&self) }
            fn tan(self) -> Self { Self::tan(&self) }
            fn asin(self) -> Self { Self::asin(&self) }
            fn acos(self) -> Self { Self::acos(&self) }
            fn atan(self) -> Self { Self::atan(&self) }
            fn sinh(self) -> Self { Self::sinh(&self) }
            fn cosh(self) -> Self { Self::cosh(&self) }
            fn tanh(self) -> Self { Self::tanh(&self) }
            fn ceil(self) -> Self { Self::ceil(&self) }
            fn floor(self) -> Self { Self::floor(&self) }
            fn frac(self) -> Self { Self::frac(&self) }
            fn round(self) -> Self { Self::round(&self) }
            fn sign(self) -> Self { Self::sign(&self) }

            fn is_undefined(self) -> bool { Self::is_undefined(&self) }
            fn is_infinite(self) -> bool { Self::is_infinite(&self) }
            fn is_zero(self) -> bool { Self::is_zero(&self) }
            fn is_exploded(self) -> bool { self.exploded() }
            fn is_vanished(self) -> bool { self.vanished() }
            fn is_positive(self) -> bool { Self::is_positive(&self) }

            fn phase_byte(self) -> u8 {
                // Top byte of the fraction — same significand slice the S43 renderer read as
                // `fraction >> 8`, generalized to any fraction width.
                let bits = ::core::mem::size_of_val(&self.fraction) * 8;
                (self.fraction >> (bits - 8)) as u8
            }
            fn display(self, base: u8) -> String {
                format!("{:.prec$}", self, prec = base as usize)
            }
        }
    )* };
}

impl_plotnum_scalar!(
    ScalarF3E3, ScalarF4E3, ScalarF5E3, ScalarF6E3, ScalarF7E3, ScalarF3E4, ScalarF4E4,
    ScalarF5E4, ScalarF6E4, ScalarF7E4, ScalarF3E5, ScalarF4E5, ScalarF5E5, ScalarF6E5,
    ScalarF7E5, ScalarF3E6, ScalarF4E6, ScalarF5E6, ScalarF6E6, ScalarF7E6, ScalarF3E7,
    ScalarF4E7, ScalarF5E7, ScalarF6E7, ScalarF7E7,
);

macro_rules! impl_plotnum_ieee {
    ($t:ty, $consts:path, $bits:ty, $phase_shift:expr) => {
        impl PlotNum for $t {
            fn from_f64(v: f64) -> Self { v as $t }
            fn to_f64(self) -> f64 { self as f64 }
            fn infinity() -> Self { <$t>::INFINITY }
            fn pi() -> Self { { use $consts as c; c::PI } }
            fn e() -> Self { { use $consts as c; c::E } }
            fn phi() -> Self { 1.618_033_988_749_894_9 as $t }
            fn gamma() -> Self { 0.577_215_664_901_532_9 as $t }
            fn catalan() -> Self { 0.915_965_594_177_219_0 as $t }
            // IEEE has no native RNG; reuse Spirix's generator and quantize to the float.
            fn random() -> Self { spirix::ScalarF6E5::random().to_f64() as $t }
            fn random_gauss() -> Self { spirix::ScalarF6E5::random_gauss().to_f64() as $t }

            fn add(self, o: Self) -> Self { self + o }
            fn sub(self, o: Self) -> Self { self - o }
            fn mul(self, o: Self) -> Self { self * o }
            fn div(self, o: Self) -> Self { self / o }
            fn rem(self, o: Self) -> Self { self % o }
            fn pow(self, o: Self) -> Self { self.powf(o) }
            fn log(self, base: Self) -> Self { self.log(base) }
            // Bitwise on the raw IEEE bit pattern — a curiosity for eyeballing, not arithmetic.
            fn bitand(self, o: Self) -> Self { <$t>::from_bits(self.to_bits() & o.to_bits()) }
            fn bitor(self, o: Self) -> Self { <$t>::from_bits(self.to_bits() | o.to_bits()) }
            fn bitxor(self, o: Self) -> Self { <$t>::from_bits(self.to_bits() ^ o.to_bits()) }
            // Spirix shift is ×2ⁿ / ÷2ⁿ (a scale), so mirror that rather than a bit shift.
            fn shl(self, n: i32) -> Self { self * (2.0 as $t).powi(n) }
            fn shr(self, n: i32) -> Self { self * (2.0 as $t).powi(-n) }

            fn min(self, o: Self) -> Self { self.min(o) }
            fn max(self, o: Self) -> Self { self.max(o) }
            fn atan2(self, x: Self) -> Self { self.atan2(x) }
            // Order the bounds so a user-typed clamp(x, hi, lo) can't trip f32::clamp's
            // `lo <= hi` assertion (which would panic mid-plot).
            fn clamp(self, lo: Self, hi: Self) -> Self { self.clamp(lo.min(hi), lo.max(hi)) }

            fn neg(self) -> Self { -self }
            fn not(self) -> Self { <$t>::from_bits(!self.to_bits()) }
            fn sqrt(self) -> Self { self.sqrt() }
            fn abs(self) -> Self { self.abs() }
            fn ln(self) -> Self { self.ln() }
            fn lb(self) -> Self { self.log2() }
            fn exp(self) -> Self { self.exp() }
            fn powb(self) -> Self { (2.0 as $t).powf(self) }
            fn square(self) -> Self { self * self }
            fn recip(self) -> Self { self.recip() }
            fn sin(self) -> Self { self.sin() }
            fn cos(self) -> Self { self.cos() }
            fn tan(self) -> Self { self.tan() }
            fn asin(self) -> Self { self.asin() }
            fn acos(self) -> Self { self.acos() }
            fn atan(self) -> Self { self.atan() }
            fn sinh(self) -> Self { self.sinh() }
            fn cosh(self) -> Self { self.cosh() }
            fn tanh(self) -> Self { self.tanh() }
            fn ceil(self) -> Self { self.ceil() }
            fn floor(self) -> Self { self.floor() }
            fn frac(self) -> Self { self.fract() }
            fn round(self) -> Self { self.round() }
            fn sign(self) -> Self { if self == 0.0 { 0.0 } else { self.signum() } }

            fn is_undefined(self) -> bool { self.is_nan() }
            fn is_infinite(self) -> bool { self.is_infinite() }
            fn is_zero(self) -> bool { self == 0.0 }
            fn is_exploded(self) -> bool { false }
            fn is_vanished(self) -> bool { self != 0.0 && self.is_subnormal() }
            fn is_positive(self) -> bool { self > 0.0 }

            fn phase_byte(self) -> u8 { (self.to_bits() >> $phase_shift) as u8 }
            fn display(self, _base: u8) -> String {
                if self.is_nan() {
                    "℘".to_string()
                } else if self.is_infinite() {
                    if self < 0.0 { "⦉-∞⦊".to_string() } else { "⦉∞⦊".to_string() }
                } else {
                    format!("{}", self)
                }
            }
        }
    };
}

impl_plotnum_ieee!(f32, ::core::f32::consts, u32, 15);
impl_plotnum_ieee!(f64, ::core::f64::consts, u64, 44);

/// Runtime-selected plot precision. Chosen by the F / E picker boxes: F ∈ 3..=7 with E ∈ 3..=7
/// selects a Spirix `ScalarFxEy`; the sentinel `F = 8` → f32 and `F = 9` → f64 (E ignored).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Precision {
    Scalar(u8, u8),
    F32,
    F64,
}

impl Precision {
    pub const DEFAULT: Precision = Precision::Scalar(4, 3);

    /// Parse from the F and E picker chars (`'3'..'7'`, plus `'8'`/`'9'` on F for IEEE).
    /// Returns `None` for out-of-range input so the caller can keep the previous value.
    pub fn from_chars(f: char, e: char) -> Option<Precision> {
        match f {
            '8' => Some(Precision::F32),
            '9' => Some(Precision::F64),
            '3'..='7' => {
                let ef = e.to_digit(10)?;
                if (3..=7).contains(&ef) {
                    Some(Precision::Scalar(f as u8 - b'0', ef as u8))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Short label for the readout / logging (e.g. `F4E3`, `f32`).
    pub fn label(self) -> String {
        match self {
            Precision::Scalar(f, e) => format!("F{f}E{e}"),
            Precision::F32 => "f32".to_string(),
            Precision::F64 => "f64".to_string(),
        }
    }
}

/// Run `$body` with `$t` bound to the concrete type named by `$prec`. This is the one place
/// the 25 Scalar combos + f32 + f64 are enumerated; every generic consumer (`evaluate`,
/// `draw_curve`) is instantiated through here.
#[macro_export]
macro_rules! with_precision {
    ($prec:expr, $t:ident, $body:block) => {{
        use spirix::*;
        match $prec {
            $crate::plotnum::Precision::F32 => { type $t = f32; $body }
            $crate::plotnum::Precision::F64 => { type $t = f64; $body }
            $crate::plotnum::Precision::Scalar(f, e) => match (f, e) {
                (3, 3) => { type $t = ScalarF3E3; $body }
                (4, 3) => { type $t = ScalarF4E3; $body }
                (5, 3) => { type $t = ScalarF5E3; $body }
                (6, 3) => { type $t = ScalarF6E3; $body }
                (7, 3) => { type $t = ScalarF7E3; $body }
                (3, 4) => { type $t = ScalarF3E4; $body }
                (4, 4) => { type $t = ScalarF4E4; $body }
                (5, 4) => { type $t = ScalarF5E4; $body }
                (6, 4) => { type $t = ScalarF6E4; $body }
                (7, 4) => { type $t = ScalarF7E4; $body }
                (3, 5) => { type $t = ScalarF3E5; $body }
                (4, 5) => { type $t = ScalarF4E5; $body }
                (5, 5) => { type $t = ScalarF5E5; $body }
                (6, 5) => { type $t = ScalarF6E5; $body }
                (7, 5) => { type $t = ScalarF7E5; $body }
                (3, 6) => { type $t = ScalarF3E6; $body }
                (4, 6) => { type $t = ScalarF4E6; $body }
                (5, 6) => { type $t = ScalarF5E6; $body }
                (6, 6) => { type $t = ScalarF6E6; $body }
                (7, 6) => { type $t = ScalarF7E6; $body }
                (3, 7) => { type $t = ScalarF3E7; $body }
                (4, 7) => { type $t = ScalarF4E7; $body }
                (5, 7) => { type $t = ScalarF5E7; $body }
                (6, 7) => { type $t = ScalarF6E7; $body }
                (7, 7) => { type $t = ScalarF7E7; $body }
                // Unreachable: from_chars only yields 3..=7; fall back to the default type.
                _ => { type $t = ScalarF4E3; $body }
            },
        }
    }};
}
