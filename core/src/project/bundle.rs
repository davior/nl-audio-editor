//! The portable `.nlae` bundle: a ZIP of the project layout.
//!
//! Written by hand rather than with a crate so bundles are reproducible and the
//! format is easy to audit: entries are stored (never compressed, so evidence
//! audio is byte-for-byte inside the archive), in sorted order, with a fixed
//! timestamp. Reading accepts only stored entries and verifies every CRC.

use super::store::{check_path, MemStore};

#[derive(Debug, thiserror::Error)]
pub enum BundleError {
    #[error("not a bundle: {0}")]
    Format(String),
    #[error("entry `{0}` is compressed; bundles only contain stored entries")]
    Compressed(String),
    #[error("entry `{0}` is corrupt (CRC mismatch)")]
    Crc(String),
    #[error("entry `{0}` has an unsafe path")]
    Path(String),
    #[error("bundle too large for this format (4 GB limit)")]
    TooLarge,
}

// 1980-01-01 00:00:00 in DOS format.
const DOS_TIME: u16 = 0;
const DOS_DATE: u16 = (1 << 5) | 1;

fn u16le(v: u16, out: &mut Vec<u8>) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn u32le(v: u32, out: &mut Vec<u8>) {
    out.extend_from_slice(&v.to_le_bytes());
}

pub fn pack(files: &MemStore) -> Result<Vec<u8>, BundleError> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    let mut count = 0u16;
    for (name, data) in &files.files {
        if data.len() > u32::MAX as usize || out.len() > u32::MAX as usize {
            return Err(BundleError::TooLarge);
        }
        let crc = crc32fast::hash(data);
        let offset = out.len() as u32;
        let nb = name.as_bytes();
        // Local file header.
        u32le(0x0403_4b50, &mut out);
        u16le(20, &mut out); // version needed
        u16le(0x0800, &mut out); // UTF-8 names
        u16le(0, &mut out); // stored
        u16le(DOS_TIME, &mut out);
        u16le(DOS_DATE, &mut out);
        u32le(crc, &mut out);
        u32le(data.len() as u32, &mut out);
        u32le(data.len() as u32, &mut out);
        u16le(nb.len() as u16, &mut out);
        u16le(0, &mut out);
        out.extend_from_slice(nb);
        out.extend_from_slice(data);
        // Central directory entry.
        u32le(0x0201_4b50, &mut central);
        u16le(20, &mut central); // version made by
        u16le(20, &mut central);
        u16le(0x0800, &mut central);
        u16le(0, &mut central);
        u16le(DOS_TIME, &mut central);
        u16le(DOS_DATE, &mut central);
        u32le(crc, &mut central);
        u32le(data.len() as u32, &mut central);
        u32le(data.len() as u32, &mut central);
        u16le(nb.len() as u16, &mut central);
        u16le(0, &mut central); // extra
        u16le(0, &mut central); // comment
        u16le(0, &mut central); // disk
        u16le(0, &mut central); // internal attrs
        u32le(0, &mut central); // external attrs
        u32le(offset, &mut central);
        central.extend_from_slice(nb);
        count = count.checked_add(1).ok_or(BundleError::TooLarge)?;
    }
    let cd_offset = out.len();
    if cd_offset > u32::MAX as usize {
        return Err(BundleError::TooLarge);
    }
    out.extend_from_slice(&central);
    u32le(0x0605_4b50, &mut out);
    u16le(0, &mut out);
    u16le(0, &mut out);
    u16le(count, &mut out);
    u16le(count, &mut out);
    u32le(central.len() as u32, &mut out);
    u32le(cd_offset as u32, &mut out);
    u16le(0, &mut out);
    Ok(out)
}

fn rd16(b: &[u8], at: usize) -> Result<u16, BundleError> {
    b.get(at..at + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
        .ok_or_else(|| BundleError::Format("truncated".into()))
}

fn rd32(b: &[u8], at: usize) -> Result<u32, BundleError> {
    b.get(at..at + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| BundleError::Format("truncated".into()))
}

pub fn unpack(bytes: &[u8]) -> Result<MemStore, BundleError> {
    // End of central directory (no archive comment in our bundles, but allow one).
    let min = bytes.len().saturating_sub(22 + 65535);
    let eocd = (min..=bytes.len().saturating_sub(22))
        .rev()
        .find(|&i| rd32(bytes, i).ok() == Some(0x0605_4b50))
        .ok_or_else(|| BundleError::Format("no end-of-directory record".into()))?;
    let count = rd16(bytes, eocd + 10)? as usize;
    let mut p = rd32(bytes, eocd + 16)? as usize;
    let mut store = MemStore::new();
    for _ in 0..count {
        if rd32(bytes, p)? != 0x0201_4b50 {
            return Err(BundleError::Format("bad central directory".into()));
        }
        let method = rd16(bytes, p + 10)?;
        let crc = rd32(bytes, p + 16)?;
        let csize = rd32(bytes, p + 20)? as usize;
        let usize_ = rd32(bytes, p + 24)? as usize;
        let nlen = rd16(bytes, p + 28)? as usize;
        let xlen = rd16(bytes, p + 30)? as usize;
        let clen = rd16(bytes, p + 32)? as usize;
        let local = rd32(bytes, p + 42)? as usize;
        let name = String::from_utf8(
            bytes
                .get(p + 46..p + 46 + nlen)
                .ok_or_else(|| BundleError::Format("name".into()))?
                .to_vec(),
        )
        .map_err(|_| BundleError::Format("name is not UTF-8".into()))?;
        p += 46 + nlen + xlen + clen;
        if name.ends_with('/') {
            continue; // directory entry
        }
        if check_path(&name).is_err() {
            return Err(BundleError::Path(name));
        }
        if method != 0 || csize != usize_ {
            return Err(BundleError::Compressed(name));
        }
        if rd32(bytes, local)? != 0x0403_4b50 {
            return Err(BundleError::Format(format!(
                "bad local header for `{name}`"
            )));
        }
        let l_nlen = rd16(bytes, local + 26)? as usize;
        let l_xlen = rd16(bytes, local + 28)? as usize;
        let start = local + 30 + l_nlen + l_xlen;
        let data = bytes
            .get(start..start + csize)
            .ok_or_else(|| BundleError::Format(format!("`{name}` truncated")))?;
        if crc32fast::hash(data) != crc {
            return Err(BundleError::Crc(name));
        }
        store.files.insert(name, data.to_vec());
    }
    Ok(store)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_is_reproducible() {
        let mut s = MemStore::new();
        s.files.insert("manifest.json".into(), b"{}".to_vec());
        s.files
            .insert("source/clip one.wav".into(), (0..=255u8).collect());
        s.files.insert("events.jsonl".into(), b"line\n".to_vec());
        let a = pack(&s).unwrap();
        let b = pack(&s).unwrap();
        assert_eq!(a, b, "packing is reproducible");
        assert_eq!(unpack(&a).unwrap(), s);
    }

    #[test]
    fn corruption_is_detected() {
        let mut s = MemStore::new();
        s.files
            .insert("events.jsonl".into(), b"some events here\n".to_vec());
        let mut a = pack(&s).unwrap();
        let at = a.windows(4).position(|w| w == b"some").unwrap();
        a[at] = b'S';
        assert!(matches!(unpack(&a), Err(BundleError::Crc(_))));
        assert!(unpack(b"not a zip").is_err());
    }
}
