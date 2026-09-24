//! Where a project's files live. The same layout works as a directory on disk,
//! in memory (bundles, WebAssembly), or in the browser's private file storage.
//!
//! The store enforces the immutability rules itself: the source and lineage
//! files can only be written once, and the event log can only be appended to.
//! Only the manifest and derived caches may be replaced.

use std::collections::BTreeMap;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("`{0}` not found")]
    NotFound(String),
    #[error("`{0}` already exists and is never overwritten")]
    Exists(String),
    #[error("`{0}` is immutable: {1}")]
    Immutable(String, &'static str),
    #[error("invalid path `{0}`")]
    BadPath(String),
    #[error("i/o error on `{0}`: {1}")]
    Io(String, String),
}

/// Paths that may never be replaced or truncated.
pub fn protection(path: &str) -> Option<&'static str> {
    if path.starts_with("source/") {
        Some("the original recording is stored byte-for-byte and never modified")
    } else if path.starts_with("lineage/") {
        Some("ancestor logs are copied once and never modified")
    } else if path == "events.jsonl" {
        Some("the event log is append-only")
    } else {
        None
    }
}

pub fn check_path(path: &str) -> Result<(), StoreError> {
    let bad = path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..");
    if bad {
        Err(StoreError::BadPath(path.to_string()))
    } else {
        Ok(())
    }
}

pub trait Store {
    fn read(&self, path: &str) -> Result<Vec<u8>, StoreError>;
    fn exists(&self, path: &str) -> bool;
    /// Create a file that must not exist yet.
    fn write_new(&mut self, path: &str, data: &[u8]) -> Result<(), StoreError>;
    /// Append to a file (creating it if needed).
    fn append(&mut self, path: &str, data: &[u8]) -> Result<(), StoreError>;
    /// Replace a rewritable file (manifest, caches). Protected paths are refused.
    fn replace(&mut self, path: &str, data: &[u8]) -> Result<(), StoreError>;
    /// Every file path, sorted.
    fn list(&self) -> Vec<String>;
}

/// In-memory store: bundles, tests, WebAssembly.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MemStore {
    pub files: BTreeMap<String, Vec<u8>>,
}

impl MemStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Store for MemStore {
    fn read(&self, path: &str) -> Result<Vec<u8>, StoreError> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| StoreError::NotFound(path.to_string()))
    }

    fn exists(&self, path: &str) -> bool {
        self.files.contains_key(path)
    }

    fn write_new(&mut self, path: &str, data: &[u8]) -> Result<(), StoreError> {
        check_path(path)?;
        if self.files.contains_key(path) {
            return Err(StoreError::Exists(path.to_string()));
        }
        self.files.insert(path.to_string(), data.to_vec());
        Ok(())
    }

    fn append(&mut self, path: &str, data: &[u8]) -> Result<(), StoreError> {
        check_path(path)?;
        if path.starts_with("source/") || path.starts_with("lineage/") {
            return Err(StoreError::Immutable(
                path.to_string(),
                protection(path).unwrap_or(""),
            ));
        }
        self.files
            .entry(path.to_string())
            .or_default()
            .extend_from_slice(data);
        Ok(())
    }

    fn replace(&mut self, path: &str, data: &[u8]) -> Result<(), StoreError> {
        check_path(path)?;
        if let Some(why) = protection(path) {
            return Err(StoreError::Immutable(path.to_string(), why));
        }
        self.files.insert(path.to_string(), data.to_vec());
        Ok(())
    }

    fn list(&self) -> Vec<String> {
        self.files.keys().cloned().collect()
    }
}

/// A project directory on disk (native builds).
#[cfg(not(target_arch = "wasm32"))]
pub struct DirStore {
    pub root: std::path::PathBuf,
}

#[cfg(not(target_arch = "wasm32"))]
impl DirStore {
    pub fn new(root: impl Into<std::path::PathBuf>) -> Self {
        DirStore { root: root.into() }
    }

    fn full(&self, path: &str) -> Result<std::path::PathBuf, StoreError> {
        check_path(path)?;
        Ok(self.root.join(path))
    }

