//! Spoken commands. The front end captures the microphone and streams it to
//! the speech-to-text service (Deepgram), holding the key, as it holds the
//! model's key. The core decides what the stream asks for ([`listen_params`])
//! and checks and logs what came back ([`record_dictation`]). Only the user's
//! own dictation is streamed, never a project's audio, and the dictation
//! itself is not stored: the transcript is.

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::prompt::check_no_audio;
use crate::math::round_to;
use crate::project::store::Store;
use crate::project::{Project, ProjectError};
use crate::provenance::{Actor, Env};

pub const PROVIDER: &str = "deepgram";
pub const DEFAULT_URL: &str = "wss://api.deepgram.com/v1/listen";
pub const DEFAULT_MODEL: &str = "nova-3";
pub const DEFAULT_LANGUAGE: &str = "en";

/// Words the recogniser is told to expect: the vocabulary of the operations
/// and the analysis, which everyday speech models tend to mishear.
pub const KEYTERMS: &[&str] = &[
    "spectrogram",
    "normalise",
    "DC offset",
    "hum",
    "whine",
    "sibilance",
    "hertz",
    "kilohertz",
    "decibels",
    "residual",
    "noise reduction",
    "band cut",
    "compressor",
    "limiter",
];

/// Parameter names that could carry a credential; they are never sent in the
/// query, so never logged.
const SECRET_NAMES: &[&str] = &[
    "token",
    "key",
    "api_key",
    "apikey",
    "access_token",
    "authorization",
];

fn plain(what: &str, v: &str) -> Result<(), String> {
    let ok = !v.is_empty()
        && v.len() <= 64
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if ok {
        Ok(())
    } else {
        Err(format!(
            "“{v}” is not a {what} (letters, digits, “-”, “_” or “.”)"
        ))
    }
}

