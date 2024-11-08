use super::{Completion, CompletionResult};
use crate::{
    backend::{CsharpClasses, YamlPrototypes},
    parse::structs::csharp::{Component, CsharpClassField, Prototype, ReflectionManager},
    utils::block,
};
use rayon::prelude::*;
use ropey::Rope;
use stringcase::camel_case;
use tower_lsp::lsp_types::{
    self, CompletionItem, CompletionItemKind, CompletionItemLabelDetails, CompletionList,
    CompletionResponse, CompletionTextEdit, Position, TextEdit,
};
use tree_sitter::{Node, Parser, Point, Tree};

pub struct YamlCompletion {
    classes: CsharpClasses,
    prototypes: YamlPrototypes,
    position: Position,
    src: String,
    tree: Tree,
}

impl YamlCompletion {
    pub fn new(
        classes: CsharpClasses,
        prototypes: YamlPrototypes,
        position: Position,
        src: &Rope,
    ) -> Self {
        let src = src.to_string();

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

    fn get_object_name(&self, node: &Node) -> Option<&str> {
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
        let nest = self.get_nesting(&node);

        if nest > 2 {
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
                    new_text: format!("type: "),
                })),
                ..Default::default()
            }]))
        }
    }

    fn block_mapping_pair(&self, node: Node) -> CompletionResult {
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
            None
        }
    }

    fn block_mapping(&self, node: Node) -> CompletionResult {
        let nest = self.get_nesting(&node);

        if nest > 2 {
            self.component_fields_completion(node)
        } else {
            self.prototype_fields_completion(node)
        }
    }

    fn flow_node(&self, node: Node) -> CompletionResult {
        self.prototype_parents_completion(node)
    }

    fn flow_sequence(&self, node: Node) -> CompletionResult {
        self.prototype_parents_completion(node)
    }

    // TODO: Rewrite it into something more understandable...
    fn prototype_parents_completion(&self, node: Node) -> CompletionResult {
        match node.kind() {
            "flow_sequence" => {
                let parent_node = node.parent()?.prev_named_sibling()?;
                if parent_node.utf8_text(self.src.as_bytes()).ok() != Some("parent") {
                    return None;
                }

                let proto = self.get_object_name(&parent_node.parent()?.parent()?)?;
                let specified_parents = self.get_specified_parents(&node).unwrap_or_default();
                let mut parents = tokio::task::block_in_place(|| self.prototypes.blocking_read())
                    .par_iter()
                    .filter(|p| p.prototype == proto)
                    .filter(|p| !specified_parents.contains(&p.id.as_str()))
                    .map(|p| {
                        let mut is_comma = false;
                        let mut is_flow_node = false;
                        CompletionItem {
                            label: p.id.clone(),
                            kind: Some(CompletionItemKind::CLASS),
                            detail: Some(p.prototype.clone()),
                            text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                                range: {
                                    let child_count = node.child_count();
                                    let last_child = if child_count > 1 {
                                        node.child(child_count - 2)
                                    } else {
                                        None
                                    };

                                    let position = Position::new(
                                        self.position.line,
                                        if let Some(last_child) = last_child {
                                            if last_child.kind() == "," {
                                                is_comma = true;
                                                last_child.end_position().column as u32 + 1
                                            } else if last_child.kind() == "flow_node" {
                                                is_flow_node = true;
                                                last_child.end_position().column as u32
                                            } else {
                                                last_child.end_position().column as u32 + 1
                                            }
                                        } else {
                                            node.start_position().column as u32 + 1
                                        },
                                    );
                                    lsp_types::Range {
                                        start: position,
                                        end: position,
                                    }
                                },
                                new_text: if is_comma {
                                    p.id.clone()
                                } else if is_flow_node {
                                    format!(", {}", p.id)
                                } else {
                                    p.id.clone()
                                },
                            })),
                            ..Default::default()
                        }
                    })
                    .collect::<Vec<_>>();

                parents.sort_by(|a, b| a.label.cmp(&b.label));
                parents.truncate(100);

                if !parents.is_empty() {
                    Some(CompletionResponse::List(CompletionList {
                        is_incomplete: true,
                        items: parents,
                    }))
                } else {
                    None
                }
            }
            "flow_node" => {
                let parent_node = node.parent()?.parent()?.prev_named_sibling()?;
                let parent_name = parent_node.utf8_text(self.src.as_bytes()).ok()?;
                if parent_name != "parent" {
                    return None;
                }

                let container_node = parent_node.parent()?.parent()?;
                if container_node.kind() != "block_mapping" {
                    return None;
                }

                let proto = self.get_object_name(&container_node)?;
                let value = node.utf8_text(self.src.as_bytes()).ok()?;
                let specified_parents = self
                    .get_specified_parents(&node.parent()?)
                    .unwrap_or_default();
                let mut parents = tokio::task::block_in_place(|| self.prototypes.blocking_read())
                    .par_iter()
                    .filter(|p| p.prototype == proto)
                    .filter(|p| !specified_parents.contains(&p.id.as_str()))
                    .map(|p| (strsim::jaro_winkler(value, &p.id), p))
                    .filter(|(diff, _)| diff > &0.6)
                    .map(|(diff, p)| {
                        (
                            diff,
                            CompletionItem {
                                label: p.id.clone(),
                                kind: Some(CompletionItemKind::CLASS),
                                detail: Some(p.prototype.clone()),
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
                                    new_text: p.id.clone(),
                                })),
                                ..Default::default()
                            },
                        )
                    })
                    .collect::<Vec<_>>();

                parents.sort_by_key(|(diff, _)| (*diff * 100.0) as u32);
                parents.reverse();
                parents.truncate(100);

                if !parents.is_empty() {
                    Some(CompletionResponse::List(CompletionList {
                        is_incomplete: true,
                        items: parents.into_iter().map(|(_, p)| p).collect::<Vec<_>>(),
                    }))
                } else {
                    None
                }
            }
            "block_mapping_pair" => match node.child_by_field_name("value") {
                Some(value_node) => {
                    let value = value_node.utf8_text(self.src.as_bytes()).ok()?;
                    let key_node = node.child_by_field_name("key")?;
                    let proto = self.get_object_name(&node.parent().unwrap())?;
                    let mut parents =
                        tokio::task::block_in_place(|| self.prototypes.blocking_read())
                            .par_iter()
                            .filter(|p| p.prototype == proto)
                            .map(|p| (strsim::jaro_winkler(value, &p.id), p))
                            .filter(|(diff, _)| diff > &0.6)
                            .map(|(diff, p)| {
                                (
                                    diff,
                                    CompletionItem {
                                        label: p.id.clone(),
                                        kind: Some(CompletionItemKind::CLASS),
                                        detail: Some(p.prototype.clone()),
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
                                            new_text: p.id.clone(),
                                        })),
                                        ..Default::default()
                                    },
                                )
                            })
                            .collect::<Vec<_>>();

                    parents.sort_by_key(|(diff, _)| (*diff * 100.0) as u32);
                    parents.reverse();
                    parents.truncate(100);

                    if !parents.is_empty() {
                        Some(CompletionResponse::List(CompletionList {
                            is_incomplete: true,
                            items: parents.into_iter().map(|(_, p)| p).collect::<Vec<_>>(),
                        }))
                    } else {
                        None
                    }
                }
                None => {
                    let key_node = node.child_by_field_name("key")?;
                    let proto = self.get_object_name(&node.parent().unwrap())?;
                    let lock = tokio::task::block_in_place(|| self.prototypes.blocking_read());
                    let mut parents = lock
                        .par_iter()
                        .filter(|p| p.prototype == proto)
                        .map(|p| CompletionItem {
                            label: p.id.clone(),
                            kind: Some(CompletionItemKind::CLASS),
                            detail: Some(p.prototype.clone()),
                            text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                                range: {
                                    let position = Position::new(
                                        self.position.line,
                                        key_node.end_position().column as u32 + 2,
                                    );
                                    lsp_types::Range {
                                        start: position,
                                        end: Position {
                                            character: position.character + p.id.len() as u32,
                                            ..position
                                        },
                                    }
                                },
                                new_text: p.id.clone(),
                            })),
                            ..Default::default()
                        })
                        .collect::<Vec<_>>();

                    parents.sort_by(|a, b| a.label.cmp(&b.label));
                    parents.truncate(100);

                    if !parents.is_empty() {
                        Some(CompletionResponse::List(CompletionList {
                            is_incomplete: true,
                            items: parents,
                        }))
                    } else {
                        None
                    }
                }
            },
            _ => None,
        }
    }

    fn component_fields_completion(&self, node: Node) -> CompletionResult {
        let comp_name = self.get_object_name(&node)?;
        let specified_fields = self.get_specified_fields(&node);
        let reflection = ReflectionManager::new(self.classes.clone());
        let comp = block(|| reflection.get_component_by_name(comp_name))?;
        let fields = block(|| reflection.get_fields(&comp))
            .into_par_iter()
            .filter(|f| f.attributes.contains("DataField"))
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
        let proto_name = self.get_object_name(&node)?;
        let specified_fields = self.get_specified_fields(&node);
        let reflection = ReflectionManager::new(self.classes.clone());
        let proto = block(|| reflection.get_prototype_by_name(proto_name))?;
        let fields = block(|| reflection.get_fields(&proto))
            .into_par_iter()
            .filter(|f| f.attributes.contains("DataField"))
            .chain([CsharpClassField {
                name: "id".to_owned(),
                type_name: "string".to_owned(),
                ..Default::default()
            }])
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
        let value_node = node
            .child_by_field_name("value")
            .map(|v| v.utf8_text(self.src.as_bytes()).unwrap());

        let lock = tokio::task::block_in_place(|| self.classes.blocking_read());
        let completions = lock
            .par_iter()
            .filter_map(|c| Prototype::try_from(c).ok())
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

        let value = node
            .child_by_field_name("value")
            .map(|node| node.utf8_text(self.src.as_bytes()).unwrap());

        let lock = tokio::task::block_in_place(|| self.classes.blocking_read());
        let completions = lock
            .par_iter()
            .filter_map(|c| Component::try_from(c).ok())
            .filter(|c| {
                if let Some(value) = value {
                    let name = c.get_component_name().to_lowercase();
                    let diff = strsim::damerau_levenshtein(value.to_lowercase().as_str(), &name);

                    diff < name.len()
                } else {
                    true
                }
            })
            .map(|c| {
                let name = c.get_component_name();

                CompletionItem {
                    label: name.clone(),
                    kind: Some(CompletionItemKind::CLASS),
                    label_details: Some(CompletionItemLabelDetails {
                        detail: Some("Component".to_owned()),
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
                        new_text: name,
                    })),
                    ..Default::default()
                }
            })
            .collect::<Vec<_>>();

        Some(CompletionResponse::Array(completions))
    }
}

