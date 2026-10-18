//! Parsing, typed CST views, descriptors and position conversion for Smali.

pub mod descriptor;
pub mod line_index;
pub mod model;

use std::sync::Arc;

pub use line_index::{Encoding, LineCol, LineIndex};
pub use model::{ClassFile, Span};
use tree_sitter::{InputEdit, Parser, Point, Tree};

pub fn language() -> tree_sitter::Language {
    tree_sitter_smali::language()
}

fn new_parser() -> Option<Parser> {
    let mut p = Parser::new();
    p.set_language(language()).ok()?;
    Some(p)
}

/// A parsed file: text, tree, line index and typed model. Cheap to clone (shared).
#[derive(Clone)]
pub struct Document {
    pub text: Arc<str>,
    pub tree: Option<Tree>,
    pub lines: Arc<LineIndex>,
    pub model: Arc<ClassFile>,
}

impl std::fmt::Debug for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Document")
            .field("len", &self.text.len())
            .finish()
    }
}

/// A text change in byte offsets of the old text.
#[derive(Debug, Clone)]
pub struct TextEdit {
    pub start: usize,
    pub end: usize,
    pub new_text: String,
}

fn point_at(text: &str, lines: &LineIndex, offset: usize) -> Point {
    let row = lines.line_of(offset);
    let _ = text;
    Point::new(row as usize, offset - lines.line_start(row))
}

impl Document {
    pub fn parse(text: &str) -> Document {
        Self::build(Arc::from(text), None)
    }

    fn build(text: Arc<str>, old: Option<&Tree>) -> Document {
        let tree = new_parser().and_then(|mut p| p.parse(text.as_bytes(), old));
        let lines = Arc::new(LineIndex::new(&text));
        let model = Arc::new(
            tree.as_ref()
                .map(|t| model::build(t.root_node(), &text))
                .unwrap_or_default(),
        );
        Document {
            text,
            tree,
            lines,
            model,
        }
    }

    /// Apply edits in order (each in the coordinates of the text after the previous one),
    /// reusing the old tree for incremental parsing.
    pub fn apply_edits(&self, edits: &[TextEdit]) -> Document {
        let mut text = self.text.to_string();
        let mut tree = self.tree.clone();
        let mut lines = LineIndex::new(&text);
        for e in edits {
            let start = e.start.min(text.len());
            let end = e.end.clamp(start, text.len());
            if !text.is_char_boundary(start) || !text.is_char_boundary(end) {
                tree = None;
                continue;
            }
            let start_position = point_at(&text, &lines, start);
            let old_end_position = point_at(&text, &lines, end);
            text.replace_range(start..end, &e.new_text);
            lines = LineIndex::new(&text);
            let new_end = start + e.new_text.len();
            if let Some(t) = tree.as_mut() {
                t.edit(&InputEdit {
                    start_byte: start,
                    old_end_byte: end,
                    new_end_byte: new_end,
                    start_position,
                    old_end_position,
                    new_end_position: point_at(&text, &lines, new_end),
                });
            }
        }
        Self::build(Arc::from(text), tree.as_ref())
    }

    pub fn span_text(&self, span: Span) -> &str {
        self.text.get(span.start..span.end).unwrap_or("")
    }

    pub fn position(&self, offset: usize, enc: Encoding) -> LineCol {
        self.lines.position(&self.text, offset, enc)
    }

    pub fn offset(&self, pos: LineCol, enc: Encoding) -> usize {
        self.lines.offset(&self.text, pos, enc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::{Item, OperandValue};

    const SRC: &str = include_str!("../tests/sample.smali");

    #[test]
    fn model_of_sample() {
        let d = Document::parse(SRC);
        let m = &d.model;
        assert!(m.syntax_errors.is_empty(), "{:?}", m.syntax_errors);
        assert_eq!(
            m.class.as_ref().map(|c| c.name.text.as_str()),
            Some("Lcom/foo/Bar$Inner;")
        );
        assert_eq!(m.interfaces.len(), 1);
        assert_eq!(m.fields.len(), 2);
        assert_eq!(m.methods.len(), 3);
        let run = &m.methods[1];
        assert_eq!(run.name.text, "run");
        assert_eq!(run.proto.text, "(IJ)Z");
        assert!(run.has_end);
        assert_eq!(run.label_defs().count(), 4);
        let invoke = run
            .instructions()
            .find(|i| i.opcode.text == "invoke-static")
            .map(|i| i.operands.clone())
            .unwrap_or_default();
        assert!(matches!(&invoke[0].value, OperandValue::RegList(r) if r.len() == 2));
        assert!(
            matches!(&invoke[1].value, OperandValue::MethodRef(s) if s == "Lcom/foo/X;->m(IJ)V")
        );
        assert!(run.items.iter().any(|i| matches!(i, Item::Catch(_))));
        assert!(m.methods[2].reg_directives.is_empty());
        assert!(m.methods[2].has_flag("abstract"));
    }

    #[test]
    fn incremental_edit_matches_full_parse() {
        let d = Document::parse(SRC);
        let at = SRC.find("const/4 v0, 0x1").unwrap_or(0);
        let edited = d.apply_edits(&[
            TextEdit {
                start: at,
                end: at + "const/4".len(),
                new_text: "const/16".into(),
            },
            TextEdit {
                start: 0,
                end: 0,
                new_text: "# ünïcødé\n".into(),
            },
        ]);
        let full = Document::parse(&edited.text);
        let a = edited.tree.as_ref().map(|t| t.root_node().to_sexp());
        let b = full.tree.as_ref().map(|t| t.root_node().to_sexp());
        assert_eq!(a, b);
        assert!(edited.text.contains("const/16 v0, 0x1"));
    }

    #[test]
    fn missing_end_method() {
        let d = Document::parse(".class LA;\n.super LB;\n.method f()V\n    return-void\n");
        assert!(!d.model.syntax_errors.is_empty());
        assert!(d.model.methods.iter().all(|m| !m.has_end));
    }
}
