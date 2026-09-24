use sha2::{Digest, Sha256};

/// Planar 32-bit float audio. This is the working copy: the original file's
/// bytes are kept separately and never touched.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioBuffer {
    pub sample_rate: u32,
    pub channels: Vec<Vec<f32>>,
}

impl AudioBuffer {
    pub fn new(sample_rate: u32, channels: Vec<Vec<f32>>) -> Self {
        assert!(!channels.is_empty(), "audio needs at least one channel");
        let len = channels[0].len();
        assert!(
            channels.iter().all(|c| c.len() == len),
            "channels differ in length"
        );
        AudioBuffer {
            sample_rate,
            channels,
        }
    }

    pub fn mono(sample_rate: u32, samples: Vec<f32>) -> Self {
        Self::new(sample_rate, vec![samples])
    }

    pub fn silent(sample_rate: u32, num_channels: usize, len: usize) -> Self {
        Self::new(sample_rate, vec![vec![0.0; len]; num_channels.max(1)])
    }

    pub fn len(&self) -> usize {
        self.channels[0].len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn num_channels(&self) -> usize {
        self.channels.len()
    }

    pub fn duration_s(&self) -> f64 {
        self.len() as f64 / self.sample_rate as f64
    }

    /// Seconds → sample index (rounded, clamped to `0..=len`).
    pub fn time_to_sample(&self, t: f64) -> usize {
        let s = (t * self.sample_rate as f64).round();
        if s <= 0.0 {
            0
        } else {
            (s as usize).min(self.len())
        }
    }

    /// Copy of `[start, end)`; samples outside the buffer are zero.
    pub fn extract_padded(&self, start: i64, end: i64) -> AudioBuffer {
        let n = (end - start).max(0) as usize;
        let len = self.len() as i64;
        let channels = self
            .channels
            .iter()
            .map(|c| {
                let mut out = vec![0.0f32; n];
                let a = start.max(0);
                let b = end.min(len);
                if b > a {
                    let dst = (a - start) as usize;
                    out[dst..dst + (b - a) as usize].copy_from_slice(&c[a as usize..b as usize]);
                }
                out
            })
            .collect();
        AudioBuffer {
            sample_rate: self.sample_rate,
            channels,
        }
    }

    /// Mix down to one channel by averaging (analysis only; never used to render).
    pub fn mixdown(&self) -> Vec<f64> {
        let n = self.num_channels() as f64;
        (0..self.len())
            .map(|i| self.channels.iter().map(|c| c[i] as f64).sum::<f64>() / n)
            .collect()
    }

    /// Canonical hash of the rendered samples. Two renders are "the same" iff
    /// their render hashes are equal. Defined as SHA-256 over:
    /// `"nlae-pcm-f32le-v1\0"`, sample rate (u32 LE), channel count (u32 LE),
    /// frame count (u64 LE), then the samples interleaved as f32 LE.
    pub fn render_hash(&self) -> String {
        let mut h = Sha256::new();
        h.update(b"nlae-pcm-f32le-v1\0");
        h.update(self.sample_rate.to_le_bytes());
        h.update((self.num_channels() as u32).to_le_bytes());
        h.update((self.len() as u64).to_le_bytes());
        let mut buf = Vec::with_capacity(4096 * 4);
        for i in 0..self.len() {
            for c in &self.channels {
                buf.extend_from_slice(&c[i].to_le_bytes());
            }
            if buf.len() >= 4096 * 4 {
                h.update(&buf);
                buf.clear();
            }
        }
        h.update(&buf);
        format!("sha256:{}", hex::encode(h.finalize()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_padded_zero_fills() {
        let a = AudioBuffer::mono(10, vec![1.0, 2.0, 3.0]);
        let b = a.extract_padded(-2, 4);
        assert_eq!(b.channels[0], vec![0.0, 0.0, 1.0, 2.0, 3.0, 0.0]);
    }

    #[test]
    fn render_hash_distinguishes_rate_and_samples() {
        let a = AudioBuffer::mono(48000, vec![0.0, 0.5]);
        let b = AudioBuffer::mono(44100, vec![0.0, 0.5]);
        let c = AudioBuffer::mono(48000, vec![0.0, 0.25]);
        assert_ne!(a.render_hash(), b.render_hash());
        assert_ne!(a.render_hash(), c.render_hash());
        assert_eq!(a.render_hash(), a.clone().render_hash());
    }
}
