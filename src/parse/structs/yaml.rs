use crate::parse::common::{DefinitionIndex, Index};
use std::hash::Hash;

#[derive(Debug, Clone, Default)]
pub struct YamlPrototype {
    pub prototype: String,
    pub id: String,
    pub parents: Vec<String>,

    index: DefinitionIndex,
}

impl YamlPrototype {
    pub fn new(prototype: String, id: String, parents: Vec<String>, index: DefinitionIndex) -> Self {
        Self { prototype, id, parents, index }
    }
}

impl Index for YamlPrototype {
    fn index(&self) -> &DefinitionIndex {
        &self.index
    }
}

impl PartialEq for YamlPrototype {
    fn eq(&self, other: &Self) -> bool {
        self.prototype == other.prototype && self.id == other.id
    }
}

impl Eq for YamlPrototype {}

impl Hash for YamlPrototype {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.prototype.hash(state);
        self.id.hash(state);
    }
}
