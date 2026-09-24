//! Clock and identifiers for the command line: wall-clock time and ULIDs.

use std::time::{SystemTime, UNIX_EPOCH};

use nlae_core::provenance::env::{format_rfc3339_ms, ulid};
use nlae_core::provenance::Env;

pub struct SystemEnv;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl Env for SystemEnv {
    fn now(&mut self) -> String {
        format_rfc3339_ms(now_ms())
    }

    fn new_id(&mut self, prefix: &str) -> String {
        let mut rnd = [0u8; 10];
        getrandom::fill(&mut rnd).expect("system randomness");
        format!("{prefix}_{}", ulid(now_ms(), rnd).to_lowercase())
    }
}
