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
    write_wav_with_cues(audio, format, &[])
}

/// A `cue ` chunk and a `LIST`/`adtl` chunk of `labl` labels, one per cue.
fn cue_chunks(cues: &[(u32, String)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"cue ");
    out.extend_from_slice(&(4 + 24 * cues.len() as u32).to_le_bytes());
    out.extend_from_slice(&(cues.len() as u32).to_le_bytes());
    for (i, (sample, _)) in cues.iter().enumerate() {
        out.extend_from_slice(&(i as u32 + 1).to_le_bytes()); // id
        out.extend_from_slice(&sample.to_le_bytes()); // position
        out.extend_from_slice(b"data");
        out.extend_from_slice(&0u32.to_le_bytes()); // chunk start
        out.extend_from_slice(&0u32.to_le_bytes()); // block start
        out.extend_from_slice(&sample.to_le_bytes()); // sample offset
    }
    let mut adtl = Vec::new();
    adtl.extend_from_slice(b"adtl");
    for (i, (_, label)) in cues.iter().enumerate() {
        let text: Vec<u8> = label.bytes().filter(|b| b.is_ascii() && *b != 0).collect();
        let size = 4 + text.len() as u32 + 1;
        adtl.extend_from_slice(b"labl");
        adtl.extend_from_slice(&size.to_le_bytes());
        adtl.extend_from_slice(&(i as u32 + 1).to_le_bytes());
        adtl.extend_from_slice(&text);
        adtl.push(0);
        if size % 2 == 1 {
            adtl.push(0);
        }
    }
    out.extend_from_slice(b"LIST");
    out.extend_from_slice(&(adtl.len() as u32).to_le_bytes());
    out.extend_from_slice(&adtl);
    out
}

/// A WAV with cue markers (output sample, label) after the audio, as most
/// audio editors show them. Without cues the file is exactly [`write_wav`]'s.
pub fn write_wav_with_cues(
    audio: &AudioBuffer,
    format: WavFormat,
    cues: &[(u32, String)],
) -> Vec<u8> {
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
    // A chunk of odd length is followed by a pad byte when another chunk follows.
    let tail = if cues.is_empty() {
        Vec::new()
    } else {
        let mut t = Vec::new();
        if data_len % 2 == 1 {
            t.push(0);
        }
        t.extend_from_slice(&cue_chunks(cues));
        t
    };
    let riff_len = 4 + (8 + fmt_len) + fact_len + (8 + data_len) + tail.len() as u32;

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
    out.extend_from_slice(&tail);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The chunks of a RIFF/WAVE file: (id, body).
    fn chunks(b: &[u8]) -> Vec<(String, Vec<u8>)> {
        assert_eq!(&b[..4], b"RIFF");
        assert_eq!(
            u32::from_le_bytes(b[4..8].try_into().unwrap()) as usize,
            b.len() - 8
        );
        let mut out = Vec::new();
        let mut i = 12;
        while i + 8 <= b.len() {
            let id = String::from_utf8(b[i..i + 4].to_vec()).unwrap();
            let n = u32::from_le_bytes(b[i + 4..i + 8].try_into().unwrap()) as usize;
            out.push((id, b[i + 8..i + 8 + n].to_vec()));
            i += 8 + n + (n % 2);
        }
        assert_eq!(i, b.len(), "chunks fill the file exactly");
        out
    }

    #[test]
    fn cues_follow_the_audio_and_nothing_changes_without_them() {
        let a = AudioBuffer::mono(8000, vec![0.25; 7]);
        assert_eq!(
            write_wav_with_cues(&a, WavFormat::F32, &[]),
            write_wav(&a, WavFormat::F32)
        );
        // 24-bit mono with an odd sample count: an odd data chunk, so a pad byte.
        let cues = vec![
            (
                3u32,
                "removed 1.000-2.000 s of the original (1.000 s)".to_string(),
            ),
            (5, "x".into()),
        ];
        let b = write_wav_with_cues(&a, WavFormat::Pcm24, &cues);
        let c = chunks(&b);
        let ids: Vec<&str> = c.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["fmt ", "data", "cue ", "LIST"]);
        assert_eq!(c[1].1.len(), 21);
        let cue = &c[2].1;
        assert_eq!(u32::from_le_bytes(cue[0..4].try_into().unwrap()), 2);
        assert_eq!(
            u32::from_le_bytes(cue[4 + 20..4 + 24].try_into().unwrap()),
            3
        );
        assert_eq!(
            u32::from_le_bytes(cue[28 + 20..28 + 24].try_into().unwrap()),
            5
        );
        let list = &c[3].1;
        assert_eq!(&list[..4], b"adtl");
        let text = String::from_utf8_lossy(list);
        assert!(text.contains("removed 1.000-2.000 s of the original (1.000 s)"));
        // Our own decoder still reads the audio.
        let (decoded, _) = crate::audio::decode(&b, Some("wav")).unwrap();
        assert_eq!(decoded.len(), 7);
    }
}
