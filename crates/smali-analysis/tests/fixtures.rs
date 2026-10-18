//! Fixture tests: `.smali` files with inline expectations.
//!
//! A line `#   ^^^ S211` marks columns on the nearest line above that is not itself an
//! expectation. Every fixture is analysed with an index built from all files in
//! `fixtures/lib` plus the fixture itself, and a hand-built `java/lang/Object`.

use std::path::{Path, PathBuf};

use smali_analysis::diagnostics::diagnostics;
use smali_analysis::Context;
use smali_index::{acc, scan, ClassInfo, Index, MemberInfo, Origin};
use smali_syntax::{Document, Encoding};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn object() -> ClassInfo {
    let m = |name: &str, desc: &str, flags| MemberInfo {
        name: name.into(),
        desc: desc.into(),
        flags,
        span: None,
    };
    ClassInfo {
        name: "Ljava/lang/Object;".into(),
        super_class: None,
        interfaces: vec![],
        flags: acc::PUBLIC,
        fields: vec![],
        methods: vec![
            m("<init>", "()V", acc::PUBLIC | acc::CONSTRUCTOR),
            m("toString", "()Ljava/lang/String;", acc::PUBLIC),
            m("hashCode", "()I", acc::PUBLIC),
        ],
        origin: Origin::ExtraRoot,
        path: None,
        span: None,
    }
}

pub fn build_index(extra: &[(PathBuf, String)]) -> Index {
    let mut idx = Index::new();
    idx.add_framework([object()]);
    let lib = fixtures_dir().join("lib");
    let mut files: Vec<_> = std::fs::read_dir(lib)
        .map(|r| r.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    files.sort();
    for p in files {
        if let Ok(t) = std::fs::read_to_string(&p) {
            idx.update_file(scan::entry_for(&p, &t, Origin::Workspace));
        }
    }
    for (p, t) in extra {
        idx.update_file(scan::entry_for(p, t, Origin::Workspace));
    }
    idx
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Expect {
    line: u32,
    start: u32,
    end: u32,
    code: String,
}

fn expectations(text: &str) -> Vec<Expect> {
    let mut out = Vec::new();
    let mut target: Option<u32> = None;
    for (i, line) in text.lines().enumerate() {
        let rest = line.strip_prefix('#');
        let caret = rest.and_then(|r| {
            let start = r.find('^')?;
            let carets = r[start..].bytes().take_while(|b| *b == b'^').count();
            let code = r[start + carets..].trim();
            (code.starts_with('S') && r[..start].trim().is_empty())
                .then(|| (start + 1, carets, code.to_string()))
        });
        match caret {
            Some((start, n, code)) => {
                if let Some(t) = target {
                    out.push(Expect {
                        line: t,
                        start: start as u32,
                        end: (start + n) as u32,
                        code,
                    });
                }
            }
            None => target = Some(i as u32),
        }
    }
    out.sort();
    out
}

fn actual(doc: &Document, ctx: &Context) -> Vec<Expect> {
    let mut out: Vec<Expect> = diagnostics(doc, ctx)
        .into_iter()
        .map(|d| {
            let s = doc.position(d.span.start, Encoding::Utf8);
            let e = doc.position(d.span.end, Encoding::Utf8);
            let line_len = doc
                .text
                .lines()
                .nth(s.line as usize)
                .map_or(0, |l| l.trim_end_matches('\r').len() as u32);
            let end = if e.line == s.line { e.col } else { line_len };
            Expect {
                line: s.line,
                start: s.col,
                end: end.max(s.col + 1),
                code: d.code.to_string(),
            }
        })
        .collect();
    out.sort();
    out
}

#[test]
fn fixtures() {
    let dir = fixtures_dir();
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .map(|r| {
            r.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "smali"))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    assert!(!files.is_empty(), "no fixtures in {}", dir.display());
    let mut failures = Vec::new();
    // Every fixture runs with LF and CRLF endings: Windows checkouts and editors produce CRLF,
    // and spans must never cover the `\r`.
    let variants = files.iter().flat_map(|path| {
        let lf = std::fs::read_to_string(path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        let crlf = lf.replace('\n', "\r\n");
        [(path, lf, ""), (path, crlf, " (CRLF)")]
    });
    for (path, text, variant) in variants {
        let doc = Document::parse(&text);
        let index = build_index(&[(path.clone(), text.clone())]);
        let expected_class =
            smali_index::workspace::expected_class(path, std::slice::from_ref(&dir));
        // Only check S110 in fixtures that opt into it by name.
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let ctx = Context {
            index: Some(&index),
            expected_class: if name.starts_with("s110") {
                expected_class.as_deref()
            } else {
                None
            },
        };
        // Fixtures that don't test S301 hide it; lib classes cover what they reference.
        let mut want = expectations(&text);
        let mut got = actual(&doc, &ctx);
        if !name.starts_with("s301") {
            got.retain(|e| e.code != "S301");
            want.retain(|e| e.code != "S301");
        }
        if !name.starts_with("s122") {
            got.retain(|e| e.code != "S122");
        }
        if want != got {
            let missing: Vec<_> = want.iter().filter(|w| !got.contains(w)).collect();
            let extra: Vec<_> = got.iter().filter(|g| !want.contains(g)).collect();
            failures.push(format!(
                "{name}{variant}:\n  missing {missing:?}\n  extra   {extra:?}"
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn every_catalog_code_has_fixtures() {
    let dir = fixtures_dir();
    let all: String = std::fs::read_dir(&dir)
        .map(|r| {
            r.flatten()
                .filter_map(|e| std::fs::read_to_string(e.path()).ok())
                .collect()
        })
        .unwrap_or_default();
    for code in [
        "S100", "S101", "S102", "S103", "S104", "S105", "S106", "S107", "S108", "S109", "S110",
        "S120", "S121", "S122", "S123", "S201", "S202", "S203", "S204", "S205", "S206", "S210",
        "S211", "S212", "S220", "S221", "S222", "S230", "S231", "S301", "S302", "S303", "S304",
        "S305",
    ] {
        let lower = code.to_lowercase();
        assert!(
            all.contains(&format!(" {code}")),
            "no positive fixture for {code}"
        );
        assert!(
            dir.join(format!("{lower}.smali")).exists()
                || dir.join(format!("{lower}_ok.smali")).exists(),
            "no fixture file for {code}"
        );
    }
}
