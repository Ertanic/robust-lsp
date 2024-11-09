use crate::{
    backend::{CsharpClasses, ParsedFiles, YamlPrototypes},
    utils::{percentage, ProgressStatus, ProgressStatusInit},
};
use async_scoped::TokioScope;
use globset::Glob;
use std::{num::NonZero, path::PathBuf, sync::Arc};
use structs::{csharp::CsharpClass, yaml::YamlPrototype};
use tokio::sync::{Mutex, Semaphore};
use tower_lsp::{lsp_types::Url, Client};
use tracing::instrument;

pub mod common;
pub mod csharp;
pub mod structs;
pub mod yaml;

#[derive(Debug)]
enum FileType {
    Prototype(PathBuf),
    Component(PathBuf),
    Lazy(PathBuf),
    YamlPrototype(PathBuf),
}

enum ParseResult {
    Prototypes(Result<Vec<CsharpClass>, ()>),
    Components(Result<Vec<CsharpClass>, ()>),
    YamlPrototypes(Result<Vec<YamlPrototype>, ()>),
}

#[instrument(skip_all)]
pub async fn parse_project(
    uri: Url,
    classes: CsharpClasses,
    prototypes: YamlPrototypes,
    parsed_files: ParsedFiles,
    client: Arc<tower_lsp::Client>,
) -> bool {
    tracing::trace!("Started parsing project");

    let folders = get_folders(&uri);
    let files = collect_files(folders).await;

    let prototypes_len = files.prototypes.len();
    let components_len = files.components.len();
    let other_len = files.other.len();
    let yaml_protos_len = files.yaml_prototypes.len();

    tracing::info!("{components_len} components files found");
    tracing::info!("{prototypes_len} C# prototypes files found");
    tracing::info!("{other_len} other C# files found");
    tracing::info!("{yaml_protos_len} prototypes files found");

    let (tx, rx) = std::sync::mpsc::channel();

    let proto_status = get_status(client.clone(), "C# prototypes").await;
    let comps_status = get_status(client.clone(), "components").await;
    let yaml_protos_status = get_status(client.clone(), "YAML prototypes").await;

    let reader = tokio::spawn({
        let classes = classes.clone();

        async move {
            let prototypes_len = prototypes_len as u32;
            let components_len = components_len as u32;
            let yaml_protos_len = yaml_protos_len as u32;
            let mut actual_prototypes = 0;
            let mut actual_components = 0;
            let mut actual_yaml_protos = 0;

            for message in rx {
                match message {
                    ParseResult::Prototypes(csharp_class) => {
                        if let Ok(csharp_class) = csharp_class {
                            classes.write().await.extend(csharp_class);
                        }

                        actual_prototypes += 1;
                        let percent = percentage(actual_prototypes, prototypes_len);
                        proto_status
                            .lock()
                            .await
                            .next_state(
                                percent as u32,
                                Some(format!("{actual_prototypes}/{prototypes_len} ({percent}%)")),
                            )
                            .await;
                    }
                    ParseResult::Components(csharp_class) => {
                        if let Ok(csharp_class) = csharp_class {
                            classes.write().await.extend(csharp_class);
                        }

                        actual_components += 1;
                        let percent = percentage(actual_components, components_len);
                        comps_status
                            .lock()
                            .await
                            .next_state(
                                percent as u32,
                                Some(format!("{actual_components}/{components_len} ({percent}%)")),
                            )
                            .await
                    }
                    ParseResult::YamlPrototypes(yaml_protos) => {
                        if let Ok(yaml_protos) = yaml_protos {
                            prototypes.write().await.extend(yaml_protos);
                        }

                        actual_yaml_protos += 1;
                        let percent = percentage(actual_yaml_protos, yaml_protos_len);
                        proto_status
                            .lock()
                            .await
                            .next_state(
                                percent as u32,
                                Some(format!(
                                    "{actual_yaml_protos}/{yaml_protos_len} ({percent}%)"
                                )),
                            )
                            .await
                    }
                }
            }

            proto_status.lock().await.finish(None).await;
            comps_status.lock().await.finish(None).await;
            yaml_protos_status.lock().await.finish(None).await;
        }
    });

    TokioScope::scope_and_block(|s| {
        // Run C# prototypes parsing
        s.spawn(async {
            TokioScope::scope_and_block(|s| {
                for p in files.prototypes {
                    let parsed_files = parsed_files.clone();
                    let tx = tx.clone();

                    s.spawn(async move {
                        let parsed_classes = csharp::parse(p.clone(), parsed_files.clone()).await;
                        tx.send(ParseResult::Prototypes(parsed_classes)).unwrap();
                    });
                }

                tracing::trace!("All C# prototypes has been sent for parsing");
            });
        });

        // Run components parsing
        s.spawn(async {
            TokioScope::scope_and_block(|s| {
                for c in files.components {
                    let parsed_files = parsed_files.clone();
                    let tx = tx.clone();

                    s.spawn(async move {
                        let parsed_classes = csharp::parse(c.clone(), parsed_files.clone()).await;
                        tx.send(ParseResult::Components(parsed_classes)).unwrap();
                    });
                }

                tracing::trace!("All components has been sent for parsing");
            });
        });

        s.spawn(async {
            TokioScope::scope_and_block(|s| {
                for p in files.yaml_prototypes {
                    let parsed_files = parsed_files.clone();
                    let tx = tx.clone();

                    s.spawn(async move {
                        let parsed_protos = yaml::parse(p.clone(), parsed_files).await;
                        tx.send(ParseResult::YamlPrototypes(parsed_protos)).unwrap();
                    });
                }
            });
        });

        // Run other files parsing
        tokio::spawn({
            let client = client.clone();
            let classes = classes.clone();
            let parsed_files = parsed_files.clone();

            async move {
                // Since the rest of the files are not of great urgency, we'll lazily parse them in the background.
                // And in order not to load the user's system, only half of the threads will be used.
                let threads = std::thread::available_parallelism()
                    .unwrap_or(NonZero::new(2).unwrap())
                    .get()
                    / 2;

                tracing::trace!(
                    "Using {} threads for parsing other files in the background",
                    threads
                );

                let semaphore = Arc::new(Semaphore::new(threads));
                let (tx, rx) = std::sync::mpsc::channel();

                let other_status = get_status(client.clone(), "C# files").await;

                tokio::spawn({
                    let other_status = other_status.clone();

                    async move {
                        let mut i = 0;
                        let other_len = other_len as u32;

                        for message in rx {
                            if let Ok(parsed_files) = message {
                                classes.write().await.extend(parsed_files);
                            }

                            i += 1;
                            let percent = percentage(i, other_len);
                            other_status
                                .lock()
                                .await
                                .next_state(percent, Some(format!("{i}/{other_len} ({percent}%)")))
                                .await;
                        }

                        other_status
                            .lock()
                            .await
                            .finish(Some("C# files parsed"))
                            .await;
                    }
                });

                tracing::trace!("All other files has been sent for parsing");

                let mut handles = Vec::with_capacity(files.other.len());

                for o in files.other {
                    let parsed_files = parsed_files.clone();
                    let semaphore = semaphore.clone();
                    let tx = tx.clone();

                    handles.push(tokio::spawn(async move {
                        let _permit = semaphore.acquire().await.unwrap();

                        let parsed_classes = csharp::parse(o.clone(), parsed_files.clone()).await;
                        tx.send(parsed_classes).unwrap();
                    }));
                }

                for handle in handles {
                    handle.await.unwrap();
                }
            }
        });
    });
    tracing::trace!("All C# files has been sent for parsing");

    let res = reader.is_finished();

    tracing::trace!("Finished parsing project");

    res
}

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
    ]
    .into_iter()
    .map(|f| uri.to_file_path().unwrap().join(f))
    .filter(|f| f.exists())
    .collect()
}

