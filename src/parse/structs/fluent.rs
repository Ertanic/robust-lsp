#![allow(dead_code)]

use crate::parse::common::{DefinitionIndex, Index};

#[derive(Debug, Clone, Default)]
pub struct FluentKey {
    pub key: String,
    pub args: std::collections::HashSet<String>,

    index: DefinitionIndex,
}

impl PartialEq for FluentKey {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl Eq for FluentKey {}

impl Index for FluentKey {
    fn index(&self) -> &DefinitionIndex {
        &self.index
    }
}

impl FluentKey {
    pub fn new(
        key: String,
        args: std::collections::HashSet<String>,
        index: DefinitionIndex,
    ) -> Self {
        Self { key, args, index }
    }

    pub fn dummy(key: impl ToString) -> Self {
        Self {
            key: key.to_string(),
            args: Default::default(),
            index: Default::default(),
        }
    }
}

impl Ord for FluentKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key.cmp(&other.key)
    }
}

impl PartialOrd for FluentKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.key.partial_cmp(&other.key)
    }
}

impl std::hash::Hash for FluentKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.key.hash(state);
    }
}
