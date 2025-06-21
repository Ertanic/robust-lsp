use super::{GotoDefinition, GotoDefinitionResult};
use crate::{
    backend::Context,
    parse::{
        common::{DefinitionIndex, Index},
        structs::{
            csharp::{Component, Prototype, ReflectionManager},
            fluent::FluentKey,
        },
    },
    utils::block,
};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use ropey::Rope;
use std::{path::PathBuf, sync::Arc};
use stringcase::camel_case;
use tokio::task::block_in_place;
use tower_lsp::lsp_types::{self, GotoDefinitionResponse, Location, LocationLink, Position, Url};
use tree_sitter::{Node, Point, Tree};

pub struct YamlGotoDefinition {
    context: Arc<Context>,
    position: Position,
    src: String,
    tree: Arc<Tree>,
    project_root: PathBuf,
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
            2 => self
                .try_goto_locid_definition(found_node, nest)
                .or_else(|| self.try_goto_prototype_definition(found_node)),
            4 => self
                .try_goto_locid_definition(found_node, nest)
                .or_else(|| self.try_goto_sprite_definition(found_node, nest))
                .or_else(|| self.try_goto_protoid_definition(found_node, nest))
                .or_else(|| self.try_goto_component_definition(found_node)),
            _ => None,
        }
    }
}

impl YamlGotoDefinition {
    pub fn new(
        context: Arc<Context>,
        position: Position,
        rope: &Rope,
        tree: Arc<Tree>,
        project_root: PathBuf,
    ) -> Self {
        let src = rope.to_string();

        Self {
            context,
            position,
            src,
            tree,
            project_root,
        }
    }

    fn try_goto_protoid_definition(
        &self,
        found_node: Node<'_>,
        nest: usize,
    ) -> GotoDefinitionResult {
        debug_assert!(nest >= 4);

        let block_mapping_pair = {
            let mut node = found_node;
            while let Some(n) = node.parent() {
                node = n;
                if n.kind() == "block_mapping_pair" {
                    break;
                }
            }
            node
        };

        if block_mapping_pair.kind() != "block_mapping_pair" {
            tracing::trace!("not a block mapping pair");
            return None;
        }

        let key_node = block_mapping_pair.child_by_field_name("key")?;
        let key_name = key_node.utf8_text(self.src.as_bytes()).ok()?;

        if key_name == found_node.utf8_text(self.src.as_bytes()).ok()? {
            return None;
        }

        let value_node = block_mapping_pair.child_by_field_name("value")?;

        let comp_name = self
            .get_field(&block_mapping_pair.parent()?, "type")?
            .child_by_field_name("value")?
            .utf8_text(self.src.as_bytes())
            .ok()?;

        let reflection = ReflectionManager::new(self.context.classes.clone());
        let comp = block(|| reflection.get_component_by_name(comp_name))?;

        let field = block(|| reflection.get_fields(Arc::clone(&comp)))
            .into_iter()
            .find(|f| f.get_data_field_name() == key_name)?;

        if !field.type_name.starts_with("ProtoId<") {
            tracing::trace!("not a ProtoId field");
            return None;
        }

        let type_name = &field.type_name[8..field.type_name.len() - 1];
        let id = value_node.utf8_text(self.src.as_bytes()).ok()?;

        let binding = block_in_place(|| self.context.prototypes.blocking_read());

        let proto = binding.par_iter().find_any(|p| {
            tracing::trace!(
                "{} == {} && {}",
                p.prototype,
                camel_case(
                    type_name
                        .trim_end_matches("Prototype")
                        .trim_end_matches(">")
                ),
                id
            );
            p.prototype
                == camel_case(
                    type_name
                        .trim_end_matches(">")
                        .trim_end_matches("Prototype"),
                )
                && p.id == id
        })?;

        let location = get_location_link(proto.index(), value_node)?;
        Some(GotoDefinitionResponse::Link(vec![location]))
    }

