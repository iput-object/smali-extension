//! Rename (methods, fields, classes) and call hierarchy over the index.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use smali_index::{acc, ClassInfo, Index, Lookup, Origin, RefKey};
use smali_syntax::descriptor::{parse_field_ref, parse_method_ref, parse_type};
use smali_syntax::{Document, Span};

use crate::hover::{target_at, Target};
use crate::nav::{ref_key_at, Loc};
use crate::{Context, Edit};

/// One edit in some file (`None` = the current document).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEdit {
    pub path: Option<PathBuf>,
    pub edit: Edit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenameError {
    NothingToRename,
    Framework(String),
    Special,
    InvalidName,
}

impl std::fmt::Display for RenameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenameError::NothingToRename => write!(f, "nothing to rename here"),
            RenameError::Framework(c) => write!(
                f,
                "{c} is not in the workspace; framework members are never renamed"
            ),
            RenameError::Special => {
                write!(f, "constructors and static initializers cannot be renamed")
            }
            RenameError::InvalidName => write!(f, "not a valid member name"),
        }
    }
}

fn valid_member_name(n: &str) -> bool {
    !n.is_empty()
        && !n.contains(|c: char| {
            c.is_whitespace()
                || matches!(c, ';' | '/' | '[' | '(' | ')' | '.' | '<' | '>' | ':' | ',')
        })
}

/// Member being renamed: (is_method, name, descriptor) and its declaring classes.
struct Family {
    method: bool,
    name: String,
    desc: String,
    owners: Vec<Arc<ClassInfo>>,
}

fn family(index: &Index, key: &RefKey) -> Result<Family, RenameError> {
    let (method, owner, name, desc) = match key {
        RefKey::Method(o, n, p) => (true, o, n, p),
        RefKey::Field(o, n, t) => (false, o, n, t),
        RefKey::Class(_) => return Err(RenameError::NothingToRename),
    };
    if name == "<init>" || name == "<clinit>" {
        return Err(RenameError::Special);
    }
    let found = if method {
        index.resolve_method(owner, name, desc)
    } else {
        index.resolve_field(owner, name, desc)
    };
    let Lookup::Found {
        owner: decl,
        member,
    } = found
    else {
        return Err(RenameError::NothingToRename);
    };
    let mut owners = vec![decl.clone()];
    // Virtual methods: every related declaration (overrides up and down the hierarchy) is renamed
    // together, and any framework declaration in the chain blocks the rename.
    let virtual_method = method && member.flags & (acc::PRIVATE | acc::STATIC) == 0;
    if virtual_method {
        let (ancestors, _) = index.hierarchy(&decl.name);
        for a in ancestors {
            if a.methods.iter().any(|m| &m.name == name && &m.desc == desc) && a.name != decl.name {
                if a.origin != Origin::Workspace {
                    return Err(RenameError::Framework(a.name.clone()));
                }
                owners.push(a);
            }
        }
        let roots: HashSet<String> = owners.iter().map(|o| o.name.clone()).collect();
        for f in index.files() {
            let Some(c) = &f.class else { continue };
            if roots.contains(&c.name)
                || !c.methods.iter().any(|m| &m.name == name && &m.desc == desc)
            {
                continue;
            }
            let (chain, _) = index.hierarchy(&c.name);
            if chain.iter().any(|a| roots.contains(&a.name)) {
                owners.push(c.clone());
            }
        }
    }
    if let Some(fw) = owners.iter().find(|o| o.origin != Origin::Workspace) {
        return Err(RenameError::Framework(fw.name.clone()));
    }
    Ok(Family {
        method,
        name: name.clone(),
        desc: desc.clone(),
        owners,
    })
}

/// Edits renaming the member under the cursor everywhere in the workspace.
pub fn rename_member(
    doc: &Document,
    ctx: &Context,
    offset: usize,
    new_name: &str,
) -> Result<Vec<FileEdit>, RenameError> {
    let index = ctx.index.ok_or(RenameError::NothingToRename)?;
    if !valid_member_name(new_name) {
        return Err(RenameError::InvalidName);
    }
    let key = ref_key_at(doc, offset).ok_or(RenameError::NothingToRename)?;
    let fam = family(index, &key)?;
    let owner_names: HashSet<&str> = fam.owners.iter().map(|o| o.name.as_str()).collect();
    let mut out = Vec::new();
    for o in &fam.owners {
        let members = if fam.method { &o.methods } else { &o.fields };
        if let Some(m) = members
            .iter()
            .find(|m| m.name == fam.name && m.desc == fam.desc)
        {
            if let (Some(path), Some(span)) = (o.path.clone(), m.span) {
                out.push(FileEdit {
                    path: Some(path),
                    edit: Edit {
                        span,
                        new_text: new_name.to_string(),
                    },
                });
            }
        }
    }
    // Call sites: written owner may be a subclass; keep those that resolve into the family.
    let sep = if fam.method { "" } else { ":" };
    let suffix = format!("->{}{sep}{}", fam.name, fam.desc);
    for f in index.files() {
        for (k, span) in &f.refs {
            let Some(owner) = k.strip_suffix(suffix.as_str()) else {
                continue;
            };
            let resolved = if fam.method {
                index.resolve_method(owner, &fam.name, &fam.desc)
            } else {
                index.resolve_field(owner, &fam.name, &fam.desc)
            };
            let hit = match resolved {
                Lookup::Found { owner: o, .. } => owner_names.contains(o.name.as_str()),
                _ => owner_names.contains(owner),
            };
            if hit {
                let start = span.start + owner.len() + 2;
                out.push(FileEdit {
                    path: Some(f.path.clone()),
                    edit: Edit {
                        span: Span::new(start, start + fam.name.len()),
                        new_text: new_name.to_string(),
                    },
                });
            }
        }
    }
    out.sort_by(|a, b| (&a.path, a.edit.span).cmp(&(&b.path, b.edit.span)));
    out.dedup();
    Ok(out)
}

