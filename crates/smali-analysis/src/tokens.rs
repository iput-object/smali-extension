//! Semantic tokens: resolved vs unresolved classes, p vs v registers, labels, members.

use smali_syntax::descriptor::{parse_field_ref, parse_method_ref};
use smali_syntax::model::{Item, OperandValue, Reg, RegKind};
use smali_syntax::{Document, Span};

use crate::Context;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenType {
    Class,
    Method,
    Property,
    Parameter,
    Variable,
    Label,
}

pub const TOKEN_TYPES: &[&str] = &[
    "class",
    "method",
    "property",
    "parameter",
    "variable",
    "label",
];
pub const TOKEN_MODIFIERS: &[&str] = &["declaration", "defaultLibrary", "unresolved"];
pub const MOD_DECLARATION: u32 = 1;
pub const MOD_LIBRARY: u32 = 2;
pub const MOD_UNRESOLVED: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub span: Span,
    pub ty: TokenType,
    pub modifiers: u32,
}

fn reg(out: &mut Vec<Token>, r: &Reg) {
    out.push(Token {
        span: r.span,
        ty: if r.kind == RegKind::P {
            TokenType::Parameter
        } else {
            TokenType::Variable
        },
        modifiers: 0,
    });
}

/// Tokens sorted by position, non-overlapping.
pub fn semantic_tokens(doc: &Document, ctx: &Context) -> Vec<Token> {
    let model = &doc.model;
    let mut out = Vec::new();
    let class_mod = |desc: &str| -> u32 {
        match ctx.index.and_then(|i| i.class(desc)) {
            Some(c) if c.origin != smali_index::Origin::Workspace => MOD_LIBRARY,
            Some(_) => 0,
            None if ctx.index.is_some() => MOD_UNRESOLVED,
            None => 0,
        }
    };
    for t in &model.type_refs {
        let decl = model.class.as_ref().is_some_and(|c| c.name.span == t.span);
        out.push(Token {
            span: t.span,
            ty: TokenType::Class,
            modifiers: class_mod(&t.text) | if decl { MOD_DECLARATION } else { 0 },
        });
    }
    for f in &model.fields {
        out.push(Token {
            span: f.name.span,
            ty: TokenType::Property,
            modifiers: MOD_DECLARATION,
        });
    }
    for m in &model.methods {
        out.push(Token {
            span: m.name.span,
            ty: TokenType::Method,
            modifiers: MOD_DECLARATION,
        });
        for item in &m.items {
            match item {
                Item::Label(l) => out.push(Token {
                    span: l.span,
                    ty: TokenType::Label,
                    modifiers: MOD_DECLARATION,
                }),
                Item::Directive { regs, .. } => regs.iter().for_each(|r| reg(&mut out, r)),
                Item::Catch(c) => {
                    for l in [&c.start, &c.end, &c.handler] {
                        out.push(Token {
                            span: l.span,
                            ty: TokenType::Label,
                            modifiers: 0,
                        });
                    }
                }
                Item::PackedSwitch(d) | Item::SparseSwitch(d) => {
                    for l in &d.targets {
                        out.push(Token {
                            span: l.span,
                            ty: TokenType::Label,
                            modifiers: 0,
                        });
                    }
                }
                Item::Insn(insn) => {
                    for o in &insn.operands {
                        match &o.value {
                            OperandValue::Reg(r) => reg(&mut out, r),
                            OperandValue::RegList(rs) => rs.iter().for_each(|r| reg(&mut out, r)),
                            OperandValue::RegRange(a, b) => {
                                [a, b].into_iter().flatten().for_each(|r| reg(&mut out, r))
                            }
                            OperandValue::Label(l) => out.push(Token {
                                span: l.span,
                                ty: TokenType::Label,
                                modifiers: 0,
                            }),
                            OperandValue::MethodRef(t) => {
                                if let (Some(arrow), Ok(r)) = (t.find("->"), parse_method_ref(t)) {
                                    let start = o.span.start + arrow + 2;
                                    out.push(Token {
                                        span: Span::new(start, start + r.name.len()),
                                        ty: TokenType::Method,
                                        modifiers: 0,
                                    });
                                }
                            }
                            OperandValue::FieldRef(t) => {
                                if let (Some(arrow), Ok(r)) = (t.find("->"), parse_field_ref(t)) {
                                    let start = o.span.start + arrow + 2;
                                    out.push(Token {
                                        span: Span::new(start, start + r.name.len()),
                                        ty: TokenType::Property,
                                        modifiers: 0,
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }
    out.sort_by_key(|t| (t.span.start, std::cmp::Reverse(t.span.end)));
    let mut last_end = 0;
    out.retain(|t| {
        let keep = t.span.start >= last_end && !t.span.is_empty();
        if keep {
            last_end = t.span.end;
        }
        keep
    });
    out
}
