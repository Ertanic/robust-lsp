use super::{Completion, CompletionResult};
use crate::{
    backend::Context,
    parse::structs::{
        csharp::{Component, CsharpClassField, Prototype, ReflectionManager},
        json::RsiMeta,
    },
    utils::{block, get_columns},
};
use rayon::prelude::*;
use ropey::Rope;
use std::{fs, path::PathBuf, sync::Arc};
use stringcase::camel_case;
use tokio::task::block_in_place;
use tower_lsp::lsp_types::{
    self, CompletionItem, CompletionItemKind, CompletionItemLabelDetails, CompletionList,
    CompletionResponse, CompletionTextEdit, Position, Range, TextEdit,
};
use tracing::instrument;
use tree_sitter::{Node, Parser, Point, Tree};

const SPRITES_RES_PATH: &str = "Resources/Textures/";

pub struct YamlCompletion {
    context: Arc<Context>,
    position: Position,
    src: String,
    tree: Tree,
    root_path: PathBuf,
}

impl Completion for YamlCompletion {
    fn completion(&self) -> CompletionResult {
        let (start_col, end_col) = get_columns(self.position, &self.src);
        let start_point = Point::new(self.position.line as usize, start_col);
        let end_point = Point::new(self.position.line as usize, end_col);

        let root_node = self.tree.root_node();
        let found_node = root_node.named_descendant_for_point_range(start_point, end_point)?;

        // If a text node was found, we climb to the parent node,
        // or an error node, we terminate altogether.
        let found_node = {
            let mut node = found_node;
            tracing::trace!("Found node: {node:#?}");
            if node.kind() == "string_scalar" {
                for _ in 0..3 {
                    node = node.parent().unwrap();
                }
            }
            if node.kind() == "ERROR" {
                return None;
            }
            node
        };

        tracing::trace!("Work with node {found_node:?}");

        match found_node.kind() {
            "block_mapping_pair" => self.block_mapping_pair(found_node),
            "block_mapping" => self.block_mapping(found_node),
            "block_sequence_item" => self.block_sequence_item(found_node),
            "block_sequence" => {
                let block_mapping = self.find_block_mapping(found_node)?;
                self.block_mapping(block_mapping)
            }
            "flow_sequence" => {
                let flow_item = self.find_flow_item(found_node)?;
                match flow_item.kind() {
                    "flow_node" => self.flow_node(flow_item),
                    "flow_sequence" => self.flow_sequence(flow_item),
                    _ => None,
                }
            }
            _ => None,
        }
    }
}

impl YamlCompletion {
    pub fn new(context: Arc<Context>, position: Position, src: &Rope, root_path: PathBuf) -> Self {
        let src = src.to_string();

        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_yaml::language()).unwrap();
        let tree = parser.parse(&src, None).unwrap();

