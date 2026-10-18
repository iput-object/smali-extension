//! LSP plumbing: documents, background indexing, request handlers.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::Duration;

use smali_analysis::{actions, completion, diagnostics, hover, nav, refactor, tokens, Context};
use smali_index::cache::{cache_file, default_cache_dir, Cache};
use smali_index::workspace::{expected_class, find_smali_files, index_files};
use smali_index::{jar, scan, Index, Origin};
use smali_syntax::{Document, Encoding, LineIndex};
use tower_lsp_server::jsonrpc::Result;
use tower_lsp_server::ls_types::*;
use tower_lsp_server::{Client, LanguageServer};

use crate::config::{Config, Override};
use crate::convert;

/// `smali.renameClass` arguments: `[uri, newDescriptor]`. Rewrites every descriptor and moves the file.
const RENAME_CLASS: &str = "smali.renameClass";

fn read<T>(l: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    l.read().unwrap_or_else(PoisonError::into_inner)
}

fn write<T>(l: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    l.write().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Clone)]
struct Open {
    doc: Document,
    version: i32,
}

pub struct State {
    docs: RwLock<HashMap<Uri, Open>>,
    index: RwLock<Index>,
    config: RwLock<Config>,
    utf8: AtomicBool,
    roots: RwLock<Vec<PathBuf>>,
    generations: RwLock<HashMap<Uri, u64>>,
    next_generation: AtomicU64,
    index_generation: AtomicU64,
}

pub struct Backend {
    client: Client,
    st: Arc<State>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Backend {
            client,
            st: Arc::new(State {
                docs: RwLock::new(HashMap::new()),
                index: RwLock::new(Index::new()),
                config: RwLock::new(Config::default()),
                utf8: AtomicBool::new(false),
                roots: RwLock::new(Vec::new()),
                generations: RwLock::new(HashMap::new()),
                next_generation: AtomicU64::new(1),
                index_generation: AtomicU64::new(0),
            }),
        }
    }
}

impl State {
    fn enc(&self) -> Encoding {
        if self.utf8.load(Ordering::Relaxed) {
            Encoding::Utf8
        } else {
            Encoding::Utf16
        }
    }

    fn doc(&self, uri: &Uri) -> Option<Document> {
        read(&self.docs).get(uri).map(|o| o.doc.clone())
    }

    fn expected_class(&self, uri: &Uri) -> Option<String> {
        let path = uri.to_file_path()?;
        let extra = read(&self.config).extra_roots.clone();
        expected_class(&path, &extra)
    }
}

/// Run a pure computation off the async runtime, converting panics into `None` plus a log line.
async fn guarded<T: Send + 'static>(
    name: &'static str,
    f: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let res = tokio::task::spawn_blocking(move || {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
    })
    .await;
    match res {
        Ok(Ok(v)) => Some(v),
        Ok(Err(_)) => {
            tracing::error!("panic in {name}; returning empty result");
            None
        }
        Err(e) => {
            tracing::error!("{name} task failed: {e}");
            None
        }
    }
}

fn path_of(uri: &Uri) -> Option<PathBuf> {
    uri.to_file_path().map(|p| p.into_owned())
}

fn uri_of(path: &Path) -> Option<Uri> {
    Uri::from_file_path(path)
}

impl Backend {
    fn schedule_diagnostics(&self, uri: Uri) {
        let generation = self.st.next_generation.fetch_add(1, Ordering::Relaxed);
        write(&self.st.generations).insert(uri.clone(), generation);
        let st = self.st.clone();
        let client = self.client.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            if read(&st.generations).get(&uri) != Some(&generation) {
                return;
            }
            publish(st, client, uri).await;
        });
    }

    fn republish_all(&self) {
        let uris: Vec<Uri> = read(&self.st.docs).keys().cloned().collect();
        for uri in uris {
            self.schedule_diagnostics(uri);
        }
    }

    /// Rebuild the whole index in the background. Requests keep using the current index;
    /// workspace files land chunk by chunk so results improve while indexing runs.
    fn start_indexing(&self) {
        let st = self.st.clone();
        let client = self.client.clone();
        let generation = st.index_generation.fetch_add(1, Ordering::SeqCst) + 1;
        tokio::spawn(async move {
            let st2 = st.clone();
            let done = guarded("indexing", move || build_index(&st2, generation)).await;
            if let Some(Some(msg)) = done {
                client.log_message(MessageType::INFO, msg).await;
                let uris: Vec<Uri> = read(&st.docs).keys().cloned().collect();
                for uri in uris {
                    publish(st.clone(), client.clone(), uri).await;
                }
                let _ = client.semantic_tokens_refresh().await;
            }
        });
    }
}

