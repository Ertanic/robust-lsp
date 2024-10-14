use std::path::PathBuf;

use tree_sitter::Range;


#[derive(Debug,Clone,Default)]
pub(super) struct DefinitionIndex(PathBuf, Option<Range>);

pub(super) trait Index {
    fn index(&self) -> &DefinitionIndex;
}