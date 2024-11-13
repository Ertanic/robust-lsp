use ropey::Rope;
use std::{path::{Path, PathBuf}, sync::Arc};
use tree_sitter::{Node, Range};

#[derive(Debug, Clone, Default)]
pub struct DefinitionIndex(pub PathBuf, pub Option<Range>);

pub trait Index {
    fn index(&self) -> &DefinitionIndex;
}

pub type ParseResult<T> = Result<T, ()>;

pub(super) trait ParseFromNode {
    fn get(node: Node, src: Arc<Rope>, path: &Path) -> ParseResult<Self>
    where
        Self: Sized;
}