fn build_index(st: &State, generation: u64) -> Option<String> {
    let t = std::time::Instant::now();
    let roots = read(&st.roots).clone();
    let config = read(&st.config).clone();
    let stale = || st.index_generation.load(Ordering::SeqCst) != generation;
    *write(&st.index) = Index::new();

    let cache_dir = config.cache_dir.clone().or_else(default_cache_dir);
    let cache_path = roots
        .first()
        .zip(cache_dir.as_ref())
        .map(|(r, d)| cache_file(d, r));
    let cache = cache_path.as_deref().map(Cache::load);

    let mut notes = Vec::new();
    let jar_path = jar::find_android_jar(config.android_jar.as_deref(), config.api_level);
    match &jar_path {
        Some(p) => match jar::read_jar(p) {
            Ok(classes) => {
                notes.push(format!(
                    "{} framework classes from {}",
                    classes.len(),
                    p.display()
                ));
                write(&st.index).add_framework(classes);
            }
            Err(e) => notes.push(format!("cannot read {}: {e}", p.display())),
        },
        None => notes.push("no android.jar found; set smali.androidJar or ANDROID_HOME".into()),
    }
    for r in &config.extra_roots {
        let classes = smali_index::workspace::index_extra_root(r, None);
        notes.push(format!(
            "{} classes from extra root {}",
            classes.len(),
            r.display()
        ));
        write(&st.index).add_extra(classes);
    }
    let files: Vec<PathBuf> = roots.iter().flat_map(|r| find_smali_files(r)).collect();
    for chunk in files.chunks(2048) {
        if stale() {
            return None;
        }
        let entries = index_files(chunk, Origin::Workspace, cache.as_ref());
        let mut idx = write(&st.index);
        for e in entries {
            idx.update_file(e);
        }
    }
    // Open documents win over what is on disk.
    let open: Vec<(PathBuf, Document)> = read(&st.docs)
        .iter()
        .filter_map(|(u, o)| Some((path_of(u)?, o.doc.clone())))
        .collect();
    {
        let mut idx = write(&st.index);
        for (p, d) in open {
            idx.update_file(scan::fast_entry(&p, &d.text, Origin::Workspace));
        }
    }
    if let Some(cp) = cache_path {
        let snapshot = Cache::from_entries(read(&st.index).files().map(|f| f.as_ref()));
        if let Err(e) = snapshot.save(&cp) {
            tracing::warn!("cache save failed: {e}");
        }
    }
    Some(format!(
        "smali-lsp: indexed {} files in {:.2}s; {}",
        files.len(),
        t.elapsed().as_secs_f64(),
        notes.join("; ")
    ))
}

async fn publish(st: Arc<State>, client: Client, uri: Uri) {
    let Some(open) = read(&st.docs).get(&uri).cloned() else {
        return;
    };
    let enc = st.enc();
    let expected = st.expected_class(&uri);
    let st2 = st.clone();
    let doc = open.doc.clone();
    let diags = guarded("diagnostics", move || {
        let idx = read(&st2.index);
        let ctx = Context {
            index: Some(&idx),
            expected_class: expected.as_deref(),
        };
        let config = read(&st2.config);
        diagnostics::diagnostics(&doc, &ctx)
            .into_iter()
            .filter_map(|d| {
                let sev = match config.severity(d.code) {
                    Override::Off => return None,
                    Override::Severity(s) => s,
                    Override::Default => d.severity,
                };
                Some(convert::diagnostic(&doc, &d, sev, enc))
            })
            .collect::<Vec<_>>()
    })
    .await
    .unwrap_or_default();
    client
        .publish_diagnostics(uri, diags, Some(open.version))
        .await;
}

/// Convert LSP content changes into byte edits, each in the coordinates after the previous one.
fn byte_edits(
    doc: &Document,
    changes: &[TextDocumentContentChangeEvent],
    enc: Encoding,
) -> Option<Vec<smali_syntax::TextEdit>> {
    let mut text = doc.text.to_string();
    let mut edits = Vec::new();
    for c in changes {
        let lines = LineIndex::new(&text);
        let (start, end) = match c.range {
            Some(r) => {
                let s = lines.offset(
                    &text,
                    smali_syntax::LineCol {
                        line: r.start.line,
                        col: r.start.character,
                    },
                    enc,
                );
                let e = lines.offset(
                    &text,
                    smali_syntax::LineCol {
                        line: r.end.line,
                        col: r.end.character,
                    },
                    enc,
                );
                (s.min(e), e.max(s))
            }
            None => (0, text.len()),
        };
        if !text.is_char_boundary(start) || !text.is_char_boundary(end) {
            return None;
        }
        text.replace_range(start..end, &c.text);
        edits.push(smali_syntax::TextEdit {
            start,
            end,
            new_text: c.text.clone(),
        });
    }
    Some(edits)
}

