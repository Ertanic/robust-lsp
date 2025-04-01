use crate::{
    backend::{Context, ParsedFiles},
    utils::{percentage, ProgressStatus, ProgressStatusInit},
};
use futures::future::{join_all};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use std::num::NonZero;
use std::thread::available_parallelism;
use itertools::Itertools;
use structs::{csharp::CsharpObject, fluent::FluentKey, yaml::YamlPrototype};
use tokio::sync::Mutex;
use tower_lsp::Client;
use tracing::instrument;
use walkdir::{DirEntry, WalkDir};

pub mod common;
pub mod csharp;
pub mod fluent;
pub mod structs;
pub mod yaml;

pub(crate) type Result<T, E = ()> = std::result::Result<T, E>;

#[inline(always)]
fn get_folders(root: &Path) -> Vec<PathBuf> {
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
        .map(|f| root.join(f))
        .filter(|f| f.exists())
        .collect()
}

pub enum ParseResult {
    Csharp(Vec<CsharpObject>),
    YamlPrototypes(Vec<YamlPrototype>),
    Fluent(Vec<FluentKey>),
    None,
}

pub struct ProjectParser<'a> {
    root: &'a Path,
    context: Arc<Context>,
    client: Arc<Client>,
}

impl<'a> ProjectParser<'a> {
    pub fn new(root: &'a Path, context: Arc<Context>, client: Arc<Client>) -> Self {
        Self { root, context, client }
    }

    pub async fn parse(&self) {
        let files = collect_files(get_folders(&self.root)).await;
        let files_count = files.len();
        tracing::info!("{} files found", files_count);

        let cpu = available_parallelism().unwrap_or(NonZero::new(4).unwrap()).get() / 2;
        let parser_status = Arc::new(Mutex::new(ParserHandler::new(files_count as u32, self.client.clone(), "project files").await));

        let results = join_all(files.into_iter()
            .chunks(files_count / cpu)
            .into_iter()
            .map(|chunk| {
                let chunk = chunk.collect::<Vec<_>>();
                let parsed_files = self.context.parsed_files.clone();
                let parser_status = parser_status.clone();

                tokio::spawn(async move {
                    let mut results = Vec::with_capacity(chunk.len());

                    for file in chunk {
                        let Some(ext) = file.extension() else { continue };

                        let result = match ext.to_str().expect("unrecognized extension") {
                            "cs" => csharp::parse(file, parsed_files.clone()).await,
                            "yml" => yaml::parse(file, parsed_files.clone()).await,
                            "ftl" => fluent::parse(file, parsed_files.clone()).await,
                            _ => continue,
                        };

                        parser_status.lock().await.increment().await;

                        results.push(result);
                    }

                    results
                })
            }))
            .await
            .into_iter()
            .filter_map(Result::ok)
            .flatten()
            .collect::<Vec<_>>();

        parser_status.lock().await.finish().await;

        for result in results.into_iter().filter(|r| !matches!(r, ParseResult::None)) {
            let mut classes = self.context.classes.write().await;
            let mut prototypes = self.context.prototypes.write().await;
            let mut locales = self.context.locales.write().await;

            match result {
                ParseResult::Csharp(objs) => {
                    objs.into_iter().for_each(|obj| { classes.insert(obj); });
                }
                ParseResult::YamlPrototypes(protos) => {
                    protos.into_iter().for_each(|proto| { prototypes.insert(proto); });
                }
                ParseResult::Fluent(ftls) => {
                    ftls.into_iter().for_each(|key| { locales.insert(key); });
                }
                ParseResult::None => {}
            }
        }
    }
}

async fn collect_files(
    folders: Vec<PathBuf>,
) -> Vec<PathBuf> {
    let tasks = folders
        .into_iter()
        .map(|folder| tokio::spawn(async {
            WalkDir::new(folder)
                .into_iter()
                .filter_map(Result::ok)
                .map(DirEntry::into_path)
                .collect::<Vec<PathBuf>>()
        }))
        .collect::<Vec<_>>();

    let files = join_all(tasks)
        .await
        .into_iter()
        .filter_map(Result::ok)
        .flatten()
        .collect::<Vec<_>>();

    files
}

struct ParserHandler {
    actual_count: u32,
    total_count: u32,
    status: Arc<Mutex<ProgressStatus>>,
    finished: bool,
}

impl ParserHandler {
    pub async fn new(total_count: u32, client: Arc<Client>, name: &str) -> Self {
        Self { actual_count: 0, total_count, status: get_status(client, name).await, finished: false }
    }

    pub async fn increment(&mut self) {
        self.actual_count += 1;

        if self.finished {
            return;
        } else if self.actual_count == self.total_count {
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

    pub async fn finish(&mut self) {
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
