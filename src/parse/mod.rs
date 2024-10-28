use crate::backend::{CsharpClasses, ParsedFiles};
use ropey::Rope;
use std::path::PathBuf;
use tower_lsp::lsp_types::Url;
use tracing::instrument;
use tree_sitter::Tree;

mod common;
pub mod csharp;

pub(crate) struct ParsedFile {
    pub tree: Option<Tree>,
    pub path: PathBuf,
    pub rope: Rope,
}

#[derive(Debug)]
enum Message {
    Important(PathBuf),
    Lazy(PathBuf),
}

#[instrument(skip_all)]
pub fn parse_project(uri: Url, classes: CsharpClasses, parsed_files: ParsedFiles) -> bool {
    tracing::trace!("Started parsing project");

    let mut prototypes = vec![];
    let mut other = vec![];

    std::thread::scope(|s| {
        let (tx, rx) = std::sync::mpsc::channel();

        for folder in get_folders(&uri) {
            let tx = tx.clone();
            s.spawn(move || {
                tracing::trace!("Start file search in {} folder", folder.display());

                for file in walkdir::WalkDir::new(&folder) {
                    if let Ok(file) = file {
                        if file.path().display().to_string().ends_with("Prototype.cs") {
                            tx.send(Message::Important(file.path().to_path_buf()))
                                .unwrap();
                        } else if file.path().extension().unwrap_or(Default::default()) == "cs" {
                            tx.send(Message::Lazy(file.path().to_path_buf())).unwrap();
                        }
                    }
                }

                tracing::trace!("End file search in {} folder", folder.display());
            });
        }

        s.spawn(|| {
            for message in rx {
                // tracing::trace!("Got message: {:?}", message);
                match message {
                    Message::Important(path) => prototypes.push(path),
                    Message::Lazy(path) => other.push(path),
                }
            }
        });
    });

    tracing::trace!("{} prototypes files found", prototypes.len());
    tracing::trace!("{} other files found", other.len());

    std::thread::scope(|s| {
        for p in prototypes {
            let parsed_files = parsed_files.clone();
            let classes = classes.clone();

            s.spawn(move || {
                let class = csharp::parse(p.clone(), parsed_files.clone());

                if let Ok(class) = class {
                    tracing::trace!("{} has been parsed, found classes: {{ count: {}, {:#?} }}", p.display(), class.len(), class);

                    let mut lock = classes.write().unwrap();
                    for c in class {
                        lock.insert(c);
                    }
                    drop(lock);
                }
            });
        }
    });

    true
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
    ]
    .into_iter()
    .map(|f| uri.to_file_path().unwrap().join(f))
    .filter(|f| f.exists())
    .collect()
}
