use super::InlayHint;
use crate::{backend::CsharpObjects, parse::structs::csharp::ReflectionManager, utils::block};
use ropey::Rope;
use std::sync::Arc;
use stringcase::camel_case;
use tower_lsp::lsp_types::{InlayHintKind, InlayHintLabel, Position, Range};
use tree_sitter::{Node, Tree};

type YamlInlayHintResult = Option<Vec<tower_lsp::lsp_types::InlayHint>>;

enum FieldType<'a> {
    Prototype(String, Node<'a>),
    Component(String, Node<'a>),
    // Tag(String, Node<'a>),
}

pub struct YamlInlayHint {
    classes: CsharpObjects,
    range: Range,
    src: String,
    tree: Arc<Tree>,
}

impl InlayHint for YamlInlayHint {
    fn inlay_hint(&self) -> YamlInlayHintResult {
        let root_node = self.tree.root_node();
        let document = find_child_node(root_node, "document")?;
        let block_node = find_child_node(document, "block_node")?;
        let block_sequence = find_child_node(block_node, "block_sequence")?;

        let mut hints = Vec::new();

        for i in 0..block_sequence.named_child_count() {
            let block_sequence_item_node = block_sequence.named_child(i).unwrap();
            if block_sequence_item_node.kind() != "block_sequence_item" {
                continue;
            }

            let block_node = find_child_node(block_sequence_item_node, "block_node")?;
            let block_mapping = find_child_node(block_node, "block_mapping")?;

            let fields = match self.collect_hints_from_prototype(block_mapping) {
                Some(fields) => fields,
                None => continue,
            };

            hints.extend(self.fields_map_to_hints(fields));
        }

        tracing::trace!("Found {} inlay hints.", hints.len());

        if hints.is_empty() {
            None
        } else {
            Some(hints)
        }
    }
}

impl YamlInlayHint {
    pub fn new(classes: CsharpObjects, range: Range, rope: &Rope, tree: Arc<Tree>) -> Self {
        let src = rope.to_string();

        Self {
            classes,
            range,
            src,
            tree,
        }
    }

    fn fields_map_to_hints(&self, fields: Vec<FieldType>) -> Vec<tower_lsp::lsp_types::InlayHint> {
        let reflection = ReflectionManager::new(self.classes.clone());
        fields
            .into_iter()
            .filter_map(|f| match f {
                FieldType::Prototype(name, node) => {
                    let proto = block(|| reflection.get_prototype_by_name(&name))?;
                    if camel_case(&proto.get_prototype_name()) != name {
                        return None;
                    }

                    let key_node = node.child_by_field_name("key")?;
                    let key_name = key_node.utf8_text(self.src.as_bytes()).ok()?;

                    let field = block(|| reflection.get_fields(Arc::clone(&**proto)))
                        .into_iter()
                        .find(|f| f.get_data_field_name() == key_name)?;

                    let hint = tower_lsp::lsp_types::InlayHint {
                        kind: Some(InlayHintKind::TYPE),
                        position: Position {
                            line: key_node.end_position().row as u32,
                            character: key_node.end_position().column as u32,
                        },
                        label: InlayHintLabel::String(field.type_name.clone()),
                        tooltip: None,
                        padding_left: Some(true),
                        padding_right: None,
                        text_edits: None,
                        data: None,
                    };

                    Some(hint)
                }
                FieldType::Component(name, node) => {
                    let reflection = ReflectionManager::new(self.classes.clone());
                    let comp = block(|| reflection.get_component_by_name(&name))?;
                    if comp.get_component_name() != name {
                        return None;
                    }

                    let key_node = node.child_by_field_name("key")?;
                    let key_name = key_node.utf8_text(self.src.as_bytes()).ok()?;

                    let field = block(|| reflection.get_fields(Arc::clone(&**comp)))
                        .into_iter()
                        .find(|f| f.get_data_field_name() == key_name)?;

                    let hint = tower_lsp::lsp_types::InlayHint {
                        kind: Some(InlayHintKind::TYPE),
                        position: Position {
                            line: key_node.end_position().row as u32,
                            character: key_node.end_position().column as u32,
                        },
                        label: InlayHintLabel::String(field.type_name.clone()),
                        tooltip: None,
                        padding_left: Some(true),
                        padding_right: None,
                        text_edits: None,
                        data: None,
                    };

                    Some(hint)
                }
            })
            .collect()
    }

    fn collect_hints_from_prototype<'a>(
        &self,
        block_mapping: Node<'a>,
    ) -> Option<Vec<FieldType<'a>>> {
        debug_assert_eq!(block_mapping.kind(), "block_mapping");

        let type_node = self
            .get_field(&block_mapping, "type")?
            .child_by_field_name("value")?;
        let proto_name = type_node.utf8_text(self.src.as_bytes()).ok()?;

        let is_entity = proto_name == "entity";

        let mut fields = Vec::new();

        for i in 0..block_mapping.named_child_count() {
            let block_mapping_pair = block_mapping.named_child(i).unwrap();
            if block_mapping_pair.kind() != "block_mapping_pair" {
                continue;
            }

            if !self.in_range(&block_mapping_pair) {
                continue;
            }

            let key_node = match block_mapping_pair.child_by_field_name("key") {
                Some(key_node) => key_node,
                None => continue,
            };
            let key_name = key_node.utf8_text(self.src.as_bytes()).ok()?;

            if is_entity && key_name == "components" {
                let block_node = match block_mapping_pair.child_by_field_name("value") {
                    Some(component_node) if component_node.kind() == "block_node" => component_node,
                    _ => continue,
                };

                let comp_fields = match self.collect_hints_from_components(block_node) {
                    Some(f) => f,
                    None => continue,
                };

                fields.extend(comp_fields);
            } else if !is_entity {
                fields.push(FieldType::Prototype(
                    proto_name.to_owned(),
                    block_mapping_pair,
                ));
            }
        }

        if fields.is_empty() {
            None
        } else {
            Some(fields)
        }
    }

    fn collect_hints_from_components<'a>(
        &self,
        block_node: Node<'a>,
    ) -> Option<Vec<FieldType<'a>>> {
        debug_assert_eq!(block_node.kind(), "block_node");

        let block_sequence = find_child_node(block_node, "block_sequence")?;

        let mut fields = Vec::new();

        // Components
        for i in 0..block_sequence.named_child_count() {
            let block_sequence_item_node = block_sequence.named_child(i).unwrap();
            if block_sequence_item_node.kind() != "block_sequence_item" {
                continue;
            }

            let block_node = match find_child_node(block_sequence_item_node, "block_node") {
                Some(n) => n,
                None => continue,
            };

            let block_mapping = match find_child_node(block_node, "block_mapping") {
                Some(n) => n,
                None => continue,
            };

            let type_pair_node = match self.get_field(&block_mapping, "type") {
                Some(n) => n,
                None => continue,
            };

            let type_node = match type_pair_node.child_by_field_name("value") {
                Some(n) => n,
                None => continue,
            };

            let comp_name = type_node.utf8_text(self.src.as_bytes()).ok()?;

            // Component fields
            for i in 0..block_mapping.named_child_count() {
                let block_mapping_pair = block_mapping.named_child(i).unwrap();
                if block_mapping_pair.kind() != "block_mapping_pair" {
                    continue;
                }

                let key_node = match block_mapping_pair.child_by_field_name("key") {
                    Some(key_node) => key_node,
                    None => continue,
                };
                let key_name = key_node.utf8_text(self.src.as_bytes()).ok()?;

                if key_name == "type" {
                    continue;
                } else if self.in_range(&block_mapping_pair) {
                    fields.push(FieldType::Component(
                        comp_name.to_owned(),
                        block_mapping_pair,
                    ));
                }
            }
        }

        if fields.is_empty() {
            None
        } else {
            Some(fields)
        }
    }

    fn in_range(&self, node: &Node) -> bool {
        let Range { start, end } = &self.range;
        let start_position = &node.start_position();
        let end_position = &node.end_position();

        (start_position.row >= start.line as usize) && (end_position.row <= end.line as usize)
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

fn find_child_node<'a>(node: Node<'a>, name: &'a str) -> Option<Node<'a>> {
    let mut n = None;
    for i in 0..node.named_child_count() {
        let found_node = node.named_child(i).unwrap();
        if found_node.kind() == name {
            n = Some(found_node);
            break;
        }
    }
    n
}
