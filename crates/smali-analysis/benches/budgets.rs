//! Benchmarks for the performance budgets (synthetic inputs, no corpus needed).

use std::path::PathBuf;

use criterion::{criterion_group, criterion_main, Criterion};
use smali_analysis::{completion, diagnostics, Context};
use smali_index::{scan, Index, Origin};
use smali_syntax::Document;

fn big_file(methods: usize) -> String {
    let mut s = String::from(".class public Lbench/Big;\n.super Ljava/lang/Object;\n");
    for i in 0..methods {
        s.push_str(&format!(
            ".method public m{i}(IJ)V\n    .locals 4\n    :start\n"
        ));
        for _ in 0..30 {
            s.push_str("    invoke-static {v0, v1, v2}, Lbench/Lib;->make(IJ)Lbench/Lib;\n    move-result-object v0\n    if-eqz v0, :start\n");
        }
        s.push_str("    return-void\n.end method\n");
    }
    s
}

fn lib() -> String {
    ".class public Lbench/Lib;\n.super Ljava/lang/Object;\n.method public static make(IJ)Lbench/Lib;\n    .registers 3\n    const/4 v0, 0x0\n    return-object v0\n.end method\n".into()
}

fn index_with(n: usize) -> Index {
    let mut idx = Index::new();
    idx.update_file(scan::fast_entry(
        &PathBuf::from("/b/smali/bench/Lib.smali"),
        &lib(),
        Origin::Workspace,
    ));
    for i in 0..n {
        let t = format!(".class public Lbench/C{i};\n.super Lbench/Lib;\n.method public run()V\n    .locals 3\n    invoke-static {{v0, v1, v2}}, Lbench/Lib;->make(IJ)Lbench/Lib;\n    return-void\n.end method\n");
        idx.update_file(scan::fast_entry(
            &PathBuf::from(format!("/b/smali/bench/C{i}.smali")),
            &t,
            Origin::Workspace,
        ));
    }
    idx
}

fn budgets(c: &mut Criterion) {
    // ~300 KB: the p99.9 file size in the corpus.
    let text = big_file(50);
    let doc = Document::parse(&text);
    let index = index_with(2000);
    let ctx = Context {
        index: Some(&index),
        expected_class: None,
    };
    let off = text.rfind("Lbench/Lib;->make").unwrap_or(0) + "Lbench/Lib;->ma".len();

    c.bench_function("completion_member", |b| {
        b.iter(|| completion::complete(&doc, &ctx, off))
    });
    c.bench_function("reanalysis_large_file", |b| {
        b.iter(|| {
            let d = Document::parse(&text);
            diagnostics::diagnostics(&d, &ctx)
        })
    });
    let files: Vec<(PathBuf, String)> = (0..5000)
        .map(|i| {
            (
                PathBuf::from(format!("/b/smali/x/F{i}.smali")),
                big_file(2).replace("Lbench/Big;", &format!("Lx/F{i};")),
            )
        })
        .collect();
    c.bench_function("index_5000_files", |b| {
        b.iter(|| {
            let mut idx = Index::new();
            for (p, t) in &files {
                idx.update_file(scan::fast_entry(p, t, Origin::Workspace));
            }
            idx
        })
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(20);
    targets = budgets
}
criterion_main!(benches);
