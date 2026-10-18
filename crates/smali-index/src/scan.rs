//! Build index entries from parsed files.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use smali_syntax::descriptor::{parse_field_ref, parse_method_ref};
use smali_syntax::model::OperandValue;
use smali_syntax::{ClassFile, Document, Span};

use crate::{acc, ClassInfo, FileEntry, MemberInfo, Origin, RefKey};

/// FNV-1a; stable across builds so it can key the on-disk cache.
pub fn hash(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

pub fn class_info(model: &ClassFile, origin: Origin, path: Option<&Path>) -> Option<ClassInfo> {
    let header = model.class.as_ref()?;
    Some(ClassInfo {
        name: header.name.text.clone(),
        super_class: model.super_class.as_ref().map(|s| s.text.clone()),
        interfaces: model.interfaces.iter().map(|i| i.text.clone()).collect(),
        flags: acc::from_words(&header.flags),
        fields: model
            .fields
            .iter()
            .map(|f| MemberInfo {
                name: f.name.text.clone(),
                desc: f.ty.text.clone(),
                flags: acc::from_words(&f.flags),
                span: Some(f.name.span),
            })
            .collect(),
        methods: model
            .methods
            .iter()
            .map(|m| MemberInfo {
                name: m.name.text.clone(),
                desc: m.proto.text.clone(),
                flags: acc::from_words(&m.flags),
                span: Some(m.name.span),
            })
            .collect(),
        origin,
        path: path.map(Path::to_path_buf),
        span: Some(header.name.span),
    })
}

pub fn refs(model: &ClassFile) -> Vec<(Arc<str>, smali_syntax::Span)> {
    // Repeated keys within a file share one allocation.
    let mut seen: HashMap<String, Arc<str>> = HashMap::new();
    let mut key = |k: RefKey| -> Arc<str> {
        let c = k.canonical();
        seen.entry(c.clone())
            .or_insert_with(|| Arc::from(c))
            .clone()
    };
    let mut out: Vec<(Arc<str>, smali_syntax::Span)> = model
        .type_refs
        .iter()
        .map(|t| (key(RefKey::Class(t.text.clone())), t.span))
        .collect();
    for m in &model.methods {
        for insn in m.instructions() {
            for op in &insn.operands {
                match &op.value {
                    OperandValue::MethodRef(t) => {
                        if let Ok(r) = parse_method_ref(t) {
                            out.push((
                                key(RefKey::Method(
                                    r.class.to_string(),
                                    r.name,
                                    r.proto.to_string(),
                                )),
                                op.span,
                            ));
                        }
                    }
                    OperandValue::FieldRef(t) => {
                        if let Ok(r) = parse_field_ref(t) {
                            out.push((
                                key(RefKey::Field(r.class.to_string(), r.name, r.ty.to_string())),
                                op.span,
                            ));
                        }
                    }
                    OperandValue::Type(t) if t.contains('L') => {
                        // Class types are already in type_refs via class_identifier nodes.
                    }
                    _ => {}
                }
            }
        }
    }
    out
}

pub fn entry_for(path: &Path, text: &str, origin: Origin) -> FileEntry {
    let doc = Document::parse(text);
    entry_from_doc(path, &doc, origin)
}

pub fn entry_from_doc(path: &Path, doc: &Document, origin: Origin) -> FileEntry {
    FileEntry {
        path: path.to_path_buf(),
        hash: hash(doc.text.as_bytes()),
        class: class_info(&doc.model, origin, Some(path)).map(Arc::new),
        refs: refs(&doc.model),
    }
}

/// Byte ranges of the line outside string/char literals and comments.
fn code_segments(line: &str) -> Vec<(usize, usize)> {
    let b = line.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'#' => {
                out.push((start, i));
                return out;
            }
            q @ (b'"' | b'\'') => {
                out.push((start, i));
                i += 1;
                while i < b.len() && b[i] != q {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
                start = (i + 1).min(b.len());
            }
            _ => {}
        }
        i += 1;
    }
    out.push((start, b.len()));
    out
}

fn is_sep(b: u8) -> bool {
    matches!(
        b,
        b' ' | b'\t' | b',' | b'{' | b'}' | b'(' | b')' | b'=' | b'@' | b'\r'
    )
}

