//! Spectrogram PNGs for reports and review.

use std::path::Path;

use nlae_core::analysis::spectrogram::{inferno, spectrogram, FreqScale, SpectrogramRequest};
use nlae_core::audio::AudioBuffer;

/// Draws `[t0, t1)` × `[0, fmax]`. The colour range spans `range` dB below the
/// loudest cell drawn, so quiet recordings are visible without amplifying them.
#[allow(clippy::too_many_arguments)]
pub fn spectrogram_png(
    audio: &AudioBuffer,
    out: &Path,
    t0: f64,
    t1: f64,
    fmax: f64,
    log: bool,
    width: u32,
    height: u32,
    range: f64,
) -> Result<(), String> {
    let mut req = SpectrogramRequest {
        t0,
        t1,
        columns: width,
        rows: height,
        f_min: if log { 40.0 } else { 0.0 },
        f_max: fmax,
        scale: if log {
            FreqScale::Log
        } else {
            FreqScale::Linear
        },
        db_min: -200.0,
        db_max: 0.0,
        fft_at_48k: 2048,
    };
    // First pass at full range to find the loudest cell, then map `range` dB below it.
    let probe = spectrogram(audio, &req);
    let top = probe.iter().copied().max().unwrap_or(0) as f64 / 255.0 * 200.0 - 200.0;
    req.db_max = top;
    req.db_min = top - range;
    let img = spectrogram(audio, &req);
    let mut rgb = Vec::with_capacity(img.len() * 3);
    for v in img {
        rgb.extend_from_slice(&inferno(v));
    }
    let file = std::fs::File::create(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(|e| e.to_string())?;
    w.write_image_data(&rgb).map_err(|e| e.to_string())?;
    Ok(())
}
