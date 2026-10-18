//! Conversion between analysis types and LSP types. Positions go through smali-syntax's line index.

use smali_analysis::{Diagnostic, Severity};
use smali_syntax::{Document, Encoding, LineCol, Span};
use tower_lsp_server::ls_types as lsp;

pub fn position(doc: &Document, offset: usize, enc: Encoding) -> lsp::Position {
    let p = doc.position(offset, enc);
    lsp::Position::new(p.line, p.col)
}

pub fn offset(doc: &Document, pos: lsp::Position, enc: Encoding) -> usize {
    doc.offset(
        LineCol {
            line: pos.line,
            col: pos.character,
        },
        enc,
    )
}

pub fn range(doc: &Document, span: Span, enc: Encoding) -> lsp::Range {
    lsp::Range::new(position(doc, span.start, enc), position(doc, span.end, enc))
}

pub fn span(doc: &Document, r: lsp::Range, enc: Encoding) -> Span {
    Span::new(offset(doc, r.start, enc), offset(doc, r.end, enc))
}

pub fn severity(s: Severity) -> lsp::DiagnosticSeverity {
    match s {
        Severity::Error => lsp::DiagnosticSeverity::ERROR,
        Severity::Warning => lsp::DiagnosticSeverity::WARNING,
        Severity::Info => lsp::DiagnosticSeverity::INFORMATION,
        Severity::Hint => lsp::DiagnosticSeverity::HINT,
    }
}

pub fn diagnostic(doc: &Document, d: &Diagnostic, sev: Severity, enc: Encoding) -> lsp::Diagnostic {
    lsp::Diagnostic {
        range: range(doc, d.span, enc),
        severity: Some(severity(sev)),
        code: Some(lsp::NumberOrString::String(d.code.to_string())),
        source: Some("smali".into()),
        message: d.message.clone(),
        tags: (d.code == "S122").then(|| vec![lsp::DiagnosticTag::UNNECESSARY]),
        ..Default::default()
    }
}

pub fn text_edit(doc: &Document, e: &smali_analysis::Edit, enc: Encoding) -> lsp::TextEdit {
    lsp::TextEdit {
        range: range(doc, e.span, enc),
        new_text: e.new_text.clone(),
    }
}

/// Range in a file that is not open: read it to convert byte offsets.
pub fn range_in_file(path: &std::path::Path, span: Span, enc: Encoding) -> lsp::Range {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let lines = smali_syntax::LineIndex::new(&text);
    let a = lines.position(&text, span.start, enc);
    let b = lines.position(&text, span.end, enc);
    lsp::Range::new(
        lsp::Position::new(a.line, a.col),
        lsp::Position::new(b.line, b.col),
    )
}