        Self {
            context,
            position,
            src,
            tree,
            root_path,
        }
    }

    fn find_flow_item<'a>(&self, node: Node<'a>) -> Option<Node<'a>> {
        debug_assert_eq!(node.kind(), "flow_sequence");

        let point = Point {
            row: self.position.line as usize,
            column: self.position.character as usize - 1,
        };
        let mut node = node.named_descendant_for_point_range(point, point)?;
        if node.kind() != "flow_sequence" {
            while let Some(n) = node.parent() {
                node = n;
                if n.kind() == "flow_node" {
                    break;
                }
            }
        }
        Some(node)
    }

    fn find_block_mapping<'a>(&self, node: Node<'a>) -> Option<Node<'a>> {
        debug_assert_eq!(node.kind(), "block_sequence");

        let point = Point {
            row: if self.position.line == 0 {
                return None;
            } else {
                self.position.line as usize - 1
            },
            column: self.position.character as usize,
        };
        let found_node = {
            let mut node = node.named_descendant_for_point_range(point, point)?;
            if node.kind() == "block_mapping" {
                node
            } else {
                while let Some(n) = node.parent() {
                    node = n;
                    if n.kind() == "block_mapping" {
                        break;
                    }
                }
                node
            }
        };

        if found_node.kind() == "block_mapping" {
            Some(found_node)
        } else {
            None
        }
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

    fn get_object_name(&self, node: &Node) -> Option<&str> {
        debug_assert_eq!(node.kind(), "block_mapping");

        let mut name = None;
        let children_count = node.named_child_count();

        for i in 0..children_count {
            let field_node = node.named_child(i)?;
            let key_node = field_node.child_by_field_name("key");
            let value_node = field_node.child_by_field_name("value");

            let (key, value) = match (key_node, value_node) {
                (Some(key_node), Some(value_node)) => (
                    key_node.utf8_text(self.src.as_bytes()).ok()?,
                    value_node.utf8_text(self.src.as_bytes()).ok()?,
                ),
                _ => continue,
            };

            if key != "type" {
                continue;
            }

            name = Some(value);
            break;
        }

        name
    }

    fn get_specified_fields<'a>(&'a self, block_mapping_node: &Node) -> Vec<&'a str> {
        debug_assert_eq!(block_mapping_node.kind(), "block_mapping");

        let children_count = block_mapping_node.named_child_count();
        let mut fields = Vec::with_capacity(children_count);
        for i in 0..children_count {
            let child = block_mapping_node.child(i).unwrap();
            if child.kind() == "block_mapping_pair" {
                let key_node = child.child_by_field_name("key");
                if let Some(key_node) = key_node {
                    fields.push(key_node.utf8_text(self.src.as_bytes()).unwrap());
                }
            }
        }
        fields
    }

    fn get_specified_parents(&self, node: &Node) -> Option<Vec<&str>> {
        debug_assert!(node.kind() == "block_mapping_pair" || node.kind() == "flow_sequence");

        let mut parents = Vec::new();
        match node.kind() {
            "block_mapping_pair" => {
                let value_node = node.child_by_field_name("value")?;
                let value = value_node.utf8_text(self.src.as_bytes()).ok()?;
                parents.push(value);
                Some(parents)
            }
            "flow_sequence" => {
                for i in 0..node.named_child_count() {
                    let child = node.named_child(i).unwrap();
                    let value = child.utf8_text(self.src.as_bytes()).ok()?;
                    parents.push(value);
                }
                Some(parents)
            }
            _ => None,
        }
    }

    fn block_sequence_item(&self, node: Node) -> CompletionResult {
        debug_assert_eq!(node.kind(), "block_sequence_item");

        if self.get_nesting(&node) > 4 {
            None
        } else {
            Some(CompletionResponse::Array(vec![CompletionItem {
                label: "type".to_owned(),
                kind: Some(CompletionItemKind::FIELD),
                detail: Some("string".to_owned()),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                    range: {
                        let position = Position::new(
                            self.position.line,
                            node.start_position().column as u32 + 2,
                        );
                        lsp_types::Range {
                            start: position,
                            end: position,
                        }
                    },
                    new_text: format!("type:"),
                })),
                ..Default::default()
            }]))
        }
    }

    fn block_mapping_pair(&self, node: Node) -> CompletionResult {
        debug_assert_eq!(node.kind(), "block_mapping_pair");

        let nest = self.get_nesting(&node);
        let key_node = node.child_by_field_name("key")?;
        let key_name = key_node.utf8_text(self.src.as_bytes()).ok()?;

        if key_name == "type" {
            match nest {
                2 => return self.prototype_completion(node, key_node),
                4 => return self.components_completion(node, key_node),
                _ => None,
            }
        } else if key_name == "parent" && nest == 2 {
            self.prototype_parents_completion(node)
        } else {
            self.object_field_type_completion(node)
        }
    }

    fn block_mapping(&self, node: Node) -> CompletionResult {
        debug_assert_eq!(node.kind(), "block_mapping");

        if self.get_nesting(&node) > 2 {
            self.component_fields_completion(node)
        } else {
            self.prototype_fields_completion(node)
        }
    }

    fn flow_node(&self, node: Node) -> CompletionResult {
        debug_assert_eq!(node.kind(), "flow_node");
        self.prototype_parents_completion(node)
    }

    fn flow_sequence(&self, node: Node) -> CompletionResult {
        debug_assert_eq!(node.kind(), "flow_sequence");
        self.prototype_parents_completion(node)
    }

    fn object_field_type_completion(&self, node: Node) -> CompletionResult {
        debug_assert_eq!(node.kind(), "block_mapping_pair");

        let key_node = node.child_by_field_name("key")?;
        let key_name = key_node.utf8_text(self.src.as_bytes()).ok()?;
        let mapping_node = node.parent()?;
        let obj_name = self.get_object_name(&mapping_node)?;
        let reflection = ReflectionManager::new(self.context.classes.clone());

        match self.get_nesting(&node) {
            2 => self.prototype_field_type_completion(node, reflection, obj_name, key_name),
            4 => self.component_field_type_completion(node, reflection, obj_name, key_name),
            _ => None,
        }
    }

    #[instrument(skip_all, ret)]
    fn component_field_type_completion(
        &self,
        node: Node,
        reflection: ReflectionManager,
        object_name: &str,
        key_name: &str,
    ) -> CompletionResult {
        debug_assert_eq!(node.kind(), "block_mapping_pair");

        let comp = block(|| reflection.get_component_by_name(object_name))?;
        let field = block(|| reflection.get_fields(Arc::clone(&comp)))
            .into_iter()
            .find(|f| f.get_data_field_name() == key_name)?;

        match (
            comp.get_component_name().as_str(),
            field.get_data_field_name().as_str(),
        ) {
            ("Sprite" | "Icon", "sprite") => self.sprite_field_type_completion(node),
            ("Sprite", "state") => self.state_field_type_completion(node),
            _ => self.field_type_completion(node, field, reflection),
        }
    }

    fn state_field_type_completion(&self, node: Node) -> CompletionResult {
        debug_assert_eq!(node.kind(), "block_mapping_pair");

        let sprites_folder = self.root_path.join(SPRITES_RES_PATH);
        if !sprites_folder.exists() {
            return None;
        }

        let sprite_node = self
            .get_field(&node.parent()?, "sprite")?
            .child_by_field_name("value")?;
        let sprite_path = sprite_node.utf8_text(self.src.as_bytes()).ok()?;

        if !sprite_path.ends_with(".rsi") {
            tracing::trace!("sprite path does not end with .rsi");
            return None;
        }

        let path = sprites_folder.join(sprite_path);
        if !path.exists() || !path.is_dir() {
            tracing::trace!("{path:?} does not exist");
            return None;
        }

        let rsi_name = path.file_name()?.to_string_lossy().into_owned();
        let meta_path = path.join("meta.json");

        if !meta_path.exists() || !meta_path.is_file() {
            tracing::trace!("{meta_path:?} does not exist");
            return None;
        }

        let meta: RsiMeta = match serde_json::from_reader(fs::File::open(&meta_path).ok()?) {
            Ok(meta) => meta,
            Err(err) => {
                tracing::error!("Failed to read {meta_path:?}: {err}");
                return None;
            }
        };

        let map = |s: String| CompletionItem {
            label: s,
            kind: Some(CompletionItemKind::VALUE),
            detail: Some(rsi_name.clone()),
            ..Default::default()
        };

        let states = match node.child_by_field_name("value") {
            Some(value_node) => {
                let value = value_node.utf8_text(self.src.as_bytes()).ok()?;
                let mut states = meta
                    .states
                    .into_iter()
                    .map(|s| (strsim::jaro_winkler(value, &s.name), s.name))
                    .filter(|(diff, _)| *diff > 0.6)
                    .map(|(diff, s)| (diff, map(s)))
                    .collect::<Vec<_>>();

                states.sort_by_key(|(diff, _)| (*diff * 100.0) as u32);
                states.reverse();

                states.into_iter().map(|(_, s)| s).collect::<Vec<_>>()
            }
            None => {
                let states = meta
                    .states
                    .into_iter()
                    .map(|s| map(s.name))
                    .collect::<Vec<_>>();

                states
            }
        };

        if states.is_empty() {
            None
        } else {
            Some(CompletionResponse::Array(states))
        }
    }

    fn sprite_field_type_completion(&self, node: Node) -> CompletionResult {
        debug_assert_eq!(node.kind(), "block_mapping_pair");

        let sprites_folder = self.root_path.join(SPRITES_RES_PATH);
        if !sprites_folder.exists() {
            return None;
        }

        let paths = match node.child_by_field_name("value") {
            Some(value_node) => {
                let value = value_node.utf8_text(self.src.as_bytes()).ok()?;
                if value.ends_with('/') {
                    let path = sprites_folder.join(value);
                    if !path.exists() || !path.is_dir() {
                        tracing::trace!("{path:?} does not exist");
                        return None;
                    }

                    let last = value.split('/').filter(|s| !s.is_empty()).last()?;
                    if last.ends_with(".rsi") {
                        tracing::trace!("{last} ends with .rsi");
                        return None;
                    }

                    let paths = fs::read_dir(path)
                        .ok()?
                        .filter_map(Result::ok)
                        .map(|f| {
                            let path = f.path();
                            let name = path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned();
                            let is_rsi = name.ends_with(".rsi");
                            CompletionItem {
                                label: name.clone(),
                                kind: Some(if is_rsi {
                                    CompletionItemKind::FILE
                                } else {
                                    CompletionItemKind::FOLDER
                                }),
                                insert_text: Some(if is_rsi { name } else { format!("{name}/") }),
                                ..Default::default()
                            }
                        })
                        .collect::<Vec<_>>();

                    paths
                } else {
                    let parts = value
                        .split('/')
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>();
                    let last = parts.last()?.to_owned();

                    if last.ends_with(".rsi") {
                        tracing::trace!("{last} ends with .rsi");
                        return None;
                    }

                    let parts_count = parts.len();
                    let sprites_path = if parts_count == 1 {
                        sprites_folder
                    } else {
                        sprites_folder
                            .join(parts.into_iter().take(parts_count - 1).collect::<PathBuf>())
                    };
                    if !sprites_path.exists() || !sprites_path.is_dir() {
                        tracing::trace!("{sprites_path:?} does not exist");
                        return None;
                    }

                    let mut paths = fs::read_dir(sprites_path)
                        .ok()?
                        .filter_map(Result::ok)
                        .map(|f| {
                            let path = f.path();
                            let name = path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned();
                            (strsim::jaro_winkler(last, &name), name)
                        })
                        .filter(|(diff, _)| *diff > 0.6)
                        .map(|(diff, name)| {
                            let is_rsi = name.ends_with(".rsi");
                            (
                                diff,
                                CompletionItem {
                                    label: name.clone(),
                                    kind: Some(if is_rsi {
                                        CompletionItemKind::FILE
                                    } else {
                                        CompletionItemKind::FOLDER
                                    }),
                                    insert_text: Some(if is_rsi {
                                        name
                                    } else {
                                        format!("{name}/")
                                    }),
                                    ..Default::default()
                                },
                            )
                        })
                        .collect::<Vec<_>>();

                    paths.sort_by_key(|p| (p.0 * 100.0) as u32);
                    paths.reverse();
                    paths.truncate(100);

                    paths.into_iter().map(|(_, p)| p).collect::<Vec<_>>()
                }
            }
            None => {
                let paths = fs::read_dir(sprites_folder)
                    .ok()?
                    .filter_map(Result::ok)
                    .map(|f| {
                        let path = f.path();
                        let name = path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned();

                        if name.ends_with(".rsi") {
                            CompletionItem {
                                label: name.clone(),
                                kind: Some(CompletionItemKind::FILE),
                                insert_text: Some(format!("{name}/")),
                                ..Default::default()
                            }
                        } else {
                            CompletionItem {
                                label: name.clone(),
                                kind: Some(if path.is_dir() {
                                    CompletionItemKind::FOLDER
                                } else {
                                    CompletionItemKind::FILE
                                }),
                                insert_text: Some(format!("{name}/")),
                                ..Default::default()
                            }
                        }
                    })
                    .collect();

                paths
            }
        };

        if !paths.is_empty() {
            Some(CompletionResponse::List(CompletionList {
                is_incomplete: true,
                items: paths,
            }))
        } else {
            None
        }
    }

    fn prototype_field_type_completion(
        &self,
        node: Node,
        reflection: ReflectionManager,
        object_name: &str,
        key_name: &str,
    ) -> CompletionResult {
        debug_assert_eq!(node.kind(), "block_mapping_pair");

        let prototype = block(|| reflection.get_prototype_by_name(object_name))?;
        let field = block(|| reflection.get_fields(Arc::clone(&prototype)))
            .into_iter()
            .find(|f| f.get_data_field_name() == key_name)?;

        self.field_type_completion(node, field, reflection)
    }

    fn field_type_completion(
        &self,
        node: Node,
        field: CsharpClassField,
        reflection: ReflectionManager,
    ) -> CompletionResult {
        debug_assert_eq!(node.kind(), "block_mapping_pair");

        let items = match field.type_name.trim_end_matches('?') {
            "bool" => vec!["true", "false"]
                .into_iter()
                .map(|value| CompletionItem {
                    label: value.to_string(),
                    kind: Some(CompletionItemKind::VALUE),
                    ..Default::default()
                })
                .collect::<Vec<_>>(),
            "EntProtoId" => {
                let lock = tokio::task::block_in_place(|| self.context.prototypes.blocking_read());
                let entity_prototypes = lock.par_iter().filter(|p| p.prototype == "entity");

                let prototypes = match node.child_by_field_name("value") {
                    Some(value_node) => {
                        let value = value_node.utf8_text(self.src.as_bytes()).ok()?;
                        let mut prototypes = entity_prototypes
                            .map(|p| (strsim::jaro_winkler(value, p.id.as_str()), p))
                            .filter(|(similarity, _)| *similarity > 0.6)
                            .map(|(d, p)| {
                                (
                                    d,
                                    CompletionItem {
                                        label: p.id.clone(),
                                        kind: Some(CompletionItemKind::CLASS),
                                        detail: Some("entity".to_owned()),
                                        ..Default::default()
                                    },
                                )
                            })
                            .collect::<Vec<_>>();

                        prototypes.sort_by_key(|(diff, _)| (*diff * 100.0) as u32);
                        prototypes.reverse();
                        prototypes.truncate(100);

                        prototypes.into_iter().map(|(_, p)| p).collect::<Vec<_>>()
                    }
                    None => {
                        let mut prototypes = entity_prototypes
                            .map(|p| CompletionItem {
                                label: p.id.clone(),
                                kind: Some(CompletionItemKind::CLASS),
                                detail: Some("entity".to_owned()),
                                ..Default::default()
                            })
                            .collect::<Vec<_>>();
                        prototypes.truncate(100);

                        prototypes
                    }
                };

                prototypes
            }
            value if value.starts_with("ProtoId<") => {
                let inner = value.trim_start_matches("ProtoId<").trim_end_matches('>');
                let prototype = block(|| reflection.get_prototype_by_name(inner))?;
                let prototype_name = camel_case(&prototype.get_prototype_name());

                let lock = tokio::task::block_in_place(|| self.context.prototypes.blocking_read());
                let filtered_prototypes = lock.par_iter().filter(|p| p.prototype == prototype_name);

                let map = |l: String| CompletionItem {
                    label: l,
                    kind: Some(CompletionItemKind::CLASS),
                    detail: Some(prototype_name.clone()),
                    ..Default::default()
                };

                let prototypes = match node.child_by_field_name("value") {
                    Some(value_node) => {
                        let value = value_node.utf8_text(self.src.as_bytes()).ok()?;
                        let mut prototypes = filtered_prototypes
                            .map(|p| (strsim::jaro_winkler(value, &p.id), p))
                            .filter(|(diff, _)| *diff > 0.6)
                            .map(|(d, p)| (d, map(p.id.clone())))
                            .collect::<Vec<_>>();

                        prototypes.sort_by_key(|(diff, _)| (*diff * 100.0) as u32);
                        prototypes.reverse();
                        prototypes.truncate(100);

                        prototypes.into_iter().map(|(_, p)| p).collect::<Vec<_>>()
                    }
                    None => {
                        let mut prototypes = filtered_prototypes
                            .map(|p| map(p.id.clone()))
                            .collect::<Vec<_>>();

                        prototypes.truncate(100);
                        prototypes
                    }
                };

                prototypes
            }
            "LocId" => {
                let lock = block_in_place(|| self.context.locales.blocking_read());
                let map = |key: String, range: Option<Range>| CompletionItem {
                    label: key.clone(),
                    kind: Some(CompletionItemKind::VALUE),
                    detail: Some("locale".to_owned()),
                    text_edit: if let Some(range) = range {
                        Some(CompletionTextEdit::Edit(TextEdit {
                            new_text: key.clone(),
                            range,
                        }))
                    } else {
                        None
                    },
                    ..Default::default()
                };

                let locales = match node.child_by_field_name("value") {
                    Some(value_node) => {
                        let value = value_node.utf8_text(self.src.as_bytes()).ok()?;

                        tracing::trace!("Searching locales for {value}");

                        let mut locales = lock
                            .par_iter()
                            .map(|l| (strsim::jaro_winkler(value, &l.key), l))
                            .filter(|(diff, _)| *diff >= 0.8)
                            .map(|(d, l)| {
                                (
                                    d,
                                    map(l.key.clone(), {
                                        let start = Position::new(
                                            value_node.start_position().row as u32,
                                            value_node.start_position().column as u32,
                                        );
                                        let end = Position::new(
                                            value_node.end_position().row as u32,
                                            value_node.end_position().column as u32,
                                        );
                                        Some(Range::new(start, end))
                                    }),
                                )
                            })
                            .collect::<Vec<_>>();

                        locales.sort_by_key(|(diff, _)| (*diff * 100.0) as u32);
                        locales.reverse();
                        locales.truncate(100);

                        locales.into_iter().map(|(_, l)| l).collect::<Vec<_>>()
                    }
                    None => {
                        let mut locales = lock
                            .par_iter()
                            .map(|l| map(l.key.clone(), None))
                            .collect::<Vec<_>>();

                        locales.truncate(100);
                        locales
                    }
                };

                locales
            }
            _ => vec![],
        };

        tracing::trace!("Items found: {}", items.len());

        Some(CompletionResponse::List(CompletionList {
            is_incomplete: true,
            items,
        }))
    }

    // Is that even a little bit readable? I don't know how else to rewrite it better...
    fn prototype_parents_completion(&self, node: Node) -> CompletionResult {
        debug_assert!(
            node.kind() == "flow_sequence"
                || node.kind() == "flow_node"
                || node.kind() == "block_mapping_pair"
        );

        #[rustfmt::skip]
        let parent_field_name = match node.kind() {
            "flow_sequence" => node.parent()?.prev_named_sibling()?.utf8_text(self.src.as_bytes()).ok()?,
            "flow_node" => node.parent()?.parent()?.prev_named_sibling()?.utf8_text(self.src.as_bytes()).ok()?,
            "block_mapping_pair" => node.child_by_field_name("key")?.utf8_text(self.src.as_bytes()).ok()?,
            _ => return None,
        };

        if parent_field_name != "parent" {
            return None;
        }

        let proto_name = match node.kind() {
            "flow_sequence" => self.get_object_name(&node.parent()?.parent()?.parent()?)?,
            "flow_node" => self.get_object_name(&node.parent()?.parent()?.parent()?.parent()?)?,
            "block_mapping_pair" => self.get_object_name(&node.parent()?)?,
            _ => return None,
        };

        #[rustfmt::skip]
        let specified_parents = match node.kind() {
            "flow_sequence" => self.get_specified_parents(&node).unwrap_or_default(),
            "flow_node" => self.get_specified_parents(&node.parent()?).unwrap_or_default(),
            "block_mapping_pair" => vec![],
            _ => return None,
        };

        let lock = tokio::task::block_in_place(|| self.context.prototypes.blocking_read());
        let filtered_prototypes = lock
            .par_iter()
            .filter(|p| p.prototype == proto_name)
            .filter(|p| !specified_parents.contains(&p.id.as_str()));

        let map = |id: String,
                   prototype: String,
                   start_position: u32,
                   end_position: Option<Point>| CompletionItem {
            label: id.clone(),
            kind: Some(CompletionItemKind::CLASS),
            detail: Some(prototype),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                range: {
                    let position = Position::new(self.position.line, start_position);
                    lsp_types::Range {
                        start: position,
                        end: if let Some(end_position) = end_position {
                            Position::new(end_position.row as u32, end_position.column as u32)
                        } else {
                            position
                        },
                    }
                },
                new_text: id,
            })),
            ..Default::default()
        };

        let parents = match node.kind() {
            "flow_sequence" => {
                let child_count = node.child_count();
                let last_child = node.child(child_count - 2)?;
                let position = Position::new(
                    self.position.line,
                    match last_child.kind() {
                        "," => last_child.end_position().column as u32 + 1,
                        "flow_node" => return None,
                        _ => node.start_position().column as u32 + 1,
                    },
                );

                let mut parents = filtered_prototypes
                    .map(|p| CompletionItem {
                        label: p.id.clone(),
                        kind: Some(CompletionItemKind::CLASS),
                        detail: Some(p.prototype.clone()),
                        text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                            range: {
                                lsp_types::Range {
                                    start: position,
                                    end: position,
                                }
                            },
                            new_text: p.id.clone(),
                        })),
                        ..Default::default()
                    })
                    .collect::<Vec<_>>();

                parents.sort_by(|a, b| a.label.cmp(&b.label));
                parents.truncate(100);

                parents
            }
            "flow_node" => {
                let value = node.utf8_text(self.src.as_bytes()).ok()?;
                let mut parents = filtered_prototypes
                    .map(|p| (strsim::jaro_winkler(value, &p.id), p))
                    .filter(|(diff, _)| diff > &0.8)
                    .map(|(diff, p)| {
                        (
                            diff,
                            map(
                                p.id.clone(),
                                p.prototype.clone(),
                                node.start_position().column as u32,
                                Some(node.end_position()),
                            ),
                        )
                    })
                    .collect::<Vec<_>>();

                parents.sort_by_key(|(diff, _)| (*diff * 100.0) as u32);
                parents.reverse();
                parents.truncate(100);

                parents.into_iter().map(|(_, p)| p).collect()
            }
            "block_mapping_pair" => match node.child_by_field_name("value") {
                Some(value_node) => {
                    let value = value_node.utf8_text(self.src.as_bytes()).ok()?;
                    let key_node = node.child_by_field_name("key")?;
                    let mut parents = filtered_prototypes
                        .map(|p| (strsim::jaro_winkler(value, &p.id), p))
                        .filter(|(diff, _)| diff > &0.8)
                        .map(|(diff, p)| {
                            (
                                diff,
                                map(
                                    p.id.clone(),
                                    p.prototype.clone(),
                                    key_node.end_position().column as u32 + 2,
                                    Some(value_node.end_position()),
                                ),
                            )
                        })
                        .collect::<Vec<_>>();

                    parents.sort_by_key(|(diff, _)| (*diff * 100.0) as u32);
                    parents.reverse();
                    parents.truncate(100);

                    parents.into_iter().map(|(_, p)| p).collect()
                }
                None => {
                    let key_node = node.child_by_field_name("key")?;
                    let mut parents = filtered_prototypes
                        .map(|p| {
                            map(
                                p.id.clone(),
                                p.prototype.clone(),
                                key_node.end_position().column as u32 + 2,
                                None,
                            )
                        })
                        .collect::<Vec<_>>();

                    parents.truncate(100);

                    parents
                }
            },
            _ => vec![],
        };

        Some(CompletionResponse::List(CompletionList {
            is_incomplete: true,
            items: parents,
        }))
    }

    fn component_fields_completion(&self, node: Node) -> CompletionResult {
        debug_assert_eq!(node.kind(), "block_mapping");

        let comp_name = self.get_object_name(&node)?;
        let specified_fields = self.get_specified_fields(&node);
        let reflection = ReflectionManager::new(self.context.classes.clone());
        let comp = block(|| reflection.get_component_by_name(comp_name))?;
        let fields = block(|| reflection.get_fields(Arc::clone(&comp)))
            .into_par_iter()
            .filter(|f| {
                f.attributes.contains("DataField") || f.attributes.contains("IncludeDataField")
            })
            .filter(|f| !specified_fields.contains(&f.get_data_field_name().as_str()))
            .map(|f| {
                let name = f.get_data_field_name();
                CompletionItem {
                    label: name.clone(),
                    kind: Some(CompletionItemKind::FIELD),
                    detail: Some(f.type_name),
                    text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                        range: {
                            let position = Position::new(
                                self.position.line,
                                node.start_position().column as u32,
                            );
                            lsp_types::Range {
                                start: position,
                                end: position,
                            }
                        },
                        new_text: format!("{name}: "),
                    })),
                    ..Default::default()
                }
            })
            .collect::<Vec<_>>();

        if fields.len() > 0 {
            Some(CompletionResponse::Array(fields))
        } else {
            None
        }
    }

    fn prototype_fields_completion(&self, node: Node) -> CompletionResult {
        debug_assert_eq!(node.kind(), "block_mapping");

        let proto_name = self.get_object_name(&node)?;
        let specified_fields = self.get_specified_fields(&node);
        let reflection = ReflectionManager::new(self.context.classes.clone());
        let proto = block(|| reflection.get_prototype_by_name(proto_name))?;
        let fields = block(|| reflection.get_fields(Arc::clone(&proto)))
            .into_par_iter()
            .filter(|f| f.attributes.contains("DataField"))
            .chain([CsharpClassField::new_empty("id", "string")])
            .filter(|f| !specified_fields.contains(&f.get_data_field_name().as_str()))
            .map(|f| {
                let name = f.get_data_field_name();

                CompletionItem {
                    label: name.clone(),
                    kind: Some(CompletionItemKind::FIELD),
                    detail: Some(f.type_name),
                    text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                        range: {
                            let position = Position::new(
                                self.position.line,
                                node.start_position().column as u32,
                            );
                            lsp_types::Range {
                                start: position,
                                end: position,
                            }
                        },
                        new_text: format!("{name}: "),
                    })),
                    sort_text: if f.name == "id" || f.name == "components" {
                        Some("0".to_owned())
                    } else {
                        Some("1".to_owned())
                    },
                    ..Default::default()
                }
            })
            .collect::<Vec<_>>();

        if fields.len() > 0 {
            Some(CompletionResponse::Array(fields))
        } else {
            None
        }
    }

    fn prototype_completion(&self, node: Node, key_node: Node) -> CompletionResult {
        debug_assert_eq!(node.kind(), "block_mapping_pair");

        let value_node = node
            .child_by_field_name("value")
            .map(|v| v.utf8_text(self.src.as_bytes()).unwrap());

        let lock = tokio::task::block_in_place(|| self.context.classes.blocking_read());
        let completions = lock
            .par_iter()
            .filter_map(|c| Prototype::try_from(Arc::clone(c)).ok())
            .filter(|p| {
                if let Some(value) = value_node {
                    let name = p.get_prototype_name().to_lowercase();
                    let diff = strsim::damerau_levenshtein(value.to_lowercase().as_str(), &name);

                    diff < name.len()
                } else {
                    true
                }
            })
            .map(|p| {
                let name = p.get_prototype_name();

                CompletionItem {
                    label: name.to_owned(),
                    kind: Some(CompletionItemKind::CLASS),
                    label_details: Some(CompletionItemLabelDetails {
                        detail: Some("Prototype".to_owned()),
                        ..Default::default()
                    }),
                    text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                        range: {
                            let position = Position::new(
                                self.position.line,
                                key_node.end_position().column as u32 + 2,
                            );
                            lsp_types::Range {
                                start: position,
                                end: position,
                            }
                        },
                        new_text: camel_case(name.as_str()),
                    })),
                    sort_text: if name.to_lowercase() == "entity" {
                        Some("0".to_owned())
                    } else {
                        Some("1".to_owned())
                    },
                    ..Default::default()
                }
            })
            .collect::<Vec<_>>();

        Some(CompletionResponse::Array(completions))
    }

    fn components_completion(&self, node: Node, key_node: Node) -> CompletionResult {
        debug_assert_eq!(node.kind(), "block_mapping_pair");

        let is_components_node = {
            let mut node = node;
            for _ in 0..6 {
                node = node.parent()?;
            }
            if node.kind() != "block_mapping_pair" {
                false
            } else {
                let key_node = node.child_by_field_name("key")?;
                let key_value = key_node.utf8_text(self.src.as_bytes()).ok()?;
                key_value == "components"
            }
        };

        if !is_components_node {
            return None;
        }

        let value = node.child_by_field_name("value");

        let lock = tokio::task::block_in_place(|| self.context.classes.blocking_read());
        let completions = lock
            .par_iter()
            .filter_map(|c| Component::try_from(Arc::clone(c)).ok());

        let map = |c: &Component| {
            let name = c.get_component_name();

            CompletionItem {
                label: name.clone(),
                kind: Some(CompletionItemKind::CLASS),
                label_details: Some(CompletionItemLabelDetails {
                    detail: Some("Component".to_owned()),
                    ..Default::default()
                }),
                ..Default::default()
            }
        };

        let items = match value {
            Some(value_node) => {
                let value = value_node.utf8_text(self.src.as_bytes()).ok()?;
                let mut items = completions
                    .map(|c| (strsim::jaro_winkler(value, &c.get_component_name()), c))
                    .filter(|(diff, _)| *diff > 0.8)
                    .map(|(diff, c)| {
                        let item = map(&c);
                        let name = c.get_component_name();
                        (
                            diff,
                            CompletionItem {
                                text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                                    range: {
                                        let start = Position::new(
                                            self.position.line,
                                            key_node.end_position().column as u32 + 2,
                                        );
                                        let end = Position::new(
                                            self.position.line,
                                            value_node.end_position().column as u32,
                                        );
                                        lsp_types::Range { start, end }
                                    },
                                    new_text: name,
                                })),
                                ..item
                            },
                        )
                    })
                    .collect::<Vec<_>>();

                items.sort_by_key(|(diff, _)| (*diff * 100.) as u32);
                items.reverse();
                items.truncate(100);

                items.into_iter().map(|(_, c)| c).collect()
            }
            None => completions
                .map(|c| CompletionItem {
                    insert_text: Some(c.get_component_name().to_owned()),
                    ..map(&c)
                })
                .collect(),
        };

        Some(CompletionResponse::List(CompletionList {
            is_incomplete: true,
            items,
        }))
    }
}
