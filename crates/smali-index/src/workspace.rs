//! Finding smali files and roots on disk, and building index entries in parallel.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rayon::prelude::*;

use crate::cache::Cache;
use crate::scan::{fast_entry, hash};
use crate::{ClassInfo, FileEntry, Origin};

/// `smali` or `smali_classesN` (apktool layout).
pub fn is_root_name(name: &str) -> bool {
    name == "smali"
        || name
            .strip_prefix("smali_classes")
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// The smali root containing `path` (nearest ancestor named like a root), if any.
pub fn root_of(path: &Path, extra_roots: &[PathBuf]) -> Option<PathBuf> {
    for anc in path.ancestors().skip(1) {
        if extra_roots.iter().any(|r| r == anc) {
            return Some(anc.to_path_buf());
        }
        if anc
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(is_root_name)
        {
            return Some(anc.to_path_buf());
        }
    }
    None
}

/// Class descriptor implied by a file's path under its root.
pub fn expected_class(path: &Path, extra_roots: &[PathBuf]) -> Option<String> {
    let root = root_of(path, extra_roots)?;
    let rel = path.strip_prefix(&root).ok()?.with_extension("");
    let parts: Option<Vec<&str>> = rel.components().map(|c| c.as_os_str().to_str()).collect();
    Some(format!("L{};", parts?.join("/")))
}

pub fn find_smali_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                let hidden = p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with('.') || n == "target" || n == "node_modules");
                if !hidden {
                    stack.push(p);
                }
            } else if p.extension().is_some_and(|x| x == "smali") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// Index every smali file under `dirs`, reusing cached entries whose hash matches.
pub fn index_files(paths: &[PathBuf], origin: Origin, cache: Option<&Cache>) -> Vec<FileEntry> {
    paths
        .par_iter()
        .filter_map(|p| {
            if let (Some(c), Some(stamp)) = (cache, crate::cache::Stamp::of(p)) {
                if let Some(hit) = c.get_by_stamp(p, stamp) {
                    return Some(hit);
                }
            }
            let bytes = std::fs::read(p).ok()?;
            let h = hash(&bytes);
            if let Some(hit) = cache.and_then(|c| c.get(p, h)) {
                return Some(hit);
            }
            let text = String::from_utf8_lossy(&bytes);
            let mut e = fast_entry(p, &text, origin);
            e.hash = h;
            Some(e)
        })
        .collect()
}

/// Index files into `index` in parallel chunks, interning as each chunk lands so peak memory
/// stays near the final index size.
pub fn index_into(
    index: &mut crate::Index,
    paths: &[PathBuf],
    origin: Origin,
    cache: Option<&Cache>,
) {
    for chunk in paths.chunks(4096) {
        for e in index_files(chunk, origin, cache) {
            index.update_file(e);
        }
    }
}

/// Classes from extra smali roots (reference only, no refs kept).
pub fn index_extra_root(dir: &Path, cache: Option<&Cache>) -> Vec<ClassInfo> {
    let files = find_smali_files(dir);
    index_files(&files, Origin::ExtraRoot, cache)
        .into_iter()
        .filter_map(|e| e.class.map(Arc::unwrap_or_clone))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roots_and_expected_class() {
        let p = Path::new("/w/app/smali_classes2/com/foo/Bar$1.smali");
        assert_eq!(
            root_of(p, &[]),
            Some(PathBuf::from("/w/app/smali_classes2"))
        );
        assert_eq!(expected_class(p, &[]), Some("Lcom/foo/Bar$1;".into()));
        assert_eq!(expected_class(Path::new("/w/x/A.smali"), &[]), None);
        let extra = vec![PathBuf::from("/fw")];
        assert_eq!(
            expected_class(Path::new("/fw/android/os/Foo.smali"), &extra),
            Some("Landroid/os/Foo;".into())
        );
        assert!(!is_root_name("smali_classes"));
        assert!(!is_root_name("smali_classesX"));
    }
}
