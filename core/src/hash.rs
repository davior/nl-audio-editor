//! SHA-256 helpers. Hashes are written as `sha256:<lowercase hex>` everywhere.

use sha2::{Digest, Sha256};

pub const ZERO_HASH: &str =
    "sha256:0000000000000000000000000000000000000000000000000000000000000000";

pub fn sha256_hex(bytes: &[u8]) -> String {
    // Fed in chunks: the same digest, and a WebAssembly engine can switch to
    // its optimised code between calls, which it cannot do inside one long
    // call (hashing a 10-minute recording in one call took 4.6 s instead of 0.8 s).
    let mut h = Sha256::new();
    for chunk in bytes.chunks(1 << 16) {
        h.update(chunk);
    }
    hex::encode(h.finalize())
}

pub fn sha256(bytes: &[u8]) -> String {
    format!("sha256:{}", sha256_hex(bytes))
}

/// Hash of several parts, each length-prefixed so boundaries cannot be forged.
pub fn sha256_parts(parts: &[&[u8]]) -> String {
    let mut h = Sha256::new();
    for p in parts {
        h.update((p.len() as u64).to_le_bytes());
        h.update(p);
    }
    format!("sha256:{}", hex::encode(h.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vector() {
        assert_eq!(
            sha256(b"abc"),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn chunked_hashing_gives_the_one_shot_digest() {
        let data: Vec<u8> = (0..300_000u32).map(|i| (i * 7 + i / 13) as u8).collect();
        assert_eq!(sha256_hex(&data), hex::encode(Sha256::digest(&data)));
    }

    #[test]
    fn parts_are_unambiguous() {
        assert_ne!(sha256_parts(&[b"ab", b"c"]), sha256_parts(&[b"a", b"bc"]));
    }
}
