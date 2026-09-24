//! Where an operation applies: the whole clip, a time range, a frequency band,
//! or a time × frequency patch.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum Scope {
    #[default]
    Clip,
    TimeRange {
        t0: f64,
        t1: f64,
    },
    Band {
        f_lo: f64,
        f_hi: f64,
    },
    TfPatch {
        t0: f64,
        t1: f64,
        f_lo: f64,
        f_hi: f64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum ScopeKind {
    Clip,
    TimeRange,
    Band,
    TfPatch,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ScopeError {
    #[error("time range {t0}–{t1} s is empty or outside the clip (0–{duration} s)")]
    Time { t0: f64, t1: f64, duration: f64 },
    #[error("band {f_lo}–{f_hi} Hz is empty or outside 0–{nyquist} Hz")]
    Band { f_lo: f64, f_hi: f64, nyquist: f64 },
}

impl Scope {
    pub fn kind(&self) -> ScopeKind {
        match self {
            Scope::Clip => ScopeKind::Clip,
            Scope::TimeRange { .. } => ScopeKind::TimeRange,
            Scope::Band { .. } => ScopeKind::Band,
            Scope::TfPatch { .. } => ScopeKind::TfPatch,
        }
    }

    pub fn time(&self) -> Option<(f64, f64)> {
        match *self {
            Scope::TimeRange { t0, t1 } | Scope::TfPatch { t0, t1, .. } => Some((t0, t1)),
            _ => None,
        }
    }

    pub fn band(&self) -> Option<(f64, f64)> {
        match *self {
            Scope::Band { f_lo, f_hi } | Scope::TfPatch { f_lo, f_hi, .. } => Some((f_lo, f_hi)),
            _ => None,
        }
    }

    /// Scope samples `[s0, s1)`; the whole clip when there is no time range.
    pub fn samples(&self, sample_rate: u32, clip_len: usize) -> (usize, usize) {
        match self.time() {
            None => (0, clip_len),
            Some((t0, t1)) => {
                let conv =
                    |t: f64| ((t * sample_rate as f64).round().max(0.0) as usize).min(clip_len);
                (conv(t0), conv(t1))
            }
        }
    }

    /// Band in Hz, `(0, Nyquist)` when there is none.
    pub fn band_or_full(&self, sample_rate: u32) -> (f64, f64) {
        self.band().unwrap_or((0.0, sample_rate as f64 / 2.0))
    }

    pub fn validate(&self, duration_s: f64, sample_rate: u32) -> Result<(), ScopeError> {
        if let Some((t0, t1)) = self.time() {
            if !(t0.is_finite() && t1.is_finite() && t0 >= 0.0 && t1 > t0 && t0 < duration_s) {
                return Err(ScopeError::Time {
                    t0,
                    t1,
                    duration: duration_s,
                });
            }
        }
        if let Some((f_lo, f_hi)) = self.band() {
            let nyquist = sample_rate as f64 / 2.0;
            if !(f_lo.is_finite()
                && f_hi.is_finite()
                && f_lo >= 0.0
                && f_hi > f_lo
                && f_lo < nyquist)
            {
                return Err(ScopeError::Band {
                    f_lo,
                    f_hi,
                    nyquist,
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialises_with_kind_tag() {
        let s = Scope::TfPatch {
            t0: 1.0,
            t1: 2.0,
            f_lo: 3000.0,
            f_hi: 3300.0,
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["kind"], "tf_patch");
        let back: Scope = serde_json::from_value(v).unwrap();
        assert_eq!(back, s);
        assert_eq!(
            serde_json::to_value(Scope::Clip).unwrap(),
            serde_json::json!({"kind": "clip"})
        );
    }

    #[test]
    fn validates_against_the_clip() {
        assert!(Scope::TimeRange { t0: 1.0, t1: 0.5 }
            .validate(10.0, 48000)
            .is_err());
        assert!(Scope::Band {
            f_lo: 100.0,
            f_hi: 30000.0
        }
        .validate(10.0, 48000)
        .is_ok());
        assert!(Scope::Band {
            f_lo: 25000.0,
            f_hi: 30000.0
        }
        .validate(10.0, 48000)
        .is_err());
        assert_eq!(
            Scope::TimeRange { t0: 0.5, t1: 1.0 }.samples(1000, 800),
            (500, 800)
        );
    }
}
