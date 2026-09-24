//! Waveform peaks for drawing: min and max per pixel column.

use crate::audio::AudioBuffer;

/// For each of `columns` equal slices of `[a, b)`, the (min, max) over all
/// channels. Columns narrower than one sample repeat the nearest sample.
pub fn peaks(audio: &AudioBuffer, a: usize, b: usize, columns: usize) -> Vec<(f32, f32)> {
    let b = b.min(audio.len());
    if columns == 0 || b <= a {
        return vec![(0.0, 0.0); columns];
    }
    let span = (b - a) as f64 / columns as f64;
    (0..columns)
        .map(|c| {
            let s0 = a + (c as f64 * span).floor() as usize;
            let s1 = (a + ((c + 1) as f64 * span).floor() as usize)
                .max(s0 + 1)
                .min(b);
            let mut lo = f32::INFINITY;
            let mut hi = f32::NEG_INFINITY;
            for ch in &audio.channels {
                for &x in &ch[s0.min(b - 1)..s1] {
                    lo = lo.min(x);
                    hi = hi.max(x);
                }
            }
            (lo, hi)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn min_max_per_column() {
        let a = AudioBuffer::mono(8, vec![0.0, 1.0, -1.0, 0.5, 0.25, -0.25, 0.0, 0.0]);
        assert_eq!(peaks(&a, 0, 8, 2), vec![(-1.0, 1.0), (-0.25, 0.25)]);
        assert_eq!(peaks(&a, 0, 2, 4).len(), 4);
    }
}
