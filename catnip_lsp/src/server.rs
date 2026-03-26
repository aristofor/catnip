// FILE: catnip_lsp/src/server.rs
use std::collections::HashMap;
use std::sync::Mutex;

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use crate::diagnostics;
use crate::symbols;

pub struct CatnipLsp {
    client: Client,
    documents: Mutex<HashMap<Url, String>>,
}

impl CatnipLsp {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            documents: Mutex::new(HashMap::new()),
        }
    }

    async fn publish_diagnostics(&self, uri: &Url, source: &str) {
        let diags = diagnostics::lint_to_diagnostics(source);
        self.client.publish_diagnostics(uri.clone(), diags, None).await;
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for CatnipLsp {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
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
                // Replace the entire document
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

        let line = params.position.line;
        let col = params.position.character;

        match symbols::find_references(&source, line, col) {
            Some((name, refs)) => {
                // Find the ref that contains the cursor position
                let r = refs
                    .iter()
                    .find(|r| r.start_line == line && r.start_col <= col && col <= r.end_col);
                match r {
                    Some(r) => Ok(Some(PrepareRenameResponse::RangeWithPlaceholder {
                        range: Range {
                            start: Position::new(r.start_line, r.start_col),
                            end: Position::new(r.end_line, r.end_col),
                        },
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

        let line = params.text_document_position.position.line;
        let col = params.text_document_position.position.character;
        let new_name = &params.new_name;

        match symbols::find_references(&source, line, col) {
            Some((_name, refs)) => {
                let edits: Vec<TextEdit> = refs
                    .iter()
                    .map(|r| TextEdit {
                        range: Range {
                            start: Position::new(r.start_line, r.start_col),
                            end: Position::new(r.end_line, r.end_col),
                        },
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
