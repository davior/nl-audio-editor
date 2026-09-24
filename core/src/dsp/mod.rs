//! Signal processing primitives. Everything here is deterministic: fixed
//! operation order, f64 arithmetic, `libm` for transcendental functions.

pub mod biquad;
pub mod fft;
pub mod smooth;
pub mod stft;
pub mod window;
