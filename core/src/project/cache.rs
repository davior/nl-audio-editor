//! Renders kept in memory, by key: a stack hash, or `output:` or `residual:`
//! and one. Every change to the stack adds renders, and a 10-minute stereo
//! render is about 230 MB, so the cache keeps to a size limit by dropping the
//! render used longest ago. Renders are shared, not copied, between keys.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Map, Value};

use crate::audio::AudioBuffer;

/// The default size limit (bytes of samples).
pub const DEFAULT_LIMIT: usize = 1 << 30;

struct Entry {
    audio: Arc<AudioBuffer>,
    measurements: Map<String, Value>,
    used: u64,
}

pub(crate) struct RenderCache {
    entries: HashMap<String, Entry>,
    clock: u64,
    bytes: usize,
    pub limit: usize,
}

impl Default for RenderCache {
    fn default() -> Self {
        RenderCache {
            entries: HashMap::new(),
            clock: 0,
            bytes: 0,
            limit: DEFAULT_LIMIT,
        }
    }
}

fn size_of(a: &AudioBuffer) -> usize {
    a.len() * a.num_channels() * std::mem::size_of::<f32>()
}

impl RenderCache {
    /// A render and the measurements made rendering it, marked as just used.
    pub fn get(&mut self, key: &str) -> Option<(Arc<AudioBuffer>, Map<String, Value>)> {
        self.clock += 1;
        let e = self.entries.get_mut(key)?;
        e.used = self.clock;
        Some((e.audio.clone(), e.measurements.clone()))
    }

    pub fn contains(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    pub fn audio(&self, key: &str) -> Option<&AudioBuffer> {
        self.entries.get(key).map(|e| e.audio.as_ref())
    }

    /// Keep a render; drop the renders used longest ago, other than this one
    /// and `keep`, until the cache is within its limit.
    pub fn insert(
        &mut self,
        key: String,
        audio: Arc<AudioBuffer>,
        measurements: Map<String, Value>,
        keep: &str,
    ) {
        self.clock += 1;
        let entry = Entry {
            audio,
            measurements,
            used: self.clock,
        };
        self.bytes += size_of(&entry.audio);
        if let Some(old) = self.entries.insert(key.clone(), entry) {
            self.bytes -= size_of(&old.audio);
        }
        while self.bytes > self.limit {
            let oldest = self
                .entries
                .iter()
                .filter(|(k, _)| **k != key && *k != keep)
                .min_by_key(|(_, e)| e.used)
                .map(|(k, _)| k.clone());
            let Some(k) = oldest else { break };
            let e = self.entries.remove(&k).expect("present");
            self.bytes -= size_of(&e.audio);
        }
    }

    /// Make the render kept under `from` available under `to` too (the same
    /// render: a step whose input and values did not change).
    pub fn alias(&mut self, from: &str, to: &str, keep: &str) {
        if let Some((audio, m)) = self.get(from) {
            self.insert(to.to_string(), audio, m, keep);
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(n: usize) -> Arc<AudioBuffer> {
        Arc::new(AudioBuffer::mono(48000, vec![0.0; n]))
    }

    #[test]
    fn the_render_used_longest_ago_goes_first() {
        let mut c = RenderCache {
            limit: 3 * 400,
            ..Default::default()
        };
        for k in ["a", "b", "c"] {
            c.insert(k.into(), buf(100), Map::new(), "");
        }
        assert_eq!(c.len(), 3);
        c.get("a");
        c.insert("d".into(), buf(100), Map::new(), "");
        assert!(
            c.contains("a") && !c.contains("b"),
            "b was used longest ago"
        );
        // What is asked to be kept stays, even when it is the oldest.
        c.insert("e".into(), buf(100), Map::new(), "c");
        assert!(c.contains("c") && !c.contains("a"));
        // One render over the limit on its own is still kept.
        c.insert("f".into(), buf(1000), Map::new(), "");
        assert!(c.contains("f"));
    }
}
