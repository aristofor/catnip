// FILE: catnip_lsp/src/server.rs
use std::collections::HashMap;
use std::sync::Mutex;

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use crate::diagnostics;
use crate::encoding::PositionEncoding;
use crate::symbols::{self, SymbolRef};

pub struct CatnipLsp {
    client: Client,
    documents: Mutex<HashMap<Url, String>>,
    /// Negotiated during `initialize`; UTF-16 until then (the LSP default).
    encoding: Mutex<PositionEncoding>,
}

impl CatnipLsp {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            documents: Mutex::new(HashMap::new()),
            encoding: Mutex::new(PositionEncoding::default()),
        }
    }

    fn encoding(&self) -> PositionEncoding {
        *self.encoding.lock().unwrap()
    }

    async fn publish_diagnostics(&self, uri: &Url, source: &str) {
        let diags = diagnostics::lint_to_diagnostics(source, self.encoding());
        self.client.publish_diagnostics(uri.clone(), diags, None).await;
    }
}

/// Line text for a 0-indexed line within `source`, or empty when out of range.
fn line_text(source: &str, line: u32) -> &str {
    source.lines().nth(line as usize).unwrap_or("")
}

/// Convert a byte-column `SymbolRef` to an LSP `Range` in the negotiated encoding.
fn ref_to_range(source: &str, r: &SymbolRef, enc: PositionEncoding) -> Range {
    Range {
        start: Position::new(
            r.start_line,
            enc.encode_column(line_text(source, r.start_line), r.start_col as usize),
        ),
        end: Position::new(
            r.end_line,
            enc.encode_column(line_text(source, r.end_line), r.end_col as usize),
        ),
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for CatnipLsp {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        let encoding = PositionEncoding::negotiate(&params.capabilities);
        *self.encoding.lock().unwrap() = encoding;

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                position_encoding: Some(encoding.kind()),
                text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
                document_formatting_provider: Some(OneOf::Left(true)),
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options: WorkDoneProgressOptions::default(),
                })),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "catnip-lsp initialized")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let text = params.text_document.text.clone();
        self.publish_diagnostics(&uri, &text).await;
        self.documents.lock().unwrap().insert(uri, text);
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        if let Some(change) = params.content_changes.into_iter().last() {
            let text = change.text;
            self.publish_diagnostics(&uri, &text).await;
            self.documents.lock().unwrap().insert(uri, text);
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = &params.text_document.uri;
        self.documents.lock().unwrap().remove(uri);
        // Clear diagnostics on close
        self.client.publish_diagnostics(uri.clone(), vec![], None).await;
    }

    async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        let uri = &params.text_document.uri;
        let docs = self.documents.lock().unwrap();
        let source = match docs.get(uri) {
            Some(s) => s.clone(),
            None => return Ok(None),
        };
        drop(docs);

        let config = catnip_tools::config::FormatConfig {
            indent_size: params.options.tab_size as usize,
            ..Default::default()
        };

        match catnip_tools::formatter::format_code(&source, &config) {
            Ok(formatted) => {
                if formatted == source {
                    return Ok(None);
                }
                // Replace the entire document. The end position is past any real
                // line/column, which clients clamp to the document end; it is
                // encoding-independent.
                Ok(Some(vec![TextEdit {
                    range: Range {
                        start: Position::new(0, 0),
                        end: Position::new(u32::MAX, u32::MAX),
                    },
                    new_text: formatted,
                }]))
            }
            Err(_) => Ok(None),
        }
    }

    async fn prepare_rename(&self, params: TextDocumentPositionParams) -> Result<Option<PrepareRenameResponse>> {
        let uri = &params.text_document.uri;
        let docs = self.documents.lock().unwrap();
        let source = match docs.get(uri) {
            Some(s) => s.clone(),
            None => return Ok(None),
        };
        drop(docs);

        let enc = self.encoding();
        let line = params.position.line;
        // The client's character is in the negotiated encoding; tree-sitter wants
        // a byte column.
        let byte_col = enc.decode_column(line_text(&source, line), params.position.character) as u32;

        match symbols::find_references(&source, line, byte_col) {
            Some((name, refs)) => {
                // Find the ref that contains the cursor position (byte columns).
                let r = refs
                    .iter()
                    .find(|r| r.start_line == line && r.start_col <= byte_col && byte_col <= r.end_col);
                match r {
                    Some(r) => Ok(Some(PrepareRenameResponse::RangeWithPlaceholder {
                        range: ref_to_range(&source, r, enc),
                        placeholder: name,
                    })),
                    None => Ok(None),
                }
            }
            None => Ok(None),
        }
    }

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let uri = &params.text_document_position.text_document.uri;
        let docs = self.documents.lock().unwrap();
        let source = match docs.get(uri) {
            Some(s) => s.clone(),
            None => return Ok(None),
        };
        drop(docs);

        let enc = self.encoding();
        let line = params.text_document_position.position.line;
        let byte_col = enc.decode_column(
            line_text(&source, line),
            params.text_document_position.position.character,
        ) as u32;
        let new_name = &params.new_name;

        match symbols::find_references(&source, line, byte_col) {
            Some((_name, refs)) => {
                let edits: Vec<TextEdit> = refs
                    .iter()
                    .map(|r| TextEdit {
                        range: ref_to_range(&source, r, enc),
                        new_text: new_name.clone(),
                    })
                    .collect();

                let mut changes = HashMap::new();
                changes.insert(uri.clone(), edits);

                Ok(Some(WorkspaceEdit {
                    changes: Some(changes),
                    ..Default::default()
                }))
            }
            None => Ok(None),
        }
    }
}
