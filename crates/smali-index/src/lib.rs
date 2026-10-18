//! Symbol index: classes, members, hierarchy, references; workspace scan, cache and framework import.

pub mod cache;
pub mod jar;
pub mod scan;
pub mod workspace;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use smali_syntax::Span;

pub mod acc {
    //! Dex access flags (same bit values as class files for the shared ones).
    pub const PUBLIC: u32 = 0x1;
    pub const PRIVATE: u32 = 0x2;
    pub const PROTECTED: u32 = 0x4;
    pub const STATIC: u32 = 0x8;
    pub const FINAL: u32 = 0x10;
    pub const SYNCHRONIZED: u32 = 0x20;
    pub const VOLATILE: u32 = 0x40;
    pub const BRIDGE: u32 = 0x40;
    pub const TRANSIENT: u32 = 0x80;
    pub const VARARGS: u32 = 0x80;
    pub const NATIVE: u32 = 0x100;
    pub const INTERFACE: u32 = 0x200;
    pub const ABSTRACT: u32 = 0x400;
    pub const STRICT: u32 = 0x800;
    pub const SYNTHETIC: u32 = 0x1000;
    pub const ANNOTATION: u32 = 0x2000;
    pub const ENUM: u32 = 0x4000;
    pub const CONSTRUCTOR: u32 = 0x10000;
    pub const DECLARED_SYNCHRONIZED: u32 = 0x20000;

    pub fn from_word(w: &str) -> u32 {
        match w {
            "public" => PUBLIC,
            "private" => PRIVATE,
            "protected" => PROTECTED,
            "static" => STATIC,
            "final" => FINAL,
            "synchronized" => SYNCHRONIZED,
            "volatile" | "bridge" => VOLATILE,
            "transient" | "varargs" => TRANSIENT,
            "native" => NATIVE,
            "interface" => INTERFACE,
            "abstract" => ABSTRACT,
            "strictfp" => STRICT,
            "synthetic" => SYNTHETIC,
            "annotation" => ANNOTATION,
            "enum" => ENUM,
            "constructor" => CONSTRUCTOR,
            "declared-synchronized" => DECLARED_SYNCHRONIZED,
            _ => 0,
        }
    }