fn semantic_legend() -> SemanticTokensLegend {
    SemanticTokensLegend {
        token_types: tokens::TOKEN_TYPES
            .iter()
            .map(|t| SemanticTokenType::new(t))
            .collect(),
        token_modifiers: tokens::TOKEN_MODIFIERS
            .iter()
            .map(|m| SemanticTokenModifier::new(m))
            .collect(),
    }
}

fn symbol_kind(k: nav::SymbolKind) -> SymbolKind {
    match k {
        nav::SymbolKind::Class => SymbolKind::CLASS,
        nav::SymbolKind::Interface => SymbolKind::INTERFACE,
        nav::SymbolKind::Method => SymbolKind::METHOD,
        nav::SymbolKind::Constructor => SymbolKind::CONSTRUCTOR,
        nav::SymbolKind::Field => SymbolKind::FIELD,
    }
}

#[allow(deprecated)]
fn doc_symbol(doc: &Document, s: &nav::Symbol, enc: Encoding) -> DocumentSymbol {
    DocumentSymbol {
        name: s.name.clone(),
        detail: (!s.detail.is_empty()).then(|| s.detail.clone()),
        kind: symbol_kind(s.kind),
        tags: None,
        deprecated: None,
        range: convert::range(doc, s.span, enc),
        selection_range: convert::range(doc, s.selection, enc),
        children: Some(s.children.iter().map(|c| doc_symbol(doc, c, enc)).collect()),
    }
}

impl Backend {
    /// Turn analysis locations into LSP locations, reading each referenced file once.
    fn locations(&self, current: &Uri, doc: &Document, locs: Vec<nav::Loc>) -> Vec<Location> {
        let enc = self.st.enc();
        let mut by_file: HashMap<PathBuf, Vec<smali_syntax::Span>> = HashMap::new();
        let mut out = Vec::new();
        for l in locs {
            match l.path {
                None => out.push(Location::new(
                    current.clone(),
                    convert::range(doc, l.span, enc),
                )),
                Some(p) => by_file.entry(p).or_default().push(l.span),
            }
        }
        for (path, spans) in by_file {
            let Some(uri) = uri_of(&path) else { continue };
            let open = self.st.doc(&uri);
            let text = match &open {
                Some(d) => d.text.to_string(),
                None => std::fs::read_to_string(&path).unwrap_or_default(),
            };
            let lines = LineIndex::new(&text);
            for s in spans {
                let a = lines.position(&text, s.start, enc);
                let b = lines.position(&text, s.end, enc);
                out.push(Location::new(
                    uri.clone(),
                    Range::new(Position::new(a.line, a.col), Position::new(b.line, b.col)),
                ));
            }
        }
        out
    }

    fn with_doc(&self, uri: &Uri) -> Option<(Document, Encoding)> {
        Some((self.st.doc(uri)?, self.st.enc()))
    }
}

impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        let utf8 = params
            .capabilities
            .general
            .as_ref()
            .and_then(|g| g.position_encodings.as_ref())
            .is_some_and(|e| e.contains(&PositionEncodingKind::UTF8));
        self.st.utf8.store(utf8, Ordering::Relaxed);
        if let Some(c) = params
            .initialization_options
            .as_ref()
            .and_then(Config::from_value)
        {
            *write(&self.st.config) = c;
        }
        let mut roots: Vec<PathBuf> = params
            .workspace_folders
            .unwrap_or_default()
            .iter()
            .filter_map(|f| path_of(&f.uri))
            .collect();
        #[allow(deprecated)]
        if roots.is_empty() {
            roots.extend(params.root_uri.as_ref().and_then(path_of));
        }
        *write(&self.st.roots) = roots;

        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: "smali-lsp".into(),
                version: Some(env!("CARGO_PKG_VERSION").into()),
            }),
            capabilities: ServerCapabilities {
                position_encoding: Some(if utf8 {
                    PositionEncodingKind::UTF8
                } else {
                    PositionEncodingKind::UTF16
                }),
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::INCREMENTAL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![">".into(), ":".into(), "{".into(), " ".into()]),
                    ..Default::default()
                }),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                document_symbol_provider: Some(OneOf::Left(true)),
                workspace_symbol_provider: Some(OneOf::Left(true)),
                folding_range_provider: Some(FoldingRangeProviderCapability::Simple(true)),
                code_action_provider: Some(CodeActionProviderCapability::Options(
                    CodeActionOptions {
                        code_action_kinds: Some(vec![CodeActionKind::QUICKFIX]),
                        ..Default::default()
                    },
                )),
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options: Default::default(),
                })),
                call_hierarchy_provider: Some(CallHierarchyServerCapability::Simple(true)),
                document_formatting_provider: Some(OneOf::Left(true)),
                execute_command_provider: Some(ExecuteCommandOptions {
                    commands: vec![RENAME_CLASS.into()],
                    ..Default::default()
                }),
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(
                        SemanticTokensOptions {
                            legend: semantic_legend(),
                            full: Some(SemanticTokensFullOptions::Bool(true)),
                            range: None,
                            ..Default::default()
                        },
                    ),
                ),
                ..Default::default()
            },
            offset_encoding: None,
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        let watch = Registration {
            id: "smali-watch".into(),
            method: "workspace/didChangeWatchedFiles".into(),
            register_options: serde_json::to_value(DidChangeWatchedFilesRegistrationOptions {
                watchers: vec![FileSystemWatcher {
                    glob_pattern: GlobPattern::String("**/*.smali".into()),
                    kind: None,
                }],
            })
            .ok(),
        };
        if let Err(e) = self.client.register_capability(vec![watch]).await {
            tracing::info!("client declined file watching: {e:?}");
        }
        self.start_indexing();
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let doc = Document::parse(&params.text_document.text);
        if let Some(p) = path_of(&uri) {
            write(&self.st.index).update_file(scan::fast_entry(&p, &doc.text, Origin::Workspace));
        }
        write(&self.st.docs).insert(
            uri.clone(),
            Open {
                doc,
                version: params.text_document.version,
            },
        );
        self.schedule_diagnostics(uri);
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        let enc = self.st.enc();
        let Some(old) = self.st.doc(&uri) else { return };
        let changes = params.content_changes;
        let new = guarded("did_change", move || {
            match byte_edits(&old, &changes, enc) {
                Some(edits) => old.apply_edits(&edits),
                None => {
                    // Fall back to rebuilding from the last full-text change if offsets were invalid.
                    let full = changes
                        .iter()
                        .rev()
                        .find(|c| c.range.is_none())
                        .map(|c| c.text.clone());
                    Document::parse(&full.unwrap_or_else(|| old.text.to_string()))
                }
            }
        })
        .await;
        let Some(doc) = new else { return };
        if let Some(p) = path_of(&uri) {
            write(&self.st.index).update_file(scan::fast_entry(&p, &doc.text, Origin::Workspace));
        }
        write(&self.st.docs).insert(
            uri.clone(),
            Open {
                doc,
                version: params.text_document.version,
            },
        );
        self.schedule_diagnostics(uri);
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        write(&self.st.docs).remove(&uri);
        write(&self.st.generations).remove(&uri);
        // Back to the on-disk content for the index.
        if let Some(p) = path_of(&uri) {
            match std::fs::read_to_string(&p) {
                Ok(t) => {
                    write(&self.st.index).update_file(scan::fast_entry(&p, &t, Origin::Workspace))
                }
                Err(_) => write(&self.st.index).remove_file(&p),
            }
        }
        self.client.publish_diagnostics(uri, Vec::new(), None).await;
    }

    async fn did_change_watched_files(&self, params: DidChangeWatchedFilesParams) {
        for ev in params.changes {
            let Some(p) = path_of(&ev.uri) else { continue };
            if self.st.doc(&ev.uri).is_some() {
                continue;
            }
            if ev.typ == FileChangeType::DELETED {
                write(&self.st.index).remove_file(&p);
            } else if let Ok(t) = std::fs::read_to_string(&p) {
                write(&self.st.index).update_file(scan::fast_entry(&p, &t, Origin::Workspace));
            }
        }
        self.republish_all();
    }

    async fn did_change_configuration(&self, params: DidChangeConfigurationParams) {
        let Some(new) = Config::from_value(&params.settings) else {
            return;
        };
        let reindex = {
            let mut c = write(&self.st.config);
            let changed = c.index_inputs() != new.index_inputs();
            *c = new;
            changed
        };
        if reindex {
            self.start_indexing();
        } else {
            self.republish_all();
        }
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let Some((doc, enc)) = self.with_doc(&uri) else {
            return Ok(None);
        };
        let pos = params.text_document_position_params.position;
        let st = self.st.clone();
        let h = guarded("hover", move || {
            let idx = read(&st.index);
            let ctx = Context {
                index: Some(&idx),
                expected_class: None,
            };
            let off = convert::offset(&doc, pos, enc);
            hover::hover(&doc, &ctx, off).map(|h| Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: h.markdown,
                }),
                range: Some(convert::range(&doc, h.span, enc)),
            })
        })
        .await;
        Ok(h.flatten())
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let Some((doc, enc)) = self.with_doc(&uri) else {
            return Ok(None);
        };
        let pos = params.text_document_position.position;
        let st = self.st.clone();
        let items = guarded("completion", move || {
            let idx = read(&st.index);
            let ctx = Context {
                index: Some(&idx),
                expected_class: None,
            };
            let off = convert::offset(&doc, pos, enc);
            completion::complete(&doc, &ctx, off)
                .into_iter()
                .enumerate()
                .map(|(i, it)| CompletionItem {
                    label: it.label.clone(),
                    kind: Some(match it.kind {
                        completion::ItemKind::Opcode => CompletionItemKind::KEYWORD,
                        completion::ItemKind::Directive => CompletionItemKind::KEYWORD,
                        completion::ItemKind::Register => CompletionItemKind::VARIABLE,
                        completion::ItemKind::Label => CompletionItemKind::REFERENCE,
                        completion::ItemKind::Class => CompletionItemKind::CLASS,
                        completion::ItemKind::Interface => CompletionItemKind::INTERFACE,
                        completion::ItemKind::Method => CompletionItemKind::METHOD,
                        completion::ItemKind::Field => CompletionItemKind::FIELD,
                    }),
                    detail: (!it.detail.is_empty()).then(|| it.detail.clone()),
                    sort_text: Some(format!("{i:05}")),
                    filter_text: it.filter.clone(),
                    insert_text_format: Some(if it.snippet {
                        InsertTextFormat::SNIPPET
                    } else {
                        InsertTextFormat::PLAIN_TEXT
                    }),
                    text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                        range: convert::range(&doc, it.replace, enc),
                        new_text: it.insert.clone(),
                    })),
                    additional_text_edits: (!it.extra_edits.is_empty()).then(|| {
                        it.extra_edits
                            .iter()
                            .map(|e| convert::text_edit(&doc, e, enc))
                            .collect()
                    }),
                    ..Default::default()
                })
                .collect::<Vec<_>>()
        })
        .await;
        Ok(items.map(CompletionResponse::Array))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let Some((doc, enc)) = self.with_doc(&uri) else {
            return Ok(None);
        };
        let pos = params.text_document_position_params.position;
        let st = self.st.clone();
        let d2 = doc.clone();
        let locs = guarded("definition", move || {
            let idx = read(&st.index);
            let ctx = Context {
                index: Some(&idx),
                expected_class: None,
            };
            nav::definition(&d2, &ctx, convert::offset(&d2, pos, enc))
        })
        .await
        .unwrap_or_default();
        let out = self.locations(&uri, &doc, locs);
        Ok((!out.is_empty()).then_some(GotoDefinitionResponse::Array(out)))
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri;
        let Some((doc, enc)) = self.with_doc(&uri) else {
            return Ok(None);
        };
        let pos = params.text_document_position.position;
        let include = params.context.include_declaration;
        let st = self.st.clone();
        let d2 = doc.clone();
        let locs = guarded("references", move || {
            let idx = read(&st.index);
            let ctx = Context {
                index: Some(&idx),
                expected_class: None,
            };
            nav::references(&d2, &ctx, convert::offset(&d2, pos, enc), include)
        })
        .await
        .unwrap_or_default();
        let me = self.clone_ref();
        let uri2 = uri.clone();
        let out = guarded("references-locations", move || {
            me.locations(&uri2, &doc, locs)
        })
        .await;
        Ok(out)
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let Some((doc, enc)) = self.with_doc(&params.text_document.uri) else {
            return Ok(None);
        };
        let syms = guarded("document_symbol", move || {
            nav::document_symbols(&doc)
                .iter()
                .map(|s| doc_symbol(&doc, s, enc))
                .collect::<Vec<_>>()
        })
        .await;
        Ok(syms.map(DocumentSymbolResponse::Nested))
    }

    async fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> Result<Option<WorkspaceSymbolResponse>> {
        let st = self.st.clone();
        let enc = self.st.enc();
        let syms = guarded("workspace_symbol", move || {
            let idx = read(&st.index);
            nav::workspace_symbols(&idx, &params.query, 200)
                .into_iter()
                .filter_map(|s| {
                    let uri = uri_of(&s.location.path)?;
                    #[allow(deprecated)]
                    Some(SymbolInformation {
                        name: s.name,
                        kind: symbol_kind(s.kind),
                        tags: None,
                        deprecated: None,
                        location: Location::new(
                            uri,
                            convert::range_in_file(&s.location.path, s.location.span, enc),
                        ),
                        container_name: (!s.container.is_empty()).then_some(s.container),
                    })
                })
                .collect::<Vec<_>>()
        })
        .await;
        Ok(syms.map(WorkspaceSymbolResponse::Flat))
    }

    async fn folding_range(&self, params: FoldingRangeParams) -> Result<Option<Vec<FoldingRange>>> {
        let Some((doc, enc)) = self.with_doc(&params.text_document.uri) else {
            return Ok(None);
        };
        let out = guarded("folding_range", move || {
            nav::folding_ranges(&doc)
                .into_iter()
                .map(|f| {
                    let r = convert::range(&doc, f.span, enc);
                    FoldingRange {
                        start_line: r.start.line,
                        end_line: r.end.line,
                        kind: Some(if f.comment {
                            FoldingRangeKind::Comment
                        } else {
                            FoldingRangeKind::Region
                        }),
                        ..Default::default()
                    }
                })
                .collect::<Vec<_>>()
        })
        .await;
        Ok(out)
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let uri = params.text_document.uri;
        let Some((doc, enc)) = self.with_doc(&uri) else {
            return Ok(None);
        };
        let st = self.st.clone();
        let expected = self.st.expected_class(&uri);
        let range = params.range;
        let diags = params.context.diagnostics;
        let out = guarded("code_action", move || {
            let idx = read(&st.index);
            let ctx = Context {
                index: Some(&idx),
                expected_class: expected.as_deref(),
            };
            let span = convert::span(&doc, range, enc);
            actions::code_actions(&doc, &ctx, span)
                .into_iter()
                .map(|a| {
                    let edits = a
                        .edits
                        .iter()
                        .map(|e| convert::text_edit(&doc, e, enc))
                        .collect();
                    let related: Vec<Diagnostic> = diags
                        .iter()
                        .filter(|d| d.code == Some(NumberOrString::String(a.code.to_string())))
                        .cloned()
                        .collect();
                    CodeActionOrCommand::CodeAction(CodeAction {
                        title: a.title,
                        kind: Some(CodeActionKind::QUICKFIX),
                        diagnostics: (!related.is_empty()).then_some(related),
                        edit: Some(WorkspaceEdit {
                            changes: Some(HashMap::from([(uri.clone(), edits)])),
                            ..Default::default()
                        }),
                        is_preferred: Some(a.preferred),
                        ..Default::default()
                    })
                })
                .collect::<Vec<_>>()
        })
        .await;
        Ok(out)
    }

    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> Result<Option<PrepareRenameResponse>> {
        let Some((doc, enc)) = self.with_doc(&params.text_document.uri) else {
            return Ok(None);
        };
        let span = refactor::rename_target(&doc, convert::offset(&doc, params.position, enc));
        Ok(span.map(|s| PrepareRenameResponse::Range(convert::range(&doc, s, enc))))
    }

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let uri = params.text_document_position.text_document.uri;
        let Some((doc, enc)) = self.with_doc(&uri) else {
            return Ok(None);
        };
        let pos = params.text_document_position.position;
        let st = self.st.clone();
        let new_name = params.new_name;
        let res = guarded("rename", move || {
            let idx = read(&st.index);
            let ctx = Context {
                index: Some(&idx),
                expected_class: None,
            };
            refactor::rename_member(&doc, &ctx, convert::offset(&doc, pos, enc), &new_name)
        })
        .await;
        match res {
            Some(Ok(edits)) => Ok(Some(WorkspaceEdit {
                changes: Some(self.workspace_edit(&uri, edits)),
                ..Default::default()
            })),
            Some(Err(e)) => Err(tower_lsp_server::jsonrpc::Error::invalid_params(
                e.to_string(),
            )),
            None => Ok(None),
        }
    }

    async fn prepare_call_hierarchy(
        &self,
        params: CallHierarchyPrepareParams,
    ) -> Result<Option<Vec<CallHierarchyItem>>> {
        let uri = params.text_document_position_params.text_document.uri;
        let Some((doc, enc)) = self.with_doc(&uri) else {
            return Ok(None);
        };
        let pos = params.text_document_position_params.position;
        let item = {
            let idx = read(&self.st.index);
            let ctx = Context {
                index: Some(&idx),
                expected_class: None,
            };
            refactor::call_item_at(&doc, &ctx, convert::offset(&doc, pos, enc))
        };
        Ok(item.and_then(|i| self.call_item(&i, &uri)).map(|i| vec![i]))
    }

    async fn incoming_calls(
        &self,
        params: CallHierarchyIncomingCallsParams,
    ) -> Result<Option<Vec<CallHierarchyIncomingCall>>> {
        let Some(item) = call_item_from(&params.item) else {
            return Ok(None);
        };
        let sites = refactor::incoming_call_sites(&read(&self.st.index), &item);
        let enc = self.st.enc();
        let mut by_caller: Vec<(refactor::CallItem, PathBuf, Vec<Range>)> = Vec::new();
        let mut docs: HashMap<PathBuf, Document> = HashMap::new();
        for (path, span) in sites {
            let doc = match docs.get(&path) {
                Some(d) => d.clone(),
                None => {
                    let Some(d) = self.file_doc(&path) else {
                        continue;
                    };
                    docs.insert(path.clone(), d.clone());
                    d
                }
            };
            let Some(mut caller) = refactor::enclosing_method(&doc, span.start) else {
                continue;
            };
            if let Some(l) = caller.loc.as_mut() {
                l.path = Some(path.clone());
            }
            let r = convert::range(&doc, span, enc);
            match by_caller
                .iter_mut()
                .find(|(c, p, _)| *c == caller && *p == path)
            {
                Some((_, _, rs)) => rs.push(r),
                None => by_caller.push((caller, path, vec![r])),
            }
        }
        let fallback = params.item.uri.clone();
        Ok(Some(
            by_caller
                .into_iter()
                .filter_map(|(c, _, ranges)| {
                    Some(CallHierarchyIncomingCall {
                        from: self.call_item(&c, &fallback)?,
                        from_ranges: ranges,
                    })
                })
                .collect(),
        ))
    }

    async fn outgoing_calls(
        &self,
        params: CallHierarchyOutgoingCallsParams,
    ) -> Result<Option<Vec<CallHierarchyOutgoingCall>>> {
        let Some(item) = call_item_from(&params.item) else {
            return Ok(None);
        };
        let Some(path) = path_of(&params.item.uri) else {
            return Ok(None);
        };
        let Some(doc) = self.file_doc(&path) else {
            return Ok(None);
        };
        let enc = self.st.enc();
        let mut out = Vec::new();
        for (mut callee, spans) in refactor::outgoing_calls(&doc, &item) {
            let resolved = match read(&self.st.index).resolve_method(
                &callee.owner,
                &callee.name,
                &callee.proto,
            ) {
                smali_index::Lookup::Found { owner, member } => owner.location_of(member.span),
                _ => None,
            };
            callee.loc = resolved.map(nav::Loc::from);
            let Some(to) = self.call_item(&callee, &params.item.uri) else {
                continue;
            };
            out.push(CallHierarchyOutgoingCall {
                to,
                from_ranges: spans
                    .iter()
                    .map(|s| convert::range(&doc, *s, enc))
                    .collect(),
            });
        }
        Ok(Some(out))
    }

    async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        let Some((doc, enc)) = self.with_doc(&params.text_document.uri) else {
            return Ok(None);
        };
        let text = doc.text.clone();
        let formatted = guarded("formatting", move || smali_analysis::format::format(&text)).await;
        Ok(formatted.filter(|f| f.as_str() != &*doc.text).map(|f| {
            vec![TextEdit {
                range: convert::range(&doc, smali_syntax::Span::new(0, doc.text.len()), enc),
                new_text: f,
            }]
        }))
    }

    async fn execute_command(&self, params: ExecuteCommandParams) -> Result<Option<LSPAny>> {
        if params.command != RENAME_CLASS {
            return Ok(None);
        }
        let arg = |i: usize| {
            params
                .arguments
                .get(i)
                .and_then(|v| v.as_str())
                .map(str::to_string)
        };
        let (Some(uri_s), Some(new)) = (arg(0), arg(1)) else {
            return Err(tower_lsp_server::jsonrpc::Error::invalid_params(
                "expected [uri, newDescriptor]",
            ));
        };
        let Ok(uri) = uri_s.parse::<Uri>() else {
            return Err(tower_lsp_server::jsonrpc::Error::invalid_params("bad uri"));
        };
        let Some(doc) = self
            .st
            .doc(&uri)
            .or_else(|| path_of(&uri).and_then(|p| self.file_doc(&p)))
        else {
            return Ok(None);
        };
        let Some(old) = doc.model.class.as_ref().map(|c| c.name.text.clone()) else {
            return Ok(None);
        };
        let edits = match refactor::rename_class(&read(&self.st.index), &old, &new) {
            Ok(e) => e,
            Err(e) => {
                return Err(tower_lsp_server::jsonrpc::Error::invalid_params(
                    e.to_string(),
                ))
            }
        };
        let mut ops: Vec<DocumentChangeOperation> = self
            .workspace_edit(&uri, edits)
            .into_iter()
            .map(|(u, edits)| {
                DocumentChangeOperation::Edit(TextDocumentEdit {
                    text_document: OptionalVersionedTextDocumentIdentifier {
                        uri: u,
                        version: None,
                    },
                    edits: edits.into_iter().map(OneOf::Left).collect(),
                })
            })
            .collect();
        // Move the file to match the new descriptor when it sits under a smali root.
        let extra = read(&self.st.config).extra_roots.clone();
        if let Some(root) = path_of(&uri).and_then(|p| smali_index::workspace::root_of(&p, &extra))
        {
            let rel = new.trim_start_matches('L').trim_end_matches(';');
            if let Some(new_uri) = uri_of(&root.join(format!("{rel}.smali"))).filter(|n| *n != uri)
            {
                ops.push(DocumentChangeOperation::Op(ResourceOp::Rename(
                    RenameFile {
                        old_uri: uri.clone(),
                        new_uri,
                        options: None,
                        annotation_id: None,
                    },
                )));
            }
        }
        let edit = WorkspaceEdit {
            document_changes: Some(DocumentChanges::Operations(ops)),
            ..Default::default()
        };
        let applied = self.client.apply_edit(edit).await;
        Ok(Some(
            serde_json::json!({ "applied": applied.map(|r| r.applied).unwrap_or(false) }),
        ))
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let Some((doc, enc)) = self.with_doc(&params.text_document.uri) else {
            return Ok(None);
        };
        let st = self.st.clone();
        let out = guarded("semantic_tokens", move || {
            let idx = read(&st.index);
            let ctx = Context {
                index: Some(&idx),
                expected_class: None,
            };
            let mut data = Vec::new();
            let (mut pl, mut pc) = (0u32, 0u32);
            for t in tokens::semantic_tokens(&doc, &ctx) {
                let a = doc.position(t.span.start, enc);
                let b = doc.position(t.span.end, enc);
                if a.line != b.line {
                    continue;
                }
                let delta_line = a.line - pl;
                let delta_start = if delta_line == 0 { a.col - pc } else { a.col };
                data.push(SemanticToken {
                    delta_line,
                    delta_start,
                    length: b.col - a.col,
                    token_type: t.ty as u32,
                    token_modifiers_bitset: t.modifiers,
                });
                pl = a.line;
                pc = a.col;
            }
            SemanticTokensResult::Tokens(SemanticTokens {
                result_id: None,
                data,
            })
        })
        .await;
        Ok(out)
    }
}

