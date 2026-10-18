//! `smali-check`: run analysis over a directory of smali files and print diagnostics.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use rayon::prelude::*;
use smali_analysis::diagnostics::diagnostics;
use smali_analysis::{Context, Severity};
use smali_index::workspace::{expected_class, find_smali_files};
use smali_index::{jar, Index, Origin};
use smali_syntax::{Document, Encoding};

const USAGE: &str = "usage: smali-check [--corpus] DIR... [options]
  --json               one JSON object per diagnostic on stdout
  --no-panics          exit 1 if analysis panics on any file
  --no-parse-errors    exit 1 if any file has S100 parse errors
  --errors-only        only print error-severity diagnostics
  --no-index           skip the workspace index (no S3xx)
  --android-jar PATH   framework classes for resolution
  --extra-root DIR     reference smali root (repeatable)
  --quiet              print only the summary
  --time               print timing per phase
  --parse-report       rank files with ERROR/MISSING nodes by their first error line pattern";

#[derive(Default)]
struct Opts {
    dirs: Vec<PathBuf>,
    json: bool,
    no_panics: bool,
    no_parse_errors: bool,
    errors_only: bool,
    no_index: bool,
    android_jar: Option<PathBuf>,
    extra_roots: Vec<PathBuf>,
    quiet: bool,
    time: bool,
    parse_report: bool,
}

fn parse_args() -> Result<Opts, String> {
    let mut o = Opts::default();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--corpus" => {}
            "--json" => o.json = true,
            "--no-panics" => o.no_panics = true,
            "--no-parse-errors" => o.no_parse_errors = true,
            "--errors-only" => o.errors_only = true,
            "--no-index" => o.no_index = true,
            "--quiet" => o.quiet = true,
            "--time" => o.time = true,
            "--parse-report" => o.parse_report = true,
            "--android-jar" => o.android_jar = args.next().map(PathBuf::from),
            "--extra-root" => o.extra_roots.extend(args.next().map(PathBuf::from)),
            "-h" | "--help" => return Err(USAGE.into()),
            s if s.starts_with('-') => return Err(format!("unknown option {s}\n{USAGE}")),
            s => o.dirs.push(PathBuf::from(s)),
        }
    }
    if o.dirs.is_empty() || o.dirs.iter().any(|d| d.as_os_str().is_empty()) {
        return Err(USAGE.into());
    }
    Ok(o)
}

fn sev(s: Severity) -> &'static str {
    match s {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
        Severity::Hint => "hint",
    }
}

struct FileResult {
    path: PathBuf,
    /// Normalized source line of the first syntax error, for `--parse-report`.
    first_error: Option<String>,
    lines: Vec<String>,
    errors: usize,
    warnings: usize,
    parse_errors: usize,
    panicked: bool,
}

fn check_file(path: &Path, ctx_index: Option<&Index>, o: &Opts) -> FileResult {
    let mut r = FileResult {
        path: path.to_path_buf(),
        first_error: None,
        lines: Vec::new(),
        errors: 0,
        warnings: 0,
        parse_errors: 0,
        panicked: false,
    };
    let Ok(bytes) = std::fs::read(path) else {
        return r;
    };
    let text = String::from_utf8_lossy(&bytes).to_string();
    let expected = expected_class(path, &o.extra_roots);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let doc = Document::parse(&text);
        let ctx = Context {
            index: ctx_index,
            expected_class: expected.as_deref(),
        };
        let diags = diagnostics(&doc, &ctx);
        (doc, diags)
    }));
    let Ok((doc, diags)) = result else {
        r.panicked = true;
        r.lines
            .push(format!("{}: PANIC during analysis", path.display()));
        return r;
    };
    for d in diags {
        match d.severity {
            Severity::Error => r.errors += 1,
            Severity::Warning => r.warnings += 1,
            _ => {}
        }
        if d.code == "S100" || d.code == "S104" {
            r.parse_errors += usize::from(d.code == "S100");
            if r.first_error.is_none() {
                let line = doc.position(d.span.start, Encoding::Utf8).line as usize;
                r.first_error = doc.text.lines().nth(line).map(normalize);
            }
        }
        if o.errors_only && d.severity != Severity::Error {
            continue;
        }
        let pos = doc.position(d.span.start, Encoding::Utf8);
        if o.json {
            r.lines.push(format!(
                "{{\"file\":{},\"line\":{},\"col\":{},\"code\":\"{}\",\"severity\":\"{}\",\"message\":{}}}",
                json_str(&path.display().to_string()),
                pos.line + 1,
                pos.col + 1,
                d.code,
                sev(d.severity),
                json_str(&d.message)
            ));
        } else {
            r.lines.push(format!(
                "{}:{}:{}: {} {}: {}",
                path.display(),
                pos.line + 1,
                pos.col + 1,
                sev(d.severity),
                d.code,
                d.message
            ));
        }
    }
    r
}

