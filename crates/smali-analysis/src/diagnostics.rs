//! Diagnostic catalog (S1xx, S2xx, S3xx).

use std::collections::{HashMap, HashSet};

use smali_index::{acc, Lookup};
use smali_opcodes::{flags, lookup, LitKind, Opcode, OperandKind, RefKind};
use smali_syntax::descriptor::{
    parse_field_ref, parse_method_ref, parse_proto, parse_type, DescError, TypeDesc,
};
use smali_syntax::model::{Instruction, Item, Method, Named, OperandValue, Reg, RegDirectiveKind};
use smali_syntax::{Document, Span};

use crate::regs::Frame;
use crate::{Context, Diagnostic, Severity};

pub fn default_severity(code: &str) -> Severity {
    match code {
        "S122" | "S301" => Severity::Hint,
        "S110" | "S201" | "S202" | "S203" | "S222" | "S230" | "S231" | "S302" | "S303" | "S304"
        | "S305" => Severity::Warning,
        _ => Severity::Error,
    }
}

struct Sink<'a> {
    doc: &'a Document,
    out: Vec<Diagnostic>,
}

impl Sink<'_> {
    fn push(&mut self, code: &'static str, span: Span, message: impl Into<String>) {
        self.out.push(Diagnostic {
            code,
            severity: default_severity(code),
            span,
            message: message.into(),
        });
    }

    fn desc_error(&mut self, base: Span, e: &DescError) {
        let start = (base.start + e.offset).min(base.end);
        let end = (start + e.len).min(base.end).max(start);
        let span = if end > start {
            Span::new(start, end)
        } else {
            base
        };
        self.push(
            "S105",
            span,
            format!("invalid type descriptor: {}", e.message),
        );
    }

    fn check_type(&mut self, n: &Named, allow_void: bool) -> Option<TypeDesc> {
        match parse_type(&n.text, allow_void) {
            Ok(t) => Some(t),
            Err(e) => {
                self.desc_error(n.span, &e);
                None
            }
        }
    }
}

pub fn diagnostics(doc: &Document, ctx: &Context) -> Vec<Diagnostic> {
    let mut s = Sink {
        doc,
        out: Vec::new(),
    };
    structure(&mut s, ctx);
    for m in &doc.model.methods {
        method(&mut s, ctx, m);
    }
    s.out
        .sort_by(|a, b| (a.span, a.code).cmp(&(b.span, b.code)));
    s.out.dedup();
    s.out
}

fn is_opcode_word(w: &str) -> bool {
    !w.is_empty()
        && w.as_bytes()[0].is_ascii_lowercase()
        && w.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'/')
}

fn structure(s: &mut Sink, ctx: &Context) {
    let doc = s.doc;
    let model = &doc.model;

    for err in &model.syntax_errors {
        if let Some(missing) = &err.missing {
            let at = Span::new(err.span.start, err.span.start);
            s.push("S100", at, format!("syntax error: missing {missing}"));
            continue;
        }
        let in_method = model
            .methods
            .iter()
            .any(|m| m.span.contains(err.span.start));
        let text = doc.span_text(err.span);
        let mut found = false;
        if in_method {
            let mut off = err.span.start;
            for line in text.split_inclusive('\n') {
                let trimmed = line.trim_start();
                let word = trimmed.split([' ', '\t', '\r', '\n']).next().unwrap_or("");
                let known = lookup(word);
                if is_opcode_word(word) && known.is_none_or(|o| o.is_odex_only()) {
                    let start = off + (line.len() - trimmed.len());
                    let msg = match known {
                        Some(_) => {
                            format!("'{word}' is an odex-only instruction; smali rejects it")
                        }
                        None => format!("unknown opcode '{word}'"),
                    };
                    s.push("S104", Span::new(start, start + word.len()), msg);
                    found = true;
                    break;
                }
                off += line.len();
            }
        }
        if !found {
            let first_line_end = text.find('\n').map_or(err.span.end, |i| err.span.start + i);
            s.push(
                "S100",
                Span::new(err.span.start, first_line_end.max(err.span.start)),
                "syntax error",
            );
        }
    }

    let Some(class) = &model.class else {
        let start = doc.text.len() - doc.text.trim_start().len();
        let end = doc.text[start..]
            .find('\n')
            .map_or(doc.text.len(), |i| start + i);
        s.push("S101", Span::new(start, end), "missing .class directive");
        return;
    };
    if s.check_type(&class.name, false).is_some() && !class.name.text.starts_with('L') {
        s.push("S105", class.name.span, "class name must be a class type");
    }
    match &model.super_class {
        None if class.name.text != "Ljava/lang/Object;" => {
            s.push("S102", class.name.span, "missing .super directive");
        }
        Some(sup) => {
            s.check_type(sup, false);
        }
        None => {}
    }
    for i in &model.interfaces {
        s.check_type(i, false);
    }
    if let Some(expected) = ctx.expected_class {
        if expected != class.name.text {
            s.push(
                "S110",
                class.name.span,
                format!(
                    "class {} does not match file path (expected {expected})",
                    class.name.text
                ),
            );
        }
    }

    let mut fields = HashSet::new();
    for f in &model.fields {
        s.check_type(&f.ty, false);
        if !fields.insert((f.name.text.as_str(), f.ty.text.as_str())) {
            s.push(
                "S107",
                f.name.span,
                format!("duplicate field {}:{}", f.name.text, f.ty.text),
            );
        }
    }
    let mut methods = HashSet::new();
    for m in &model.methods {
        if let Err(e) = parse_proto(&m.proto.text) {
            s.desc_error(m.proto.span, &e);
        }
        if !methods.insert((m.name.text.as_str(), m.proto.text.as_str())) {
            s.push(
                "S106",
                m.name.span,
                format!("duplicate method {}{}", m.name.text, m.proto.text),
            );
        }
    }
    if let Some(index) = ctx.index {
        for t in &model.type_refs {
            if index.class_count() > 0 && index.class(&t.text).is_none() {
                s.push("S301", t.span, format!("class {} not found", t.text));
            }
        }
    }
}