impl Backend {
    /// Text of a file: the open document if any, else disk.
    fn file_doc(&self, path: &Path) -> Option<Document> {
        if let Some(d) = uri_of(path).and_then(|u| self.st.doc(&u)) {
            return Some(d);
        }
        std::fs::read_to_string(path)
            .ok()
            .map(|t| Document::parse(&t))
    }

    fn workspace_edit(
        &self,
        current: &Uri,
        edits: Vec<refactor::FileEdit>,
    ) -> HashMap<Uri, Vec<TextEdit>> {
        let enc = self.st.enc();
        let mut by_path: HashMap<Option<PathBuf>, Vec<smali_analysis::Edit>> = HashMap::new();
        for e in edits {
            by_path.entry(e.path).or_default().push(e.edit);
        }
        let mut out = HashMap::new();
        for (path, edits) in by_path {
            let (uri, text) = match &path {
                None => (
                    Some(current.clone()),
                    self.st.doc(current).map(|d| d.text.to_string()),
                ),
                Some(p) => {
                    let u = uri_of(p);
                    let open = u
                        .as_ref()
                        .and_then(|u| self.st.doc(u))
                        .map(|d| d.text.to_string());
                    (u, open.or_else(|| std::fs::read_to_string(p).ok()))
                }
            };
            let (Some(uri), Some(text)) = (uri, text) else {
                continue;
            };
            let lines = LineIndex::new(&text);
            let conv = |o: usize| {
                let p = lines.position(&text, o, enc);
                Position::new(p.line, p.col)
            };
            let list = edits
                .iter()
                .map(|e| TextEdit {
                    range: Range::new(conv(e.span.start), conv(e.span.end)),
                    new_text: e.new_text.clone(),
                })
                .collect();
            out.insert(uri, list);
        }
        out
    }

