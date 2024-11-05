use crate::{
    backend::CsharpClasses,
    parse::structs::{CsharpAttributeArgumentType, CsharpClassField, Prototype, ReflectionManager},
    utils::block,
};
use rayon::prelude::*;
use ropey::Rope;
use std::ops::Range;
use stringcase::camel_case;
use tower_lsp::{
    jsonrpc::Result,
    lsp_types::{
        self, CompletionItem, CompletionItemKind, CompletionItemLabelDetails, CompletionResponse,
        CompletionTextEdit, Position, TextEdit,
    },
};
use tracing::instrument;
use tree_sitter::{Node, Parser, Point, Query, QueryCapture, QueryCursor, QueryMatch};

#[instrument(skip_all)]
pub fn completion(
    rope: &Rope,
    position: Position,
    classes: CsharpClasses,
) -> Result<Option<CompletionResponse>> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_yaml::language()).unwrap();

    let src = rope.to_string();

    let tree = parser.parse(&src, None).unwrap();
    let query = Query::new(
        &tree_sitter_yaml::language(),
        include_str!("../queries/prototype_yml_field.scm"),
    )
    .unwrap();
    let mut query_cursor = QueryCursor::new();
    query_cursor.set_point_range(Range {
        start: Point::new(position.line as usize, 0),
        end: Point::new(position.line as usize, position.character as usize),
    });

    let captures = query_cursor.captures(&query, tree.root_node(), src.as_bytes());
    for (m, _) in captures {
        let completions = match_patterns(m, classes.clone(), &src, &position);
        if completions.is_some() {
            return Ok(completions);
        }
    }

    Ok(None)
}