    fn try_goto_sprite_definition(
        &self,
        found_node: Node<'_>,
        nest: usize,
    ) -> GotoDefinitionResult {
        debug_assert!(nest >= 4);

        let block_mapping_pair = {
            let mut node = found_node;
            while let Some(n) = node.parent() {
                node = n;
                if n.kind() == "block_mapping_pair" {
                    break;
                }
            }
            node
        };

        if block_mapping_pair.kind() != "block_mapping_pair" {
            tracing::trace!("not a block mapping pair");
            return None;
        }

        let key_node = block_mapping_pair.child_by_field_name("key")?;
        let key_name = key_node.utf8_text(self.src.as_bytes()).ok()?;

        if key_name == found_node.utf8_text(self.src.as_bytes()).ok()? {
            return None;
        }

        let value_node = block_mapping_pair.child_by_field_name("value")?;
        let value = value_node.utf8_text(self.src.as_bytes()).ok()?;

        let comp_name = self
            .get_field(&block_mapping_pair.parent()?, "type")?
            .child_by_field_name("value")?
            .utf8_text(self.src.as_bytes())
            .ok()?;

        let reflection = ReflectionManager::new(self.context.classes.clone());
        let comp = block(|| reflection.get_component_by_name(comp_name))?;

        let field = block(|| reflection.get_fields(Arc::clone(&comp)))
            .into_iter()
            .find(|f| f.get_data_field_name() == key_name)?;

        if field.get_data_field_name() != "sprite" {
            tracing::trace!("not a sprite field");
            return None;
        }

        let rsi_path = self
            .project_root
            .join(format!("Resources/Textures/{}/meta.json", value));

        let location = LocationLink {
            origin_selection_range: Some(lsp_types::Range {
                start: Position {
                    line: value_node.start_position().row as u32,
                    character: value_node.start_position().column as u32,
                },
                end: Position {
                    line: value_node.end_position().row as u32,
                    character: value_node.end_position().column as u32,
                },
            }),
            target_uri: Url::from_directory_path(rsi_path).ok()?,
            target_range: Default::default(),
            target_selection_range: Default::default(),
        };

        Some(GotoDefinitionResponse::Link(vec![location]))
    }

    #[tracing::instrument(skip(self), ret)]
    fn try_goto_locid_definition(&self, found_node: Node<'_>, nest: usize) -> GotoDefinitionResult {
        let block_mapping_pair = {
            let mut node = found_node;
            while let Some(n) = node.parent() {
                node = n;
                if n.kind() == "block_mapping_pair" {
                    break;
                }
            }
            node
        };

        if block_mapping_pair.kind() != "block_mapping_pair" {
            tracing::trace!("not a block mapping pair");
            return None;
        }

        let key_node = block_mapping_pair.child_by_field_name("key")?;
        let key_name = key_node.utf8_text(self.src.as_bytes()).ok()?;

        if key_name == found_node.utf8_text(self.src.as_bytes()).ok()? {
            return None;
        }

        let value_node = block_mapping_pair.child_by_field_name("value")?;
        let value = value_node.utf8_text(self.src.as_bytes()).ok()?;

        match nest {
            2 => {
                let proto_name = self
                    .get_field(&block_mapping_pair.parent()?, "type")?
                    .child_by_field_name("value")?
                    .utf8_text(self.src.as_bytes())
                    .ok()?;

                let reflection = ReflectionManager::new(self.context.classes.clone());
                let proto = block(|| reflection.get_prototype_by_name(proto_name))?;

                let field = block(|| reflection.get_fields(Arc::clone(&proto)))
                    .into_iter()
                    .find(|f| f.get_data_field_name() == key_name)?;

                match field.type_name.as_str() {
                    "LocId" => {
                        let lock = block_in_place(|| self.context.locales.blocking_read());
                        let locale = lock.get(&FluentKey::dummy(value))?;

                        let location = get_location_link(locale.index(), value_node)?;

                        Some(GotoDefinitionResponse::Link(vec![location]))
                    }
                    _ => {
                        tracing::trace!("unknown field");
                        None
                    }
                }
            }
            4 => {
                let comp_name = self
                    .get_field(&block_mapping_pair.parent()?, "type")?
                    .child_by_field_name("value")?
                    .utf8_text(self.src.as_bytes())
                    .ok()?;

                let reflection = ReflectionManager::new(self.context.classes.clone());
                let comp = block(|| reflection.get_component_by_name(comp_name))?;

                let field = block(|| reflection.get_fields(Arc::clone(&comp)))
                    .into_iter()
                    .find(|f| f.get_data_field_name() == key_name)?;

                if field.type_name != "LocId" {
                    tracing::trace!("not a locid field");
                    return None;
                }

                let lock = block_in_place(|| self.context.locales.blocking_read());
                let locale = lock.get(&FluentKey::dummy(value))?;

                let location = get_location_link(locale.index(), value_node)?;

                Some(GotoDefinitionResponse::Link(vec![location]))
            }
            _ => None,
        }
    }

