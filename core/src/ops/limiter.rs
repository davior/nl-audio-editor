//! The final limiter. Always last in a render. Offline and sample-accurate:
//! every sample above the ceiling gets exactly the reduction it needs, spread
//! linearly (in dB) over the look-ahead before it and the release after it.
//! When nothing exceeds the ceiling it does nothing at all, bit for bit.

use serde::Deserialize;
use serde_json::{json, Map, Value};

use super::{copy_window, typed, Descriptor, Op, OpError, RenderOut};
use crate::audio::AudioBuffer;
use crate::math::{amp_to_db, db_to_amp, round_to};
use crate::scope::Scope;

pub struct Limiter {
    pub desc: Descriptor,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Params {
    ceiling_dbfs: f64,
    lookahead_ms: f64,
    release_ms: f64,
}

fn spans(p: &Params, sr: u32) -> (i64, i64) {
    let s = |ms: f64| ((ms / 1000.0 * sr as f64).round() as i64).max(1);
    (s(p.lookahead_ms), s(p.release_ms))
}

impl Op for Limiter {
    fn descriptor(&self) -> &Descriptor {
        &self.desc
    }

    fn resolve(
        &self,
        params: &Value,
        _scope: &Scope,
        _input: &AudioBuffer,
    ) -> Result<Value, OpError> {
        typed::<Params>(params)?;
        Ok(params.clone())
    }

    fn radius(&self, resolved: &Value, sr: u32) -> usize {
        typed::<Params>(resolved)
            .map(|p| {
                let (att, rel) = spans(&p, sr);
                (att + rel) as usize
            })
            .unwrap_or(0)
    }

    fn render(
        &self,
        resolved: &Value,
        _scope: &Scope,
        input: &AudioBuffer,
        offset: i64,
        a: i64,
        b: i64,
        clip_len: usize,
    ) -> Result<RenderOut, OpError> {
        let p: Params = typed(resolved)?;
        let (att, rel) = spans(&p, input.sample_rate);
        let ceiling = db_to_amp(p.ceiling_dbfs);
        let n = (b - a) as usize;
        let mut env = vec![0.0f64; n];
        // Peaks that can influence [a, b).
        let lo = (a - rel).max(offset).max(0);
        let hi = (b + att)
            .min(offset + input.len() as i64)
            .min(clip_len as i64);
        for pk in lo..hi {
            let j = (pk - offset) as usize;
            let level = input
                .channels
                .iter()
                .fold(0.0f64, |m, c| m.max((c[j] as f64).abs()));
            if level <= ceiling {
                continue;
            }
            let red = p.ceiling_dbfs - amp_to_db(level);
            let from = (pk - att).max(a);
            let to = (pk + rel).min(b - 1);
            for i in from..=to {
                let (d, side) = if i < pk { (pk - i, att) } else { (i - pk, rel) };
                let v = red * (1.0 - d as f64 / (side + 1) as f64);
                let e = &mut env[(i - a) as usize];
                if v < *e {
                    *e = v;
                }
            }
        }
        let mut out = copy_window(input, offset, a, b);
        let mut limited = 0u64;
        for (j, &e) in env.iter().enumerate() {
            if e < 0.0 {
                limited += 1;
                let g = db_to_amp(e);
                for ch in out.channels.iter_mut() {
                    ch[j] = (ch[j] as f64 * g) as f32;
                }
            }
        }
        let mut m = Map::new();
        m.insert(
            "max_reduction_db".into(),
            json!(round_to(env.iter().fold(0.0f64, |x, &y| x.min(y)), 2)),
        );
        m.insert("samples_limited".into(), json!(limited));
        Ok(RenderOut {
            audio: out,
            measurements: m,
        })
    }
}