impl Completion for YamlCompletion {
    fn completion(&self) -> CompletionResult {
        let (start_col, end_col) = {
            // Calculate the position for the correct node search.
            // P.S. Why on tree-sitter playground everything works correctly (in javascript)
            // even without dancing with tambourine - idk.
            let line = self
                .src
                .lines()
                .nth(self.position.line as usize)
                .unwrap_or_default();

            // If the string is empty, we use the cursor coordinates
            // and minus them by one, otherwise the root node `stream` will be searched.
            let trim_str = line.trim();
            if trim_str.len() == 0 {
                let col = if self.position.character == 0 {
                    self.position.character
                } else {
                    self.position.character - 1
                } as usize;

                (col, col)

            // If the string starts with `-`, we try to find the coordinate starting before
            // the `-` character, since only there tree-sitter can detect the `block_sequence_item` node.
            } else if trim_str.len() == 1 && trim_str.chars().all(|c| c == '-') {
                let mut col = 0;
                let mut chars = line.chars();
                while let Some(ch) = chars.next() {
                    if ch == '-' {
                        break;
                    }
                    col += 1;
                }
                (col, col)

            // If the string is not empty, we catch the beginning of the text
            // and the end of the text to properly search for child nodes.
            } else {
                let mut scol = line.chars().count();
                let mut ecol = scol;
                let mut chars = {
                    let mut c = line.chars();
                    while let Some(_) = c.next_back() {
                        scol -= 1;

                        if scol == self.position.character as usize {
                            break;
                        } else if scol < self.position.character as usize {
                            c.next();
                            scol += 1;
                            break;
                        }
                    }
                    c
                };
                let mut text = false;
                while let Some(ch) = chars.next_back() {
                    scol -= 1;

                    if !ch.is_whitespace() {
                        text = true;
                    } else if text && ch.is_whitespace() {
                        break;
                    }

                    if !text {
                        ecol -= 1;
                    }
                }

                (scol + 1, ecol - 1)
            }
        };
        let start_point = Point::new(self.position.line as usize, start_col);
        let end_point = Point::new(self.position.line as usize, end_col);

        let root_node = self.tree.root_node();
        let found_node = root_node.named_descendant_for_point_range(start_point, end_point);

        if found_node.is_none() {
            return None;
        }

        // If a text node was found, we climb to the parent node,
        // or an error node, we terminate altogether.
        let found_node = {
            let mut node = found_node.unwrap();
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
                let point = Point {
                    row: if self.position.line == 0 {
                        return None;
                    } else {
                        self.position.line as usize - 1
                    },
                    column: self.position.character as usize,
                };
                let found_node = {
                    let mut node = found_node.named_descendant_for_point_range(point, point)?;
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
                    self.block_mapping(found_node)
                } else {
                    None
                }
            }
            "flow_sequence" => {
                let point = Point {
                    row: self.position.line as usize,
                    column: self.position.character as usize - 1,
                };
                let found_node = {
                    let mut node = found_node.named_descendant_for_point_range(point, point)?;
                    if node.kind() != "flow_sequence" {
                        while let Some(n) = node.parent() {
                            node = n;

                            if n.kind() == "flow_node" {
                                break;
                            }
                        }
                    }
                    node
                };

                match found_node.kind() {
                    "flow_node" => self.flow_node(found_node),
                    "flow_sequence" => self.flow_sequence(found_node),
                    _ => None,
                }
            }
            _ => return None,
        }
    }
}
