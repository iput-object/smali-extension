//! Go to definition, references, document/workspace symbols, folding.

use std::path::PathBuf;

use smali_index::{Index, Location, Lookup, RefKey};
use smali_syntax::descriptor::{parse_field_ref, parse_method_ref};
use smali_syntax::model::Item;
use smali_syntax::{Document, Span};

use crate::hover::{target_at, Target};
use crate::{method_at, Context};

/// A location; `path: None` means the current document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loc {
    pub path: Option<PathBuf>,
    pub span: Span,
}

impl From<Location> for Loc {
    fn from(l: Location) -> Self {
        Loc {
            path: Some(l.path),
            span: l.span,
        }
    }
}

pub fn definition(doc: &Document, ctx: &Context, offset: usize) -> Vec<Loc> {
    let here = |span| Loc { path: None, span };
    match target_at(doc, offset) {
        Some(Target::LabelRef(name, _)) => method_at(doc, offset)
            .into_iter()
            .flat_map(|m| {
                m.label_defs()
                    .filter(|l| l.text == name)
                    .map(|l| here(l.span))
                    .collect::<Vec<_>>()
            })
            .collect(),
        Some(Target::MethodRef(t, _)) => {
            let (Some(index), Ok(r)) = (ctx.index, parse_method_ref(&t)) else {
                return Vec::new();
            };
            match index.resolve_method(&r.class.to_string(), &r.name, &r.proto.to_string()) {
                Lookup::Found { owner, member } => owner
                    .location_of(member.span)
                    .map(Loc::from)
                    .into_iter()
                    .collect(),
                _ => Vec::new(),
            }
        }
        Some(Target::FieldRef(t, _)) => {
            let (Some(index), Ok(r)) = (ctx.index, parse_field_ref(&t)) else {
                return Vec::new();
            };
            match index.resolve_field(&r.class.to_string(), &r.name, &r.ty.to_string()) {
                Lookup::Found { owner, member } => owner
                    .location_of(member.span)
                    .map(Loc::from)
                    .into_iter()
                    .collect(),
                _ => Vec::new(),
            }
        }
        Some(Target::Class(desc, _)) => {
            if doc
                .model
                .class
                .as_ref()
                .is_some_and(|c| c.name.text == desc)
            {
                return doc.model.class.iter().map(|c| here(c.name.span)).collect();
            }
            ctx.index
                .and_then(|i| i.class(&desc))
                .and_then(|c| c.location_of(None))
                .map(Loc::from)
                .into_iter()
                .collect()
        }
        _ => Vec::new(),
    }
}

/// The index key for the symbol under the cursor, for cross-file references.
pub fn ref_key_at(doc: &Document, offset: usize) -> Option<RefKey> {
    let owner = || doc.model.class.as_ref().map(|c| c.name.text.clone());
    match target_at(doc, offset)? {
        Target::MethodRef(t, _) => {
            let r = parse_method_ref(&t).ok()?;
            Some(RefKey::Method(
                r.class.to_string(),
                r.name,
                r.proto.to_string(),
            ))
        }
        Target::FieldRef(t, _) => {
            let r = parse_field_ref(&t).ok()?;
            Some(RefKey::Field(r.class.to_string(), r.name, r.ty.to_string()))
        }
        Target::Class(d, _) => Some(RefKey::Class(d)),
        Target::MethodDef(i) => {
            let m = doc.model.methods.get(i)?;
            Some(RefKey::Method(
                owner()?,
                m.name.text.clone(),
                m.proto.text.clone(),
            ))
        }
        Target::FieldDef(i) => {
            let f = doc.model.fields.get(i)?;
            Some(RefKey::Field(
                owner()?,
                f.name.text.clone(),
                f.ty.text.clone(),
            ))
        }
        _ => None,
    }
}

/// References to the symbol under the cursor. Labels are method-local; everything else
/// comes from the index. Member references are matched on the exact owner written at the call site.
pub fn references(doc: &Document, ctx: &Context, offset: usize, include_decl: bool) -> Vec<Loc> {
    let here = |span| Loc { path: None, span };
    if let Some(Target::LabelRef(name, _) | Target::LabelDef(name, _)) = target_at(doc, offset) {
        let Some(m) = method_at(doc, offset) else {
            return Vec::new();
        };
        let mut out: Vec<Loc> = m
            .label_refs()
            .into_iter()
            .filter(|l| l.text == name)
            .map(|l| here(l.span))
            .collect();
        if include_decl {
            out.extend(
                m.label_defs()
                    .filter(|l| l.text == name)
                    .map(|l| here(l.span)),
            );
        }
        return out;
    }
    let (Some(index), Some(key)) = (ctx.index, ref_key_at(doc, offset)) else {
        return Vec::new();
    };
    let mut out: Vec<Loc> = index.references(&key).into_iter().map(Loc::from).collect();
    if include_decl {
        out.extend(declaration_of(index, &key));
    }
    out
}