fn method(s: &mut Sink, ctx: &Context, m: &Method) {
    let doc = s.doc;
    let has_end = m.has_end || doc.span_text(m.span).trim_end().ends_with(".end method");
    if !has_end {
        s.push("S103", m.name.span, ".method without .end method");
    }
    let bodyless = m.has_flag("abstract") || m.has_flag("native");
    let has_insns = m.instructions().next().is_some();
    if bodyless {
        if !m.reg_directives.is_empty() || has_insns {
            let span = m
                .reg_directives
                .first()
                .map(|d| d.span)
                .or_else(|| m.instructions().next().map(|i| i.span))
                .unwrap_or(m.header);
            s.push(
                "S109",
                span,
                "abstract/native method cannot have a body or register directive",
            );
        }
    } else if m.reg_directives.len() != 1 {
        let (span, msg) = match m.reg_directives.get(1) {
            Some(d) => (d.span, "method has both or repeated .registers/.locals"),
            None => (
                m.name.span,
                "method with a body needs exactly one of .registers or .locals",
            ),
        };
        s.push("S108", span, msg);
    }

    let frame = Frame::of(m);
    if let [d] = m.reg_directives.as_slice() {
        if d.kind == RegDirectiveKind::Registers {
            if let Some(v) = d.value {
                if v < u64::from(frame.params) {
                    s.push(
                        "S205",
                        d.value_span,
                        format!(
                            ".registers {v} is smaller than the {} parameter registers",
                            frame.params
                        ),
                    );
                }
            }
        }
        if frame.total.is_some_and(|t| t > 65535) {
            s.push("S206", d.value_span, "register count over 65535");
        }
    }

    labels(s, m);
    let insns: Vec<&Instruction> = m.instructions().collect();
    let ret = frame.proto.as_ref().map(|p| p.ret.clone());
    let mut prev: Option<&Opcode> = None;
    let broken = |sp: Span| {
        doc.model
            .syntax_errors
            .iter()
            .any(|e| e.span.start <= sp.end && sp.start <= e.span.end)
    };
    for insn in insns {
        let op = lookup(&insn.opcode.text);
        if broken(insn.span) {
            prev = op;
            continue;
        }
        match op {
            None => s.push(
                "S104",
                insn.opcode.span,
                format!("unknown opcode '{}'", insn.opcode.text),
            ),
            Some(op) if op.is_odex_only() => s.push(
                "S104",
                insn.opcode.span,
                format!(
                    "'{}' is an odex-only instruction; smali rejects it",
                    op.name
                ),
            ),
            Some(op) => {
                instruction(s, ctx, &frame, insn, op);
                if op.is_move_result() && !prev.is_some_and(|p| p.sets_result()) {
                    s.push(
                        "S230",
                        insn.span,
                        format!(
                            "{} must directly follow an invoke or filled-new-array",
                            op.name
                        ),
                    );
                }
                if let (true, Some(ret)) = (op.is_return(), &ret) {
                    let want = match ret {
                        TypeDesc::Prim('V') => "return-void",
                        TypeDesc::Prim('J' | 'D') => "return-wide",
                        TypeDesc::Prim(_) => "return",
                        _ => "return-object",
                    };
                    if op.name != want && op.name.starts_with("return") {
                        s.push(
                            "S231",
                            insn.opcode.span,
                            format!("method returns {ret}; use {want}"),
                        );
                    }
                }
            }
        }
        prev = op;
    }
}

