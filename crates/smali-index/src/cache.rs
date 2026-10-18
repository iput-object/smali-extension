//! On-disk cache of file entries, validated by file size + mtime (then content hash on mismatch).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use smali_syntax::Span;

use crate::{ClassInfo, FileEntry};

/// Bump when `FileEntry` or anything it contains changes shape or meaning.
pub const SCHEMA_VERSION: u32 = 2;
const MAGIC: &[u8; 8] = b"SMLIDX\0\0";

/// Size and modification time, used to skip reading unchanged files on warm start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stamp {
    pub size: u64,
    pub mtime_ns: u128,
}

impl Stamp {
    pub fn of(path: &Path) -> Option<Stamp> {
        let m = std::fs::metadata(path).ok()?;
        let mtime_ns = m
            .modified()
            .ok()?
            .duration_since(UNIX_EPOCH)
            .ok()?
            .as_nanos();
        Some(Stamp {
            size: m.len(),
            mtime_ns,
        })
    }
}

#[derive(Default)]
pub struct Cache {
    entries: HashMap<PathBuf, (Option<Stamp>, FileEntry)>,
}

/// Refs are stored as indexes into one string table so each distinct key is decoded once.
#[derive(Serialize, Deserialize)]
struct StoredEntry {
    path: PathBuf,
    stamp: Option<Stamp>,
    hash: u64,
    class: Option<ClassInfo>,
    refs: Vec<(u32, Span)>,
}

#[derive(Serialize, Deserialize)]
struct Envelope {
    version: u32,
    crate_version: String,
    strings: Vec<String>,
    entries: Vec<StoredEntry>,
}

impl Cache {
    /// Entry for `path` if the file is unchanged since it was cached.
    pub fn get_by_stamp(&self, path: &Path, stamp: Stamp) -> Option<FileEntry> {
        self.entries
            .get(path)
            .filter(|(s, _)| *s == Some(stamp))
            .map(|(_, e)| e.clone())
    }

    pub fn get(&self, path: &Path, hash: u64) -> Option<FileEntry> {
        self.entries
            .get(path)
            .filter(|(_, e)| e.hash == hash)
            .map(|(_, e)| e.clone())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Snapshot entries, stamping each with the current on-disk size/mtime.
    pub fn from_entries<'a>(entries: impl IntoIterator<Item = &'a FileEntry>) -> Self {
        Cache {
            entries: entries
                .into_iter()
                .map(|e| (e.path.clone(), (Stamp::of(&e.path), e.clone())))
                .collect(),
        }
    }

    /// Load a cache file. Any problem (missing, corrupt, other version) yields an empty cache.
    pub fn load(file: &Path) -> Cache {
        let Ok(bytes) = std::fs::read(file) else {
            return Cache::default();
        };
        Self::decode(&bytes).unwrap_or_default()
    }

    fn decode(bytes: &[u8]) -> Option<Cache> {
        let body = bytes.strip_prefix(MAGIC.as_slice())?;
        let env: Envelope = postcard::from_bytes(body).ok()?;
        if env.version != SCHEMA_VERSION || env.crate_version != env!("CARGO_PKG_VERSION") {
            return None;
        }
        let strings: Vec<Arc<str>> = env.strings.into_iter().map(Arc::from).collect();
        let mut entries = HashMap::with_capacity(env.entries.len());
        for e in env.entries {
            let refs = e
                .refs
                .into_iter()
                .map(|(i, s)| Some((strings.get(i as usize)?.clone(), s)))
                .collect::<Option<Vec<_>>>()?;
            let entry = FileEntry {
                path: e.path.clone(),
                hash: e.hash,
                class: e.class.map(Arc::new),
                refs,
            };
            entries.insert(e.path, (e.stamp, entry));
        }
        Some(Cache { entries })
    }

    pub fn encode(self) -> Option<Vec<u8>> {
        let mut ids: HashMap<Arc<str>, u32> = HashMap::new();
        let mut strings = Vec::new();
        let mut stored = Vec::with_capacity(self.entries.len());
        for (path, (stamp, e)) in self.entries {
            let refs = e
                .refs
                .iter()
                .map(|(k, s)| {
                    let id = *ids.entry(k.clone()).or_insert_with(|| {
                        strings.push(k.to_string());
                        (strings.len() - 1) as u32
                    });
                    (id, *s)
                })
                .collect();
            stored.push(StoredEntry {
                path,
                stamp,
                hash: e.hash,
                class: e.class.map(Arc::unwrap_or_clone),
                refs,
            });
        }
        let env = Envelope {
            version: SCHEMA_VERSION,
            crate_version: env!("CARGO_PKG_VERSION").to_string(),
            strings,
            entries: stored,
        };
        let mut out = MAGIC.to_vec();
        out.extend(postcard::to_allocvec(&env).ok()?);
        Some(out)
    }

    /// Write atomically (temp file + rename) so a crash never leaves a half-written cache.
    pub fn save(self, file: &Path) -> std::io::Result<()> {
        let bytes = self
            .encode()
            .ok_or_else(|| std::io::Error::other("cache encode failed"))?;
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = file.with_extension("tmp");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(tmp, file)
    }
}

/// Cache file for a workspace root inside `cache_dir`.
pub fn cache_file(cache_dir: &Path, root: &Path) -> PathBuf {
    let key = crate::scan::hash(root.to_string_lossy().as_bytes());
    cache_dir.join(format!("index-{key:016x}.bin"))
}

/// Default cache directory: `$XDG_CACHE_HOME`/`~/.cache`, `~/Library/Caches` or `%LOCALAPPDATA%`, plus `smali-lsp`.
pub fn default_cache_dir() -> Option<PathBuf> {
    let env = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let base = if cfg!(target_os = "windows") {
        env("LOCALAPPDATA")
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library/Caches"))
    } else {
        env("XDG_CACHE_HOME").or_else(|| env("HOME").map(|h| h.join(".cache")))
    }?;
    Some(base.join("smali-lsp"))
}
