use super::{GotoDefinition, GotoDefinitionResult};
use crate::{
    backend::{CsharpClasses, YamlPrototypes},
    parse::{
        common::{DefinitionIndex, Index},
        structs::csharp::Prototype,
    },
};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use ropey::Rope;
use stringcase::camel_case;
use tokio::task::block_in_place;
use tower_lsp::lsp_types::{self, GotoDefinitionResponse, Location, Position, Url};
use tree_sitter::{Node, Parser, Point, Tree};

pub struct YamlGotoDefinition {
    classes: CsharpClasses,
    prototypes: YamlPrototypes,
    position: Position,
    src: String,
    tree: Tree,
}

impl GotoDefinition for YamlGotoDefinition {
    fn goto_definition(&self) -> GotoDefinitionResult {
        let point = Point::new(
            self.position.line as usize,
            self.position.character as usize,
        );

        let root_node = self.tree.root_node();
        let found_node = root_node.named_descendant_for_point_range(point, point)?;

        let nest = self.get_nesting(&found_node);
        match nest {
            2 => self.try_goto_prototype_definition(found_node),
            _ => None,
        }
    }
}

impl YamlGotoDefinition {
    pub fn new(
        classes: CsharpClasses,
        prototypes: YamlPrototypes,
        position: Position,
        rope: &Rope,
    ) -> Self {
        let src = rope.to_string();

        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_yaml::language()).unwrap();
        let tree = parser.parse(&src, None).unwrap();

        Self {
            classes,
            prototypes,
            position,
            src,
            tree,
        }
    }

    fn try_goto_prototype_definition(&self, node: Node) -> GotoDefinitionResult {
        let seeking = node.utf8_text(self.src.as_bytes()).ok()?;

        let mapping_pair_node = {
            let mut node = node;
            while let Some(n) = node.parent() {
                node = n;
                if n.kind() == "block_mapping_pair" {
                    break;
                }
            }
            node
        };

        let value_node = mapping_pair_node.child_by_field_name("value")?;
        let value = value_node.utf8_text(self.src.as_bytes()).ok()?;

        if seeking != value {
            return None;
        }

        let key_node = mapping_pair_node.child_by_field_name("key")?;
        let key_name = key_node.utf8_text(self.src.as_bytes()).ok()?;

        match key_name {
            "type" => {
                let prototype = block_in_place(|| self.classes.blocking_read())
                    .par_iter()
                    .filter_map(|c| Prototype::try_from(c).ok())
                    .inspect(|p| tracing::trace!("Found prototype: {:#?}", p.get_prototype_name()))
                    .find_any(|p| camel_case(&p.get_prototype_name()) == seeking)?;

                let index = prototype.index();
                self.index_to_definition(index)
            }
            "parent" => {
                let type_field_node = self.get_field(&mapping_pair_node.parent()?, "type")?;
                let type_field_value = type_field_node
                    .child_by_field_name("value")?
                    .utf8_text(self.src.as_bytes())
                    .ok()?;

                let lock = block_in_place(|| self.prototypes.blocking_read());
                let prototype = lock
                    .par_iter()
                    .filter(|p| p.prototype == type_field_value)
                    .find_any(|p| p.id == seeking)?;

                let index = prototype.index();
                self.index_to_definition(index)
            }
            _ => None,
        }
    }

    fn index_to_definition(&self, index: &DefinitionIndex) -> Option<GotoDefinitionResponse> {
        let url = Url::from_file_path(index.0.clone()).ok()?;
        let (start_position, end_position) = {
            let range = index.1?;
            (
                Position::new(
                    range.start_point.row as u32,
                    range.start_point.column as u32,
                ),
                Position::new(range.end_point.row as u32, range.end_point.column as u32),
            )
        };
        let range = lsp_types::Range::new(start_position, end_position);
        let definition = GotoDefinitionResponse::Scalar(Location {
            uri: url,
            range: range,
        });

        Some(definition)
    }

    fn get_nesting(&self, node: &Node) -> usize {
        let mut nest = 0;

        let mut parent = node.parent();
        while let Some(node) = parent {
            if node.kind() == "block_node" {
                nest += 1;
            }
            parent = node.parent();
        }

        nest
    }

    fn get_field<'a>(&self, node: &Node<'a>, name: &str) -> Option<Node<'a>> {
        debug_assert_eq!(node.kind(), "block_mapping");

        for i in 0..node.named_child_count() {
            let field_node = node.named_child(i)?;
            let key = field_node
                .child_by_field_name("key")?
                .utf8_text(self.src.as_bytes())
                .ok()?
                .to_owned();

            if key == name {
                return Some(field_node);
            }
        }
        None
    }
}