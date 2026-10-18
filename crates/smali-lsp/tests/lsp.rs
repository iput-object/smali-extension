//! Spawn the server binary and drive it over stdio.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

struct Lsp {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Value>,
    next_id: i64,
    backlog: Vec<Value>,
}

impl Lsp {
    fn start(root: &Path) -> Lsp {
        let mut child = Command::new(env!("CARGO_BIN_EXE_smali-lsp"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(if std::env::var_os("LSP_TEST_STDERR").is_some() {
                Stdio::inherit()
            } else {
                Stdio::null()
            })
            .env("XDG_CACHE_HOME", root.join(".cache"))
            .spawn()
            .expect("spawn smali-lsp");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = child.stdout.take().expect("stdout");
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let mut r = BufReader::new(stdout);
            loop {
                let mut len = 0usize;
                loop {
                    let mut line = String::new();
                    if r.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let line = line.trim();
                    if line.is_empty() {
                        break;
                    }
                    if let Some(v) = line.strip_prefix("Content-Length: ") {
                        len = v.parse().unwrap_or(0);
                    }
                }
                let mut buf = vec![0; len];
                if r.read_exact(&mut buf).is_err() {
                    return;
                }
                if let Ok(v) = serde_json::from_slice(&buf) {
                    if tx.send(v).is_err() {
                        return;
                    }
                }
            }
        });
        Lsp {
            child,
            stdin,
            rx,
            next_id: 1,
            backlog: Vec::new(),
        }
    }

    fn send(&mut self, msg: Value) {
        let body = msg.to_string();
        let _ = write!(self.stdin, "Content-Length: {}\r\n\r\n{body}", body.len());
        let _ = self.stdin.flush();
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    fn request_async(&mut self, method: &str, params: Value) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        id
    }