struct CollectedFiles {
    prototypes: Vec<PathBuf>,
    components: Vec<PathBuf>,
    other: Vec<PathBuf>,
    yaml_prototypes: Vec<PathBuf>,
}

async fn collect_files(folders: Vec<PathBuf>) -> CollectedFiles {
    let mut yaml_prototypes = vec![];
    let mut prototypes = vec![];
    let mut components = vec![];
    let mut other = vec![];

    TokioScope::scope_and_block(|s| {
        let (tx, rx) = std::sync::mpsc::channel();

        for folder in folders {
            let tx = tx.clone();

            s.spawn(async move {
                tracing::trace!("Start file search in {} folder", folder.display());
                let proto_set = Glob::new("*Prototype.cs").unwrap().compile_matcher();
                let comp_set = Glob::new("*Component.cs").unwrap().compile_matcher();
                let csharp_set = Glob::new("*.cs").unwrap().compile_matcher();
                let yaml_set = Glob::new("**/Prototypes/**/*.{yml,yaml}")
                    .unwrap()
                    .compile_matcher();

                for file in walkdir::WalkDir::new(&folder) {
                    if let Ok(file) = file {
                        let path = file.path();
                        if proto_set.is_match(path) {
                            tx.send(FileType::Prototype(path.to_owned())).unwrap();
                        } else if comp_set.is_match(path) {
                            tx.send(FileType::Component(path.to_owned())).unwrap();
                        } else if csharp_set.is_match(path) {
                            tx.send(FileType::Lazy(path.to_owned())).unwrap();
                        } else if yaml_set.is_match(path) {
                            tx.send(FileType::YamlPrototype(path.to_owned())).unwrap();
                        }
                    }
                }

                tracing::trace!("End file search in {} folder", folder.display());
            });
        }

        s.spawn(async {
            for message in rx {
                match message {
                    FileType::Prototype(path) => prototypes.push(path),
                    FileType::Component(path) => components.push(path),
                    FileType::Lazy(path) => other.push(path),
                    FileType::YamlPrototype(path) => yaml_prototypes.push(path),
                }
            }
        });
    });

    CollectedFiles {
        prototypes,
        components,
        other,
        yaml_prototypes,
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
            ..Default::default()
        },
    )
    .await;

    Arc::new(Mutex::new(status))
}
