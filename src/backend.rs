use ropey::Rope;
use tracing::instrument;
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, RwLock},
};
use tower_lsp::{
    jsonrpc::{Error, Result},
    lsp_types::{
        DidChangeTextDocumentParams, DidOpenTextDocumentParams, InitializeParams, InitializeResult,
        InitializedParams, MessageType, ServerCapabilities, TextDocumentSyncCapability,
        TextDocumentSyncKind, Url,
    },
    Client, LanguageServer,
};

use crate::{parse::csharp::CsharpClass, utils::check_project_compliance};

pub(crate) struct Backend {
    client: Client,
    opened_files: RwLock<HashMap<Url, Rope>>,
    prototypes: Arc<RwLock<HashSet<CsharpClass>>>,
}

impl Backend {
    pub(crate) fn new(client: Client) -> Self {
        Self {
            client,
            opened_files: Default::default(),
            prototypes: Arc::new(Default::default()),
        }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        if !check_project_compliance(&params) {
            return Err(Error::request_cancelled());
        }

        

        Ok(InitializeResult {
            server_info: None,
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::INCREMENTAL,
                )),
                completion_provider: Some(Default::default()),
                ..Default::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "server initialized!")
            .await;
    }

    #[instrument(skip_all, fields(uri = %params.text_document.uri))]
    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        if let Ok(path) = params.text_document.uri.to_file_path() {
            if path.exists() {
                if let Ok(handler) = std::fs::File::open(&path) {
                    let rope = Rope::from_reader(handler).unwrap();
                    self.opened_files
                        .write()
                        .unwrap()
                        .insert(params.text_document.uri, rope);
                    tracing::trace!("Document has been cached.");
                } else {
                    tracing::trace!("File can't be opened.");
                }
            } else {
                tracing::trace!("File don't exists.");
            }
        } else {
            tracing::trace!("Document is not a file.");
        }
    }

    #[instrument(skip_all, fields(uri = %params.text_document.uri))]
    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        match self
            .opened_files
            .write()
            .unwrap()
            .get_mut(&params.text_document.uri)
        {
            Some(rope) => {
                for change in params.content_changes {
                    if let Some(range) = change.range {
                        let start_idx = rope.line_to_char(range.start.line as usize)
                            + range.start.character as usize;
                        let end_idx = rope.line_to_char(range.end.line as usize)
                            + range.end.character as usize;

                        rope.remove(start_idx..end_idx);
                        rope.insert(start_idx, &change.text);

                        tracing::trace!(
                            "Document has been changed. New changes: {}",
                            change.text
                        );
                    }
                }
            }
            None => {
                tracing::warn!("File wasn't cached.");
            }
        }
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}