fn labels(s: &mut Sink, m: &Method) {
    let mut defs: HashMap<&str, &Named> = HashMap::new();
    for l in m.label_defs() {
        if defs.insert(&l.text, l).is_some() {
            s.push("S121", l.span, format!("duplicate label :{}", l.text));
        }
    }
    let refs = m.label_refs();
    let used: HashSet<&str> = refs.iter().map(|r| r.text.as_str()).collect();
    for r in &refs {
        if !defs.contains_key(r.text.as_str()) {
            s.push("S120", r.span, format!("undefined label :{}", r.text));
        }
    }
    for l in m.label_defs() {
        if !used.contains(l.text.as_str()) {
            s.push("S122", l.span, format!("unused label :{}", l.text));
        }
    }
    // S123: the label a payload instruction targets must be followed by the right block.
    let follows: HashMap<&str, Option<&Item>> = m
        .items
        .iter()
        .enumerate()
        .filter_map(|(i, it)| match it {
            Item::Label(l) => Some((
                l.text.as_str(),
                m.items[i + 1..]
                    .iter()
                    .find(|x| !matches!(x, Item::Label(_) | Item::Directive { .. })),
            )),
            _ => None,
        })
        .collect();
    for insn in m.instructions() {
        let want = match insn.opcode.text.as_str() {
            "packed-switch" => "packed-switch",
            "sparse-switch" => "sparse-switch",
            "fill-array-data" => "array-data",
            _ => continue,
        };
        for op in &insn.operands {
            let OperandValue::Label(l) = &op.value else {
                continue;
            };
            let Some(next) = follows.get(l.text.as_str()) else {
                continue;
            };
            let ok = matches!(
                (want, next),
                ("packed-switch", Some(Item::PackedSwitch(_)))
                    | ("sparse-switch", Some(Item::SparseSwitch(_)))
                    | ("array-data", Some(Item::ArrayData(_)))
            );
            if !ok {
                s.push(
                    "S123",
                    l.span,
                    format!(":{} does not point at a .{want} block", l.text),
                );
            }
        }
    }
}

fn operand_matches(kind: OperandKind, v: &OperandValue) -> bool {
    use OperandValue as V;
    match kind {
        OperandKind::Reg(_) => matches!(v, V::Reg(_)),
        OperandKind::RegList => matches!(v, V::RegList(_)),
        // `{v0}` and `{}` parse as lists but smali's register_range accepts them.
        OperandKind::RegRange => {
            matches!(v, V::RegRange(..)) || matches!(v, V::RegList(r) if r.len() <= 1)
        }
        OperandKind::Label => matches!(v, V::Label(_)),
        OperandKind::Lit(l) => match v {
            V::Int(_) | V::Char(_) => true,
            V::Float | V::Bool => matches!(
                l,
                LitKind::Bits32 | LitKind::Bits64 | LitKind::High16 | LitKind::WideHigh16
            ),
            _ => false,
        },
        OperandKind::Ref(RefKind::String) => matches!(v, V::Str),
        OperandKind::Ref(RefKind::Type) => matches!(v, V::Type(_)),
        OperandKind::Ref(RefKind::Field) => matches!(v, V::FieldRef(_)),
        OperandKind::Ref(RefKind::Method) => matches!(v, V::MethodRef(_)),
        OperandKind::Proto => matches!(v, V::Proto(_)),
        OperandKind::Ref(_) | OperandKind::Odex => true,
    }
}

fn lit_value(v: &OperandValue) -> Option<Option<i128>> {
    match v {
        OperandValue::Int(i) => Some(*i),
        OperandValue::Char(c) => Some(Some(i128::from(*c))),
        _ => None,
    }
}

/// Accepted literal range per format, as verified against smali 3.0.10.
fn lit_range(kind: LitKind) -> (i128, i128) {
    match kind {
        LitKind::Bits4 => (-8, 7),
        LitKind::Bits8 => (-128, 127),
        LitKind::Bits16 => (-32768, 65535),
        LitKind::Bits32 | LitKind::High16 => (i128::from(i32::MIN), i128::from(u32::MAX)),
        LitKind::Bits64 | LitKind::WideHigh16 => (i128::from(i64::MIN), i128::from(u64::MAX)),
    }
}

