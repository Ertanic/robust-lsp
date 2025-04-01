use crate::{
    completion::{yml::YamlCompletion, Completion},
    goto::{yml::YamlGotoDefinition, GotoDefinition},
    hint::{yaml::YamlInlayHint, InlayHint},
    parse::{
        common::Index,
        csharp,
        structs::{csharp::CsharpObject, fluent::FluentKey, yaml::YamlPrototype},
        yaml, ParseResult, ProjectParser,
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
use tokio::sync::{Mutex, RwLock};
use tower_lsp::{
    jsonrpc::{Error, Result},
    lsp_types::{
        CompletionOptions, CompletionParams, CompletionResponse, DidChangeTextDocumentParams,
        DidOpenTextDocumentParams, DidSaveTextDocumentParams, GotoDefinitionParams,
        GotoDefinitionResponse, InitializeParams, InitializeResult, InitializedParams,
        InlayHintParams, MessageType, OneOf::Left, ServerCapabilities, TextDocumentSyncCapability,
        TextDocumentSyncKind, Url,
    },
    Client, LanguageServer,
};
use tracing::instrument;
use tree_sitter::Tree;

pub type FluentLocales = Arc<RwLock<HashSet<FluentKey>>>;
pub type CsharpClasses = Arc<RwLock<HashSet<CsharpObject>>>;
pub type YamlPrototypes = Arc<RwLock<HashSet<YamlPrototype>>>;
pub type ParsedFiles = Arc<RwLock<HashMap<PathBuf, Tree>>>;

#[derive(Default)]
pub struct Context {
    pub parsed_files: ParsedFiles,
    pub classes: CsharpClasses,
    pub prototypes: YamlPrototypes,
    pub locales: FluentLocales,
}

pub(crate) struct Backend {
    client: Arc<Client>,
    opened_files: RwLock<HashMap<Url, Rope>>,
    context: Arc<Context>,
    root_uri: Arc<Mutex<Option<Url>>>,
}

impl Backend {
    pub(crate) fn new(client: Client) -> Self {
        Self {
            client: Arc::new(client),
            opened_files: Default::default(),
            context: Default::default(),
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

        self.root_uri.lock().await.replace(params.root_uri.expect("root_uri is not found"));

        Ok(InitializeResult {
            server_info: None,
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::INCREMENTAL,
                )),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![" ".to_string()]),
                    ..Default::default()
                }),
                definition_provider: Some(Left(true)),
                inlay_hint_provider: Some(Left(true)),
                ..Default::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "Server initialized!")
            .await;

        let guard = self.root_uri.lock().await;
        let root_path = guard.as_ref().unwrap().to_file_path().expect("invalid path");

        let parser = ProjectParser::new(&root_path, self.context.clone(), self.client.clone());
        parser.parse().await;
    }

    #[instrument(skip_all, fields(uri = %params.text_document.uri))]
    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let Ok(path) = params.text_document.uri.to_file_path() else {
            return;
        };

        if !path.is_file() {
            return;
        }

        match std::fs::File::open(&path) {
            Ok(handler) => {
                let rope = Rope::from_reader(handler).unwrap();
                self.opened_files
                    .write()
                    .await
                    .insert(params.text_document.uri, rope);
                tracing::trace!("Document has been cached.");
            }
            Err(err) => {
                tracing::trace!("File can't be opened: {}", err);
            }
        }
    }

    #[instrument(skip_all, fields(uri = %params.text_document.uri))]
    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let mut lock = self.opened_files.write().await;
        let found_rope = lock.get_mut(&params.text_document.uri);

        match found_rope {
            Some(rope) => {
                for change in params.content_changes {
                    if let Some(range) = change.range {
                        let start_idx = rope.line_to_char(range.start.line as usize)
                            + range.start.character as usize;
                        let end_idx = rope.line_to_char(range.end.line as usize)
                            + range.end.character as usize;

                        if let Err(err) = rope.try_remove(start_idx..end_idx) {
                            tracing::warn!("Failed to remove text from document: {}.", err);
                        };
                        if let Err(err) = rope.try_insert(start_idx, &change.text) {
                            tracing::warn!("Failed to insert text into document: {}.", err);
                        }

                        tracing::trace!("Document has been changed.");
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
                let result = csharp::parse(path.clone(), self.context.parsed_files.clone()).await;
                match result {
                    ParseResult::Csharp(_) => {
                        let ParseResult::Csharp(parsed_classes) = result else {
                            tracing::warn!("Failed to parse C# prototypes while saving file.");
                            return;
                        };

                        let mut lock = self.context.classes.write().await;
                        let diff = lock
                            .par_iter()
                            .filter(|c| c.index().0 == path)
                            .filter(|c| !parsed_classes.contains(c))
                            .cloned()
                            .collect::<Vec<_>>();

                        for class in parsed_classes {
                            tracing::info!("New/changed class: {}", class.name);
                            lock.insert(class);
                        }
                        for class in diff {
                            tracing::info!("Remove class: {}", class.name);
                            lock.remove(&class);
                        }
                    }
                    _ => {
                        tracing::warn!("Failed to parse file: {}.", params.text_document.uri);
                        return;
                    }
                }
            }
            "yml" | "yaml" => {
                let result = yaml::parse(path.clone(), self.context.parsed_files.clone()).await;
                match result {
                    ParseResult::YamlPrototypes(_) => {
                        let ParseResult::YamlPrototypes(parsed_prototypes) = result else {
                            tracing::warn!("Failed to parse YAML prototypes while saving file.");
                            return;
                        };
                        let mut lock = self.context.prototypes.write().await;
                        let diff = lock
                            .par_iter()
                            .filter(|p| p.index().0 == path)
                            .filter(|p| !parsed_prototypes.contains(p))
                            .cloned()
                            .collect::<Vec<_>>();

                        for proto in parsed_prototypes {
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
                    _ => {
                        tracing::warn!("Failed to parse file: {}.", params.text_document.uri);
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

        let root_path = self.root_uri.lock().await.as_ref().unwrap().to_file_path().unwrap_or_default();

        match extension {
            "yml" | "yaml" => {
                let opened = self.opened_files.read().await;
                let rope = opened.get(&params.text_document_position.text_document.uri);

                match rope {
                    Some(rope) => {
                        let completion = YamlCompletion::new(self.context.clone(), params.text_document_position.position, rope, root_path);
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

    #[rustfmt::skip]
    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        tracing::trace!("Goto definition request has been received.");

        let file = params.text_document_position_params.text_document.uri.to_file_path().unwrap_or_default();
        let extension = file.extension().unwrap_or_default().to_str().unwrap_or_default();

        match extension {
            "yml" | "yaml" => {
                let opened = self.opened_files.read().await;
                let rope = opened.get(&params.text_document_position_params.text_document.uri);

                match rope {
                    Some(rope) => {
                        let definition = YamlGotoDefinition::new(self.context.clone(), params.text_document_position_params.position, rope);
                        Ok(definition.goto_definition())
                    }
                    None => {
                        tracing::trace!("File wasn't cached.");
                        Ok(None)
                    }
                }
            }
            _ => Ok(None)
        }
    }

    async fn inlay_hint(
        &self,
        params: InlayHintParams,
    ) -> Result<Option<Vec<tower_lsp::lsp_types::InlayHint>>> {
        tracing::trace!("Inlay hint request has been received.");

        let file = params.text_document.uri.to_file_path().unwrap_or_default();
        let extension = file
            .extension()
            .unwrap_or_default()
            .to_str()
            .unwrap_or_default();

        match extension {
            "yml" | "yaml" => {
                let opened = self.opened_files.read().await;
                let rope = opened.get(&params.text_document.uri);

                match rope {
                    Some(rope) => {
                        let hint =
                            YamlInlayHint::new(self.context.classes.clone(), params.range, rope);
                        Ok(hint.inlay_hint())
                    }
                    None => {
                        tracing::trace!("File wasn't cached.");
                        Ok(None)
                    }
                }
            }
            _ => Ok(None),
        }
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}
