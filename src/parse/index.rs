use super::Result;
use ropey::Rope;
use tower_lsp::lsp_types;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tree_sitter::Node;

// Row/column
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IndexPosition(pub usize, pub usize);

impl Into<lsp_types::Position> for IndexPosition {
    fn into(self) -> lsp_types::Position {
        lsp_types::Position {
            line: self.0 as u32,
            character: self.1 as u32,
        }
    }
}

impl Into<tree_sitter::Point> for IndexPosition {
    fn into(self) -> tree_sitter::Point {
        tree_sitter::Point {
            row: self.0,
            column: self.1,
        }
    }
}

impl From<tree_sitter::Point> for IndexPosition {
    fn from(value: tree_sitter::Point) -> Self {
        Self(value.row, value.column)
    }
}

impl From<lsp_types::Position> for IndexPosition {
    fn from(value: lsp_types::Position) -> Self {
        Self(value.line as usize, value.character as usize)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IndexRange(pub IndexPosition, pub IndexPosition, pub Option<(usize, usize)>);

impl Into<lsp_types::Range> for IndexRange {
    fn into(self) -> lsp_types::Range {
        lsp_types::Range {
            start: self.0.into(),
            end: self.1.into(),
        }
    }
}

impl Into<tree_sitter::Range> for IndexRange {
    fn into(self) -> tree_sitter::Range {
        let (start_byte, end_byte) = self.2.unwrap_or_default();
        tree_sitter::Range {
            start_point: self.0.into(),
            end_point: self.1.into(),
            start_byte,
            end_byte,
        }
    }
}

impl From<tree_sitter::Range> for IndexRange {
    fn from(value: tree_sitter::Range) -> Self {
        Self(
            value.start_point.into(),
            value.end_point.into(),
            Some((value.start_byte, value.end_byte)),
        )
    }
}

impl From<lsp_types::Range> for IndexRange {
    fn from(value: lsp_types::Range) -> Self {
        Self(value.start.into(), value.end.into(), None)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DefinitionIndex(pub PathBuf, pub IndexRange);

pub trait Index {
    fn index(&self) -> &DefinitionIndex;
}

pub(super) trait ParseFromNode {
    fn get(node: Node, src: Arc<Rope>, path: &Path) -> Result<Self>
    where
        Self: Sized;
}