/// Collapse literals, descriptors and registers so similar failures group together.
fn normalize(line: &str) -> String {
    let mut out = Vec::new();
    for w in line.split_whitespace().take(4) {
        let w = w.trim_end_matches(',');
        let n = if w.starts_with('"') {
            "\"…\""
        } else if w.starts_with('L') && w.contains(';') {
            "L…;"
        } else if (w.starts_with('v') || w.starts_with('p'))
            && w[1..].bytes().all(|b| b.is_ascii_digit())
            && w.len() > 1
        {
            "vN"
        } else if w.starts_with("0x") || w.starts_with('-') || w.bytes().all(|b| b.is_ascii_digit())
        {
            "N"
        } else {
            w
        };
        out.push(n.to_string());
    }
    out.join(" ")
}

fn json_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn main() -> ExitCode {
    let o = match parse_args() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let t0 = Instant::now();
    let files: Vec<PathBuf> = o
        .dirs
        .iter()
        .flat_map(|d| {
            if d.is_file() {
                vec![d.clone()]
            } else {
                find_smali_files(d)
            }
        })
        .collect();
    let t_walk = t0.elapsed();

    let t1 = Instant::now();
    let index = (!o.no_index).then(|| {
        let mut idx = Index::new();
        smali_index::workspace::index_into(&mut idx, &files, Origin::Workspace, None);
        for r in &o.extra_roots {
            idx.add_extra(smali_index::workspace::index_extra_root(r, None));
        }
        if let Some(j) = &o.android_jar {
            match jar::read_jar(j) {
                Ok(c) => idx.add_framework(c),
                Err(e) => eprintln!("smali-check: cannot read {}: {e}", j.display()),
            }
        }
        idx
    });
    let t_index = t1.elapsed();

    let t2 = Instant::now();
    let results: Vec<FileResult> = files
        .par_iter()
        .map(|p| check_file(p, index.as_ref(), &o))
        .collect();
    let t_analysis = t2.elapsed();

    let (mut errors, mut warnings, mut parse_files, mut panics) = (0, 0, 0, 0);
    for r in &results {
        errors += r.errors;
        warnings += r.warnings;
        parse_files += usize::from(r.parse_errors > 0);
        panics += usize::from(r.panicked);
        if !o.quiet {
            for l in &r.lines {
                println!("{l}");
            }
        }
    }
    eprintln!(
        "smali-check: {} files, {errors} errors, {warnings} warnings, {parse_files} files with parse errors, {panics} panics in {:.2}s",
        results.len(),
        t0.elapsed().as_secs_f64()
    );
    if o.time {
        eprintln!(
            "  walk {:.2}s, index {:.2}s, analysis {:.2}s",
            t_walk.as_secs_f64(),
            t_index.as_secs_f64(),
            t_analysis.as_secs_f64()
        );
    }
    if o.parse_report {
        let mut groups: std::collections::HashMap<&str, (usize, &Path)> =
            std::collections::HashMap::new();
        for r in &results {
            if let Some(k) = &r.first_error {
                let e = groups.entry(k.as_str()).or_insert((0, r.path.as_path()));
                e.0 += 1;
            }
        }
        let mut ranked: Vec<_> = groups.into_iter().collect();
        ranked.sort_by(|a, b| b.1 .0.cmp(&a.1 .0).then(a.0.cmp(b.0)));
        eprintln!(
            "parse failure patterns ({} files):",
            ranked.iter().map(|g| g.1 .0).sum::<usize>()
        );
        for (k, (n, example)) in ranked.iter().take(30) {
            eprintln!("{n:7} | {k} | e.g. {}", example.display());
        }
    }
    if (o.no_panics && panics > 0) || (o.no_parse_errors && parse_files > 0) {
        if o.no_parse_errors && parse_files > 0 {
            for r in results.iter().filter(|r| r.parse_errors > 0).take(20) {
                eprintln!("  parse errors: {}", r.path.display());
            }
        }
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}
