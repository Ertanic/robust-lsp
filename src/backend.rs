use ropey::Rope;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
};
use tokio::sync::RwLock;
use tower_lsp::{
    jsonrpc::{Error, Result},
    lsp_types::{
        CompletionParams, CompletionResponse, DidChangeTextDocumentParams,
        DidOpenTextDocumentParams, InitializeParams, InitializeResult, InitializedParams,
        MessageType, ServerCapabilities, TextDocumentSyncCapability, TextDocumentSyncKind, Url,
    },
    Client, LanguageServer,
};
use tracing::instrument;
use tree_sitter::Tree;

use crate::{completion::yml, parse::{parse_project, structs::CsharpClass}, utils::check_project_compliance};

pub(crate) type CsharpClasses = Arc<RwLock<HashSet<CsharpClass>>>;
pub(crate) type ParsedFiles = Arc<RwLock<HashMap<PathBuf, Tree>>>;

pub(crate) struct Backend {
    client: Arc<Client>,
    opened_files: RwLock<HashMap<Url, Rope>>,
    parsed_files: ParsedFiles,
    classes: CsharpClasses,
    root_uri: Arc<RwLock<Option<Url>>>,
}

impl Backend {
    pub(crate) fn new(client: Client) -> Self {
        Self {
            client: Arc::new(client),
            opened_files: Default::default(),
            parsed_files: ParsedFiles::default(),
            classes: CsharpClasses::default(),
            root_uri: Default::default(),
        }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        tracing::info!("Server is initializing...");

        if !check_project_compliance(&params) {
            return Err(Error::request_cancelled());
        }

        self.root_uri
            .write()
            .await
            .replace(params.root_uri.unwrap());

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

        // I'm shocked by this myself O_O
        let uri = self.root_uri.read().await.clone().unwrap().clone();

        parse_project(
            uri,
            self.classes.clone(),
            self.parsed_files.clone(),
            self.client.clone(),
        )
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
                        .await
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
            .await
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

                        tracing::trace!("Document has been changed. New changes: {}", change.text);
                    }
                }
            }
            None => {
                tracing::warn!("File wasn't cached.");
            }
        }
    }

    #[rustfmt::skip]
    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        tracing::trace!("Completion request has been received.");

        let file = params.text_document_position.text_document.uri.to_file_path().unwrap_or_default();
        let extension = file.extension().unwrap_or_default().to_str().unwrap_or_default();

        match extension {
            "yml" | "yaml" => {
                let opened = self.opened_files.read().await;
                let rope = opened.get(&params.text_document_position.text_document.uri);

                match rope {
                    Some(rope) => yml::completion(rope, params.text_document_position.position, self.classes.clone()),
                    None => Ok(None)
                }
            },
            _ => {
                tracing::trace!("File extension is not supported.");
                Ok(None)
            }
        }
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}
