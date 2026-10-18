use std::path::{Path, PathBuf};
use std::sync::Arc;

use smali_syntax::Span;

use super::*;
use crate::cache::Cache;
use crate::scan::entry_for;

fn add(idx: &mut Index, path: &str, src: &str) {
    idx.update_file(entry_for(Path::new(path), src, Origin::Workspace));
}

fn class(name: &str, sup: &str, ifaces: &[&str], body: &str) -> String {
    let mut s = format!(".class public {name}\n.super {sup}\n");
    for i in ifaces {
        s.push_str(&format!(".implements {i}\n"));
    }
    s.push_str(body);
    s
}

fn object() -> String {
    ".class public Ljava/lang/Object;\n.method public constructor <init>()V\n    .registers 1\n    return-void\n.end method\n.method public toString()Ljava/lang/String;\n    .registers 1\n    const/4 v0, 0\n    return-object v0\n.end method\n".to_string()
}

fn base_index() -> Index {
    let mut idx = Index::new();
    // Object has no .super, which the grammar rejects; index it from a hand-built entry.
    let obj = ClassInfo {
        name: "Ljava/lang/Object;".into(),
        super_class: None,
        interfaces: vec![],
        flags: acc::PUBLIC,
        fields: vec![],
        methods: vec![
            MemberInfo {
                name: "<init>".into(),
                desc: "()V".into(),
                flags: acc::PUBLIC | acc::CONSTRUCTOR,
                span: None,
            },
            MemberInfo {
                name: "toString".into(),
                desc: "()Ljava/lang/String;".into(),
                flags: acc::PUBLIC,
                span: None,
            },
        ],
        origin: Origin::ExtraRoot,
        path: None,
        span: None,
    };
    let _ = object();
    idx.add_framework([obj]);
    add(
        &mut idx,
        "/w/smali/I.smali",
        &class(
            "interface abstract LI;",
            "Ljava/lang/Object;",
            &[],
            ".method public greet()V\n    .registers 1\n    return-void\n.end method\n.field public static final K:I = 0x1\n",
        ),
    );
    add(
        &mut idx,
        "/w/smali/A.smali",
        &class(
            "LA;",
            "Ljava/lang/Object;",
            &["LI;"],
            ".field public x:I\n.method public foo(J)V\n    .registers 3\n    return-void\n.end method\n",
        ),
    );
    add(&mut idx, "/w/smali/B.smali", &class("LB;", "LA;", &[], ""));
    add(
        &mut idx,
        "/w/smali/C.smali",
        &class("LC;", "Lmissing/Base;", &[], ""),
    );
    idx
}

#[test]
fn inherited_method_and_field() {
    let idx = base_index();
    assert!(matches!(
        idx.resolve_method("LB;", "foo", "(J)V"),
        Lookup::Found { owner, .. } if owner.name == "LA;"
    ));
    assert!(matches!(
        idx.resolve_method("LB;", "toString", "()Ljava/lang/String;"),
        Lookup::Found { owner, .. } if owner.name == "Ljava/lang/Object;"
    ));
    assert!(matches!(
        idx.resolve_field("LB;", "x", "I"),
        Lookup::Found { .. }
    ));
}

#[test]
fn interface_default_method_and_static_field() {
    let idx = base_index();
    assert!(matches!(
        idx.resolve_method("LB;", "greet", "()V"),
        Lookup::Found { owner, .. } if owner.name == "LI;"
    ));
    assert!(matches!(
        idx.resolve_field("LB;", "K", "I"),
        Lookup::Found { .. }
    ));
}

#[test]
fn unknown_ancestor_is_unknown() {
    let idx = base_index();
    assert_eq!(idx.resolve_method("LC;", "nope", "()V"), Lookup::Unknown);
    assert_eq!(
        idx.resolve_method("Lnot/There;", "x", "()V"),
        Lookup::Unknown
    );
}

#[test]
fn genuinely_missing_member() {
    let idx = base_index();
    assert_eq!(idx.resolve_method("LB;", "nope", "()V"), Lookup::Missing);
    assert_eq!(idx.resolve_method("LB;", "foo", "(I)V"), Lookup::Missing);
    assert_eq!(idx.resolve_field("LA;", "y", "I"), Lookup::Missing);
}

#[test]
fn incremental_update_adds_and_removes_methods() {
    let mut idx = base_index();
    assert_eq!(idx.resolve_method("LB;", "bar", "()V"), Lookup::Missing);
    add(
        &mut idx,
        "/w/smali/B.smali",
        &class(
            "LB;",
            "LA;",
            &[],
            ".method public bar()V\n    .registers 1\n    return-void\n.end method\n",
        ),
    );
    assert!(matches!(
        idx.resolve_method("LB;", "bar", "()V"),
        Lookup::Found { .. }
    ));
    idx.remove_file(Path::new("/w/smali/A.smali"));
    assert_eq!(idx.resolve_method("LB;", "foo", "(J)V"), Lookup::Unknown);
}

#[test]
fn references_are_collected() {
    let mut idx = base_index();
    add(
        &mut idx,
        "/w/smali/D.smali",
        &class(
            "LD;",
            "Ljava/lang/Object;",
            &[],
            ".method public run()V\n    .registers 3\n    invoke-virtual {v0, v1, v2}, LA;->foo(J)V\n    iget v0, v0, LA;->x:I\n    return-void\n.end method\n",
        ),
    );
    let key = RefKey::Method("LA;".into(), "foo".into(), "(J)V".into());
    assert_eq!(idx.references(&key).len(), 1);
    let key = RefKey::Field("LA;".into(), "x".into(), "I".into());
    assert_eq!(idx.references(&key).len(), 1);
    assert!(idx.references(&RefKey::Class("LA;".into())).len() >= 3);
}

