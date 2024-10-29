use ropey::Rope;
use std::ops::Range;
use stringcase::{camel_case, pascal_case};
use tower_lsp::{
    jsonrpc::Result,
    lsp_types::{
        self, CompletionItem, CompletionItemKind, CompletionItemLabelDetails, CompletionResponse,
        CompletionTextEdit, Position, TextEdit,
    },
};
use tracing::instrument;
use tree_sitter::{Parser, Point, Query, QueryCursor};

use crate::{backend::CsharpClasses, parse::csharp::CsharpAttributeArgumentType};

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
        for capture in m.captures {
            let mut has_type = false;
            let block_mapping_node = capture.node.parent().unwrap().parent().unwrap();
            let mut cursor = block_mapping_node.walk();
            for block_mapping_pair_node in block_mapping_node.named_children(&mut cursor) {
                if let Some(key) = block_mapping_pair_node.child_by_field_name("key") {
                    let key = key.utf8_text(src.as_bytes()).unwrap().to_owned();

                    match (has_type, key.as_str()) {
                        (false, "type") => has_type = true,
                        (true, "type") => return Ok(None),
                        _ => {}
                    }
                }
            }

            let lock = classes.read().unwrap();
            let completions = lock
                .iter()
                .filter(|c| {
                    c.base.iter().any(|b| b == "IPrototype")
                        && c.attributes.iter().any(|a| a.name == "Prototype")
                })
                .map(|c| {
                    let name = {
                        let mut name = None;

                        if let Some(attr) = c.attributes.iter().find(|a| a.name == "Prototype") {
                            if let Some(type_name) = attr.arguments.get("type") {
                                if let CsharpAttributeArgumentType::String(type_name) =
                                    &type_name.value
                                {
                                    let type_name = pascal_case(type_name.trim_matches('\"'));

                                    name = Some(type_name);
                                }
                            }
                        }

                        match name {
                            Some(name) if name.ends_with("Prototype") => {
                                name.strip_suffix("Prototype").unwrap().to_owned()
                            }
                            Some(name) => name,
                            None if c.name.ends_with("Prototype") => {
                                c.name.strip_suffix("Prototype").unwrap().to_owned()
                            }
                            None => c.name.clone(),
                        }
                    };

                    let get_col = || capture.node.end_position().column as u32 + 2;
                    CompletionItem {
                        label: name.to_owned(),
                        kind: Some(CompletionItemKind::VALUE),
                        label_details: Some(CompletionItemLabelDetails {
                            detail: Some("Prototype".to_owned()),
                            ..Default::default()
                        }),

                        text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                            range: {
                                let position = Position::new(position.line, get_col());
                                lsp_types::Range {
                                    start: position,
                                    end: position,
                                }
                            },

                            // VS Code ignores coordinates for some reason, so it only works via space.
                            new_text: if position.character < get_col() {
                                String::from(" ") + camel_case(name.as_str()).as_str()
                            } else {
                                camel_case(name.as_str())
                            },
                        })),
                        
                        preselect: if name.to_lowercase() == "entity" {
                            Some(true)
                        } else {
                            None
                        },
                        ..Default::default()
                    }
                })
                .collect::<Vec<_>>();

            return Ok(Some(CompletionResponse::Array(completions)));
        }
    }

    Ok(None)
}
