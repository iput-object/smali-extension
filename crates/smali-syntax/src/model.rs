//! Typed view over the tree-sitter CST. Built once per parse; analysis never touches nodes.

use tree_sitter::Node;

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Default,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Span { start, end }
    }

    pub fn of(node: Node) -> Self {
        Span::new(node.start_byte(), node.end_byte())
    }

    pub fn contains(&self, offset: usize) -> bool {
        self.start <= offset && offset <= self.end
    }

    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Named {
    pub text: String,
    pub span: Span,
}

#[derive(Debug, Clone, Default)]
pub struct ClassFile {
    pub class: Option<ClassHeader>,
    pub super_class: Option<Named>,
    pub source: Option<Named>,
    pub interfaces: Vec<Named>,
    pub fields: Vec<Field>,
    pub methods: Vec<Method>,
    pub annotations: Vec<Annotation>,
    pub syntax_errors: Vec<SyntaxError>,
    /// Every class descriptor appearing in the file (types, refs, annotations), for indexing references.
    pub type_refs: Vec<Named>,
}

#[derive(Debug, Clone)]
pub struct ClassHeader {
    /// Descriptor text, e.g. `Lcom/foo/Bar;`.
    pub name: Named,
    pub flags: Vec<String>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Annotation {
    pub ty: Named,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Field {
    pub name: Named,
    pub ty: Named,
    pub flags: Vec<String>,
    pub span: Span,
}

impl Field {
    pub fn is_static(&self) -> bool {
        self.flags.iter().any(|f| f == "static")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegDirectiveKind {
    Registers,
    Locals,
}

#[derive(Debug, Clone)]
pub struct RegDirective {
    pub kind: RegDirectiveKind,
    pub value: Option<u64>,
    pub span: Span,
    pub value_span: Span,
}

#[derive(Debug, Clone)]
pub struct Method {
    pub name: Named,
    /// Prototype text, e.g. `(IJ)V`.
    pub proto: Named,
    pub flags: Vec<String>,
    pub span: Span,
    /// The `.method ...` line.
    pub header: Span,
    pub has_end: bool,
    pub reg_directives: Vec<RegDirective>,
    pub items: Vec<Item>,
}

impl Method {
    pub fn has_flag(&self, f: &str) -> bool {
        self.flags.iter().any(|x| x == f)
    }

    pub fn is_static(&self) -> bool {
        self.has_flag("static")
    }

    pub fn instructions(&self) -> impl Iterator<Item = &Instruction> {
        self.items.iter().filter_map(|i| match i {
            Item::Insn(insn) => Some(insn),
            _ => None,
        })
    }

    pub fn label_defs(&self) -> impl Iterator<Item = &Named> {
        self.items.iter().filter_map(|i| match i {
            Item::Label(l) => Some(l),
            _ => None,
        })
    }

    /// Every label reference in the method: branch operands, catch ranges, switch targets.
    pub fn label_refs(&self) -> Vec<&Named> {
        let mut out = Vec::new();
        for item in &self.items {
            match item {
                Item::Insn(insn) => {
                    for op in &insn.operands {
                        if let OperandValue::Label(l) = &op.value {
                            out.push(l);
                        }
                    }
                }
                Item::Catch(c) => {
                    out.extend([&c.start, &c.end, &c.handler]);
                }
                Item::PackedSwitch(d) | Item::SparseSwitch(d) => out.extend(d.targets.iter()),
                _ => {}
            }
        }
        out
    }
}

#[derive(Debug, Clone)]
pub enum Item {
    Label(Named),
    Insn(Instruction),
    Catch(Catch),
    PackedSwitch(DataBlock),
    SparseSwitch(DataBlock),
    ArrayData(DataBlock),
    /// `.line`, `.local`, `.param`, `.prologue`, annotations, ... Registers they mention are kept.
    Directive {
        kind: String,
        span: Span,
        regs: Vec<Reg>,
    },
}

#[derive(Debug, Clone)]
pub struct Catch {
    pub exception: Option<Named>,
    pub start: Named,
    pub end: Named,
    pub handler: Named,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct DataBlock {
    pub span: Span,
    pub targets: Vec<Named>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RegKind {
    V,
    P,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reg {
    pub kind: RegKind,
    pub num: u32,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Instruction {
    pub opcode: Named,
    pub operands: Vec<Operand>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Operand {
    pub value: OperandValue,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum OperandValue {
    Reg(Reg),
    RegList(Vec<Reg>),
    RegRange(Option<Reg>, Option<Reg>),
    Label(Named),
    /// Integral literal. `None` when it does not fit in 128 bits or is malformed.
    Int(Option<i128>),
    Float,
    Char(u32),
    Str,
    Bool,
    Null,
    Type(String),
    MethodRef(String),
    FieldRef(String),
    Proto(String),
    Other,
}

#[derive(Debug, Clone)]
pub struct SyntaxError {
    pub span: Span,
    /// Set for MISSING nodes: what the parser expected.
    pub missing: Option<String>,
}

/// Parse a smali integral literal: optional sign, `0x` hex, leading-0 octal or decimal,
/// optional `L`/`S`/`T` suffix. Matches smali's LiteralTools radix rules.
pub fn parse_int(text: &str) -> Option<i128> {
    let mut s = text.trim();
    if let Some(stripped) = s.strip_suffix(['L', 'l', 'S', 's', 'T', 't']) {
        s = stripped;
    }
    let (neg, body) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let body = body.replace('_', "");
    let (radix, digits) = if let Some(h) = body.strip_prefix("0x").or(body.strip_prefix("0X")) {
        (16, h.to_string())
    } else if body.len() > 1 && body.starts_with('0') {
        (8, body[1..].to_string())
    } else {
        (10, body)
    };
    if digits.is_empty() || digits.len() > 40 {
        return None;
    }
    let v = i128::from_str_radix(&digits, radix).ok()?;
    Some(if neg { -v } else { v })
}

fn text<'a>(node: Node, src: &'a str) -> &'a str {
    src.get(node.byte_range()).unwrap_or("")
}

fn named(node: Node, src: &str) -> Named {
    Named {
        text: text(node, src).to_string(),
        span: Span::of(node),
    }
}

fn parse_reg(node: Node, src: &str) -> Option<Reg> {
    let t = text(node, src);
    let kind = match t.as_bytes().first()? {
        b'v' => RegKind::V,
        b'p' => RegKind::P,
        _ => return None,
    };
    let num = t.get(1..)?.parse().ok()?;
    Some(Reg {
        kind,
        num,
        span: Span::of(node),
    })
}

fn children(node: Node) -> Vec<Node> {
    let mut cursor = node.walk();
    node.children(&mut cursor).collect()
}

fn named_children(node: Node) -> Vec<Node> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

struct Builder<'a> {
    src: &'a str,
    out: ClassFile,
}

/// Build the typed model from a parsed tree.
pub fn build(root: Node, src: &str) -> ClassFile {
    let mut b = Builder {
        src,
        out: ClassFile::default(),
    };
    b.collect_errors(root);
    b.collect_type_refs(root);
    for child in named_children(root) {
        b.top_level(child);
    }
    b.out
}

impl<'a> Builder<'a> {
    fn collect_errors(&mut self, root: Node) {
        if !root.has_error() {
            return;
        }
        let mut stack = vec![root];
        while let Some(n) = stack.pop() {
            if n.is_error() {
                self.out.syntax_errors.push(SyntaxError {
                    span: Span::of(n),
                    missing: None,
                });
                continue;
            }
            if n.is_missing() {
                // Report right after the last real token, not after trailing comments/EOF.
                let mut at = n.start_byte().min(self.src.len());
                let before = self.src.get(..at).unwrap_or("");
                let mut trimmed = before.trim_end();
                while let Some(line_start) = trimmed.rfind('\n').map(|i| i + 1).or(Some(0)) {
                    let line = &trimmed[line_start..];
                    if line.trim_start().starts_with('#') && line_start > 0 {
                        trimmed = trimmed[..line_start].trim_end();
                    } else {
                        break;
                    }
                }
                at = at.min(trimmed.len());
                self.out.syntax_errors.push(SyntaxError {
                    span: Span::new(at, at),
                    missing: Some(n.kind().to_string()),
                });
                continue;
            }
            if n.has_error() {
                stack.extend(children(n));
            }
        }
        self.out.syntax_errors.sort_by_key(|e| e.span);
    }

    fn collect_type_refs(&mut self, root: Node) {
        let mut stack = vec![root];
        while let Some(n) = stack.pop() {
            if n.kind() == "class_identifier" {
                self.out.type_refs.push(named(n, self.src));
                continue;
            }
            stack.extend(named_children(n));
        }
        self.out.type_refs.sort_by_key(|n| n.span);
    }

    fn flags(&self, node: Node) -> Vec<String> {
        children(node)
            .into_iter()
            .filter(|c| c.kind() == "access_modifier" || c.kind() == "constructor")
            .map(|c| text(c, self.src).to_string())
            .collect()
    }

    fn top_level(&mut self, node: Node) {
        match node.kind() {
            "class_directive" => {
                let Some(id) = named_children(node)
                    .into_iter()
                    .find(|c| c.kind() == "class_identifier")
                else {
                    return;
                };
                let flags = named_children(node)
                    .into_iter()
                    .find(|c| c.kind() == "access_modifiers")
                    .map(|m| self.flags(m))
                    .unwrap_or_default();
                if self.out.class.is_none() {
                    self.out.class = Some(ClassHeader {
                        name: named(id, self.src),
                        flags,
                        span: Span::of(node),
                    });
                }
            }
            "super_directive" => {
                if let Some(id) = node.named_child(0) {
                    self.out.super_class = Some(named(id, self.src));
                }
            }
            "source_directive" => {
                if let Some(s) = node.named_child(0) {
                    self.out.source = Some(named(s, self.src));
                }
            }
            "implements_directive" => {
                if let Some(id) = node.named_child(0) {
                    self.out.interfaces.push(named(id, self.src));
                }
            }
            "annotation_directive" => {
                if let Some(a) = self.annotation(node) {
                    self.out.annotations.push(a);
                }
            }
            "field_definition" => self.field(node),
            "method_definition" => self.method(node),
            _ => {}
        }
    }

    fn annotation(&self, node: Node) -> Option<Annotation> {
        let ty = named_children(node)
            .into_iter()
            .find(|c| c.kind() == "class_identifier")?;
        Some(Annotation {
            ty: named(ty, self.src),
            span: Span::of(node),
        })
    }

    fn field(&mut self, node: Node) {
        let mut name = None;
        let mut ty = None;
        let mut flags = Vec::new();
        for c in named_children(node) {
            match c.kind() {
                "access_modifiers" => flags = self.flags(c),
                "field_identifier" if name.is_none() => {
                    // Declarations lex the name together with its ':' (see grammar VENDORED.md).
                    let mut n = named(c, self.src);
                    if n.text.ends_with(':') {
                        n.text.pop();
                        n.span.end -= 1;
                    }
                    name = Some(n);
                }
                "field_type" if ty.is_none() => ty = Some(named(c, self.src)),
                _ => {}
            }
        }
        if let (Some(name), Some(ty)) = (name, ty) {
            self.out.fields.push(Field {
                name,
                ty,
                flags,
                span: Span::of(node),
            });
        }
    }

    fn method(&mut self, node: Node) {
        let Some(sig) = named_children(node)
            .into_iter()
            .find(|c| c.kind() == "method_signature")
        else {
            return;
        };
        // Split at '(' rather than trusting nodes: `-name` parses as two method_identifiers.
        let sig_text = text(sig, self.src);
        let Some(paren) = sig_text.find('(') else {
            return;
        };
        let name_start = sig.start_byte();
        let proto_start = name_start + paren;
        let name = Named {
            text: sig_text[..paren].to_string(),
            span: Span::new(name_start, proto_start),
        };
        let proto = Named {
            text: sig_text[paren..].to_string(),
            span: Span::new(proto_start, sig.end_byte()),
        };
        let has_end = children(node)
            .iter()
            .any(|c| c.kind() == ".end method" && !c.is_missing());
        let mut m = Method {
            name,
            proto,
            flags: self.flags(node),
            span: Span::of(node),
            header: Span::new(node.start_byte(), sig.end_byte()),
            has_end,
            reg_directives: Vec::new(),
            items: Vec::new(),
        };
        for c in named_children(node) {
            self.statement(c, &mut m);
        }
        self.out.methods.push(m);
    }

    fn statement(&self, node: Node, m: &mut Method) {
        let src = self.src;
        match node.kind() {
            "label" => {
                let mut l = named(node, src);
                l.text = l.text.trim_start_matches(':').to_string();
                m.items.push(Item::Label(l));
            }
            "jmp_label" => {
                let mut l = named(node, src);
                l.text = l.text.trim_end_matches(':').to_string();
                m.items.push(Item::Label(l));
            }
            "expression" => m.items.push(Item::Insn(self.instruction(node))),
            "registers_directive" | "locals_directive" => {
                let num = node.named_child(0);
                m.reg_directives.push(RegDirective {
                    kind: if node.kind() == "locals_directive" {
                        RegDirectiveKind::Locals
                    } else {
                        RegDirectiveKind::Registers
                    },
                    value: num
                        .and_then(|n| parse_int(text(n, src)))
                        .and_then(|v| u64::try_from(v).ok()),
                    span: Span::of(node),
                    value_span: num.map(Span::of).unwrap_or(Span::of(node)),
                });
            }
            "catch_directive" | "catchall_directive" => {
                let kids = named_children(node);
                let exception = kids
                    .iter()
                    .find(|c| c.kind() == "class_identifier")
                    .map(|c| named(*c, src));
                let labels: Vec<Named> = kids
                    .iter()
                    .filter(|c| c.kind() == "label" || c.kind() == "jmp_label")
                    .map(|c| label_ref(*c, src))
                    .collect();
                if let [start, end, handler] = labels.as_slice() {
                    m.items.push(Item::Catch(Catch {
                        exception,
                        start: start.clone(),
                        end: end.clone(),
                        handler: handler.clone(),
                        span: Span::of(node),
                    }));
                }
            }
            "packed_switch_directive" | "sparse_switch_directive" | "array_data_directive" => {
                let block = DataBlock {
                    span: Span::of(node),
                    targets: named_children(node)
                        .into_iter()
                        .filter(|c| c.kind() == "label" || c.kind() == "jmp_label")
                        .map(|c| label_ref(c, src))
                        .collect(),
                };
                m.items.push(match node.kind() {
                    "packed_switch_directive" => Item::PackedSwitch(block),
                    "sparse_switch_directive" => Item::SparseSwitch(block),
                    _ => Item::ArrayData(block),
                });
            }
            "comment" | "method_signature" | "access_modifier" | "ERROR" => {}
            kind => {
                let mut regs = Vec::new();
                let mut stack = vec![node];
                while let Some(n) = stack.pop() {
                    if matches!(n.kind(), "variable" | "parameter") {
                        regs.extend(parse_reg(n, src));
                    } else {
                        stack.extend(named_children(n));
                    }
                }
                m.items.push(Item::Directive {
                    kind: kind.to_string(),
                    span: Span::of(node),
                    regs,
                });
            }
        }
    }

    fn instruction(&self, node: Node) -> Instruction {
        let src = self.src;
        let kids = named_children(node);
        let opcode = kids
            .iter()
            .find(|c| c.kind() == "opcode")
            .map(|c| named(*c, src))
            .unwrap_or(Named {
                text: String::new(),
                span: Span::new(node.start_byte(), node.start_byte()),
            });
        let operands = kids
            .iter()
            .filter(|c| c.kind() != "opcode" && c.kind() != "comment")
            .map(|c| self.operand(*c))
            .collect();
        // The expression node includes the trailing newline; keep the span on one line.
        let end = src
            .get(node.start_byte()..node.end_byte())
            .map(|t| node.start_byte() + t.trim_end().len())
            .unwrap_or(node.end_byte());
        Instruction {
            opcode,
            operands,
            span: Span::new(node.start_byte(), end),
        }
    }

    fn operand(&self, node: Node) -> Operand {
        let src = self.src;
        let span = Span::of(node);
        let value = match node.kind() {
            "variable" | "parameter" => parse_reg(node, src)
                .map(OperandValue::Reg)
                .unwrap_or(OperandValue::Other),
            "list" => {
                let regs: Vec<Reg> = named_children(node)
                    .into_iter()
                    .filter_map(|c| parse_reg(c, src))
                    .collect();
                OperandValue::RegList(regs)
            }
            "range" => OperandValue::RegRange(
                node.child_by_field_name("start")
                    .and_then(|n| parse_reg(n, src)),
                node.child_by_field_name("end")
                    .and_then(|n| parse_reg(n, src)),
            ),
            "label" | "jmp_label" => OperandValue::Label(label_ref(node, src)),
            "number" => OperandValue::Int(parse_int(text(node, src))),
            "float" | "NaN" | "Infinity" => OperandValue::Float,
            "character" => OperandValue::Char(char_value(text(node, src))),
            "string" => OperandValue::Str,
            "boolean" => OperandValue::Bool,
            "null" => OperandValue::Null,
            "class_identifier" | "array_type" | "primitive_type" => {
                OperandValue::Type(text(node, src).to_string())
            }
            "body" => {
                let t = text(node, src).to_string();
                let first = node.named_child(0).map(|c| c.kind());
                match first {
                    Some("full_method_signature") => OperandValue::MethodRef(t),
                    Some("method_signature") => OperandValue::Proto(t),
                    Some("class_identifier" | "array_type") => OperandValue::FieldRef(t),
                    _ => OperandValue::Other,
                }
            }
            _ => OperandValue::Other,
        };
        Operand { value, span }
    }
}

fn label_ref(node: Node, src: &str) -> Named {
    let mut l = named(node, src);
    l.text = l
        .text
        .trim_start_matches(':')
        .trim_end_matches(':')
        .to_string();
    l
}

fn char_value(t: &str) -> u32 {
    let inner = t.trim_start_matches('\'').trim_end_matches('\'');
    let mut chars = inner.chars();
    match (chars.next(), chars.next()) {
        (Some('\\'), Some('u')) => u32::from_str_radix(chars.as_str(), 16).unwrap_or(0),
        (Some('\\'), Some(c)) => match c {
            'n' => 10,
            't' => 9,
            'r' => 13,
            'b' => 8,
            'f' => 12,
            '0' => 0,
            c => c as u32,
        },
        (Some(c), _) => c as u32,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int_literals() {
        assert_eq!(parse_int("0x10"), Some(16));
        assert_eq!(parse_int("-0x8"), Some(-8));
        assert_eq!(parse_int("017"), Some(15));
        assert_eq!(parse_int("0"), Some(0));
        assert_eq!(parse_int("0x7fL"), Some(127));
        assert_eq!(
            parse_int("0xffffffffffffffffL"),
            Some(0xffff_ffff_ffff_ffff)
        );
        assert_eq!(parse_int("12t"), Some(12));
        assert_eq!(parse_int("-"), None);
    }
}
