//! Hover, completion, navigation and code-action tests.

#[path = "fixtures.rs"]
#[allow(dead_code)]
mod fixtures;

use std::path::PathBuf;

use smali_analysis::actions::code_actions;
use smali_analysis::completion::complete;
use smali_analysis::diagnostics::diagnostics;
use smali_analysis::hover::hover;
use smali_analysis::nav::{definition, document_symbols, folding_ranges, references};
use smali_analysis::{Context, Edit};
use smali_syntax::{Document, Span};

const SRC: &str = r#".class public LT;
.super Llib/Base;

.field private count:I

.method public static run(IJ)V
    .locals 2
    invoke-static {v0, v1, v2}, Llib/Base;->make(IJ)Llib/Base;
    move-result-object v0
    iget-object v1, v0, Llib/Base;->name:Ljava/lang/String;
    if-eqz p0, :done
    :done
    return-void
.end method
"#;

fn with<R>(src: &str, f: impl FnOnce(&Document, &Context) -> R) -> R {
    let doc = Document::parse(src);
    let index = fixtures::build_index(&[(PathBuf::from("/w/smali/T.smali"), src.to_string())]);
    let ctx = Context {
        index: Some(&index),
        expected_class: None,
    };
    f(&doc, &ctx)
}

fn at(src: &str, needle: &str) -> usize {
    src.find(needle).unwrap_or(0)
}

