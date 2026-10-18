//! Completion.

use std::collections::HashSet;

use smali_index::acc;
use smali_opcodes::{LitKind, OperandKind, RefKind, OPCODES};
use smali_syntax::descriptor::parse_proto;
use smali_syntax::model::RegDirectiveKind;
use smali_syntax::{Document, Span};

use crate::hover::method_signature_java;
use crate::regs::Frame;
use crate::{method_at, Context, Edit};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Opcode,
    Directive,
    Register,
    Label,
    Class,
    Interface,
    Method,
    Field,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub label: String,
    pub kind: ItemKind,
    pub detail: String,
    /// Replaces `replace`.
    pub insert: String,
    pub snippet: bool,
    pub replace: Span,
    pub filter: Option<String>,
    pub extra_edits: Vec<Edit>,
}

const CLASS_DIRECTIVES: &[&str] = &[
    ".class",
    ".super",
    ".source",
    ".implements",
    ".field",
    ".end field",
    ".method",
    ".annotation",
    ".end annotation",
];
const METHOD_DIRECTIVES: &[&str] = &[
    ".registers",
    ".locals",
    ".param",
    ".end param",
    ".line",
    ".local",
    ".end local",
    ".restart local",
    ".prologue",
    ".epilogue",
    ".catch",
    ".catchall",
    ".packed-switch",
    ".end packed-switch",
    ".sparse-switch",
    ".end sparse-switch",
    ".array-data",
    ".end array-data",
    ".annotation",
    ".end annotation",
    ".end method",
];

fn opcode_snippet(name: &str, ops: &[OperandKind]) -> String {
    let mut parts = Vec::new();
    let mut n = 1;
    let mut ph = |s: &str| {
        let r = format!("${{{n}:{s}}}");
        n += 1;
        r
    };
    for k in ops {
        parts.push(match k {
            OperandKind::Reg(_) => ph("v0"),
            OperandKind::RegList => format!("{{{}}}", ph("v0")),
            OperandKind::RegRange => format!("{{{} .. {}}}", ph("v0"), ph("v1")),
            OperandKind::Label => format!(":{}", ph("label")),
            OperandKind::Lit(LitKind::High16) => ph("0x10000"),
            OperandKind::Lit(_) => ph("0x0"),
            OperandKind::Ref(RefKind::String) => format!("\"{}\"", ph("")),
            OperandKind::Ref(RefKind::Type) => ph("Ljava/lang/Object;"),
            OperandKind::Ref(RefKind::Field) => ph("Lcls;->field:I"),
            OperandKind::Ref(RefKind::Method) => ph("Lcls;->method()V"),
            OperandKind::Proto => ph("()V"),
            OperandKind::Ref(_) | OperandKind::Odex => ph("ref"),
        });
    }
    if parts.is_empty() {
        name.to_string()
    } else {
        format!("{name} {}", parts.join(", "))
    }
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric()
        || matches!(
            b,
            b'_' | b'$' | b'-' | b'/' | b'<' | b'>' | b'.' | b':' | b';' | b'['
        )
}

pub fn complete(doc: &Document, ctx: &Context, offset: usize) -> Vec<Item> {
    let text = &doc.text;
    let offset = offset.min(text.len());
    let line_start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
    let before = &text[line_start..offset];
    if before.trim_start().starts_with('#') {
        return Vec::new();
    }
    let bytes = text.as_bytes();
    let mut tok_start = offset;
    while tok_start > line_start && is_word_byte(bytes[tok_start - 1]) {
        tok_start -= 1;
    }
    // A member reference token runs back to its owner class even across `->`.
    let tok = &text[tok_start..offset];
    let replace = Span::new(tok_start, offset);
    let in_method = method_at(doc, offset).filter(|m| m.header.end < offset);
    let first_word = before.split_whitespace().next().unwrap_or("");
    let at_line_start = before.trim_start().len() == tok.len();

    if let Some(arrow) = tok.rfind("->") {
        return members(
            ctx,
            &tok[..arrow],
            Span::new(tok_start + arrow + 2, offset),
            doc,
            offset,
        );
    }
    if at_line_start {
        let mut out = Vec::new();
        let directives = if in_method.is_some() {
            METHOD_DIRECTIVES
        } else {
            CLASS_DIRECTIVES
        };
        for d in directives {
            out.push(Item {
                label: d.to_string(),
                kind: ItemKind::Directive,
                detail: String::new(),
                insert: d.to_string(),
                snippet: false,
                replace,
                filter: None,
                extra_edits: Vec::new(),
            });
        }
        if in_method.is_some() && !tok.starts_with('.') {
            for op in OPCODES.iter().filter(|o| !o.is_odex_only()) {
                out.push(Item {
                    label: op.name.to_string(),
                    kind: ItemKind::Opcode,
                    detail: op.summary(),
                    insert: opcode_snippet(op.name, op.operands),
                    snippet: true,
                    replace,
                    filter: None,
                    extra_edits: Vec::new(),
                });
            }
        }
        return out;
    }
    let Some(m) = in_method else {
        return classes(ctx, tok, replace);
    };
    if tok.starts_with(':') {
        return m
            .label_defs()
            .map(|l| Item {
                label: format!(":{}", l.text),
                kind: ItemKind::Label,
                detail: String::new(),
                insert: format!(":{}", l.text),
                snippet: false,
                replace,
                filter: None,
                extra_edits: Vec::new(),
            })
            .collect();
    }
    let looks_reg = tok.is_empty()
        || (tok.len() <= 6
            && matches!(tok.as_bytes()[0], b'v' | b'p')
            && tok[1..].bytes().all(|b| b.is_ascii_digit()));
    let is_directive_line = first_word.starts_with('.');
    if looks_reg && (!first_word.is_empty() || is_directive_line) {
        let frame = Frame::of(m);
        let mut out = Vec::new();
        if let Some(t) = frame.total {
            let l = frame.locals().unwrap_or(0);
            for v in 0..t.min(256) {
                let detail = if v >= l {
                    format!("p{}", v - l)
                } else {
                    "local".into()
                };
                out.push(reg_item(format!("v{v}"), detail, replace));
            }
            for p in 0..frame.params {
                out.push(reg_item(
                    format!("p{p}"),
                    format!("v{}", l + u64::from(p)),
                    replace,
                ));
            }
        }
        if !tok.is_empty() {
            return out;
        }
        out.extend(classes(ctx, tok, replace));
        return out;
    }
    classes(ctx, tok, replace)
}

