//! Pure analysis over a parsed document and the index: diagnostics, completion, hover,
//! navigation, code actions, semantic tokens. No I/O, no threads, no LSP types.

pub mod actions;
pub mod completion;
pub mod diagnostics;
pub mod format;
pub mod hover;
pub mod nav;
pub mod refactor;
pub mod regs;
pub mod tokens;

use smali_index::Index;
use smali_syntax::{Document, Span};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Error,
    Warning,
    Info,
    Hint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: &'static str,
    pub severity: Severity,
    pub span: Span,
    pub message: String,
}

/// Everything analysis needs besides the document.
#[derive(Clone, Copy, Default)]
pub struct Context<'a> {
    pub index: Option<&'a Index>,
    /// Class descriptor implied by the file path (for S110), if the file is under a root.
    pub expected_class: Option<&'a str>,
}

/// A text replacement in byte offsets of the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub span: Span,
    pub new_text: String,
}

/// Method enclosing `offset`, if any.
pub fn method_at(doc: &Document, offset: usize) -> Option<&smali_syntax::model::Method> {
    doc.model
        .methods
        .iter()
        .find(|m| m.span.start <= offset && offset <= m.span.end)
}
