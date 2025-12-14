use super::Result;
use bincode::{Decode, Encode};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tree_sitter::{Node, Point, Range};

#[derive(Debug, Clone, Default, PartialEq, Eq, Encode, Decode)]
pub struct IndexPoint {
    pub start: usize,
    pub end: usize,
}

impl From<tree_sitter::Point> for IndexPoint {
    fn from(Point { row, column }: Point) -> Self {
        Self { start: row, end: column }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Encode, Decode)]
pub struct IndexRange {
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_point: IndexPoint,
    pub end_point: IndexPoint,
}

impl From<tree_sitter::Range> for IndexRange {
    fn from(
        Range {
            start_byte,
            end_byte,
            start_point,
            end_point,
        }: tree_sitter::Range,
    ) -> Self {
        let start_point = start_point.into();
        let end_point = end_point.into();
        Self {
            start_byte,
            end_byte,
            start_point,
            end_point,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Encode, Decode)]
pub struct DefinitionIndex(pub PathBuf, pub Option<IndexRange>);

pub trait Index {
    fn index(&self) -> &DefinitionIndex;
}

pub(super) trait ParseFromNode {
    fn get(node: Node, src: Arc<String>, path: &Path) -> Result<Self>
    where
        Self: Sized;
}