    /// Next message matching `pred`, answering server-to-client requests along the way.
    fn wait(&mut self, what: &str, pred: impl Fn(&Value) -> bool) -> Value {
        if let Some(i) = self.backlog.iter().position(&pred) {
            return self.backlog.remove(i);
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let msg = self
                .rx
                .recv_timeout(left)
                .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
            if msg.get("method").is_some() && msg.get("id").is_some() {
                let id = msg["id"].clone();
                self.send(json!({"jsonrpc": "2.0", "id": id, "result": null}));
                continue;
            }
            if pred(&msg) {
                return msg;
            }
            self.backlog.push(msg);
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.request_async(method, params);
        let resp = self.wait(method, |m| m["id"] == json!(id));
        resp["result"].clone()
    }

    fn diagnostics_for(&mut self, uri: &str) -> Vec<Value> {
        let uri = uri.to_string();
        let m = self.wait("diagnostics", move |m| {
            m["method"] == "textDocument/publishDiagnostics" && m["params"]["uri"] == json!(uri)
        });
        m["params"]["diagnostics"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }
}

impl Drop for Lsp {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn workspace() -> PathBuf {
    let root = std::env::temp_dir().join(format!("smali-lsp-it-{}", std::process::id()));
    let smali = root.join("smali/com/x");
    let _ = std::fs::create_dir_all(&smali);
    let _ = std::fs::write(
        smali.join("Lib.smali"),
        ".class public Lcom/x/Lib;\n.super Ljava/lang/Object;\n\n.method public static make(I)Lcom/x/Lib;\n    .registers 2\n    const/4 v0, 0x0\n    return-object v0\n.end method\n",
    );
    for i in 0..300 {
        let _ = std::fs::write(
            smali.join(format!("Gen{i}.smali")),
            format!(".class public Lcom/x/Gen{i};\n.super Ljava/lang/Object;\n"),
        );
    }
    root
}

fn uri(p: &Path) -> String {
    format!("file://{}", p.display())
}

const MAIN: &str = ".class public Lcom/x/Main;\n.super Ljava/lang/Object;\n\n.method public static run()V\n    .locals 1\n    const/4 v0, 0x8\n    invoke-static {v0}, Lcom/x/Lib;->make(I)Lcom/x/Lib;\n    return-void\n.end method\n";

#[test]
fn full_session() {
    let root = workspace();
    let main_path = root.join("smali/com/x/Main.smali");
    let _ = std::fs::write(&main_path, MAIN);
    let main = uri(&main_path);
    let mut lsp = Lsp::start(&root);

    let init = lsp.request(
        "initialize",
        json!({
            "processId": null,
            "rootUri": uri(&root),
            "capabilities": {"general": {"positionEncodings": ["utf-8", "utf-16"]}},
            "workspaceFolders": [{"uri": uri(&root), "name": "ws"}],
        }),
    );
    assert_eq!(init["capabilities"]["positionEncoding"], "utf-8");
    lsp.notify("initialized", json!({}));

    // Requests during indexing must answer (possibly partially) rather than block; cancel one.
    let id = lsp.request_async("workspace/symbol", json!({"query": "Gen"}));
    lsp.notify("$/cancelRequest", json!({"id": id}));
    let resp = lsp.wait("cancelled symbol", |m| m["id"] == json!(id));
    assert!(resp.get("error").is_some() || resp.get("result").is_some());
    lsp.wait("index done", |m| {
        m["method"] == "window/logMessage"
            && m["params"]["message"]
                .as_str()
                .is_some_and(|t| t.contains("indexed"))
    });

    lsp.notify(
        "textDocument/didOpen",
        json!({"textDocument": {"uri": main, "languageId": "smali", "version": 1, "text": MAIN}}),
    );
    let diags = lsp.diagnostics_for(&main);
    let codes: Vec<&str> = diags.iter().filter_map(|d| d["code"].as_str()).collect();
    assert!(codes.contains(&"S211"), "{codes:?}");

    let hover = lsp.request(
        "textDocument/hover",
        json!({"textDocument": {"uri": main}, "position": {"line": 6, "character": 40}}),
    );
    let text = hover["contents"]["value"].as_str().unwrap_or("");
    assert!(text.contains("make(int)"), "{hover}");

    let def = lsp.request(
        "textDocument/definition",
        json!({"textDocument": {"uri": main}, "position": {"line": 6, "character": 40}}),
    );
    assert!(
        def[0]["uri"]
            .as_str()
            .is_some_and(|u| u.ends_with("Lib.smali")),
        "{def}"
    );

    // Fix the literal with an incremental edit; S211 must go away.
    lsp.notify(
        "textDocument/didChange",
        json!({"textDocument": {"uri": main, "version": 2},
               "contentChanges": [{"range": {"start": {"line": 5, "character": 4}, "end": {"line": 5, "character": 11}}, "text": "const/16"}]}),
    );
    let diags = lsp.diagnostics_for(&main);
    assert!(diags.iter().all(|d| d["code"] != "S211"), "{diags:?}");

    let completion = lsp.request(
        "textDocument/completion",
        json!({"textDocument": {"uri": main}, "position": {"line": 6, "character": 4}}),
    );
    let labels: Vec<&str> = completion
        .as_array()
        .map(|a| a.iter().filter_map(|i| i["label"].as_str()).collect())
        .unwrap_or_default();
    assert!(labels.contains(&"invoke-static"), "{labels:?}");

    let symbols = lsp.request(
        "textDocument/documentSymbol",
        json!({"textDocument": {"uri": main}}),
    );
    assert_eq!(symbols[0]["name"], "Lcom/x/Main;");

    let tokens = lsp.request(
        "textDocument/semanticTokens/full",
        json!({"textDocument": {"uri": main}}),
    );
    assert!(tokens["data"].as_array().is_some_and(|d| !d.is_empty()));

    let refs = lsp.request(
        "textDocument/references",
        json!({"textDocument": {"uri": main}, "position": {"line": 6, "character": 40}, "context": {"includeDeclaration": false}}),
    );
    assert!(refs.as_array().is_some_and(|r| !r.is_empty()), "{refs}");

    let renamed = lsp.request(
        "textDocument/rename",
        json!({"textDocument": {"uri": main}, "position": {"line": 6, "character": 40}, "newName": "build"}),
    );
    let changes = renamed["changes"].as_object().map(|m| m.len()).unwrap_or(0);
    assert_eq!(changes, 2, "{renamed}");

    let fmt = lsp.request(
        "textDocument/formatting",
        json!({"textDocument": {"uri": main}, "options": {"tabSize": 4, "insertSpaces": true}}),
    );
    assert!(fmt.is_null() || fmt.as_array().is_some(), "{fmt}");

    let items = lsp.request(
        "textDocument/prepareCallHierarchy",
        json!({"textDocument": {"uri": main}, "position": {"line": 6, "character": 40}}),
    );
    let incoming = lsp.request("callHierarchy/incomingCalls", json!({"item": items[0]}));
    assert_eq!(incoming[0]["from"]["name"], "run()V", "{incoming}");

    let _ = lsp.request("shutdown", json!(null));
    lsp.notify("exit", json!(null));
    let _ = std::fs::remove_dir_all(&root);
}
