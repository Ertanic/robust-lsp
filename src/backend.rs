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
    references::csharp::CsharpReferencesProvider,
    references::ReferencesProvider,
    semantic::fluent::SemanticAnalyzer,
    utils::check_project_compliance,
    utils::get_ext,
};
use fluent_syntax::ast::Entry;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use ropey::Rope;
use std::{
    collections::{HashMap, HashSet},
    ops::Deref,
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
        InlayHintParams, Location, MessageType, OneOf::Left, ReferenceParams, ServerCapabilities,
        TextDocumentSyncCapability, TextDocumentSyncKind, Url,
    },
    lsp_types::{SemanticTokenType, SemanticTokens, SemanticTokensLegend},
    lsp_types::{
        SemanticTokensFullOptions, SemanticTokensOptions, SemanticTokensParams,
        SemanticTokensResult, SemanticTokensServerCapabilities,
    },
    Client, LanguageServer,
};
use tracing::instrument;
use tree_sitter::{Parser, Tree};
use crate::semantic::fluent::to_relative_semantic_tokens;

pub type FluentLocales = Arc<RwLock<HashSet<Arc<FluentKey>>>>;
pub type CsharpObjects = Arc<RwLock<HashSet<Arc<CsharpObject>>>>;
pub type YamlPrototypes = Arc<RwLock<HashSet<Arc<YamlPrototype>>>>;
pub type ParsedFiles = Arc<RwLock<HashMap<PathBuf, Arc<Tree>>>>;

#[derive(Default)]
pub struct Context {
    pub parsed_files: ParsedFiles,
    pub classes: CsharpObjects,
    pub prototypes: YamlPrototypes,
    pub locales: FluentLocales,
}

struct OpenedFile {
    pub rope: Arc<RwLock<Rope>>,
    pub tree: Arc<Tree>,
}

pub(crate) struct Backend {
    client: Arc<Client>,
    opened_files: RwLock<HashMap<Url, OpenedFile>>,
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

