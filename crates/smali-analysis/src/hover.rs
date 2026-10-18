//! Hover text.

use smali_index::{acc, ClassInfo, Index, Lookup, MemberInfo};
use smali_opcodes::{flags, lookup};
use smali_syntax::descriptor::{
    parse_field_ref, parse_method_ref, parse_proto, parse_type, TypeDesc,
};
use smali_syntax::model::{Instruction, Item, OperandValue, Reg, RegKind};
use smali_syntax::{Document, Span};

use crate::regs::Frame;
use crate::{method_at, Context};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hover {
    pub markdown: String,
    pub span: Span,
}

/// What sits under a cursor position.
#[derive(Debug, Clone)]
pub enum Target<'a> {
    Opcode(&'a Instruction),
    Reg(Reg),
    MethodRef(String, Span),
    FieldRef(String, Span),
    Class(String, Span),
    LabelRef(String, Span),
    LabelDef(String, Span),
    MethodDef(usize),
    FieldDef(usize),
}

pub fn target_at(doc: &Document, offset: usize) -> Option<Target<'_>> {
    let model = &doc.model;
    if let Some(m) = method_at(doc, offset) {
        for item in &m.items {
            match item {
                Item::Insn(insn) if insn.span.contains(offset) => {
                    if insn.opcode.span.contains(offset) {
                        return Some(Target::Opcode(insn));
                    }
                    for o in &insn.operands {
                        if !o.span.contains(offset) {
                            continue;
                        }
                        return match &o.value {
                            OperandValue::Reg(r) => Some(Target::Reg(*r)),
                            OperandValue::RegList(rs) => rs
                                .iter()
                                .find(|r| r.span.contains(offset))
                                .map(|r| Target::Reg(*r)),
                            OperandValue::RegRange(a, b) => [a, b]
                                .into_iter()
                                .flatten()
                                .find(|r| r.span.contains(offset))
                                .map(|r| Target::Reg(*r)),
                            OperandValue::Label(l) => {
                                Some(Target::LabelRef(l.text.clone(), l.span))
                            }
                            OperandValue::MethodRef(t) => {
                                // Cursor on the owner class goes to the class.
                                let arrow = t.find("->").unwrap_or(0);
                                if offset < o.span.start + arrow {
                                    Some(Target::Class(
                                        t[..arrow].to_string(),
                                        Span::new(o.span.start, o.span.start + arrow),
                                    ))
                                } else {
                                    Some(Target::MethodRef(t.clone(), o.span))
                                }
                            }
                            OperandValue::FieldRef(t) => {
                                let arrow = t.find("->").unwrap_or(0);
                                if offset < o.span.start + arrow {
                                    Some(Target::Class(
                                        t[..arrow].to_string(),
                                        Span::new(o.span.start, o.span.start + arrow),
                                    ))
                                } else {
                                    Some(Target::FieldRef(t.clone(), o.span))
                                }
                            }
                            _ => None,
                        };
                    }
                }
                Item::Label(l) if l.span.contains(offset) => {
                    return Some(Target::LabelDef(l.text.clone(), l.span))
                }
                Item::Catch(c) => {
                    for l in [&c.start, &c.end, &c.handler] {
                        if l.span.contains(offset) {
                            return Some(Target::LabelRef(l.text.clone(), l.span));
                        }
                    }
                }
                Item::PackedSwitch(d) | Item::SparseSwitch(d) => {
                    if let Some(l) = d.targets.iter().find(|l| l.span.contains(offset)) {
                        return Some(Target::LabelRef(l.text.clone(), l.span));
                    }
                }
                Item::Directive { regs, .. } => {
                    if let Some(r) = regs.iter().find(|r| r.span.contains(offset)) {
                        return Some(Target::Reg(*r));
                    }
                }
                _ => {}
            }
        }
    }
    if let Some(i) = model
        .methods
        .iter()
        .position(|m| m.name.span.contains(offset))
    {
        return Some(Target::MethodDef(i));
    }
    if let Some(i) = model
        .fields
        .iter()
        .position(|f| f.name.span.contains(offset))
    {
        return Some(Target::FieldDef(i));
    }
    // Innermost class identifier covering the offset (field types, annotations, header...).
    model
        .type_refs
        .iter()
        .find(|t| t.span.contains(offset))
        .map(|t| Target::Class(t.text.clone(), t.span))
}

