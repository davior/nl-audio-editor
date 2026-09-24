//! Decoding with our own decoder (symphonia, pure Rust).
//!
//! The browser's `decodeAudioData` is never used: it resamples to the audio
//! context's rate and differs between browsers. Decoding here is identical on
//! every platform, so the working copy is the same wherever a project is opened.
//!
//! Decoding is strict: a corrupt packet is an error, not a silent gap, because
//! a skipped packet would shift every later timestamp.

use std::io::Cursor;

use serde::{Deserialize, Serialize};
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::codecs::CodecParameters;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

use super::AudioBuffer;

/// Decoder name and version recorded in every import event.
pub const DECODER_NAME: &str = "symphonia";
pub const DECODER_VERSION: &str = "0.6.1";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct SourceInfo {
    /// Container format as the decoder names it, e.g. `wave`, `mp3`, `flac`.
    pub container: String,
    /// Codec short name, e.g. `pcm_s16le`, `mp3`.
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u32,
    pub frames: u64,
    pub duration_s: f64,
    pub bits_per_sample: Option<u32>,
    pub decoder: String,
}

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("unsupported or unrecognised audio format: {0}")]
    Unsupported(String),
    #[error("the file has no audio track")]
    NoAudioTrack,
    #[error("corrupt audio data at packet {packet}: {message}")]
    Corrupt { packet: u64, message: String },
    #[error("the file contains no audio samples")]
    Empty,
}

/// Decode a complete file held in memory. `ext_hint` is the file extension, if known.
pub fn decode(
    bytes: &[u8],
    ext_hint: Option<&str>,
) -> Result<(AudioBuffer, SourceInfo), DecodeError> {
    let mss = MediaSourceStream::new(Box::new(Cursor::new(bytes.to_vec())), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = ext_hint {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| DecodeError::Unsupported(e.to_string()))?;
    let container = format.format_info().short_name.to_string();

    let track = format
        .default_track(TrackType::Audio)
        .ok_or(DecodeError::NoAudioTrack)?;
    let track_id = track.id;
    let params = match track.codec_params.as_ref() {
        Some(CodecParameters::Audio(p)) => p.clone(),
        _ => return Err(DecodeError::NoAudioTrack),
    };
    let registry = symphonia::default::get_codecs();
    let codec = registry
        .get_audio_decoder(params.codec)
        .map(|d| d.codec.info.short_name.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let mut decoder = registry
        .make_audio_decoder(&params, &AudioDecoderOptions::default().gapless(true))
        .map_err(|e| DecodeError::Unsupported(e.to_string()))?;

    let mut channels: Vec<Vec<f32>> = Vec::new();
    let mut sample_rate = params.sample_rate.unwrap_or(0);
    let mut scratch: Vec<Vec<f32>> = Vec::new();
    let mut packet_no: u64 = 0;
    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => {
                return Err(DecodeError::Corrupt {
                    packet: packet_no,
                    message: e.to_string(),
                })
            }
        };
        if packet.track_id != track_id {
            continue;
        }
        let decoded = decoder.decode(&packet).map_err(|e| DecodeError::Corrupt {
            packet: packet_no,
            message: e.to_string(),
        })?;
        packet_no += 1;
        if sample_rate == 0 {
            sample_rate = decoded.spec().rate();
        }
        decoded.copy_to_vecs_planar::<f32>(&mut scratch);
        if channels.is_empty() {
            channels = vec![Vec::new(); scratch.len()];
        }
        for (dst, src) in channels.iter_mut().zip(scratch.iter()) {
            dst.extend_from_slice(src);
        }
    }
    if channels.is_empty() || channels[0].is_empty() || sample_rate == 0 {
        return Err(DecodeError::Empty);
    }
    let buffer = AudioBuffer::new(sample_rate, channels);
    let info = SourceInfo {
        container,
        codec,
        sample_rate,
        channels: buffer.num_channels() as u32,
        frames: buffer.len() as u64,
        duration_s: buffer.duration_s(),
        bits_per_sample: params.bits_per_sample,
        decoder: format!("{DECODER_NAME} {DECODER_VERSION}"),
    };
    Ok((buffer, info))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::wav::{write_wav, WavFormat};

    fn ramp(n: usize) -> Vec<f32> {
        (0..n).map(|i| (i as f32 / n as f32) - 0.5).collect()
    }

    #[test]
    fn float_wav_round_trips_exactly() {
        let a = AudioBuffer::new(
            48000,
            vec![ramp(1000), ramp(1000).iter().map(|x| -x).collect()],
        );
        let bytes = write_wav(&a, WavFormat::F32);
        let (b, info) = decode(&bytes, Some("wav")).unwrap();
        assert_eq!(a, b);
        assert_eq!(info.container, "wave");
        assert_eq!(info.sample_rate, 48000);
        assert_eq!(info.channels, 2);
        assert_eq!(info.frames, 1000);
    }

    #[test]
    fn pcm16_decodes_to_exact_fractions() {
        let a = AudioBuffer::mono(8000, vec![0.5, -0.5, 0.25, 0.0]);
        let bytes = write_wav(&a, WavFormat::Pcm16);
        let (b, info) = decode(&bytes, None).unwrap();
        assert_eq!(b.channels[0], vec![0.5, -0.5, 0.25, 0.0]);
        assert_eq!(info.bits_per_sample, Some(16));
    }

    #[test]
    fn garbage_is_rejected() {
        assert!(decode(b"definitely not audio", None).is_err());
    }
}
