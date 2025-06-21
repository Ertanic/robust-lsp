use crate::{
    backend::Context,
    parse::common::Index,
    references::{GetReferencesResult, ReferencesProvider},
};
use ropey::Rope;
use std::sync::Arc;
use stringcase::camel_case;
use tokio::task::block_in_place;
use tower_lsp::{
    lsp_types,
    lsp_types::{Location, Position, Url},
};
use tree_sitter::{Node, Point, Tree};

pub struct CsharpReferencesProvider {
    context: Arc<Context>,
    position: Position,
    src: String,
    tree: Arc<Tree>,
}

impl ReferencesProvider for CsharpReferencesProvider {
    fn get_references(&self) -> GetReferencesResult {
        let point = Point::new(
            self.position.line as usize,
            self.position.character as usize,
        );

        let root_node = self.tree.root_node();
        let found_node = root_node.named_descendant_for_point_range(point, point)?;

        self.try_get_references_for_class_name(found_node)
    }
}

impl CsharpReferencesProvider {
    pub fn new(context: Arc<Context>, position: Position, rope: &Rope, tree: Arc<Tree>) -> Self {
        let src = rope.to_string();

        Self {
            context,
            position,
            src,
            tree,
        }
    }

    fn try_get_references_for_class_name(&self, node: Node) -> GetReferencesResult {
        let parent_node = node.parent();
        let Some(parent_node) = parent_node else {
            return None;
        };

        if node.kind() != "identifier" || parent_node.kind() != "class_declaration" {
            return None;
        }

        let value = node.utf8_text(self.src.as_bytes()).ok()?;
        let guard = block_in_place(|| self.context.prototypes.blocking_read());

        let result = guard
            .iter()
            .filter(|p| p.prototype == camel_case(value.trim_end_matches("Prototype")))
            .map(|p| {
                let index = p.index();
                let uri = Url::from_file_path(index.0.clone())
                    .expect("Can't get location from file path");
                let range = index.1.expect("Can't get location from index");
                Location::new(
                    uri,
                    lsp_types::Range {
                        start: lsp_types::Position {
                            line: range.start_point.row as u32,
                            character: range.start_point.column as u32,
                        },
                        end: Position {
                            line: range.end_point.row as u32,
                            character: range.end_point.column as u32,
                        },
                    },
                )
            })
            .collect::<Vec<_>>();

        Some(result)
    }
}