fn declaration_of(index: &Index, key: &RefKey) -> Option<Loc> {
    match key {
        RefKey::Class(d) => index.class(d)?.location_of(None).map(Loc::from),
        RefKey::Method(o, n, p) => match index.resolve_method(o, n, p) {
            Lookup::Found { owner, member } => owner.location_of(member.span).map(Loc::from),
            _ => None,
        },
        RefKey::Field(o, n, t) => match index.resolve_field(o, n, t) {
            Lookup::Found { owner, member } => owner.location_of(member.span).map(Loc::from),
            _ => None,
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Class,
    Interface,
    Method,
    Constructor,
    Field,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub detail: String,
    pub kind: SymbolKind,
    pub span: Span,
    pub selection: Span,
    pub children: Vec<Symbol>,
}

pub fn document_symbols(doc: &Document) -> Vec<Symbol> {
    let model = &doc.model;
    let mut children = Vec::new();
    for f in &model.fields {
        children.push(Symbol {
            name: f.name.text.clone(),
            detail: f.ty.text.clone(),
            kind: SymbolKind::Field,
            span: f.span,
            selection: f.name.span,
            children: Vec::new(),
        });
    }
    for m in &model.methods {
        children.push(Symbol {
            name: format!("{}{}", m.name.text, m.proto.text),
            detail: m.flags.join(" "),
            kind: if m.name.text == "<init>" {
                SymbolKind::Constructor
            } else {
                SymbolKind::Method
            },
            span: m.span,
            selection: m.name.span,
            children: Vec::new(),
        });
    }
    let Some(c) = &model.class else {
        return children;
    };
    let interface = c.flags.iter().any(|f| f == "interface");
    vec![Symbol {
        name: c.name.text.clone(),
        detail: model
            .super_class
            .as_ref()
            .map(|s| format!("extends {}", s.text))
            .unwrap_or_default(),
        kind: if interface {
            SymbolKind::Interface
        } else {
            SymbolKind::Class
        },
        span: Span::new(c.span.start, doc.text.len()),
        selection: c.name.span,
        children,
    }]
}

pub struct WorkspaceSymbol {
    pub name: String,
    pub container: String,
    pub kind: SymbolKind,
    pub location: Location,
}

/// Case-insensitive subsequence match on class simple names and member names.
pub fn workspace_symbols(index: &Index, query: &str, limit: usize) -> Vec<WorkspaceSymbol> {
    let q = query.to_lowercase();
    let matches = |s: &str| fuzzy(&s.to_lowercase(), &q);
    let mut out = Vec::new();
    for f in index.files() {
        let Some(c) = &f.class else { continue };
        let simple = c
            .name
            .trim_end_matches(';')
            .rsplit('/')
            .next()
            .unwrap_or(&c.name);
        if matches(simple) {
            if let Some(loc) = c.location_of(None) {
                out.push(WorkspaceSymbol {
                    name: c.name.clone(),
                    container: String::new(),
                    kind: if c.is_interface() {
                        SymbolKind::Interface
                    } else {
                        SymbolKind::Class
                    },
                    location: loc,
                });
            }
        }
        for (members, kind) in [
            (&c.methods, SymbolKind::Method),
            (&c.fields, SymbolKind::Field),
        ] {
            for m in members.iter().filter(|m| matches(&m.name)) {
                if let Some(loc) = c.location_of(m.span) {
                    out.push(WorkspaceSymbol {
                        name: m.name.clone(),
                        container: c.name.clone(),
                        kind,
                        location: loc,
                    });
                }
            }
        }
        if out.len() >= limit {
            break;
        }
    }
    out.truncate(limit);
    out
}

pub fn fuzzy(hay: &str, needle: &str) -> bool {
    let mut it = hay.chars();
    needle.chars().all(|c| it.any(|h| h == c))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fold {
    pub span: Span,
    pub comment: bool,
}

pub fn folding_ranges(doc: &Document) -> Vec<Fold> {
    let model = &doc.model;
    let mut out = Vec::new();
    for m in &model.methods {
        out.push(Fold {
            span: m.span,
            comment: false,
        });
        for it in &m.items {
            match it {
                Item::PackedSwitch(d) | Item::SparseSwitch(d) | Item::ArrayData(d) => {
                    out.push(Fold {
                        span: d.span,
                        comment: false,
                    })
                }
                Item::Directive { kind, span, .. } if kind == "annotation_directive" => {
                    out.push(Fold {
                        span: *span,
                        comment: false,
                    })
                }
                _ => {}
            }
        }
    }
    for a in &model.annotations {
        out.push(Fold {
            span: a.span,
            comment: false,
        });
    }
    for f in &model.fields {
        if doc.span_text(f.span).contains('\n') {
            out.push(Fold {
                span: f.span,
                comment: false,
            });
        }
    }
    out.retain(|f| doc.span_text(f.span).trim_end().contains('\n'));
    out
}
