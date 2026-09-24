//! WAV writer. Renders and previews are written as 32-bit float by default so
//! nothing is lost; integer formats round to nearest with clamping and no dither
//! (deterministic).

use super::AudioBuffer;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WavFormat {
    F32,
    Pcm16,
    Pcm24,
}

impl WavFormat {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "f32" | "float" => Some(WavFormat::F32),
            "pcm16" | "s16" => Some(WavFormat::Pcm16),
            "pcm24" | "s24" => Some(WavFormat::Pcm24),
            _ => None,
        }
    }
}

pub fn write_wav(audio: &AudioBuffer, format: WavFormat) -> Vec<u8> {
    let ch = audio.num_channels() as u16;
    let (tag, bits): (u16, u16) = match format {
        WavFormat::F32 => (3, 32),
        WavFormat::Pcm16 => (1, 16),
        WavFormat::Pcm24 => (1, 24),
    };
    let block_align = ch * bits / 8;
    let data_len = audio.len() as u32 * block_align as u32;
    let is_float = format == WavFormat::F32;
    let fmt_len: u32 = if is_float { 18 } else { 16 };
    let fact_len: u32 = if is_float { 12 } else { 0 };
    let riff_len = 4 + (8 + fmt_len) + fact_len + (8 + data_len);

    let mut out = Vec::with_capacity(riff_len as usize + 8);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_len.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&fmt_len.to_le_bytes());
    out.extend_from_slice(&tag.to_le_bytes());
    out.extend_from_slice(&ch.to_le_bytes());
    out.extend_from_slice(&audio.sample_rate.to_le_bytes());
    out.extend_from_slice(&(audio.sample_rate * block_align as u32).to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    if is_float {
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(b"fact");
        out.extend_from_slice(&4u32.to_le_bytes());
        out.extend_from_slice(&(audio.len() as u32).to_le_bytes());
    }
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..audio.len() {
        for c in &audio.channels {
            let x = c[i];
            match format {
                WavFormat::F32 => out.extend_from_slice(&x.to_le_bytes()),
                WavFormat::Pcm16 => {
                    let v = (x as f64 * 32768.0).round().clamp(-32768.0, 32767.0) as i16;
                    out.extend_from_slice(&v.to_le_bytes());
                }
                WavFormat::Pcm24 => {
                    let v = (x as f64 * 8388608.0).round().clamp(-8388608.0, 8388607.0) as i32;
                    out.extend_from_slice(&v.to_le_bytes()[..3]);
                }
            }
        }
    }
    out
}
