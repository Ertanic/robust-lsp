use std::{hash::Hash, path::PathBuf};

#[derive(Debug, Clone, Default)]
pub struct YamlPrototype {
    pub prototype: String,
    pub id: String,
    pub parents: Vec<String>,
    pub file: PathBuf,
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