pub fn method_signature_java(name: &str, proto: &str, flags: u32, owner: &str) -> String {
    let Ok(p) = parse_proto(proto) else {
        return format!("{name}{proto}");
    };
    let params: Vec<String> = p.params.iter().map(|t| t.java(true)).collect();
    let mods = acc::java_modifiers(flags, true);
    let mods = if mods.is_empty() {
        String::new()
    } else {
        format!("{mods} ")
    };
    if name == "<init>" {
        let cls = parse_type(owner, false).map_or(owner.to_string(), |t| t.java(true));
        return format!("{mods}{cls}({})", params.join(", "));
    }
    if name == "<clinit>" {
        return "static {}".to_string();
    }
    format!("{mods}{} {name}({})", p.ret.java(true), params.join(", "))
}

fn java_class(desc: &str) -> String {
    parse_type(desc, false).map_or(desc.to_string(), |t| t.java(false))
}

fn origin_line(c: &ClassInfo) -> String {
    format!(
        "Declared in `{}` ({})",
        java_class(&c.name),
        c.origin.label()
    )
}

fn member_hover(owner: &ClassInfo, m: &MemberInfo, method: bool) -> String {
    let sig = if method {
        method_signature_java(&m.name, &m.desc, m.flags, &owner.name)
    } else {
        let mods = acc::java_modifiers(m.flags, false);
        let ty = parse_type(&m.desc, false).map_or(m.desc.clone(), |t| t.java(true));
        format!(
            "{mods}{}{ty} {}",
            if mods.is_empty() { "" } else { " " },
            m.name
        )
    };
    format!("```java\n{sig}\n```\n{}", origin_line(owner))
}

fn class_hover(index: Option<&Index>, desc: &str) -> String {
    let Some(c) = index.and_then(|i| i.class(desc)) else {
        return format!("```java\n{}\n```\nNot found in the index", java_class(desc));
    };
    let kind = if c.is_interface() {
        "interface"
    } else {
        "class"
    };
    let mods = acc::java_modifiers(
        c.flags & !acc::ABSTRACT
            | if c.is_interface() {
                0
            } else {
                c.flags & acc::ABSTRACT
            },
        false,
    );
    let mut s = format!(
        "```java\n{mods}{}{kind} {}",
        if mods.is_empty() { "" } else { " " },
        java_class(&c.name)
    );
    if let Some(sup) = &c.super_class {
        if sup != "Ljava/lang/Object;" {
            s.push_str(&format!(" extends {}", java_class(sup)));
        }
    }
    if !c.interfaces.is_empty() {
        let list: Vec<String> = c.interfaces.iter().map(|i| java_class(i)).collect();
        s.push_str(&format!(" implements {}", list.join(", ")));
    }
    s.push_str(&format!("\n```\nSource: {}", c.origin.label()));
    s
}