    fn io(path: &str, e: std::io::Error) -> StoreError {
        StoreError::Io(path.to_string(), e.to_string())
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Store for DirStore {
    fn read(&self, path: &str) -> Result<Vec<u8>, StoreError> {
        let p = self.full(path)?;
        std::fs::read(&p).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                StoreError::NotFound(path.to_string())
            } else {
                Self::io(path, e)
            }
        })
    }

    fn exists(&self, path: &str) -> bool {
        self.full(path).map(|p| p.exists()).unwrap_or(false)
    }

    fn write_new(&mut self, path: &str, data: &[u8]) -> Result<(), StoreError> {
        use std::io::Write;
        let p = self.full(path)?;
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Self::io(path, e))?;
        }
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&p)
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    StoreError::Exists(path.to_string())
                } else {
                    Self::io(path, e)
                }
            })?;
        f.write_all(data).map_err(|e| Self::io(path, e))?;
        f.sync_all().map_err(|e| Self::io(path, e))?;
        if protection(path).is_some() && path != "events.jsonl" {
            // Belt and braces: the original and ancestor logs are read-only on disk.
            let mut perm = f.metadata().map_err(|e| Self::io(path, e))?.permissions();
            perm.set_readonly(true);
            std::fs::set_permissions(&p, perm).map_err(|e| Self::io(path, e))?;
        }
        Ok(())
    }

    fn append(&mut self, path: &str, data: &[u8]) -> Result<(), StoreError> {
        use std::io::Write;
        if path.starts_with("source/") || path.starts_with("lineage/") {
            return Err(StoreError::Immutable(
                path.to_string(),
                protection(path).unwrap_or(""),
            ));
        }
        let p = self.full(path)?;
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Self::io(path, e))?;
        }
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&p)
            .map_err(|e| Self::io(path, e))?;
        f.write_all(data).map_err(|e| Self::io(path, e))?;
        f.sync_all().map_err(|e| Self::io(path, e))
    }

    fn replace(&mut self, path: &str, data: &[u8]) -> Result<(), StoreError> {
        if let Some(why) = protection(path) {
            return Err(StoreError::Immutable(path.to_string(), why));
        }
        let p = self.full(path)?;
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Self::io(path, e))?;
        }
        let tmp = p.with_extension("tmp-write");
        std::fs::write(&tmp, data).map_err(|e| Self::io(path, e))?;
        std::fs::rename(&tmp, &p).map_err(|e| Self::io(path, e))
    }

    fn list(&self) -> Vec<String> {
        fn walk(dir: &std::path::Path, root: &std::path::Path, out: &mut Vec<String>) {
            if let Ok(rd) = std::fs::read_dir(dir) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        walk(&p, root, out);
                    } else if let Ok(rel) = p.strip_prefix(root) {
                        let s = rel.to_string_lossy().replace('\\', "/");
                        if !s.ends_with(".tmp-write") {
                            out.push(s);
                        }
                    }
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.root, &self.root, &mut out);
        out.sort();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected_paths_cannot_be_replaced_or_overwritten() {
        let mut s = MemStore::new();
        s.write_new("source/clip.wav", b"abc").unwrap();
        assert!(matches!(
            s.write_new("source/clip.wav", b"x"),
            Err(StoreError::Exists(_))
        ));
        assert!(matches!(
            s.replace("source/clip.wav", b"x"),
            Err(StoreError::Immutable(..))
        ));
        assert!(matches!(
            s.append("source/clip.wav", b"x"),
            Err(StoreError::Immutable(..))
        ));
        s.append("events.jsonl", b"1\n").unwrap();
        s.append("events.jsonl", b"2\n").unwrap();
        assert!(matches!(
            s.replace("events.jsonl", b""),
            Err(StoreError::Immutable(..))
        ));
        s.replace("manifest.json", b"{}").unwrap();
        s.replace("manifest.json", b"{ }").unwrap();
        assert_eq!(s.read("events.jsonl").unwrap(), b"1\n2\n");
        assert!(matches!(
            s.write_new("../escape", b""),
            Err(StoreError::BadPath(_))
        ));
    }

    #[test]
    fn dir_store_enforces_the_same_rules() {
        let dir = std::env::temp_dir().join(format!("nlae-store-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut s = DirStore::new(&dir);
        s.write_new("source/a.wav", b"abc").unwrap();
        assert!(s.write_new("source/a.wav", b"x").is_err());
        assert!(s.replace("source/a.wav", b"x").is_err());
        s.append("events.jsonl", b"e\n").unwrap();
        s.replace("manifest.json", b"{}").unwrap();
        assert_eq!(
            s.list(),
            vec!["events.jsonl", "manifest.json", "source/a.wav"]
        );
        assert_eq!(s.read("source/a.wav").unwrap(), b"abc");
        // Read-only on disk.
        assert!(std::fs::metadata(dir.join("source/a.wav"))
            .unwrap()
            .permissions()
            .readonly());
        let mut perm = std::fs::metadata(dir.join("source/a.wav"))
            .unwrap()
            .permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perm.set_readonly(false);
        std::fs::set_permissions(dir.join("source/a.wav"), perm).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
