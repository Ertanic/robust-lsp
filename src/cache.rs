use crate::parse::structs::{csharp::CsharpObject, fluent::FluentKey, yaml::YamlPrototype};
use bincode::{config::Configuration, Decode, Encode};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    hash::Hash,
    path::{Path, PathBuf},
    sync::Arc,
};
use tracing::{error, info, warn};

pub enum CacheContext {
    Csharp,
    Fluent,
    Yaml,
}

pub enum CacheContent<'a> {
    None,
    Csharp(&'a Vec<CsharpObject>),
    Fluent(&'a Vec<FluentKey>),
    Yaml(&'a Vec<YamlPrototype>),
}

#[derive(Clone, Debug, Hash, Eq, PartialEq, Encode, Decode)]
pub struct CacheKey(Arc<String>);

impl CacheKey {
    pub fn new(hash: impl ToString) -> Self {
        Self(Arc::new(hash.to_string()))
    }
}

#[derive(Default, Encode, Decode)]
struct Cache {
    csharp: HashMap<CacheKey, Vec<CsharpObject>>,
    fluent: HashMap<CacheKey, Vec<FluentKey>>,
    yaml: HashMap<CacheKey, Vec<YamlPrototype>>,
}

#[derive(Default)]
pub struct ProjectCache {
    cache: Cache,
    root: PathBuf,
    hash: String,
}

impl ProjectCache {
    pub async fn new(project_path: &Path, app_folder: &Path) -> Self {
        let root = app_folder.join(".cache");

        if !root.exists() {
            info!("cache directory does not exist, attempt to create a folder: {}", root.display());
            tokio::fs::create_dir(root.as_path())
                .await
                .expect("failed to create project cache directory");
        }

        let project_path = project_path.to_string_lossy().to_string();

        info!("calculate the hash of the project path: {project_path}");

        let hash = Sha256::digest(project_path);
        let hash = hex::encode(hash);

        let cache_path = root.join(&hash);
        let cache;

        info!("checking the existence of the project cache at the path: {}", cache_path.display());

        if cache_path.exists() {
            cache = decode_cache(cache_path).await;
        }
        else {
            cache = Cache::default();
            let empty_cache =
                bincode::encode_to_vec::<_, Configuration>(Cache::default(), Configuration::default()).expect("failed to encode empty cache");

            tokio::fs::write(cache_path, empty_cache).await.expect("failed to write cache");
        }

        Self { cache, hash, root }
    }

    pub fn get(&'_ self, key: &CacheKey, context: CacheContext) -> CacheContent<'_> {
        match context {
            CacheContext::Csharp => match self.cache.csharp.get(key) {
                Some(content) => CacheContent::Csharp(content),
                None => CacheContent::None,
            },
            CacheContext::Fluent => match self.cache.fluent.get(key) {
                Some(content) => CacheContent::Fluent(content),
                None => CacheContent::None,
            },
            CacheContext::Yaml => match self.cache.yaml.get(key) {
                Some(content) => CacheContent::Yaml(content),
                None => CacheContent::None,
            },
        }
    }

    pub fn insert(&mut self, key: CacheKey, content: CacheContent) {
        match content {
            CacheContent::Csharp(content) => {
                self.cache.csharp.insert(key, content.clone());
            }
            CacheContent::Fluent(content) => {
                self.cache.fluent.insert(key, content.clone());
            }
            CacheContent::Yaml(content) => {
                self.cache.yaml.insert(key, content.clone());
            }
            CacheContent::None => {
                warn!("attempt to insert a None value into the cache");
            }
        }
    }

    pub async fn write(&self) {
        info!("attempt to write cache to file...");
        let cache = bincode::encode_to_vec::<_, Configuration>(&self.cache, Configuration::default()).expect("failed to encode cache");
        let filepath = self.root.join(&self.hash);

        match tokio::fs::write(&filepath, cache).await {
            Ok(_) => {
                info!("cache written to {}", filepath.display());
            }
            Err(err) => {
                error!("cache failed to write to {}: {err:?}", filepath.display());
            }
        }
    }
}

async fn decode_cache(filepath: impl AsRef<Path>) -> Cache {
    let result = tokio::fs::read_to_string(filepath).await;
    match result {
        Ok(content) => {
            info!("attempt to decode cache...");
            let cache = bincode::decode_from_slice::<Cache, Configuration>(content.as_bytes(), Configuration::default());
            match cache {
                Ok((cache, _)) => {
                    info!("cache has been decoded");
                    cache
                }
                Err(err) => {
                    error!("error during cache decoding: {err:?}");
                    Cache::default()
                }
            }
        }
        Err(err) => {
            error!("error during cache reading: {err:?}");
            Cache::default()
        }
    }
}
