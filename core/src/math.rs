//! Deterministic maths.
//!
//! Every transcendental function used on a render or analysis path goes through
//! this module, which calls the pure-Rust `libm` crate. The platform maths
//! library differs in the last bit between native targets and WebAssembly, and
//! that would break the guarantee that native and browser renders are
//! bit-identical. `clippy.toml` forbids the `std` equivalents.
//!
//! Basic IEEE-754 operations (`+ - * /`, `sqrt`) are correctly rounded on every
//! target and need no wrapper.

/// Level floor used whenever a level would be −∞ (digital silence).
/// JSON cannot carry infinities, and canonical hashing rejects them.
pub const DB_FLOOR: f64 = -200.0;

#[inline]
pub fn pow(x: f64, y: f64) -> f64 {
    libm::pow(x, y)
}

#[inline]
pub fn exp(x: f64) -> f64 {
    libm::exp(x)
}

#[inline]
pub fn ln(x: f64) -> f64 {
    libm::log(x)
}

#[inline]
pub fn log10(x: f64) -> f64 {
    libm::log10(x)
}

#[inline]
pub fn log2(x: f64) -> f64 {
    libm::log2(x)
}

#[inline]
pub fn sin(x: f64) -> f64 {
    libm::sin(x)
}

#[inline]
pub fn cos(x: f64) -> f64 {
    libm::cos(x)
}

#[inline]
pub fn tan(x: f64) -> f64 {
    libm::tan(x)
}

/// Amplitude ratio → dB, floored at [`DB_FLOOR`].
#[inline]
pub fn amp_to_db(a: f64) -> f64 {
    if a <= 0.0 {
        return DB_FLOOR;
    }
    (20.0 * log10(a)).max(DB_FLOOR)
}

/// Power ratio → dB, floored at [`DB_FLOOR`].
#[inline]
pub fn power_to_db(p: f64) -> f64 {
    if p <= 0.0 {
        return DB_FLOOR;
    }
    (10.0 * log10(p)).max(DB_FLOOR)
}

/// dB → amplitude ratio. Exactly 1.0 for 0 dB, so identity settings stay bit-exact.
#[inline]
pub fn db_to_amp(db: f64) -> f64 {
    if db == 0.0 {
        return 1.0;
    }
    pow(10.0, db / 20.0)
}

/// dB → power ratio.
#[inline]
pub fn db_to_power(db: f64) -> f64 {
    if db == 0.0 {
        return 1.0;
    }
    pow(10.0, db / 10.0)
}

/// Round to a fixed number of decimals, used to keep recorded values compact
/// and stable in JSON (the rounded value is what later renders use).
#[inline]
pub fn round_to(x: f64, decimals: i32) -> f64 {
    let m = pow(10.0, decimals as f64);
    (x * m).round() / m
}

pub const PI: f64 = core::f64::consts::PI;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_db_is_exact_unity() {
        assert_eq!(db_to_amp(0.0), 1.0);
        assert_eq!(db_to_power(0.0), 1.0);
        assert_eq!(amp_to_db(1.0), 0.0);
    }

    #[test]
    fn silence_is_floored() {
        assert_eq!(amp_to_db(0.0), DB_FLOOR);
        assert_eq!(power_to_db(0.0), DB_FLOOR);
        assert_eq!(amp_to_db(1e-300), DB_FLOOR);
    }

    #[test]
    fn round_trips() {
        let a = db_to_amp(-6.0);
        assert!((amp_to_db(a) + 6.0).abs() < 1e-12);
        assert_eq!(round_to(1.23456, 2), 1.23);
    }
}
