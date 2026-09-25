//! Finite-support smoothing and order statistics.
//!
//! Recursive (IIR) smoothers have infinite memory, which would make a preview
//! of a window differ from the same span of a full render. Everything here has
//! a known, finite reach instead.

/// Median of a slice (the lower middle element for even lengths). Reorders `buf`.
pub fn median_in_place(buf: &mut [f32]) -> f32 {
    assert!(!buf.is_empty());
    let mid = (buf.len() - 1) / 2;
    let (_, m, _) = buf.select_nth_unstable_by(mid, |a, b| a.total_cmp(b));
    *m
}

/// Value at quantile `q` (0..=1), lower-rank convention. Reorders `buf`.
pub fn quantile_in_place(buf: &mut [f32], q: f64) -> f32 {
    assert!(!buf.is_empty());
    let idx = ((buf.len() - 1) as f64 * q.clamp(0.0, 1.0)).floor() as usize;
    let (_, m, _) = buf.select_nth_unstable_by(idx, |a, b| a.total_cmp(b));
    *m
}

/// Median over `[i − w, i + w]` (clamped to the sequence) for every `i`.
pub fn sliding_median(xs: &[f32], w: usize) -> Vec<f32> {
    let n = xs.len();
    let mut out = Vec::with_capacity(n);
    let mut buf = Vec::with_capacity(2 * w + 1);
    for i in 0..n {
        let a = i.saturating_sub(w);
        let b = (i + w + 1).min(n);
        buf.clear();
        buf.extend_from_slice(&xs[a..b]);
        out.push(median_in_place(&mut buf));
    }
    out
}

/// Quantile `q` over `[i − w, i + w]` (clamped to the sequence) for every `i`.
pub fn sliding_quantile(xs: &[f32], w: usize, q: f64) -> Vec<f32> {
    let n = xs.len();
    let mut out = Vec::with_capacity(n);
    let mut buf = Vec::with_capacity(2 * w + 1);
    for i in 0..n {
        let a = i.saturating_sub(w);
        let b = (i + w + 1).min(n);
        buf.clear();
        buf.extend_from_slice(&xs[a..b]);
        out.push(quantile_in_place(&mut buf, q));
    }
    out
}

/// Envelope of reductions `g` (dB, ≤ 0): each reduction also acts on the
/// `att` frames before it (look-ahead attack) and the `rel` frames after it
/// (release), fading linearly in dB. The result is never less reduction than
/// the input, and each output depends only on `g[k − rel ..= k + att]`.
pub fn attack_release(g: &[f32], att: usize, rel: usize) -> Vec<f32> {
    let n = g.len();
    let mut r = vec![0.0f32; n];
    for k in 0..n {
        let mut m = g[k];
        for j in 1..=rel.min(k) {
            let v = g[k - j] * (1.0 - j as f32 / (rel + 1) as f32);
            if v < m {
                m = v;
            }
        }
        r[k] = m;
    }
    let mut e = vec![0.0f32; n];
    for k in 0..n {
        let mut m = r[k];
        for j in 1..=att.min(n - 1 - k) {
            let v = r[k + j] * (1.0 - j as f32 / (att + 1) as f32);
            if v < m {
                m = v;
            }
        }
        e[k] = m;
    }
    e
}

/// Maximum over `[i − w, i]` (clamped to the sequence) for every `i`: each
/// value is held for `w` places after it. Linear time.
pub fn trailing_max(xs: &[f32], w: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(xs.len());
    let mut q: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
    for (i, &x) in xs.iter().enumerate() {
        while q.back().is_some_and(|&j| xs[j] <= x) {
            q.pop_back();
        }
        q.push_back(i);
        while q.front().is_some_and(|&j| j + w < i) {
            q.pop_front();
        }
        out.push(xs[q[0]]);
    }
    out
}

/// Minimum over `[i − w, i + w]` followed by a mean over the same span. Widens
/// reductions by `w` on each side while keeping their full depth at the centre.
pub fn min_then_mean(g: &[f32], w: usize) -> Vec<f32> {
    if w == 0 {
        return g.to_vec();
    }
    let n = g.len();
    let mins: Vec<f32> = (0..n)
        .map(|i| {
            let a = i.saturating_sub(w);
            let b = (i + w + 1).min(n);
            g[a..b].iter().copied().fold(f32::INFINITY, f32::min)
        })
        .collect();
    (0..n)
        .map(|i| {
            let a = i.saturating_sub(w);
            let b = (i + w + 1).min(n);
            let s: f64 = mins[a..b].iter().map(|&v| v as f64).sum();
            (s / (b - a) as f64) as f32
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn medians() {
        assert_eq!(median_in_place(&mut [3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median_in_place(&mut [4.0, 1.0, 3.0, 2.0]), 2.0);
        assert_eq!(
            sliding_median(&[1.0, 9.0, 1.0, 1.0, 9.0], 1),
            vec![1.0, 1.0, 1.0, 1.0, 1.0]
        );
    }

    #[test]
    fn attack_release_spreads_and_keeps_depth() {
        let mut g = vec![0.0f32; 11];
        g[5] = -12.0;
        let e = attack_release(&g, 2, 3);
        let close = |a: f32, b: f32| (a - b).abs() < 1e-5;
        assert_eq!(e[5], -12.0);
        assert!(close(e[4], -8.0) && close(e[3], -4.0), "{e:?}");
        assert_eq!(e[2], 0.0);
        assert!(close(e[6], -9.0) && close(e[8], -3.0), "{e:?}");
        assert_eq!(e[9], 0.0);
        assert!(e.iter().zip(&g).all(|(a, b)| a <= b));
    }

    #[test]
    fn trailing_max_holds_each_value_for_its_width() {
        let xs = [0.0f32, 5.0, 1.0, 0.0, 0.0, 3.0, 0.0];
        assert_eq!(
            trailing_max(&xs, 2),
            vec![0.0, 5.0, 5.0, 5.0, 1.0, 3.0, 3.0]
        );
        assert_eq!(trailing_max(&xs, 0), xs.to_vec());
    }

    #[test]
    fn all_zero_stays_exactly_zero() {
        let g = vec![0.0f32; 50];
        assert!(attack_release(&g, 5, 5).iter().all(|&v| v == 0.0));
        assert!(min_then_mean(&g, 2).iter().all(|&v| v == 0.0));
    }
}