/// The query for a live stream: raw 16-bit mono audio at `sample_rate`,
/// interim results so the words appear while the user speaks, and an
/// `UtteranceEnd` message when they stop. `opt_out` asks Deepgram not to keep
/// the audio for improving its models (`mip_opt_out`).
pub fn listen_params(
    model: &str,
    language: &str,
    sample_rate: u32,
    opt_out: bool,
) -> Result<Vec<(String, String)>, String> {
    plain("model name", model)?;
    plain("language code", language)?;
    if !(8000..=192_000).contains(&sample_rate) {
        return Err(format!(
            "a sample rate of {sample_rate} Hz is outside 8,000–192,000 Hz"
        ));
    }
    let mut p: Vec<(String, String)> = [
        ("model", model.to_string()),
        ("language", language.to_string()),
        ("encoding", "linear16".into()),
        ("sample_rate", sample_rate.to_string()),
        ("channels", "1".into()),
        ("interim_results", "true".into()),
        ("smart_format", "true".into()),
        ("utterance_end_ms", "1000".into()),
        ("vad_events", "true".into()),
        ("endpointing", "300".into()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    // Key terms are a Nova-3 feature; other models refuse them.
    if model.starts_with("nova-3") {
        p.extend(KEYTERMS.iter().map(|t| ("keyterm".into(), t.to_string())));
    }
    if opt_out {
        p.push(("mip_opt_out".into(), "true".into()));
    }
    Ok(p)
}

/// One final result from the recogniser.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Segment {
    pub text: String,
    pub confidence: f64,
}

/// A spoken request, as logged (`speech.transcribed`): what was asked of the
/// recogniser, what it heard, and what the user sent. The key is never here.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Dictation {
    pub provider: String,
    pub model: String,
    /// The host the audio was streamed to (no path, no key).
    pub host: String,
    /// The stream's query parameters, in the order sent.
    pub params: Vec<(String, String)>,
    /// The recogniser's id for each time the microphone was opened.
    pub request_ids: Vec<String>,
    /// The final results, in order.
    pub segments: Vec<Segment>,
    /// What the user sent: the words heard, with any changes they made.
    pub words: String,
    /// Seconds of audio streamed.
    pub audio_s: f64,
    /// From the end of listening to the last result.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub latency_ms: u64,
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl Dictation {
    /// The final results joined: what the recogniser heard.
    pub fn heard(&self) -> String {
        collapse(
            &self
                .segments
                .iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
        )
    }

    /// Whether the words sent differ from what was heard.
    pub fn edited(&self) -> bool {
        collapse(&self.words) != self.heard()
    }

    fn check(&self) -> Result<(), String> {
        for (what, v) in [
            ("provider", &self.provider),
            ("model", &self.model),
            ("host", &self.host),
        ] {
            if v.trim().is_empty() {
                return Err(format!("the dictation has no {what}"));
            }
        }
        if self
            .host
            .contains(|c: char| matches!(c, '/' | '?' | '#' | '@') || c.is_whitespace())
        {
            return Err(format!(
                "“{}” is not a host name (no path, query or credentials)",
                self.host
            ));
        }
        if let Some((k, _)) = self
            .params
            .iter()
            .find(|(k, _)| SECRET_NAMES.contains(&k.to_ascii_lowercase().as_str()))
        {
            return Err(format!(
                "the parameter `{k}` could hold a key; keys are never logged"
            ));
        }
        if self.heard().is_empty() {
            return Err("nothing was heard".into());
        }
        if self.words.trim().is_empty() {
            return Err("no words were sent".into());
        }
        if let Some(s) = self
            .segments
            .iter()
            .find(|s| !(0.0..=1.0).contains(&s.confidence))
        {
            return Err(format!(
                "a confidence of {} is outside 0–1 (“{}”)",
                s.confidence, s.text
            ));
        }
        if !self.audio_s.is_finite() || self.audio_s < 0.0 {
            return Err(format!("{} s is not a length of audio", self.audio_s));
        }
        Ok(())
    }
}

/// Log a spoken request just before it is acted on; returns the event's hash,
/// which the previews made from it refer to. Dictation that is never sent is
/// not logged: nothing from it reached the project.
pub fn record_dictation<S: Store>(
    project: &mut Project<S>,
    env: &mut dyn Env,
    d: &Dictation,
) -> Result<String, ProjectError> {
    d.check().map_err(ProjectError::Invalid)?;
    let data = json!({
        "provider": d.provider,
        "model": d.model,
        "host": d.host,
        "params": d.params,
        "request_ids": d.request_ids,
        "heard": d.heard(),
        "segments": d.segments,
        "words": d.words.trim(),
        "edited": d.edited(),
        "audio_s": round_to(d.audio_s, 3),
        "latency_ms": d.latency_ms,
    });
    check_no_audio(&data).map_err(ProjectError::Invalid)?;
    let ev = project.record(env, "speech.transcribed", Some(Actor::user()), data)?;
    Ok(ev["hash"].as_str().unwrap_or_default().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn param<'a>(p: &'a [(String, String)], k: &str) -> Vec<&'a str> {
        p.iter()
            .filter(|(n, _)| n == k)
            .map(|(_, v)| v.as_str())
            .collect()
    }

    #[test]
    fn the_stream_asks_for_raw_audio_interim_results_and_the_key_terms() {
        let p = listen_params("nova-3", "en", 48_000, false).unwrap();
        assert_eq!(param(&p, "encoding"), ["linear16"]);
        assert_eq!(param(&p, "sample_rate"), ["48000"]);
        assert_eq!(param(&p, "channels"), ["1"]);
        assert_eq!(param(&p, "interim_results"), ["true"]);
        assert_eq!(param(&p, "utterance_end_ms"), ["1000"]);
        assert_eq!(param(&p, "keyterm"), KEYTERMS);
        // Taking part in Deepgram's model improvement is its default: nothing is sent.
        assert!(param(&p, "mip_opt_out").is_empty());

        let p = listen_params("nova-3", "en-GB", 16_000, true).unwrap();
        assert_eq!(param(&p, "mip_opt_out"), ["true"]);
        assert_eq!(param(&p, "language"), ["en-GB"]);

        // Key terms only go to a model that accepts them.
        let p = listen_params("nova-2", "en", 16_000, false).unwrap();
        assert!(param(&p, "keyterm").is_empty());
    }

    #[test]
    fn a_stream_with_odd_settings_is_refused() {
        assert!(listen_params("", "en", 48_000, false).is_err());
        assert!(listen_params("nova-3&key=x", "en", 48_000, false).is_err());
        assert!(listen_params("nova-3", "en us", 48_000, false).is_err());
        assert!(listen_params("nova-3", "en", 4_000, false).is_err());
    }

    fn dictation() -> Dictation {
        Dictation {
            provider: PROVIDER.into(),
            model: DEFAULT_MODEL.into(),
            host: "api.deepgram.com".into(),
            params: listen_params("nova-3", "en", 48_000, false).unwrap(),
            request_ids: vec!["req-1".into()],
            segments: vec![
                Segment {
                    text: "Cut 3100 to 3200".into(),
                    confidence: 0.97,
                },
                Segment {
                    text: " hertz by 12 dB.".into(),
                    confidence: 0.91,
                },
            ],
            words: "Cut 3100 to 3200 hertz by 12 dB.".into(),
            audio_s: 2.4,
            latency_ms: 180,
        }
    }

    #[test]
    fn heard_and_edited_compare_the_words_not_the_spacing() {
        let d = dictation();
        assert_eq!(d.heard(), "Cut 3100 to 3200 hertz by 12 dB.");
        assert!(!d.edited());
        let d = Dictation {
            words: "Cut 3100 to 3200 hertz by 10 dB.".into(),
            ..dictation()
        };
        assert!(d.edited());
    }

    #[test]
    fn a_dictation_that_could_carry_a_key_or_nothing_is_refused() {
        let mut d = dictation();
        d.params.push(("token".into(), "secret".into()));
        assert!(d.check().unwrap_err().contains("never logged"));

        let d = Dictation {
            host: "user:pw@api.deepgram.com".into(),
            ..dictation()
        };
        assert!(d.check().is_err());

        let d = Dictation {
            segments: vec![],
            ..dictation()
        };
        assert_eq!(d.check().unwrap_err(), "nothing was heard");

        let d = Dictation {
            words: "  ".into(),
            ..dictation()
        };
        assert!(d.check().is_err());

        let mut d = dictation();
        d.segments[0].confidence = 1.5;
        assert!(d.check().is_err());
    }
}