fn reg_name(r: &Reg) -> String {
    match r.kind {
        smali_syntax::model::RegKind::V => format!("v{}", r.num),
        smali_syntax::model::RegKind::P => format!("p{}", r.num),
    }
}

fn check_reg(s: &mut Sink, frame: &Frame, r: &Reg, width: u8, wide: bool) {
    if r.kind == smali_syntax::model::RegKind::P && r.num >= frame.params {
        s.push(
            "S202",
            r.span,
            format!(
                "p{} is out of range; this method has {} parameter registers",
                r.num, frame.params
            ),
        );
    }
    let Some(v) = frame.v_number(r) else {
        return;
    };
    if let Some(t) = frame.total {
        if r.kind == smali_syntax::model::RegKind::V && v >= t {
            s.push(
                "S201",
                r.span,
                format!("v{v} is out of range; this method has {t} registers"),
            );
        } else if wide && v + 1 >= t && v < t {
            s.push(
                "S203",
                r.span,
                format!(
                    "wide pair {}/v{} is out of range; this method has {t} registers",
                    reg_name(r),
                    v + 1
                ),
            );
        }
    }
    let max = (1u64 << width) - 1;
    if v > max {
        let mapped = if r.kind == smali_syntax::model::RegKind::P {
            format!(" ({} is v{v})", reg_name(r))
        } else {
            String::new()
        };
        s.push(
            "S204",
            r.span,
            format!("register{mapped} does not fit a {width}-bit operand (max v{max})"),
        );
    }
}

fn instruction(s: &mut Sink, ctx: &Context, frame: &Frame, insn: &Instruction, op: &Opcode) {
    let ops = &insn.operands;
    if ops.len() != op.operands.len()
        || !op
            .operands
            .iter()
            .zip(ops)
            .all(|(k, o)| operand_matches(*k, &o.value))
    {
        let expected = op.summary();
        s.push(
            "S210",
            insn.span,
            format!("wrong operands for {}; expected `{expected}`", op.name),
        );
        return;
    }
    let mut reg_index = 0usize;
    let mut invoke_regs: Option<u64> = None;
    for (kind, o) in op.operands.iter().zip(ops) {
        match (kind, &o.value) {
            (OperandKind::Reg(w), OperandValue::Reg(r)) => {
                check_reg(s, frame, r, *w, op.is_wide_reg(reg_index));
                reg_index += 1;
            }
            (OperandKind::RegList, OperandValue::RegList(regs)) => {
                if regs.len() > 5 {
                    s.push(
                        "S220",
                        o.span,
                        format!(
                            "{} registers in a non-range invoke; the maximum is 5, use /range",
                            regs.len()
                        ),
                    );
                }
                for r in regs {
                    check_reg(s, frame, r, 4, false);
                }
                invoke_regs = Some(regs.len() as u64);
            }
            (OperandKind::RegRange, OperandValue::RegRange(a, b)) => {
                for r in [a, b].into_iter().flatten() {
                    check_reg(s, frame, r, 16, false);
                }
                if let (Some(a), Some(b)) = (a, b) {
                    match (frame.v_number(a), frame.v_number(b)) {
                        (Some(x), Some(y)) if y < x => s.push(
                            "S221",
                            o.span,
                            "register range must list the lower register first",
                        ),
                        (Some(x), Some(y)) => invoke_regs = Some(y - x + 1),
                        _ => {}
                    }
                }
            }
            (OperandKind::RegRange, OperandValue::RegList(regs)) => {
                for r in regs {
                    check_reg(s, frame, r, 16, false);
                }
                invoke_regs = Some(regs.len() as u64);
            }
            (OperandKind::Lit(l), v) => literal(s, *l, o.span, v),
            (OperandKind::Ref(RefKind::Type), OperandValue::Type(t)) => {
                let named = Named {
                    text: t.clone(),
                    span: o.span,
                };
                s.check_type(&named, false);
            }
            (OperandKind::Ref(RefKind::Method), OperandValue::MethodRef(t)) => {
                match parse_method_ref(t) {
                    Err(e) => s.desc_error(o.span, &e),
                    Ok(r) => {
                        if op.is_invoke() && !op.name.starts_with("invoke-polymorphic") {
                            let want = u64::from(!op.name.starts_with("invoke-static"))
                                + u64::from(r.proto.param_slots());
                            if let Some(have) = invoke_regs.filter(|h| *h != want) {
                                s.push(
                                    "S222",
                                    insn.span,
                                    format!(
                                        "{} passes {have} registers but the signature needs {want}",
                                        op.name
                                    ),
                                );
                            }
                        }
                        resolve_method(s, ctx, op, &r, o.span);
                    }
                }
            }
            (OperandKind::Ref(RefKind::Field), OperandValue::FieldRef(t)) => {
                match parse_field_ref(t) {
                    Err(e) => s.desc_error(o.span, &e),
                    Ok(r) => resolve_field(s, ctx, op, &r, o.span),
                }
            }
            (OperandKind::Proto, OperandValue::Proto(t)) => {
                if let Err(e) = parse_proto(t) {
                    s.desc_error(o.span, &e);
                }
            }
            _ => {}
        }
    }
}