        self.root_uri
            .lock()
            .await
            .replace(params.root_uri.expect("root_uri is not found"));

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
                references_provider: Some(Left(true)),
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(
                        SemanticTokensOptions {
                            full: Some(SemanticTokensFullOptions::Bool(true)),
                            legend: SemanticTokensLegend {
                                token_types: vec![
                                    SemanticTokenType::new("enumMember"),
                                    SemanticTokenType::new("string"),
                                    SemanticTokenType::new("comment"),
                                    SemanticTokenType::new("number"),
                                    SemanticTokenType::new("function"),
                                    SemanticTokenType::new("operator"),
                                    SemanticTokenType::new("variable"),
                                    SemanticTokenType::new("parameter"),
                                ],
                                ..Default::default()
                            },
                            ..Default::default()
                        },
                    ),
                ),
                ..Default::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "Server initialized!")
            .await;

        let guard = self.root_uri.lock().await;
        let root_path = guard
            .as_ref()
            .unwrap()
            .to_file_path()
            .expect("invalid path");

        let parser = ProjectParser::new(&root_path, self.context.clone(), self.client.clone());
        parser.parse().await;
    }

    #[instrument(skip_all, fields(uri = %params.text_document.uri))]
    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let path = params.text_document.uri.to_file_path().unwrap_or_default();

        let tree = self
            .context
            .parsed_files
            .read()
            .await
            .get(&path)
            .map(Arc::clone);

        if let Some(tree) = tree {
            let content = std::fs::read_to_string(path).unwrap_or_default();
            let rope = Arc::new(RwLock::new(Rope::from(content)));
            let opened_file = OpenedFile { rope, tree };

            self.opened_files
                .write()
                .await
                .insert(params.text_document.uri, opened_file);

            tracing::trace!("Document has been cached.");
        } else {
            tracing::trace!("File can't be cached.");
        }
    }

    #[instrument(skip_all, fields(uri = %params.text_document.uri))]
    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let opened_files_guard = self.opened_files.read().await;
        let found_rope = opened_files_guard.get(&params.text_document.uri);

        match found_rope {
            Some(OpenedFile { rope, tree }) => {
                let mut rope_guard = rope.write().await;

                for change in params.content_changes {
                    if let Some(range) = change.range {
                        let start_idx = rope_guard.line_to_char(range.start.line as usize)
                            + range.start.character as usize;
                        let end_idx = rope_guard.line_to_char(range.end.line as usize)
                            + range.end.character as usize;

                        if let Err(err) = rope_guard.try_remove(start_idx..end_idx) {
                            tracing::warn!("Failed to remove text from document: {}.", err);
                        };
                        if let Err(err) = rope_guard.try_insert(start_idx, &change.text) {
                            tracing::warn!("Failed to insert text into document: {}.", err);
                        }

                        tracing::trace!("Document has been changed.");
                    }
                }

                let mut parser = Parser::new();
                let path = params.text_document.uri.to_file_path().unwrap_or_default();
                match get_ext(&path) {
                    "cs" => {
                        parser
                            .set_language(&tree_sitter_c_sharp::LANGUAGE.into())
                            .unwrap();

                        let new_tree = parser.parse(rope_guard.to_string(), Some(tree.deref()));

                        if let Some(new_tree) = new_tree {
                            let tree = Arc::new(new_tree);
                            let rope = Arc::clone(rope);
                            let opened_file = OpenedFile {
                                rope,
                                tree: Arc::clone(&tree),
                            };

                            drop(rope_guard);
                            drop(opened_files_guard);

                            self.opened_files
                                .write()
                                .await
                                .insert(params.text_document.uri, opened_file);

                            self.context.parsed_files.write().await.insert(path, tree);
                        }
                    }
                    "yaml" | "yml" => {
                        parser.set_language(&tree_sitter_yaml::language()).unwrap();

                        let new_tree = parser.parse(rope_guard.to_string(), Some(tree.deref()));

                        if let Some(new_tree) = new_tree {
                            let tree = Arc::new(new_tree);
                            let rope = Arc::clone(rope);
                            let opened_file = OpenedFile {
                                rope,
                                tree: Arc::clone(&tree),
                            };

                            drop(rope_guard);
                            drop(opened_files_guard);

                            self.opened_files
                                .write()
                                .await
                                .insert(params.text_document.uri, opened_file);

                            self.context.parsed_files.write().await.insert(path, tree);
                        }
                    }
                    _ => {}
                }
            }
            None => {
                tracing::warn!("File wasn't cached.");
            }
        }
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let path = params.text_document.uri.to_file_path().unwrap_or_default();
        let ext = get_ext(&path);

        match ext {
            "cs" => {
                let result = csharp::parse(path.clone(), self.context.parsed_files.clone()).await;
                match result {
                    ParseResult::Csharp(parsed_classes) => {
                        let mut lock = self.context.classes.write().await;
                        let diff = lock
                            .par_iter()
                            .filter(|c| c.index().0 == path && !parsed_classes.contains(c))
                            .cloned()
                            .collect::<Vec<_>>();

                        for class in parsed_classes {
                            lock.insert(Arc::new(class));
                        }

                        for class in diff {
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
                    ParseResult::YamlPrototypes(parsed_prototypes) => {
                        let mut lock = self.context.prototypes.write().await;
                        let diff = lock
                            .par_iter()
                            .filter(|p| p.index().0 == path && !parsed_prototypes.contains(p))
                            .cloned()
                            .collect::<Vec<_>>();

                        for proto in parsed_prototypes {
                            lock.insert(Arc::new(proto));
                        }

                        for proto in diff {
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
        let file = params.text_document_position.text_document.uri.to_file_path().unwrap_or_default();
        let extension = get_ext(&file);

        let root_path = self.root_uri.lock().await.as_ref().unwrap().to_file_path().unwrap_or_default();

        match extension {
            "yml" | "yaml" => {
                let opened = self.opened_files.read().await;
                let opened_file = opened.get(&params.text_document_position.text_document.uri);

                if let Some(OpenedFile { rope, tree }) = opened_file {
                    let completion = YamlCompletion::new(
                            self.context.clone(),
                            params.text_document_position.position,
                            rope.read().await.deref(),
                            Arc::clone(tree),
                            root_path
                        );
                        Ok(completion.completion())
                } else  {
                    Ok(None)
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
        let file = params.text_document_position_params.text_document.uri.to_file_path().unwrap_or_default();
        let extension = get_ext(&file);

        match extension {
            "yml" | "yaml" => {
                let opened = self.opened_files.read().await;
                let opened_file = opened.get(&params.text_document_position_params.text_document.uri);

                if let Some(OpenedFile { rope, tree }) = opened_file {
                    let root = self.root_uri.lock().await.as_ref().unwrap().to_file_path().unwrap_or_default();
                    let definition = YamlGotoDefinition::new(
                        self.context.clone(),
                        params.text_document_position_params.position,
                        rope.read().await.deref(),
                        Arc::clone(tree),
                        root,
                    );
                    Ok(definition.goto_definition())
                } else {
                    tracing::trace!("File wasn't cached.");
                    Ok(None)
                }
            }
            _ => Ok(None)
        }
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let file = params
            .text_document_position
            .text_document
            .uri
            .to_file_path()
            .unwrap_or_default();
        let extension = get_ext(&file);

        match extension {
            "cs" => {
                let opened = self.opened_files.read().await;
                let opened_file = opened.get(&params.text_document_position.text_document.uri);

                if let Some(OpenedFile { rope, tree }) = opened_file {
                    let provider = CsharpReferencesProvider::new(
                        self.context.clone(),
                        params.text_document_position.position,
                        rope.read().await.deref(),
                        Arc::clone(tree),
                    );
                    Ok(provider.get_references())
                } else {
                    tracing::trace!("File wasn't cached.");
                    Ok(None)
                }
            }
            _ => Ok(None),
        }
    }

    async fn inlay_hint(
        &self,
        params: InlayHintParams,
    ) -> Result<Option<Vec<tower_lsp::lsp_types::InlayHint>>> {
        let file = params.text_document.uri.to_file_path().unwrap_or_default();
        let extension = get_ext(&file);

        match extension {
            "yml" | "yaml" => {
                let opened = self.opened_files.read().await;
                let opened_file = opened.get(&params.text_document.uri);

                if let Some(OpenedFile { rope, tree }) = opened_file {
                    let hint = YamlInlayHint::new(
                        self.context.classes.clone(),
                        params.range,
                        rope.read().await.deref(),
                        Arc::clone(tree),
                    );
                    Ok(hint.inlay_hint())
                } else {
                    tracing::trace!("File wasn't cached.");
                    Ok(None)
                }
            }
            _ => Ok(None),
        }
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let path = params.text_document.uri.to_file_path().unwrap_or_default();

        if get_ext(&path) != "ftl" {
            tracing::trace!("Not .ftl file");
            return Ok(None);
        }

        let content = std::fs::read_to_string(path).unwrap_or_default();
        let resource =
            fluent_syntax::parser::parse(content.as_str()).unwrap_or_else(|(res, errors)| {
                tracing::warn!("{:?}", errors);
                res
            });

        let values = resource
            .body
            .into_iter()
            .inspect(|msg| tracing::trace!("Message: {:?}", msg))
            .filter_map(|e| {
                let analyzer = SemanticAnalyzer::new(&content);
                match e {
                    Entry::Message(msg) => Some(analyzer.message_to_semantic(msg)),
                    Entry::Comment(comment)
                    | Entry::GroupComment(comment)
                    | Entry::ResourceComment(comment) => {
                        Some(vec![analyzer.comment_to_semantic(comment)])
                    }
                    Entry::Term(term) => Some(analyzer.term_to_semantic(term)),
                    _ => None,
                }
            })
            .flatten()
            .collect::<Vec<_>>();

        let relative_tokens = to_relative_semantic_tokens(values);

        Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
            data: relative_tokens,
            ..Default::default()
        })))
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}