/// Span of the name to rename under the cursor (for prepareRename).
pub fn rename_target(doc: &Document, offset: usize) -> Option<Span> {
    match target_at(doc, offset)? {
        Target::MethodDef(i) => doc.model.methods.get(i).map(|m| m.name.span).filter(|_| {
            doc.model
                .methods
                .get(i)
                .is_some_and(|m| m.name.text != "<init>" && m.name.text != "<clinit>")
        }),
        Target::FieldDef(i) => doc.model.fields.get(i).map(|f| f.name.span),
        Target::MethodRef(t, span) => {
            let r = parse_method_ref(&t).ok()?;
            let start = span.start + t.find("->")? + 2;
            (r.name != "<init>" && r.name != "<clinit>")
                .then(|| Span::new(start, start + r.name.len()))
        }
        Target::FieldRef(t, span) => {
            let r = parse_field_ref(&t).ok()?;
            let start = span.start + t.find("->")? + 2;
            Some(Span::new(start, start + r.name.len()))
        }
        _ => None,
    }
}

/// Rewrite every occurrence of class `old` to `new` (descriptor form). The caller moves the file.
pub fn rename_class(index: &Index, old: &str, new: &str) -> Result<Vec<FileEdit>, RenameError> {
    if !matches!(
        parse_type(new, false),
        Ok(smali_syntax::descriptor::TypeDesc::Class(_))
    ) {
        return Err(RenameError::InvalidName);
    }
    match index.class(old) {
        Some(c) if c.origin == Origin::Workspace => {}
        Some(c) => return Err(RenameError::Framework(c.name.clone())),
        None => return Err(RenameError::NothingToRename),
    }
    let mut out = Vec::new();
    for f in index.files() {
        for (k, span) in &f.refs {
            if k.as_ref() == old {
                out.push(FileEdit {
                    path: Some(f.path.clone()),
                    edit: Edit {
                        span: *span,
                        new_text: new.to_string(),
                    },
                });
            }
        }
    }
    out.sort_by(|a, b| (&a.path, a.edit.span).cmp(&(&b.path, b.edit.span)));
    Ok(out)
}

/// A method for call hierarchy purposes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallItem {
    /// Owner descriptor, name and prototype.
    pub owner: String,
    pub name: String,
    pub proto: String,
    pub loc: Option<Loc>,
}

pub fn call_item_at(doc: &Document, ctx: &Context, offset: usize) -> Option<CallItem> {
    match ref_key_at(doc, offset)? {
        RefKey::Method(owner, name, proto) => {
            let loc = match target_at(doc, offset)? {
                Target::MethodDef(i) => doc.model.methods.get(i).map(|m| Loc {
                    path: None,
                    span: m.name.span,
                }),
                _ => match ctx.index.map(|i| i.resolve_method(&owner, &name, &proto)) {
                    Some(Lookup::Found { owner, member }) => {
                        owner.location_of(member.span).map(Loc::from)
                    }
                    _ => None,
                },
            };
            Some(CallItem {
                owner,
                name,
                proto,
                loc,
            })
        }
        _ => None,
    }
}

/// Calls made from method `item` as written in `doc` (outgoing), with the call-site spans.
pub fn outgoing_calls(doc: &Document, item: &CallItem) -> Vec<(CallItem, Vec<Span>)> {
    let Some(m) = doc
        .model
        .methods
        .iter()
        .find(|m| m.name.text == item.name && m.proto.text == item.proto)
    else {
        return Vec::new();
    };
    let mut out: Vec<(CallItem, Vec<Span>)> = Vec::new();
    for insn in m.instructions() {
        for o in &insn.operands {
            let smali_syntax::model::OperandValue::MethodRef(t) = &o.value else {
                continue;
            };
            let Ok(r) = parse_method_ref(t) else { continue };
            let callee = CallItem {
                owner: r.class.to_string(),
                name: r.name,
                proto: r.proto.to_string(),
                loc: None,
            };
            match out.iter_mut().find(|(c, _)| *c == callee) {
                Some((_, spans)) => spans.push(o.span),
                None => out.push((callee, vec![o.span])),
            }
        }
    }
    out
}

/// Files and call-site spans that call `item` (written owner resolving to the same declaration).
pub fn incoming_call_sites(index: &Index, item: &CallItem) -> Vec<(PathBuf, Span)> {
    let suffix = format!("->{}{}", item.name, item.proto);
    let target = match index.resolve_method(&item.owner, &item.name, &item.proto) {
        Lookup::Found { owner, .. } => Some(owner.name.clone()),
        _ => None,
    };
    let mut out = Vec::new();
    for f in index.files() {
        for (k, span) in &f.refs {
            let Some(owner) = k.strip_suffix(suffix.as_str()) else {
                continue;
            };
            let same = match (
                &target,
                index.resolve_method(owner, &item.name, &item.proto),
            ) {
                (Some(t), Lookup::Found { owner: o, .. }) => &o.name == t,
                _ => owner == item.owner,
            };
            if same {
                out.push((f.path.clone(), *span));
            }
        }
    }
    out
}

/// The method in `doc` containing `offset`, as a call item.
pub fn enclosing_method(doc: &Document, offset: usize) -> Option<CallItem> {
    let owner = doc.model.class.as_ref()?.name.text.clone();
    let m = crate::method_at(doc, offset)?;
    Some(CallItem {
        owner,
        name: m.name.text.clone(),
        proto: m.proto.text.clone(),
        loc: Some(Loc {
            path: None,
            span: m.name.span,
        }),
    })
}
