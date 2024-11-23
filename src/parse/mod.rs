use crate::{
    backend::{Context, ParsedFiles},
    utils::{percentage, ProgressStatus, ProgressStatusInit},
};
use async_scoped::TokioScope;
use futures::future::BoxFuture;
use globset::{Glob, GlobMatcher};
use rayon::prelude::*;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use structs::{csharp::CsharpClass, fluent::FluentKey, yaml::YamlPrototype};
use tokio::sync::Mutex;
use tower_lsp::{lsp_types::Url, Client};
use tracing::instrument;

pub mod index;
pub mod csharp;
pub mod fluent;
pub mod structs;
pub mod yaml;

pub(crate) type Result<T, E = ()> = std::result::Result<T, E>;
#[rustfmt::skip]
pub(crate) type Parser = Arc<dyn (Fn(PathBuf, ParsedFiles, Arc<Client>) -> BoxFuture<'static, Result<ParseResult>>) + Send + Sync>;
#[rustfmt::skip]
pub(crate) type ResultDispatcher = Arc<dyn (Fn(ParseResult, Arc<Context>) -> BoxFuture<'static, ()>) + Send + Sync>;

#[inline(always)]
fn get_folders(uri: &Url) -> Vec<PathBuf> {
    vec![
        "RobustToolbox/Robust.Client",
        "RobustToolbox/Robust.Server",
        "RobustToolbox/Robust.Shared",
        "Content.Client",
        "Content.Server",
        "Content.Shared",
        "Resources/Prototypes",
        "Resources/Locale",
    ]
    .into_iter()
    .map(|f| uri.to_file_path().unwrap().join(f))
    .filter(|f| f.exists())
    .collect()
}

pub enum ParseResult {
    Csharp(Vec<CsharpClass>),
    YamlPrototypes(Vec<YamlPrototype>),
    Fluent(Vec<FluentKey>),
}

pub struct ProjectParser {
    uri: Url,
    context: Arc<Context>,
    client: Arc<Client>,
}

impl ProjectParser {
    pub fn new(uri: Url, context: Arc<Context>, client: Arc<Client>) -> Self {
        Self {
            uri,
            context,
            client,
        }
    }

    pub async fn parse<'a>(&self, matchers: Vec<FileGroup>) {
        let matchers = Arc::new(matchers);

        let folders = get_folders(&self.uri);
        let collected_files = collect_files(folders, matchers.clone());

        let mut files_handlers = futures::future::join_all(
            collected_files
                .iter()
                .inspect(|(id, files)| tracing::info!("{} {id} files found", files.len()))
                .map(|(id, files)| async {
                    ParserHandler {
                        id: id.clone(),
                        actual_count: 0,
                        total_count: files.len() as u32,
                        status: get_status(self.client.clone(), &id.clone()).await,
                        finished: false,
                    }
                }),
        )
        .await;

        let (tx, mut rx) = tokio::sync::mpsc::channel(100);

        tokio::spawn({
            let matchers = matchers.clone();
            let context = self.context.clone();
            async move {
                while let Some((id, result)) = rx.recv().await {
                    files_handlers
                        .iter_mut()
                        .find(|h| h.id == id)
                        .unwrap()
                        .increment()
                        .await;

                    let Ok(result) = result else {
                        continue;
                    };

                    let matcher = matchers.par_iter().find_any(|m| m.id == id).unwrap();
                    let dispatcher = &matcher.dispatcher;
                    dispatcher(result, context.clone()).await;
                }
                futures::future::join_all(
                    files_handlers
                        .iter_mut()
                        .map(|h| async move { h.finish().await }),
                )
                .await;

                tracing::trace!("Parsing finished.");
            }
        });

        let handlers = futures::future::join_all(
            collected_files
                .into_iter()
                .map(|(id, files)| {
                    let id = id.clone();
                    let matchers = matchers.clone();
                    let tx = tx.clone();

                    files.into_iter().map(move |f| {
                        let tx = tx.clone();
                        let context = self.context.clone();
                        let client = self.client.clone();
                        let matchers = matchers.clone();
                        let id = id.clone();

                        tokio::spawn(async move {
                            let matcher = matchers.iter().find(|m| m.id == id).unwrap();
                            let parser = matcher.parser.clone();
                            let result = parser(f, context.parsed_files.clone(), client).await;

                            if let Err(err) = tx.send((matcher.id.clone(), result)).await {
                                tracing::error!("Failed to send result: {}", err);
                            }
                        })
                    })
                })
                .flatten(),
        );

        handlers.await;
    }
}

pub struct FileGroup {
    id: String,
    parser: Parser,
    dispatcher: ResultDispatcher,
    set: GlobMatcher,
}

impl FileGroup {
    pub fn new(
        id: impl ToString,
        set: impl AsRef<str>,
        parser: Parser,
        dispatcher: ResultDispatcher,
    ) -> Self {
        Self {
            id: id.to_string(),
            parser,
            dispatcher,
            set: Glob::new(set.as_ref()).unwrap().compile_matcher(),
        }
    }

    fn is_match(&self, path: &Path) -> bool {
        self.set.is_match(path)
    }
}

fn collect_files(
    folders: Vec<PathBuf>,
    matches: Arc<Vec<FileGroup>>,
) -> Vec<(String, Vec<PathBuf>)> {
    let mut files: Vec<(String, Vec<PathBuf>)> = vec![];

    TokioScope::scope_and_block(|s| {
        let (tx, rx) = std::sync::mpsc::channel();

        folders.into_iter().for_each(|folder| {
            let tx = tx.clone();
            let matches = matches.clone();

            s.spawn(async move {
                tracing::trace!("Start file search in {} folder", folder.display());

                for file in walkdir::WalkDir::new(&folder) {
                    match file {
                        Ok(file) => {
                            let path = file.path();
                            let matcher = matches.iter().find(|m| m.is_match(path));

                            if let Some(matcher) = matcher {
                                if let Err(err) = tx.send((matcher.id.clone(), path.to_path_buf()))
                                {
                                    tracing::error!("Failed to send file: {}", err);
                                }
                            }
                        }
                        Err(err) => {
                            tracing::warn!("Failed to read file: {}", err);
                        }
                    }
                }

                tracing::trace!("End file search in {} folder", folder.display());
            });
        });

        s.spawn(async {
            for (id, path) in rx {
                match files.iter_mut().find(|(gid, _)| *gid == *id) {
                    Some(f) => f.1.push(path.to_path_buf()),
                    None => files.push((id.to_owned(), vec![path.to_path_buf()])),
                }
            }
        });
    });

    files.par_sort_by_key(|(_, files)| files.len());
    files
}

struct ParserHandler {
    id: String,
    actual_count: u32,
    total_count: u32,
    status: Arc<Mutex<ProgressStatus>>,
    finished: bool,
}

impl ParserHandler {
    async fn increment(&mut self) {
        self.actual_count += 1;

        if self.finished {
            return;
        } else {
            if self.actual_count == self.total_count {
                self.finished = true;
                self.status.lock().await.finish(None).await;
            } else {
                let percent = percentage(self.actual_count, self.total_count);

                self.status
                    .lock()
                    .await
                    .next_state(
                        percent,
                        Some(format!(
                            "{}/{} ({percent}%)",
                            self.actual_count, self.total_count
                        )),
                    )
                    .await;
            }
        }
    }

    async fn finish(&mut self) {
        if self.finished {
            return;
        } else {
            self.finished = true;
            self.status.lock().await.finish(None).await;
        }
    }
}

#[instrument(skip(client))]
#[inline(always)]
async fn get_status(client: Arc<Client>, name: &str) -> Arc<Mutex<ProgressStatus>> {
    let status = ProgressStatus::new_with(
        client.clone(),
        ProgressStatusInit {
            id: format!("parse-{name}"),
            title: format!("Parsing {name}"),
            cancellable: true,
            ..Default::default()
        },
    )
    .await;

    Arc::new(Mutex::new(status))
}