    fn try_goto_component_definition(&self, found_node: Node<'_>) -> GotoDefinitionResult {
        let seeking = found_node.utf8_text(self.src.as_bytes()).ok()?;

        let mapping_pair_node = {
            let mut node = found_node;
            while let Some(n) = node.parent() {
                node = n;
                if n.kind() == "block_mapping_pair" {
                    break;
                }
            }
            node
        };

        let key_node = mapping_pair_node.child_by_field_name("key")?;
        let key_name = key_node.utf8_text(self.src.as_bytes()).ok()?;

        match key_name {
            "type" => {
                let value = mapping_pair_node
                    .child_by_field_name("value")?
                    .utf8_text(self.src.as_bytes())
                    .ok()?;

                if seeking != value {
                    return None;
                }

                let comp = block_in_place(|| self.context.classes.blocking_read())
                    .par_iter()
                    .filter_map(|c| Component::try_from(Arc::clone(c)).ok())
                    .find_any(|p| camel_case(&p.get_component_name()) == camel_case(seeking))?;

                let index = comp.index();
                self.index_to_definition(index)
            }
            _ => {
                if seeking != key_name {
                    return None;
                }

                let comp_name = self
                    .get_field(&mapping_pair_node.parent()?, "type")?
                    .child_by_field_name("value")?
                    .utf8_text(self.src.as_bytes())
                    .ok()?;

                let comp = block_in_place(|| self.context.classes.blocking_read())
                    .par_iter()
                    .filter_map(|c| Component::try_from(Arc::clone(c)).ok())
                    .find_any(|p| camel_case(&p.get_component_name()) == camel_case(comp_name))?;

                let reflection = ReflectionManager::new(self.context.classes.clone());
                let field = block(|| reflection.get_fields(Arc::clone(&comp)))
                    .into_iter()
                    .find(|f| f.get_data_field_name() == key_name)?;

                let index = field.index();
                self.index_to_definition(index)
            }
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

        let key_node = mapping_pair_node.child_by_field_name("key")?;
        let key_name = key_node.utf8_text(self.src.as_bytes()).ok()?;

        match key_name {
            "type" => {
                let value = mapping_pair_node
                    .child_by_field_name("value")?
                    .utf8_text(self.src.as_bytes())
                    .ok()?;

                if seeking != value {
                    return None;
                }

                let prototype = block_in_place(|| self.context.classes.blocking_read())
                    .par_iter()
                    .filter_map(|c| Prototype::try_from(Arc::clone(c)).ok())
                    .find_any(|p| camel_case(&p.get_prototype_name()) == seeking)?;

                let index = prototype.index();
                self.index_to_definition(index)
            }
            "parent" => {
                let value = mapping_pair_node
                    .child_by_field_name("value")?
                    .utf8_text(self.src.as_bytes())
                    .ok()?;

                if seeking != value {
                    return None;
                }

                let type_field_node = self.get_field(&mapping_pair_node.parent()?, "type")?;
                let type_field_value = type_field_node
                    .child_by_field_name("value")?
                    .utf8_text(self.src.as_bytes())
                    .ok()?;

                let lock = block_in_place(|| self.context.prototypes.blocking_read());
                let prototype = lock
                    .par_iter()
                    .filter(|p| p.prototype == type_field_value)
                    .find_any(|p| p.id == seeking)?;

                let index = prototype.index();
                self.index_to_definition(index)
            }
            _ => {
                if seeking != key_name {
                    return None;
                }

                let proto_name = self
                    .get_field(&mapping_pair_node.parent()?, "type")?
                    .child_by_field_name("value")?
                    .utf8_text(self.src.as_bytes())
                    .ok()?;

                let prototype = block_in_place(|| self.context.classes.blocking_read())
                    .par_iter()
                    .filter_map(|c| Prototype::try_from(Arc::clone(c)).ok())
                    .find_any(|p| camel_case(&p.get_prototype_name()) == proto_name)?;

                let reflection = ReflectionManager::new(self.context.classes.clone());
                let field = block(|| reflection.get_fields(Arc::clone(&prototype)))
                    .into_iter()
                    .find(|f| f.get_data_field_name() == key_name)?;

                let index = field.index();
                self.index_to_definition(index)
            }
        }
    }

    fn index_to_definition(&self, index: &DefinitionIndex) -> GotoDefinitionResult {
        let uri = Url::from_file_path(index.0.clone()).ok()?;
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
        let definition = GotoDefinitionResponse::Scalar(Location { uri, range });

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

fn get_location_link(index: &DefinitionIndex, node: Node) -> Option<LocationLink> {
    let DefinitionIndex(path, Some(locale_range)) = index else {
        return None;
    };
    let url_range = lsp_types::Range {
        start: Position {
            line: node.start_position().row as u32,
            character: node.start_position().column as u32,
        },
        end: Position {
            line: node.end_position().row as u32,
            character: node.end_position().column as u32,
        },
    };
    let selection_range = lsp_types::Range {
        start: Position {
            line: locale_range.start_point.row as u32,
            character: locale_range.start_point.column as u32,
        },
        end: Position {
            line: locale_range.end_point.row as u32,
            character: locale_range.end_point.column as u32,
        },
    };

    Some(LocationLink {
        origin_selection_range: Some(url_range),
        target_uri: Url::from_file_path(path).ok()?,
        target_selection_range: selection_range,
        target_range: selection_range,
    })
}
