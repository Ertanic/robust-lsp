use crate::{
    completion::{yml::YamlCompletion, Completion},
    parse::{
        csharp, parse_project,
        structs::{csharp::CsharpClass, yaml::YamlPrototype},
        yaml,
    },
    utils::check_project_compliance,
};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
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
        DidOpenTextDocumentParams, DidSaveTextDocumentParams, InitializeParams, InitializeResult,
        InitializedParams, MessageType, ServerCapabilities, TextDocumentSyncCapability,
        TextDocumentSyncKind, Url,
    },
    Client, LanguageServer,
};
use tracing::instrument;
use tree_sitter::Tree;

pub(crate) type CsharpClasses = Arc<RwLock<HashSet<CsharpClass>>>;
pub(crate) type YamlPrototypes = Arc<RwLock<HashSet<YamlPrototype>>>;
pub(crate) type ParsedFiles = Arc<RwLock<HashMap<PathBuf, Tree>>>;

pub(crate) struct Backend {
    client: Arc<Client>,
    opened_files: RwLock<HashMap<Url, Rope>>,
    parsed_files: ParsedFiles,
    classes: CsharpClasses,
    prototypes: YamlPrototypes,
    root_uri: Arc<RwLock<Option<Url>>>,
}

impl Backend {
    pub(crate) fn new(client: Client) -> Self {
        Self {
            client: Arc::new(client),
            opened_files: Default::default(),
            parsed_files: ParsedFiles::default(),
            classes: CsharpClasses::default(),
            prototypes: Default::default(),
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
            self.prototypes.clone(),
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

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let path = match params.text_document.uri.to_file_path() {
            Ok(p) => p,
            Err(_) => {
                tracing::warn!(
                    "Failed to convert uri to path: {}.",
                    params.text_document.uri
                );
                return;
            }
        };
        let ext = path
            .extension()
            .unwrap_or_default()
            .to_str()
            .unwrap_or_default();

        match ext {
            "cs" => {
                let classes = csharp::parse(path.clone(), self.parsed_files.clone()).await;
                match classes {
                    Ok(classes) => {
                        let mut lock = self.classes.write().await;
                        let diff = lock
                            .par_iter()
                            .filter(|c| c.file == path)
                            .filter(|c| !classes.contains(c))
                            .cloned()
                            .collect::<Vec<_>>();

                        for class in classes {
                            tracing::info!("New/changed class: {}", class.name);
                            lock.insert(class);
                        }
                        for class in diff {
                            tracing::info!("Remove class: {}", class.name);
                            lock.remove(&class);
                        }
                    }
                    Err(_) => {
                        tracing::warn!("Failed to parse the file {}", path.display());
                        return;
                    }
                }
            }
            "yml" | "yaml" => {
                let prototypes = yaml::parse(path.clone(), self.parsed_files.clone()).await;
                match prototypes {
                    Ok(prototypes) => {
                        let mut lock = self.prototypes.write().await;
                        let diff = lock
                            .par_iter()
                            .filter(|p| p.file == path)
                            .filter(|p| !prototypes.contains(p))
                            .cloned()
                            .collect::<Vec<_>>();

                        for proto in prototypes {
                            tracing::info!(
                                "New/changed prototype: {} with id {}",
                                proto.prototype,
                                proto.id
                            );
                            lock.insert(proto);
                        }

                        for proto in diff {
                            tracing::info!(
                                "Remove prototype: {} with id {}",
                                proto.prototype,
                                proto.id
                            );
                            lock.remove(&proto);
                        }
                    }
                    Err(_) => {
                        tracing::warn!("Failed to parse the file {}", path.display());
                        return;
                    }
                }
            }
            _ => {}
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
                    Some(rope) => {
                        let completion = YamlCompletion::new(self.classes.clone(), params.text_document_position.position, rope);
                        Ok(completion.completion())
                    },
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