fn reg_item(label: String, detail: String, replace: Span) -> Item {
    Item {
        insert: label.clone(),
        label,
        kind: ItemKind::Register,
        detail,
        snippet: false,
        replace,
        filter: None,
        extra_edits: Vec::new(),
    }
}

/// Class descriptors matching a simple-name prefix or a descriptor prefix.
fn classes(ctx: &Context, tok: &str, replace: Span) -> Vec<Item> {
    let Some(index) = ctx.index else {
        return Vec::new();
    };
    let tok_trim = tok.trim_start_matches('[');
    let dims = &tok[..tok.len() - tok_trim.len()];
    let replace = Span::new(replace.start + dims.len(), replace.end);
    if tok_trim.is_empty() {
        return Vec::new();
    }
    let lower = tok_trim.to_lowercase();
    let by_desc = tok_trim.starts_with('L')
        && (tok_trim.contains('/')
            || tok_trim.len() > 1 && tok_trim.as_bytes()[1].is_ascii_lowercase());
    let mut out = Vec::new();
    for name in index.class_names() {
        let simple = name
            .trim_end_matches(';')
            .rsplit('/')
            .next()
            .unwrap_or(name);
        let ok = if by_desc {
            name.starts_with(tok_trim)
        } else {
            simple.to_lowercase().starts_with(&lower)
        };
        if !ok {
            continue;
        }
        let interface = index.class(name).is_some_and(|c| c.is_interface());
        out.push(Item {
            label: simple.to_string(),
            kind: if interface {
                ItemKind::Interface
            } else {
                ItemKind::Class
            },
            detail: name.clone(),
            insert: name.clone(),
            snippet: false,
            replace,
            filter: Some(if by_desc {
                name.clone()
            } else {
                tok_trim.to_string()
            }),
            extra_edits: Vec::new(),
        });
        if out.len() >= 200 {
            break;
        }
    }
    out.sort_by(|a, b| (a.label.len(), &a.label).cmp(&(b.label.len(), &b.label)));
    out
}

/// Methods and fields of `owner` and its ancestors.
fn members(ctx: &Context, owner: &str, replace: Span, doc: &Document, offset: usize) -> Vec<Item> {
    let Some(index) = ctx.index else {
        return Vec::new();
    };
    let (chain, _) = index.hierarchy(owner);
    let line_start = doc.text[..offset].rfind('\n').map_or(0, |i| i + 1);
    let line = &doc.text[line_start..offset];
    let opcode = line.split_whitespace().next().unwrap_or("");
    let want_fields = opcode.contains("get") || opcode.contains("put");
    let want_methods = !want_fields;
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for c in chain {
        if want_methods {
            for m in &c.methods {
                if !seen.insert((m.name.clone(), m.desc.clone())) {
                    continue;
                }
                if m.name == "<clinit>" {
                    continue;
                }
                let insert = format!("{}{}", m.name, m.desc);
                let extra = locals_bump(doc, offset, opcode, m.flags & acc::STATIC != 0, &m.desc);
                out.push(Item {
                    label: insert.clone(),
                    kind: ItemKind::Method,
                    detail: method_signature_java(&m.name, &m.desc, m.flags, &c.name),
                    insert,
                    snippet: false,
                    replace,
                    filter: Some(m.name.clone()),
                    extra_edits: extra,
                });
            }
        }
        if want_fields {
            for f in &c.fields {
                if !seen.insert((f.name.clone(), f.desc.clone())) {
                    continue;
                }
                let insert = format!("{}:{}", f.name, f.desc);
                out.push(Item {
                    label: insert.clone(),
                    kind: ItemKind::Field,
                    detail: format!("{} {}", acc::java_modifiers(f.flags, false), c.name),
                    insert,
                    snippet: false,
                    replace,
                    filter: Some(f.name.clone()),
                    extra_edits: Vec::new(),
                });
            }
        }
    }
    out
}

/// If a non-range invoke of this target needs more registers than the method has, raise `.locals`.
fn locals_bump(
    doc: &Document,
    offset: usize,
    opcode: &str,
    target_static: bool,
    desc: &str,
) -> Vec<Edit> {
    if !opcode.starts_with("invoke-") || opcode.ends_with("/range") {
        return Vec::new();
    }
    let Some(m) = method_at(doc, offset) else {
        return Vec::new();
    };
    let [d] = m.reg_directives.as_slice() else {
        return Vec::new();
    };
    let (Some(v), RegDirectiveKind::Locals) = (d.value, d.kind) else {
        return Vec::new();
    };
    let Ok(p) = parse_proto(desc) else {
        return Vec::new();
    };
    let is_static_call = opcode.starts_with("invoke-static") || target_static;
    let need = u64::from(!is_static_call) + u64::from(p.param_slots());
    let frame = Frame::of(m);
    let total = v + u64::from(frame.params);
    if need <= total {
        return Vec::new();
    }
    vec![Edit {
        span: d.value_span,
        new_text: (v + need - total).to_string(),
    }]
}