// I don't like using locks, but I haven't thought of anything better to prevent the compiler from
// swearing at the lack of Sync and Send on tree-sitter types that contain raw references inside them.
fn match_patterns(
    m: QueryMatch,
    classes: CsharpClasses,
    src: &str,
    position: &Position,
) -> Option<CompletionResponse> {
    let get_nesting = |node: Node| {
        let mut i = 0;
        let mut parent = node.parent();
        while let Some(node) = parent {
            if node.kind() == "block_node" {
                i += 1;
            }
            parent = node.parent();
        }
        i
    };

    match m.pattern_index {
        0 => {
            for capture in m.captures {
                let block_mapping_pair_node = capture.node;

                if get_nesting(block_mapping_pair_node) > 2 {
                    return None;
                }

                let key_node = match block_mapping_pair_node.child_by_field_name("key") {
                    Some(node) => node,
                    None => return None,
                };
                let key_name = key_node.utf8_text(src.as_bytes()).unwrap();

                let completions = match key_name {
                    "type" => get_type_completion(classes, capture, key_node, *position, &src),
                    _ => return None,
                };

                return Some(CompletionResponse::Array(completions));
            }
        }
        1 => {
            let get_specified_fields = |block_mapping_node: Node| {
                let mut fields = Vec::new();
                for i in 0..block_mapping_node.child_count() {
                    let child = block_mapping_node.child(i).unwrap();
                    if child.kind() == "block_mapping_pair" {
                        let key_node = child.child_by_field_name("key");
                        if let Some(key_node) = key_node {
                            fields.push(key_node.utf8_text(src.as_bytes()).unwrap());
                        }
                    }
                }
                fields
            };

            for capture in m.captures {
                let block_mapping_node = capture.node;

                if get_nesting(block_mapping_node) > 2 {
                    return None;
                }

                let proto_node = {
                    let mut node = None;

                    for i in 0..block_mapping_node.child_count() {
                        let child = block_mapping_node.child(i).unwrap();
                        let key_node = child.child_by_field_name("key");
                        if let Some(key_node) = key_node {
                            if key_node.utf8_text(src.as_bytes()).unwrap() == "type" {
                                node = Some(child);
                                break;
                            }
                        }
                    }

                    node
                };

                let key_node = {
                    let mut node = None;

                    for i in 0..block_mapping_node.named_child_count() {
                        let child = block_mapping_node.named_child(i).unwrap();
                        if child.kind() == "ERROR" {
                            node = Some(child);
                            break;
                        }
                    }

                    node.or_else(|| block_mapping_node.child_by_field_name("key"))
                };

                match (key_node, proto_node) {
                    (Some(key_node), Some(proto_node)) => {
                        let proto_name = proto_node.child_by_field_name("value");
                        if proto_name.is_none() {
                            return None;
                        }

                        let specified_fields = get_specified_fields(block_mapping_node);

                        let proto_name = proto_name.unwrap().utf8_text(src.as_bytes()).unwrap();
                        let reflection = ReflectionManager::new(classes.clone());

                        if let Some(proto) = block(|| reflection.get_prototype_by_name(proto_name))
                        {
                            let fields = block(|| reflection.get_fields(&proto))
                                .into_par_iter()
                                .filter(|f| f.attributes.contains("DataField"))
                                .filter(|f| {
                                    !specified_fields.contains(&f.get_data_field_name().as_str())
                                })
                                .filter(|f| {
                                    let attr = f.attributes.get("DataField").unwrap();
                                    let name = attr.arguments.get("tag");

                                    if let Some(name) = name {
                                        if let CsharpAttributeArgumentType::String(ref name) =
                                            name.value
                                        {
                                            strsim::damerau_levenshtein(
                                                key_node
                                                    .utf8_text(src.as_bytes())
                                                    .unwrap()
                                                    .trim_matches('"'),
                                                name,
                                            ) < name.len()
                                        } else {
                                            true
                                        }
                                    } else {
                                        strsim::damerau_levenshtein(
                                            key_node.utf8_text(src.as_bytes()).unwrap(),
                                            &f.name,
                                        ) < f.name.len()
                                    }
                                })
                                .collect::<Vec<_>>();

                            return Some(CompletionResponse::Array(get_field_completion(
                                fields,
                                block_mapping_node,
                                *position,
                            )));
                        }
                    }
                    (None, Some(proto_node)) => {
                        let proto_name = proto_node.child_by_field_name("value");
                        if proto_name.is_none() {
                            return None;
                        }

                        let specified_fields = get_specified_fields(block_mapping_node);

                        let reflection = ReflectionManager::new(classes.clone());
                        let proto_name = proto_name.unwrap().utf8_text(src.as_bytes()).unwrap();

                        if let Some(proto) = block(|| reflection.get_prototype_by_name(proto_name))
                        {
                            let fields = block(|| reflection.get_fields(&proto))
                                .into_par_iter()
                                .filter(|f| f.attributes.contains("DataField"))
                                .chain([CsharpClassField {
                                    name: "id".to_owned(),
                                    type_name: "string".to_owned(),
                                    ..Default::default()
                                }])
                                .filter(|f| {
                                    !specified_fields.contains(&f.get_data_field_name().as_str())
                                })
                                .collect::<Vec<_>>();

                            return Some(CompletionResponse::Array(get_field_completion(
                                fields,
                                block_mapping_node,
                                *position,
                            )));
                        }
                    }
                    (None, None) => {
                        let field = CsharpClassField {
                            name: "type".to_string(),
                            ..Default::default()
                        };
                        let fields = vec![field];

                        return Some(CompletionResponse::Array(get_field_completion(
                            fields,
                            block_mapping_node,
                            *position,
                        )));
                    }
                    _ => return None,
                }
            }

            return None;
        }
        _ => {
            tracing::trace!("Not a single match.");
            return None;
        }
    }

    None
}

fn get_field_completion(
    fields: Vec<CsharpClassField>,
    block_mapping_node: Node,
    position: Position,
) -> Vec<CompletionItem> {
    fields
        .into_par_iter()
        .map(|f| {
            let name = f.get_data_field_name();

            CompletionItem {
                label: name.clone(),
                kind: Some(CompletionItemKind::FIELD),
                detail: Some(f.type_name),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                    range: {
                        let position = Position::new(
                            position.line,
                            block_mapping_node.start_position().column as u32,
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
        .collect()
}

fn get_type_completion(
    classes: CsharpClasses,
    capture: &QueryCapture,
    key_node: Node,
    position: Position,
    src: &str,
) -> Vec<CompletionItem> {
    let value_node = capture
        .node
        .child_by_field_name("value")
        .map(|node| node.utf8_text(src.as_bytes()).unwrap());

    let lock = tokio::task::block_in_place(|| classes.blocking_read());
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
                kind: Some(CompletionItemKind::VALUE),
                label_details: Some(CompletionItemLabelDetails {
                    detail: Some("Prototype".to_owned()),
                    ..Default::default()
                }),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                    range: {
                        let position =
                            Position::new(position.line, key_node.end_position().column as u32 + 2);
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

    completions
}
