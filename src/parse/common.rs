use ropey::Rope;
use std::{path::PathBuf, sync::Arc};
use tree_sitter::{Node, Range};

#[derive(Debug, Clone, Default)]
pub(super) struct DefinitionIndex(PathBuf, Option<Range>);

pub(super) trait Index {
    fn index(&self) -> &DefinitionIndex;
}

pub type ParseResult<T> = Result<T, ()>;

pub(super) trait ParseFromNode {
    fn get(node: Node, src: Arc<Rope>) -> ParseResult<Self>
    where
        Self: Sized;
}