    fn call_item(&self, item: &refactor::CallItem, fallback: &Uri) -> Option<CallHierarchyItem> {
        let enc = self.st.enc();
        let (uri, range) = match &item.loc {
            Some(nav::Loc {
                path: Some(p),
                span,
            }) => (uri_of(p)?, convert::range_in_file(p, *span, enc)),
            Some(nav::Loc { path: None, span }) => {
                let d = self.st.doc(fallback)?;
                (fallback.clone(), convert::range(&d, *span, enc))
            }
            None => return None,
        };
        Some(CallHierarchyItem {
            name: format!("{}{}", item.name, item.proto),
            kind: if item.name == "<init>" {
                SymbolKind::CONSTRUCTOR
            } else {
                SymbolKind::METHOD
            },
            tags: None,
            detail: Some(item.owner.clone()),
            uri,
            range,
            selection_range: range,
            data: Some(serde_json::json!([item.owner, item.name, item.proto])),
        })
    }
}

fn call_item_from(item: &CallHierarchyItem) -> Option<refactor::CallItem> {
    let d = item.data.as_ref()?.as_array()?;
    let s = |i: usize| d.get(i).and_then(|v| v.as_str()).map(str::to_string);
    Some(refactor::CallItem {
        owner: s(0)?,
        name: s(1)?,
        proto: s(2)?,
        loc: None,
    })
}

impl Backend {
    fn clone_ref(&self) -> Backend {
        Backend {
            client: self.client.clone(),
            st: self.st.clone(),
        }
    }
}
