//! Formatter: indentation and blank lines between members only (PLAN P6.7).
//!
//! Only whitespace at line starts/ends and blank lines change, so the token stream (and the
//! assembled dex) is identical. Lines inside string literals never exist in smali (strings are
//! single-line), so line-based reindenting is safe.

const INDENT: &str = "    ";

fn opens(t: &str) -> bool {
    let w = t.split_whitespace().next().unwrap_or("");
    matches!(
        w,
        ".method"
            | ".annotation"
            | ".subannotation"
            | ".packed-switch"
            | ".sparse-switch"
            | ".array-data"
            | ".param"
    )
}

fn closes(t: &str) -> bool {
    t.starts_with(".end ") && !t.starts_with(".end local") && !t.starts_with(".end field")
}

fn member_start(t: &str) -> bool {
    t.starts_with(".method") || t.starts_with(".field") || t.starts_with(".annotation")
}

/// Format a whole file. Idempotent.
pub fn format(text: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut depth: usize = 0;
    let mut brace: usize = 0;
    // `.param` only opens a block when an `.end param` follows before the next `.param`/instruction.
    let lines: Vec<&str> = text.lines().collect();
    let param_has_end = |i: usize| -> bool {
        for l in &lines[i + 1..] {
            let t = l.trim();
            if t.starts_with(".end param") {
                return true;
            }
            if t.starts_with(".annotation")
                || t.starts_with(".end annotation")
                || t.is_empty()
                || t.starts_with('#')
                || !t.starts_with('.') && depth_neutral(t)
            {
                continue;
            }
            return false;
        }
        false
    };
    for (i, raw) in lines.iter().enumerate() {
        let t = raw.trim();
        if t.is_empty() {
            if out.last().is_some_and(|l| !l.is_empty()) {
                out.push(String::new());
            }
            continue;
        }
        let mut d = depth + brace;
        if closes(t) || t.starts_with('}') {
            d = d.saturating_sub(1);
        }
        let top_member = depth == 0 && member_start(t);
        if top_member && out.last().is_some_and(|l| !l.is_empty()) {
            out.push(String::new());
        }
        out.push(format!("{}{}", INDENT.repeat(d), t));
        let is_param = t.starts_with(".param") || t.starts_with(".parameter");
        if closes(t) {
            depth = depth.saturating_sub(1);
        } else if opens(t) && (!is_param || param_has_end(i)) {
            depth += 1;
        }
        let code = strip_literals(t);
        let opened = code.matches('{').count();
        let closed = code.matches('}').count();
        if !code.contains('}') || opened > closed {
            brace += opened.saturating_sub(closed);
        } else {
            brace = brace.saturating_sub(closed - opened.min(closed));
        }
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

fn depth_neutral(t: &str) -> bool {
    t.contains('=') || t.starts_with('"') || t.starts_with('}') || t.starts_with('L')
}

/// Line with string/char literals and comments removed, for brace counting.
fn strip_literals(t: &str) -> String {
    let mut out = String::new();
    let mut chars = t.chars();
    while let Some(c) = chars.next() {
        match c {
            '#' => break,
            '"' | '\'' => {
                let q = c;
                while let Some(x) = chars.next() {
                    if x == '\\' {
                        chars.next();
                    } else if x == q {
                        break;
                    }
                }
            }
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const MESSY: &str = ".class public LA;\n.super Ljava/lang/Object;\n.field private x:I\n.method public f()V\n.registers 2\n  const/4 v0, 0x1\n\n\n\n:cond_0\n packed-switch v0, :pswitch_data_0\n  .annotation runtime LAnn;\nvalue = {\n\"a\",\n\"{b\"\n}\n.end annotation\n.param p1, \"x\"\n.line 3\n:pswitch_data_0\n.packed-switch 0x0\n:cond_0\n.end packed-switch\nreturn-void\n.end method\n.method public g()V\n.registers 1\nreturn-void\n.end method\n";

    #[test]
    fn indents_and_spaces_members() {
        let f = format(MESSY);
        assert!(f.contains(
            "\n\n.method public f()V\n    .registers 2\n    const/4 v0, 0x1\n\n    :cond_0\n"
        ));
        assert!(f.contains("    .annotation runtime LAnn;\n        value = {\n            \"a\",\n            \"{b\"\n        }\n    .end annotation\n"));
        assert!(f.contains("    .packed-switch 0x0\n        :cond_0\n    .end packed-switch\n"));
        assert!(f.contains("    .param p1, \"x\"\n    .line 3\n"));
        assert!(f.contains(".end method\n\n.method public g()V"));
    }

    #[test]
    fn idempotent_and_token_preserving() {
        let once = format(MESSY);
        assert_eq!(format(&once), once);
        let tokens = |s: &str| s.split_whitespace().map(str::to_string).collect::<Vec<_>>();
        assert_eq!(tokens(&once), tokens(MESSY));
    }
}