#[test]
fn cache_round_trip_and_corruption() {
    let dir = std::env::temp_dir().join(format!("smali-index-test-{}", std::process::id()));
    let file = dir.join("c.bin");
    let idx = base_index();
    let n = idx.file_count();
    assert!(Cache::from_entries(idx.files().map(|f| f.as_ref()))
        .save(&file)
        .is_ok());
    let loaded = Cache::load(&file);
    assert_eq!(loaded.len(), n);
    let entry = idx.file(Path::new("/w/smali/A.smali")).cloned();
    let hit = entry
        .as_ref()
        .and_then(|e| loaded.get(&PathBuf::from("/w/smali/A.smali"), e.hash));
    assert_eq!(hit.as_ref(), entry.as_deref());
    assert!(std::fs::write(&file, b"SMLIDX\0\0garbage").is_ok());
    assert!(Cache::load(&file).is_empty());
    assert!(std::fs::write(&file, b"").is_ok());
    assert!(Cache::load(&file).is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn android_jar_toast_resolves() {
    let Some(jar) = jar::find_android_jar(None, None) else {
        eprintln!("no android.jar found; skipping");
        return;
    };
    let mut idx = Index::new();
    idx.add_framework(jar::read_jar(&jar).unwrap_or_default());
    assert!(matches!(
        idx.resolve_method(
            "Landroid/widget/Toast;",
            "makeText",
            "(Landroid/content/Context;Ljava/lang/CharSequence;I)Landroid/widget/Toast;"
        ),
        Lookup::Found { member, .. } if member.is_static()
    ));
}

fn compare_scanners(path: &Path, text: &str) -> Result<(), String> {
    let slow = entry_for(path, text, Origin::Workspace);
    let fast = scan::fast_entry(path, text, Origin::Workspace);
    if slow.class != fast.class {
        return Err(format!(
            "{}: class differs\n slow {:?}\n fast {:?}",
            path.display(),
            slow.class,
            fast.class
        ));
    }
    let key = |r: &(Arc<str>, Span)| (r.1, r.0.to_string());
    let mut a: Vec<_> = slow.refs.iter().map(key).collect();
    let mut b: Vec<_> = fast.refs.iter().map(key).collect();
    a.sort();
    b.sort();
    // The fast scanner also sees member refs inside annotations (enum values); everything the
    // tree-sitter path finds must be found too, and class refs must match exactly.
    let classes = |v: &[(Span, String)]| {
        v.iter()
            .filter(|x| !x.1.contains("->"))
            .cloned()
            .collect::<Vec<_>>()
    };
    if !a.iter().all(|x| b.contains(x)) || classes(&a) != classes(&b) {
        let only_slow: Vec<_> = a.iter().filter(|x| !b.contains(x)).take(3).collect();
        let only_fast: Vec<_> = classes(&b)
            .into_iter()
            .filter(|x| !a.contains(x))
            .take(3)
            .collect();
        return Err(format!(
            "{}: refs differ\n slow only {only_slow:?}\n fast only {only_fast:?}",
            path.display()
        ));
    }
    Ok(())
}

#[test]
fn fast_scanner_matches_tree_sitter_on_fixtures() {
    let dirs = [
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../smali-analysis/tests/fixtures"),
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../smali-analysis/tests/fixtures/lib"),
    ];
    let mut errs = Vec::new();
    for d in dirs {
        for p in workspace::find_smali_files(&d) {
            let t = std::fs::read_to_string(&p).unwrap_or_default();
            // Files with syntax errors legitimately differ; the tree-sitter model drops broken parts.
            if smali_syntax::Document::parse(&t)
                .model
                .syntax_errors
                .is_empty()
            {
                errs.extend(compare_scanners(&p, &t).err());
            }
        }
    }
    let sample = include_str!("../../smali-syntax/tests/sample.smali");
    errs.extend(compare_scanners(Path::new("/s/sample.smali"), sample).err());
    assert!(errs.is_empty(), "{}", errs.join("\n"));
}

#[test]
fn fast_scanner_matches_tree_sitter_on_corpus() {
    let Some(corpus) = std::env::var_os("SMALI_CORPUS") else {
        return;
    };
    let files = workspace::find_smali_files(Path::new(&corpus));
    let step = (files.len() / 3000).max(1);
    let mut errs = Vec::new();
    for p in files.iter().step_by(step) {
        let t = std::fs::read_to_string(p).unwrap_or_default();
        if smali_syntax::Document::parse(&t)
            .model
            .syntax_errors
            .is_empty()
        {
            errs.extend(compare_scanners(p, &t).err());
        }
    }
    assert!(
        errs.is_empty(),
        "{} mismatches:\n{}",
        errs.len(),
        errs.iter().take(5).cloned().collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn android_jar_hierarchy_is_never_missing() {
    let mut idx = Index::new();
    idx.add_framework([ClassInfo {
        name: "Landroid/os/Stub;".into(),
        super_class: None,
        interfaces: vec![],
        flags: acc::PUBLIC,
        fields: vec![],
        methods: vec![],
        origin: Origin::AndroidJar,
        path: None,
        span: None,
    }]);
    // Hidden APIs are absent from android.jar, so absence proves nothing.
    assert_eq!(
        idx.resolve_method("Landroid/os/Stub;", "hidden", "()V"),
        Lookup::Unknown
    );
}
