//! Code actions. Each is computed from the diagnostic it fixes.

use smali_index::Lookup;
use smali_opcodes::lookup;
use smali_syntax::descriptor::parse_method_ref;
use smali_syntax::model::{OperandValue, RegDirectiveKind, RegKind};
use smali_syntax::{Document, Span};

use crate::diagnostics::diagnostics;
use crate::regs::Frame;
use crate::{method_at, Context, Diagnostic, Edit};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    pub title: String,
    pub code: &'static str,
    pub edits: Vec<Edit>,
    pub preferred: bool,
}

fn overlaps(a: Span, b: Span) -> bool {
    a.start <= b.end && b.start <= a.end
}

pub fn code_actions(doc: &Document, ctx: &Context, range: Span) -> Vec<Action> {
    let mut out = Vec::new();
    for d in diagnostics(doc, ctx)
        .iter()
        .filter(|d| overlaps(d.span, range))
    {
        let a = match d.code {
            "S201" | "S203" => raise_locals(doc, d),
            "S211" => widen_const(doc, d),
            "S304" => swap_invoke(doc, ctx, d),
            "S222" => fix_reg_count(doc, d),
            "S120" => create_label(doc, d, range),
            "S110" => rename_class(doc, ctx, d),
            _ => None,
        };
        out.extend(a);
    }
    out
}

fn raise_locals(doc: &Document, d: &Diagnostic) -> Option<Action> {
    let m = method_at(doc, d.span.start)?;
    let frame = Frame::of(m);
    let [dir] = m.reg_directives.as_slice() else {
        return None;
    };
    let reg = doc.span_text(d.span);
    let v: u64 = reg.get(1..)?.parse().ok()?;
    let v = if reg.starts_with('p') {
        frame.locals()? + v
    } else {
        v
    };
    let need_total = v + if d.code == "S203" { 2 } else { 1 };
    let (title, value) = match dir.kind {
        RegDirectiveKind::Locals => {
            let l = need_total.checked_sub(u64::from(frame.params))?;
            (format!("Raise .locals to {l}"), l)
        }
        RegDirectiveKind::Registers => (format!("Raise .registers to {need_total}"), need_total),
    };
    Some(Action {
        title,
        code: d.code,
        edits: vec![Edit {
            span: dir.value_span,
            new_text: value.to_string(),
        }],
        preferred: true,
    })
}

fn widen_const(doc: &Document, d: &Diagnostic) -> Option<Action> {
    let m = method_at(doc, d.span.start)?;
    let insn = m.instructions().find(|i| i.span.contains(d.span.start))?;
    let value = insn.operands.iter().find_map(|o| match o.value {
        OperandValue::Int(Some(v)) if o.span == d.span => Some(v),
        _ => None,
    })?;
    let ladder: &[&str] = match insn.opcode.text.as_str() {
        "const/4" | "const/16" => &["const/16", "const"],
        "const-wide/16" | "const-wide/32" => &["const-wide/32", "const-wide"],
        _ => return None,
    };
    let fits = |name: &str| -> bool {
        let Some(op) = lookup(name) else { return false };
        op.operands.iter().any(|k| match k {
            smali_opcodes::OperandKind::Lit(l) => {
                let (lo, hi) = match l {
                    smali_opcodes::LitKind::Bits16 => (-32768, 65535),
                    smali_opcodes::LitKind::Bits32 => (i128::from(i32::MIN), i128::from(u32::MAX)),
                    _ => (i128::from(i64::MIN), i128::from(u64::MAX)),
                };
                (lo..=hi).contains(&value)
            }
            _ => false,
        })
    };
    let target = ladder.iter().find(|n| **n != insn.opcode.text && fits(n))?;
    Some(Action {
        title: format!("Change to {target}"),
        code: d.code,
        edits: vec![Edit {
            span: insn.opcode.span,
            new_text: target.to_string(),
        }],
        preferred: true,
    })
}

fn swap_invoke(doc: &Document, ctx: &Context, d: &Diagnostic) -> Option<Action> {
    let index = ctx.index?;
    let m = method_at(doc, d.span.start)?;
    let insn = m.instructions().find(|i| i.span.contains(d.span.start))?;
    let r = parse_method_ref(doc.span_text(d.span)).ok()?;
    let Lookup::Found { owner, member } =
        index.resolve_method(&r.class.to_string(), &r.name, &r.proto.to_string())
    else {
        return None;
    };
    let kind = if member.is_static() {
        "invoke-static"
    } else if member.flags & smali_index::acc::PRIVATE != 0 || member.name == "<init>" {
        "invoke-direct"
    } else if owner.is_interface() {
        "invoke-interface"
    } else {
        "invoke-virtual"
    };
    let range = if insn.opcode.text.ends_with("/range") {
        "/range"
    } else {
        ""
    };
    let new = format!("{kind}{range}");
    Some(Action {
        title: format!("Change to {new}"),
        code: d.code,
        edits: vec![Edit {
            span: insn.opcode.span,
            new_text: new,
        }],
        preferred: true,
    })
}

fn fix_reg_count(doc: &Document, d: &Diagnostic) -> Option<Action> {
    let m = method_at(doc, d.span.start)?;
    let insn = m.instructions().find(|i| i.span == d.span)?;
    let r = insn.operands.iter().find_map(|o| match &o.value {
        OperandValue::MethodRef(t) => parse_method_ref(t).ok(),
        _ => None,
    })?;
    let want = u32::from(!insn.opcode.text.starts_with("invoke-static")) + r.proto.param_slots();
    let regs = insn.operands.first()?;
    let new_text = match &regs.value {
        OperandValue::RegRange(Some(a), Some(_)) if want > 0 => {
            let p = if a.kind == RegKind::P { "p" } else { "v" };
            format!("{{{p}{} .. {p}{}}}", a.num, a.num + want - 1)
        }
        OperandValue::RegList(rs) if rs.len() as u32 > want => {
            let names: Vec<&str> = rs
                .iter()
                .take(want as usize)
                .map(|r| doc.span_text(r.span))
                .collect();
            format!("{{{}}}", names.join(", "))
        }
        _ => return None,
    };
    Some(Action {
        title: format!("Pass {want} registers"),
        code: d.code,
        edits: vec![Edit {
            span: regs.span,
            new_text,
        }],
        preferred: false,
    })
}

fn create_label(doc: &Document, d: &Diagnostic, at: Span) -> Option<Action> {
    let name = doc.span_text(d.span).trim_start_matches(':');
    let m = method_at(doc, d.span.start)?;
    // Insert at the start of the line holding the cursor if it is inside the method body,
    // otherwise right before `.end method`.
    let text = &doc.text;
    let mut line_start = text[..at.start.min(text.len())]
        .rfind('\n')
        .map_or(0, |i| i + 1);
    if line_start <= m.header.end || line_start >= m.span.end {
        line_start = text[..m.span.end]
            .rfind(".end method")
            .map(|i| text[..i].rfind('\n').map_or(0, |j| j + 1))?;
    }
    Some(Action {
        title: format!("Create label :{name} here"),
        code: d.code,
        edits: vec![Edit {
            span: Span::new(line_start, line_start),
            new_text: format!("    :{name}\n"),
        }],
        preferred: false,
    })
}

fn rename_class(doc: &Document, ctx: &Context, d: &Diagnostic) -> Option<Action> {
    let expected = ctx.expected_class?;
    let c = doc.model.class.as_ref()?;
    Some(Action {
        title: format!("Rename class to {expected}"),
        code: d.code,
        edits: vec![Edit {
            span: c.name.span,
            new_text: expected.to_string(),
        }],
        preferred: false,
    })
}