pub fn hover(doc: &Document, ctx: &Context, offset: usize) -> Option<Hover> {
    let target = target_at(doc, offset)?;
    let (markdown, span) = match target {
        Target::Opcode(insn) => {
            let op = lookup(&insn.opcode.text)?;
            let mut s = format!(
                "```smali\n{}\n```\nFormat `{}`",
                op.summary(),
                op.format.id()
            );
            if op.reference != smali_opcodes::RefKind::None {
                s.push_str(&format!(", reference: {:?}", op.reference));
            }
            let mut notes = Vec::new();
            if op.has(flags::CAN_THROW) {
                notes.push("can throw");
            }
            if op.sets_result() {
                notes.push("sets result (use move-result next)");
            }
            if op.has(flags::SETS_WIDE_REGISTER) {
                notes.push("writes a wide register pair");
            }
            if op.is_odex_only() {
                notes.push("odex only; smali rejects it");
            }
            if !notes.is_empty() {
                s.push_str(&format!("\n\n{}", notes.join(", ")));
            }
            (s, insn.opcode.span)
        }
        Target::Reg(r) => {
            let m = method_at(doc, offset)?;
            let frame = Frame::of(m);
            let mut s = String::new();
            match (r.kind, frame.locals()) {
                (RegKind::P, Some(l)) => {
                    s.push_str(&format!("`p{}` = `v{}`", r.num, l + u64::from(r.num)))
                }
                (RegKind::V, Some(l)) if u64::from(r.num) >= l => {
                    s.push_str(&format!("`v{}` = `p{}`", r.num, u64::from(r.num) - l))
                }
                (RegKind::V, _) => s.push_str(&format!("`v{}` (local)", r.num)),
                (RegKind::P, None) => s.push_str(&format!("`p{}`", r.num)),
            }
            let p = match (r.kind, frame.locals()) {
                (RegKind::P, _) => Some(r.num),
                (RegKind::V, Some(l)) => u64::from(r.num)
                    .checked_sub(l)
                    .and_then(|x| u32::try_from(x).ok()),
                _ => None,
            };
            if let Some((idx, ty)) = p.and_then(|p| frame.param_of(p)) {
                match idx {
                    None => s.push_str("\n\n`this`"),
                    Some(i) => {
                        let jt = parse_type(&ty, false).map_or(ty.clone(), |t| t.java(true));
                        s.push_str(&format!("\n\nparameter {i}: `{jt}`"));
                        if let Ok(TypeDesc::Prim('J' | 'D')) = parse_type(&ty, false) {
                            s.push_str(" (wide pair)");
                        }
                    }
                }
            }
            (s, r.span)
        }
        Target::MethodRef(t, span) => {
            let r = parse_method_ref(&t).ok()?;
            let owner = r.class.to_string();
            let found = ctx
                .index
                .map(|i| i.resolve_method(&owner, &r.name, &r.proto.to_string()));
            let s = match found {
                Some(Lookup::Found { owner, member }) => member_hover(&owner, &member, true),
                Some(Lookup::Missing) => format!(
                    "```java\n{}\n```\nNot found in `{}` or its ancestors",
                    method_signature_java(&r.name, &r.proto.to_string(), 0, &owner),
                    java_class(&owner)
                ),
                _ => format!(
                    "```java\n{}\n```\n`{}` (not indexed)",
                    method_signature_java(&r.name, &r.proto.to_string(), 0, &owner),
                    java_class(&owner)
                ),
            };
            (s, span)
        }
        Target::FieldRef(t, span) => {
            let r = parse_field_ref(&t).ok()?;
            let owner = r.class.to_string();
            let found = ctx
                .index
                .map(|i| i.resolve_field(&owner, &r.name, &r.ty.to_string()));
            let s = match found {
                Some(Lookup::Found { owner, member }) => member_hover(&owner, &member, false),
                _ => format!(
                    "```java\n{} {}\n```\n`{}`",
                    r.ty.java(true),
                    r.name,
                    java_class(&owner)
                ),
            };
            (s, span)
        }
        Target::Class(desc, span) => (class_hover(ctx.index, &desc), span),
        Target::MethodDef(i) => {
            let m = doc.model.methods.get(i)?;
            let owner = doc
                .model
                .class
                .as_ref()
                .map_or("", |c| c.name.text.as_str());
            let flags = acc::from_words(&m.flags);
            let frame = Frame::of(m);
            let regs = frame
                .total
                .map(|t| {
                    format!(
                        "\n\n{t} registers ({} locals, {} parameter)",
                        t.saturating_sub(u64::from(frame.params)),
                        frame.params
                    )
                })
                .unwrap_or_default();
            (
                format!(
                    "```java\n{}\n```{regs}",
                    method_signature_java(&m.name.text, &m.proto.text, flags, owner)
                ),
                m.name.span,
            )
        }
        Target::FieldDef(i) => {
            let f = doc.model.fields.get(i)?;
            let mods = acc::java_modifiers(acc::from_words(&f.flags), false);
            let ty = parse_type(&f.ty.text, false).map_or(f.ty.text.clone(), |t| t.java(true));
            (
                format!("```java\n{mods} {ty} {}\n```", f.name.text)
                    .replace("``` ", "```")
                    .replace("\n ", "\n"),
                f.name.span,
            )
        }
        Target::LabelRef(..) | Target::LabelDef(..) => return None,
    };
    Some(Hover { markdown, span })
}