fn literal(s: &mut Sink, kind: LitKind, span: Span, v: &OperandValue) {
    let Some(value) = lit_value(v) else {
        return;
    };
    let (lo, hi) = lit_range(kind);
    let Some(value) = value.filter(|x| (lo..=hi).contains(x)) else {
        let shown = value.map_or_else(|| s.doc.span_text(span).to_string(), |x| x.to_string());
        s.push(
            "S211",
            span,
            format!("literal {shown} is out of range {lo}..{hi} for this instruction"),
        );
        return;
    };
    let low_bits = match kind {
        LitKind::High16 => 16,
        LitKind::WideHigh16 => 48,
        _ => return,
    };
    if value & ((1i128 << low_bits) - 1) != 0 {
        s.push(
            "S212",
            span,
            format!("high16 literal must have its low {low_bits} bits zero"),
        );
    }
}

fn resolve_method(
    s: &mut Sink,
    ctx: &Context,
    op: &Opcode,
    r: &smali_syntax::descriptor::MethodRef,
    span: Span,
) {
    let Some(index) = ctx.index else { return };
    let owner = r.class.to_string();
    match index.resolve_method(&owner, &r.name, &r.proto.to_string()) {
        Lookup::Missing => s.push(
            "S302",
            span,
            format!(
                "method {}{} not found in {owner} or its ancestors",
                r.name, r.proto
            ),
        ),
        Lookup::Found { owner: _, member } => {
            if !op.is_invoke()
                || op.name.starts_with("invoke-polymorphic")
                || op.name.starts_with("invoke-custom")
            {
                return;
            }
            let kind = op.name.split('/').next().unwrap_or("");
            let is_static = member.flags & acc::STATIC != 0;
            let is_private = member.flags & acc::PRIVATE != 0;
            let is_init = member.name == "<init>";
            // Interface-ness is judged on the class written at the call site: Object methods are
            // invoked through interfaces, and default methods through classes.
            let written_interface = index.class(&owner).map(|c| c.is_interface());
            let want: Option<&str> = match kind {
                "invoke-static" if !is_static => Some("method is not static"),
                "invoke-direct" if !(is_private || is_init) => {
                    Some("invoke-direct needs a private method or a constructor")
                }
                // Private targets are allowed: D8 emits invoke-virtual for nest-mate access.
                "invoke-virtual" if is_static || written_interface == Some(true) => {
                    Some("invoke-virtual needs a non-static method of a class")
                }
                "invoke-interface" if written_interface == Some(false) => {
                    Some("method owner is not an interface")
                }
                "invoke-super" if is_static => Some("invoke-super on a static method"),
                _ => None,
            };
            if let Some(why) = want {
                s.push(
                    "S304",
                    span,
                    format!("{kind} does not match {}: {why}", member.name),
                );
            }
        }
        Lookup::Unknown => {}
    }
}

fn resolve_field(
    s: &mut Sink,
    ctx: &Context,
    op: &Opcode,
    r: &smali_syntax::descriptor::FieldRef,
    span: Span,
) {
    let Some(index) = ctx.index else { return };
    let owner = r.class.to_string();
    match index.resolve_field(&owner, &r.name, &r.ty.to_string()) {
        Lookup::Missing => s.push(
            "S303",
            span,
            format!(
                "field {}:{} not found in {owner} or its ancestors",
                r.name, r.ty
            ),
        ),
        Lookup::Found { member, .. } => {
            let static_op = op.has(flags::STATIC_FIELD_ACCESSOR) || op.name.starts_with('s');
            if static_op != member.is_static() {
                let what = if member.is_static() {
                    "static"
                } else {
                    "instance"
                };
                s.push(
                    "S305",
                    span,
                    format!("{} used on {what} field {}", op.name, member.name),
                );
            }
        }
        Lookup::Unknown => {}
    }
}