/// Line-based scanner producing the same `FileEntry` as the tree-sitter path, ~20x faster.
/// Used for indexing; full analysis still parses with tree-sitter.
pub fn fast_entry(path: &Path, text: &str, origin: Origin) -> FileEntry {
    let mut seen: HashMap<&str, Arc<str>> = HashMap::new();
    let mut refs: Vec<(Arc<str>, Span)> = Vec::new();
    let mut class: Option<ClassInfo> = None;
    let mut super_class = None;
    let mut interfaces = Vec::new();
    let mut fields = Vec::new();
    let mut methods = Vec::new();
    let mut line_start = 0usize;
    for raw in text.split_inclusive('\n') {
        let base = line_start;
        line_start += raw.len();
        let line = raw.trim_end_matches(['\n', '\r']);
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = line.len() - trimmed.len();
        let segs = code_segments(line);
        let code: String = {
            // Literals and comments blanked out so offsets stay aligned.
            let mut c = vec![b' '; line.len()];
            for (s, e) in &segs {
                c[*s..*e].copy_from_slice(&line.as_bytes()[*s..*e]);
            }
            String::from_utf8(c).unwrap_or_default()
        };
        let cb = code.as_bytes();
        let directive = trimmed.split_whitespace().next().unwrap_or("");
        let words: Vec<(usize, &str)> = {
            let mut v = Vec::new();
            let mut i = indent;
            for w in code[indent..].split(' ') {
                if !w.is_empty() {
                    v.push((i, w));
                }
                i += w.len() + 1;
            }
            v
        };
        match directive {
            ".class" if class.is_none() => {
                if let Some((off, name)) = words.last().filter(|w| w.1.starts_with('L')) {
                    let flags = words[1..words.len() - 1]
                        .iter()
                        .fold(0, |a, (_, w)| a | acc::from_word(w));
                    class = Some(ClassInfo {
                        name: name.to_string(),
                        super_class: None,
                        interfaces: Vec::new(),
                        flags,
                        fields: Vec::new(),
                        methods: Vec::new(),
                        origin,
                        path: Some(path.to_path_buf()),
                        span: Some(Span::new(base + off, base + off + name.len())),
                    });
                }
            }
            ".super" => super_class = words.get(1).map(|w| w.1.to_string()),
            ".implements" => interfaces.extend(words.get(1).map(|w| w.1.to_string())),
            ".field" => {
                if let Some((off, w)) = words.iter().skip(1).find(|w| w.1.contains(':')) {
                    let colon = w.find(':').unwrap_or(0);
                    let flags = words
                        .iter()
                        .skip(1)
                        .take_while(|x| x.0 < *off)
                        .fold(0, |a, (_, x)| a | acc::from_word(x));
                    fields.push(MemberInfo {
                        name: w[..colon].to_string(),
                        desc: w[colon + 1..].to_string(),
                        flags,
                        span: Some(Span::new(base + off, base + off + colon)),
                    });
                }
            }
            ".method" => {
                if let Some(p) = code.find('(') {
                    let name_start = code[..p].rfind(' ').map_or(indent, |i| i + 1);
                    let sig_end = code[p..].find(' ').map_or(code.len(), |i| p + i);
                    let flags = code[indent..name_start]
                        .split(' ')
                        .fold(0, |a, w| a | acc::from_word(w));
                    methods.push(MemberInfo {
                        name: code[name_start..p].to_string(),
                        desc: code[p..sig_end].to_string(),
                        flags,
                        span: Some(Span::new(base + name_start, base + p)),
                    });
                }
            }
            _ => {}
        }
        // References: class descriptors anywhere in code, and `owner->member` operands.
        let mut i = indent;
        let mut in_proto = false;
        while i < cb.len() {
            let c = cb[i];
            match c {
                b'(' => in_proto = true,
                b')' | b' ' => in_proto = false,
                _ => {}
            }
            let prev = if i == 0 { b' ' } else { cb[i - 1] };
            let boundary = is_sep(prev)
                || matches!(prev, b'[' | b':' | b')' | b'(' | b'>')
                || (in_proto
                    && matches!(
                        prev,
                        b'Z' | b'B' | b'S' | b'C' | b'I' | b'J' | b'F' | b'D' | b';'
                    ));
            if c == b'L' && boundary {
                let len = cb[i..]
                    .iter()
                    .position(|b| {
                        matches!(
                            b,
                            b';' | b':'
                                | b'('
                                | b')'
                                | b' '
                                | b','
                                | b'{'
                                | b'}'
                                | b'<'
                                | b'>'
                                | b'='
                        )
                    })
                    .unwrap_or(cb.len() - i);
                if cb.get(i + len) == Some(&b';') && len > 1 {
                    let (s0, e0) = (base + i, base + i + len + 1);
                    let d = text.get(s0..e0).unwrap_or("");
                    let k = seen.entry(d).or_insert_with(|| Arc::from(d)).clone();
                    refs.push((k, Span::new(s0, e0)));
                    i += len + 1;
                    continue;
                }
            }
            i += 1;
        }
        if directive.starts_with('.') && !directive.starts_with(".annotation") {
            continue;
        }
        let mut search = indent;
        while let Some(rel) = code[search..].find("->") {
            let arrow = search + rel;
            search = arrow + 2;
            let owner_start = code[..arrow]
                .rfind([' ', ',', '{', '@'])
                .map_or(indent, |x| x + 1);
            let end = code[arrow + 2..]
                .find([' ', ',', '}', '\r'])
                .map_or(code.len(), |x| arrow + 2 + x);
            let owner = &code[owner_start..arrow];
            let member = &code[arrow + 2..end];
            if !(owner.starts_with('[') || owner.starts_with('L') && owner.ends_with(';')) {
                continue;
            }
            let full = &code[owner_start..end];
            let key = if member.contains('(') {
                parse_method_ref(full)
                    .ok()
                    .map(|r| RefKey::Method(r.class.to_string(), r.name, r.proto.to_string()))
            } else {
                parse_field_ref(full)
                    .ok()
                    .map(|r| RefKey::Field(r.class.to_string(), r.name, r.ty.to_string()))
            };
            if let Some(k) = key {
                refs.push((
                    Arc::from(k.canonical()),
                    Span::new(base + owner_start, base + end),
                ));
            }
        }
    }
    let class = class.map(|mut c| {
        c.super_class = super_class;
        c.interfaces = interfaces;
        c.fields = fields;
        c.methods = methods;
        Arc::new(c)
    });
    refs.sort_by_key(|r| r.1);
    FileEntry {
        path: path.to_path_buf(),
        hash: hash(text.as_bytes()),
        class,
        refs,
    }
}