    pub fn from_words<'a>(ws: impl IntoIterator<Item = &'a String>) -> u32 {
        ws.into_iter().fold(0, |a, w| a | from_word(w))
    }

    /// Java-style modifier list for hover. `method` picks bridge/varargs over volatile/transient.
    pub fn java_modifiers(flags: u32, method: bool) -> String {
        let mut out = Vec::new();
        for (bit, word) in [
            (PUBLIC, "public"),
            (PRIVATE, "private"),
            (PROTECTED, "protected"),
            (STATIC, "static"),
            (FINAL, "final"),
            (ABSTRACT, "abstract"),
            (NATIVE, "native"),
            (SYNCHRONIZED, "synchronized"),
        ] {
            if flags & bit != 0 {
                out.push(word);
            }
        }
        if !method && flags & VOLATILE != 0 {
            out.push("volatile");
        }
        if !method && flags & TRANSIENT != 0 {
            out.push("transient");
        }
        out.join(" ")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Origin {
    Workspace,
    ExtraRoot,
    AndroidJar,
}

impl Origin {
    pub fn label(self) -> &'static str {
        match self {
            Origin::Workspace => "workspace",
            Origin::ExtraRoot => "extra root",
            Origin::AndroidJar => "android.jar",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    pub path: PathBuf,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberInfo {
    pub name: String,
    /// Method prototype `(..)R` or field type descriptor.
    pub desc: String,
    pub flags: u32,
    pub span: Option<Span>,
}

impl MemberInfo {
    pub fn is_static(&self) -> bool {
        self.flags & acc::STATIC != 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassInfo {
    /// Descriptor, e.g. `Lcom/foo/Bar;`.
    pub name: String,
    pub super_class: Option<String>,
    pub interfaces: Vec<String>,
    pub flags: u32,
    pub fields: Vec<MemberInfo>,
    pub methods: Vec<MemberInfo>,
    pub origin: Origin,
    pub path: Option<PathBuf>,
    pub span: Option<Span>,
}

impl ClassInfo {
    pub fn is_interface(&self) -> bool {
        self.flags & acc::INTERFACE != 0
    }

    pub fn location_of(&self, span: Option<Span>) -> Option<Location> {
        Some(Location {
            path: self.path.clone()?,
            span: span.or(self.span)?,
        })
    }
}

/// What a reference in a file points at.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RefKey {
    Class(String),
    /// Owner descriptor, name, prototype.
    Method(String, String, String),
    /// Owner descriptor, name, type.
    Field(String, String, String),
}

impl RefKey {
    /// Single-string form used for storage: `Lc;`, `Lc;->m(I)V` or `Lc;->f:I`.
    pub fn canonical(&self) -> String {
        match self {
            RefKey::Class(c) => c.clone(),
            RefKey::Method(o, n, p) => format!("{o}->{n}{p}"),
            RefKey::Field(o, n, t) => format!("{o}->{n}:{t}"),
        }
    }
}

/// Per-file index entry; this is what the cache stores. Ref keys are canonical strings
/// (see `RefKey::canonical`), interned across files once inserted into an `Index`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: PathBuf,
    pub hash: u64,
    pub class: Option<Arc<ClassInfo>>,
    pub refs: Vec<(Arc<str>, Span)>,
}

/// Result of a member lookup.
#[derive(Debug, Clone, PartialEq)]
pub enum Lookup {
    Found {
        owner: Arc<ClassInfo>,
        member: MemberInfo,
    },
    /// Not found, and the whole walked hierarchy is known.
    Missing,
    /// Not found, and some class in the hierarchy is not indexed.
    Unknown,
}

#[derive(Default, Clone)]
pub struct Index {
    files: HashMap<PathBuf, Arc<FileEntry>>,
    /// Class descriptor → files declaring it, in insertion order.
    workspace: HashMap<String, Vec<PathBuf>>,
    extra: HashMap<String, Arc<ClassInfo>>,
    framework: HashMap<String, Arc<ClassInfo>>,
    workspace_classes: HashMap<PathBuf, Arc<ClassInfo>>,
    interner: HashSet<Arc<str>>,
}

impl Index {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    pub fn class_count(&self) -> usize {
        self.workspace.len() + self.extra.len() + self.framework.len()
    }

    pub fn file(&self, path: &Path) -> Option<&Arc<FileEntry>> {
        self.files.get(path)
    }

    pub fn files(&self) -> impl Iterator<Item = &Arc<FileEntry>> {
        self.files.values()
    }

    fn intern(&mut self, s: &Arc<str>) -> Arc<str> {
        if let Some(existing) = self.interner.get(s) {
            return existing.clone();
        }
        self.interner.insert(s.clone());
        s.clone()
    }

    /// Insert or replace a workspace file.
    pub fn update_file(&mut self, mut entry: FileEntry) {
        self.remove_file(&entry.path.clone());
        for r in entry.refs.iter_mut() {
            r.0 = self.intern(&r.0);
        }
        entry.refs.shrink_to_fit();
        if let Some(class) = &entry.class {
            self.workspace
                .entry(class.name.clone())
                .or_default()
                .push(entry.path.clone());
            self.workspace_classes
                .insert(entry.path.clone(), class.clone());
        }
        self.files.insert(entry.path.clone(), Arc::new(entry));
    }

    pub fn remove_file(&mut self, path: &Path) {
        let Some(old) = self.files.remove(path) else {
            return;
        };
        self.workspace_classes.remove(path);
        if let Some(class) = &old.class {
            if let Some(paths) = self.workspace.get_mut(&class.name) {
                paths.retain(|p| p != path);
                if paths.is_empty() {
                    self.workspace.remove(&class.name);
                }
            }
        }
    }

    pub fn add_extra(&mut self, classes: impl IntoIterator<Item = ClassInfo>) {
        for c in classes {
            self.extra
                .entry(c.name.clone())
                .or_insert_with(|| Arc::new(c));
        }
    }

    pub fn add_framework(&mut self, classes: impl IntoIterator<Item = ClassInfo>) {
        for c in classes {
            self.framework
                .entry(c.name.clone())
                .or_insert_with(|| Arc::new(c));
        }
    }

    /// Class by descriptor, in source priority order: workspace, extra roots, android.jar.
    pub fn class(&self, desc: &str) -> Option<Arc<ClassInfo>> {
        if let Some(path) = self.workspace.get(desc).and_then(|p| p.first()) {
            if let Some(c) = self.workspace_classes.get(path) {
                return Some(c.clone());
            }
        }
        self.extra
            .get(desc)
            .or_else(|| self.framework.get(desc))
            .cloned()
    }

    /// All class descriptors, deduplicated.
    pub fn class_names(&self) -> impl Iterator<Item = &String> {
        let ws = self.workspace.keys();
        let extra = self
            .extra
            .keys()
            .filter(|k| !self.workspace.contains_key(*k));
        let fw = self
            .framework
            .keys()
            .filter(|k| !self.workspace.contains_key(*k) && !self.extra.contains_key(*k));
        ws.chain(extra).chain(fw)
    }

    /// Superclass chain then all interfaces (breadth-first). The bool is false if any
    /// class encountered is not indexed.
    pub fn hierarchy(&self, desc: &str) -> (Vec<Arc<ClassInfo>>, bool) {
        let mut out = Vec::new();
        let mut complete = true;
        let mut seen = HashSet::new();
        let mut interfaces = VecDeque::new();
        let mut cur = Some(desc.to_string());
        while let Some(name) = cur.take() {
            if !seen.insert(name.clone()) {
                break;
            }
            match self.class(&name) {
                Some(c) => {
                    interfaces.extend(c.interfaces.iter().cloned());
                    cur = c.super_class.clone();
                    out.push(c);
                }
                None => complete = false,
            }
        }
        while let Some(name) = interfaces.pop_front() {
            if !seen.insert(name.clone()) {
                continue;
            }
            match self.class(&name) {
                Some(c) => {
                    interfaces.extend(c.interfaces.iter().cloned());
                    out.push(c);
                }
                None => complete = false,
            }
        }
        (out, complete)
    }

    fn lookup(&self, owner: &str, pick: impl Fn(&ClassInfo) -> Option<&MemberInfo>) -> Lookup {
        // Arrays only have Object's members plus clone(); treat as unknown rather than guess.
        if !owner.starts_with('L') {
            return Lookup::Unknown;
        }
        let (chain, complete) = self.hierarchy(owner);
        // android.jar is a stub without hidden (@hide) members, so it cannot prove a member is
        // missing; extra roots (decompiled framework) are complete.
        let complete = complete && chain.iter().all(|c| c.origin != Origin::AndroidJar);
        for c in chain {
            if let Some(m) = pick(&c) {
                let member = m.clone();
                return Lookup::Found { owner: c, member };
            }
        }
        if complete {
            Lookup::Missing
        } else {
            Lookup::Unknown
        }
    }

    pub fn resolve_method(&self, owner: &str, name: &str, proto: &str) -> Lookup {
        self.lookup(owner, |c| {
            c.methods.iter().find(|m| m.name == name && m.desc == proto)
        })
    }

    pub fn resolve_field(&self, owner: &str, name: &str, ty: &str) -> Lookup {
        self.lookup(owner, |c| {
            c.fields.iter().find(|f| f.name == name && f.desc == ty)
        })
    }

    /// Every location referencing `key` across workspace files.
    pub fn references(&self, key: &RefKey) -> Vec<Location> {
        let mut out = Vec::new();
        let Some(key) = self.interner.get(key.canonical().as_str()) else {
            return out;
        };
        for f in self.files.values() {
            for (k, span) in &f.refs {
                if Arc::ptr_eq(k, key) {
                    out.push(Location {
                        path: f.path.clone(),
                        span: *span,
                    });
                }
            }
        }
        out.sort_by(|a, b| (&a.path, a.span).cmp(&(&b.path, b.span)));
        out
    }
}

#[cfg(test)]
mod tests;