#[test]
fn hover_method_ref() {
    let h = with(SRC, |d, c| hover(d, c, at(SRC, "make(IJ)") + 1));
    insta::assert_snapshot!(h.map(|h| h.markdown).unwrap_or_default(), @r"
    ```java
    public static Base make(int, long)
    ```
    Declared in `lib.Base` (workspace)
    ");
}

#[test]
fn hover_register_and_opcode() {
    let h = with(SRC, |d, c| hover(d, c, at(SRC, "p0, :done")));
    insta::assert_snapshot!(h.map(|h| h.markdown).unwrap_or_default(), @r"
    `p0` = `v2`

    parameter 0: `int`
    ");
    let h = with(SRC, |d, c| hover(d, c, at(SRC, "iget-object")));
    insta::assert_snapshot!(h.map(|h| h.markdown).unwrap_or_default(), @r"
    ```smali
    iget-object vA (4-bit), vA (4-bit), Lcls;->field:Ltype;
    ```
    Format `22c`, reference: Field

    can throw
    ");
}

#[test]
fn hover_field_and_class() {
    let h = with(SRC, |d, c| hover(d, c, at(SRC, "name:Ljava")));
    insta::assert_snapshot!(h.map(|h| h.markdown).unwrap_or_default(), @r"
    ```java
    public String name
    ```
    Declared in `lib.Base` (workspace)
    ");
    let h = with(SRC, |d, c| hover(d, c, at(SRC, "Llib/Base;\n") + 2));
    insta::assert_snapshot!(h.map(|h| h.markdown).unwrap_or_default(), @r"
    ```java
    public class lib.Base
    ```
    Source: workspace
    ");
}

fn labels(items: &[smali_analysis::completion::Item]) -> Vec<String> {
    items.iter().map(|i| i.label.clone()).collect()
}

#[test]
fn complete_members_with_full_signature() {
    let src = SRC.replace(
        "move-result-object v0\n",
        "move-result-object v0\n    invoke-static {}, Llib/Child;->ma\n",
    );
    let off = at(&src, "Llib/Child;->ma") + "Llib/Child;->ma".len();
    let items = with(&src, |d, c| complete(d, c, off));
    let make = items.iter().find(|i| i.label.starts_with("make"));
    insta::assert_snapshot!(make.map(|i| format!("{} | {}", i.insert, i.detail)).unwrap_or_default(),
        @"make(IJ)Llib/Base; | public static Base make(int, long)");
    assert!(
        items.iter().any(|i| i.label == "run()V"),
        "inherited members missing"
    );
}

#[test]
fn complete_member_bumps_locals() {
    let src = SRC.replace(".locals 2", ".locals 0").replace(
        "move-result-object v0\n",
        "move-result-object v0\n    invoke-static {}, Llib/Base;->m\n",
    );
    let off = at(&src, "Llib/Base;->m\n") + "Llib/Base;->m".len();
    let items = with(&src, |d, c| complete(d, c, off));
    let make = items.iter().find(|i| i.label.starts_with("make"));
    let edits = make.map(|i| i.extra_edits.clone()).unwrap_or_default();
    // make(IJ) needs 3 registers; the method has 0 locals + 3 params (p0, p1/p2) = 3, so no bump.
    assert!(edits.is_empty());
    let src = src.replace("run(IJ)V", "run()V");
    let off = at(&src, "Llib/Base;->m\n") + "Llib/Base;->m".len();
    let items = with(&src, |d, c| complete(d, c, off));
    let make = items.iter().find(|i| i.label.starts_with("make"));
    let edits = make.map(|i| i.extra_edits.clone()).unwrap_or_default();
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0].new_text, "3");
}

#[test]
fn complete_class_from_simple_name() {
    let src = SRC.replace(
        "move-result-object v0\n",
        "move-result-object v0\n    new-instance v0, Chi\n",
    );
    let off = at(&src, "Chi\n") + 3;
    let items = with(&src, |d, c| complete(d, c, off));
    insta::assert_snapshot!(items.iter().map(|i| format!("{} -> {}", i.label, i.insert)).collect::<Vec<_>>().join("\n"),
        @"Child -> Llib/Child;");
}

#[test]
fn complete_opcodes_registers_labels() {
    let src = SRC.replace("    :done\n", "    :done\n    inv\n");
    let off = at(&src, "    inv\n") + 7;
    let items = with(&src, |d, c| complete(d, c, off));
    let invoke = items.iter().find(|i| i.label == "invoke-virtual");
    insta::assert_snapshot!(invoke.map(|i| i.insert.clone()).unwrap_or_default(), @"invoke-virtual {${1:v0}}, ${2:Lcls;->method()V}");
    assert!(
        items.iter().all(|i| i.label != "iget-quick"),
        "odex opcodes offered"
    );

    let src = SRC.replace("if-eqz p0, :done", "if-eqz p0, :d");
    let off = at(&src, ":d\n") + 2;
    let items = with(&src, |d, c| complete(d, c, off));
    assert_eq!(labels(&items), vec![":done"]);

    let src = SRC.replace("if-eqz p0, :done", "if-eqz v");
    let off = at(&src, "if-eqz v") + 8;
    let items = with(&src, |d, c| complete(d, c, off));
    insta::assert_snapshot!(labels(&items).join(" "), @"v0 v1 v2 v3 v4 p0 p1 p2");
}

#[test]
fn completion_latency() {
    let mut big = String::from(".class public LBig;\n.super Ljava/lang/Object;\n");
    for i in 0..400 {
        big.push_str(&format!(".method public m{i}()V\n    .locals 4\n"));
        for _ in 0..40 {
            big.push_str("    invoke-static {v0, v1, v2}, Llib/Base;->make(IJ)Llib/Base;\n    move-result-object v0\n");
        }
        big.push_str("    return-void\n.end method\n");
    }
    let off = big.rfind("Llib/Base;->make").unwrap_or(0) + "Llib/Base;->ma".len();
    let doc = Document::parse(&big);
    let index = fixtures::build_index(&[]);
    let ctx = Context {
        index: Some(&index),
        expected_class: None,
    };
    let mut times: Vec<u128> = (0..40)
        .map(|_| {
            let t = std::time::Instant::now();
            let _ = complete(&doc, &ctx, off);
            t.elapsed().as_micros()
        })
        .collect();
    times.sort();
    let p95 = times[(times.len() * 95) / 100 - 1];
    assert!(p95 < 50_000, "p95 completion {p95}us");
}

fn apply(src: &str, edits: &[Edit]) -> String {
    let mut out = src.to_string();
    let mut edits = edits.to_vec();
    edits.sort_by_key(|e| std::cmp::Reverse(e.span.start));
    for e in edits {
        out.replace_range(e.span.start..e.span.end, &e.new_text);
    }
    out
}

/// Apply the action for `code` at `needle` and check the diagnostic is gone.
fn fixes(src: &str, needle: &str, code: &str) {
    let off = at(src, needle);
    let fixed = with(src, |d, c| {
        assert!(
            diagnostics(d, c).iter().any(|x| x.code == code),
            "{code} not reported"
        );
        let acts = code_actions(d, c, Span::new(off, off + needle.len()));
        let a = acts.iter().find(|a| a.code == code);
        assert!(a.is_some(), "no action for {code}: {acts:?}");
        apply(src, &a.map(|a| a.edits.clone()).unwrap_or_default())
    });
    with(&fixed, |d, c| {
        let left: Vec<_> = diagnostics(d, c)
            .into_iter()
            .filter(|x| x.code == code)
            .collect();
        assert!(
            left.is_empty(),
            "{code} still reported after fix:\n{fixed}\n{left:?}"
        );
    });
}

const M: &str =
    ".class public LT;\n.super Ljava/lang/Object;\n.method public static f()V\n    .locals 1\n";

#[test]
fn action_raise_locals() {
    fixes(
        &format!("{M}    const/4 v3, 0x0\n    return-void\n.end method\n"),
        "v3",
        "S201",
    );
    fixes(
        &format!("{M}    const-wide/16 v0, 0x0\n    return-void\n.end method\n"),
        "v0",
        "S203",
    );
}

#[test]
fn action_widen_const() {
    fixes(
        &format!("{M}    const/4 v0, 0x100\n    return-void\n.end method\n"),
        "0x100",
        "S211",
    );
    fixes(
        &format!("{M}    const/16 v0, 0x10000\n    return-void\n.end method\n"),
        "0x10000",
        "S211",
    );
}

#[test]
fn action_swap_invoke() {
    fixes(
        &format!("{M}    invoke-virtual {{}}, Llib/Base;->g(III)V\n    return-void\n.end method\n")
            .replace(".locals 1", ".locals 3")
            .replace("{}", "{v0, v1, v2}"),
        "Llib/Base;->g",
        "S304",
    );
}

#[test]
fn action_fix_register_count() {
    fixes(&format!("{M}    invoke-static/range {{v0 .. v1}}, Llib/Base;->g(III)V\n    return-void\n.end method\n").replace(".locals 1", ".locals 3"), "invoke-static", "S222");
    fixes(
        &format!(
            "{M}    invoke-static {{v0, v0}}, Llib/Base;->run()V\n    return-void\n.end method\n"
        ),
        "invoke-static",
        "S222",
    );
}

#[test]
fn action_create_label() {
    fixes(
        &format!("{M}    goto :later\n    return-void\n.end method\n"),
        ":later",
        "S120",
    );
}

#[test]
fn action_rename_class() {
    let src = ".class public LWrong;\n.super Ljava/lang/Object;\n";
    let doc = Document::parse(src);
    let ctx = Context {
        index: None,
        expected_class: Some("Lcom/x/Right;"),
    };
    let acts = code_actions(&doc, &ctx, Span::new(15, 16));
    let fixed = apply(
        src,
        &acts.first().map(|a| a.edits.clone()).unwrap_or_default(),
    );
    assert!(fixed.contains(".class public Lcom/x/Right;"));
    let d = Document::parse(&fixed);
    assert!(diagnostics(&d, &ctx).iter().all(|x| x.code != "S110"));
}

#[test]
fn navigation() {
    with(SRC, |d, c| {
        let defs = definition(d, c, at(SRC, ":done\n    return") + 1);
        assert!(defs.is_empty(), "definition on a label def has no target");
        let defs = definition(d, c, at(SRC, "p0, :done") + 5);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].path, None);
        let defs = definition(d, c, at(SRC, "make(IJ)") + 1);
        assert!(defs[0]
            .path
            .as_ref()
            .is_some_and(|p| p.ends_with("lib/Base.smali")));
        let refs = references(d, c, at(SRC, ":done\n") + 1, true);
        assert_eq!(refs.len(), 2);
        let refs = references(d, c, at(SRC, "make(IJ)") + 1, false);
        assert_eq!(refs.len(), 1);
    });
    let doc = Document::parse(SRC);
    let syms = document_symbols(&doc);
    insta::assert_snapshot!(syms.iter().flat_map(|s| std::iter::once(s).chain(&s.children)).map(|s| format!("{:?} {}", s.kind, s.name)).collect::<Vec<_>>().join("\n"), @r"
    Class LT;
    Field count
    Method run(IJ)V
    ");
    assert_eq!(folding_ranges(&doc).len(), 1);
}

mod refactoring {
    use super::*;
    use smali_analysis::refactor::{
        call_item_at, incoming_call_sites, outgoing_calls, rename_class, rename_member, RenameError,
    };

    fn ctx_for<R>(src: &str, f: impl FnOnce(&Document, &Context) -> R) -> R {
        with(src, f)
    }

    #[test]
    fn rename_method_across_workspace_including_overrides() {
        // lib/Child overrides nothing named run, but Base.run is called through Child here.
        let src = SRC.replace(
            "    :done\n",
            "    :done\n    invoke-virtual {v0}, Llib/Child;->run()V\n",
        );
        let off = at(&src, "Llib/Child;->run") + "Llib/Child;->".len() + 1;
        let edits = ctx_for(&src, |d, c| rename_member(d, c, off, "go"));
        let edits = edits.unwrap_or_default();
        let files: Vec<String> = edits
            .iter()
            .map(|e| {
                e.path
                    .as_ref()
                    .map(|p| {
                        p.file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default()
                    })
                    .unwrap_or_default()
            })
            .collect();
        assert!(
            files.contains(&"Base.smali".to_string()),
            "declaration not renamed: {files:?}"
        );
        assert!(
            files.contains(&"T.smali".to_string()),
            "call site not renamed: {files:?}"
        );
        assert!(edits
            .iter()
            .all(|e| e.edit.new_text == "go" && e.edit.span.len() == 3));
    }

    #[test]
    fn rename_refuses_framework_and_constructors() {
        let src = SRC.replace("    :done\n", "    :done\n    invoke-virtual {v0}, Ljava/lang/Object;->hashCode()I\n    invoke-direct {v0}, Llib/Base;-><init>()V\n");
        let off = at(&src, "->hashCode") + 3;
        assert!(matches!(
            ctx_for(&src, |d, c| rename_member(d, c, off, "h")),
            Err(RenameError::Framework(_))
        ));
        let off = at(&src, "-><init>") + 3;
        assert_eq!(
            ctx_for(&src, |d, c| rename_member(d, c, off, "x")),
            Err(RenameError::Special)
        );
        let off = at(&src, "make(IJ)") + 1;
        assert_eq!(
            ctx_for(&src, |d, c| rename_member(d, c, off, "a b")),
            Err(RenameError::InvalidName)
        );
    }

    #[test]
    fn rename_field() {
        let off = at(SRC, "name:Ljava") + 1;
        let edits = with(SRC, |d, c| rename_member(d, c, off, "title")).unwrap_or_default();
        assert_eq!(edits.len(), 2, "{edits:?}");
    }

    #[test]
    fn class_rename_rewrites_descriptors() {
        let index = fixtures::build_index(&[(PathBuf::from("/w/smali/T.smali"), SRC.to_string())]);
        let edits = rename_class(&index, "Llib/Base;", "Llib/Root;").unwrap_or_default();
        assert!(edits.len() >= 4, "{edits:?}");
        assert!(edits
            .iter()
            .all(|e| e.edit.new_text == "Llib/Root;" && e.edit.span.len() == "Llib/Base;".len()));
        assert!(rename_class(&index, "Ljava/lang/Object;", "LX;").is_err());
        assert!(rename_class(&index, "Llib/Base;", "not a type").is_err());
    }

    #[test]
    fn call_hierarchy() {
        with(SRC, |d, c| {
            let item = call_item_at(d, c, at(SRC, "make(IJ)") + 1);
            let item = item.expect("item");
            assert_eq!(item.name, "make");
            let sites = incoming_call_sites(c.index.expect("index"), &item);
            assert_eq!(sites.len(), 1);
            let run = call_item_at(d, c, at(SRC, "run(IJ)") + 1).expect("run");
            let out = outgoing_calls(d, &run);
            assert_eq!(out.len(), 1);
            assert_eq!(out[0].0.name, "make");
        });
    }
}
