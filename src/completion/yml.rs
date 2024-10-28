use ropey::Rope;
use std::ops::Range;
use stringcase::camel_case;
use tower_lsp::{
    jsonrpc::Result,
    lsp_types::{CompletionItem, CompletionResponse, Position},
};
use tracing::instrument;
use tree_sitter::{Parser, Point, Query, QueryCursor};

use crate::{backend::CsharpClasses, parse::csharp::CsharpAttributeArgumentType};

#[instrument(skip(rope))]
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
                        && c.attributes
                            .iter()
                            .any(|a| a.name == "Prototype" || a.name == "PrototypeAttribute")
                })
                .map(|c| {
                    let name = {
                        let mut name= None;
                        
                        if let Some(attr) = c.attributes.iter().find(|a| a.name == "Prototype" || a.name == "PrototypeAttribute") {
                            if let Some(type_name) = attr.arguments.get("type") {
                                if let CsharpAttributeArgumentType::String(type_name) = &type_name.value {
                                    name = Some(type_name);
                                }
                            }
                        }

                        let name = match name {
                            Some(name) if name.ends_with("Prototype") => name.strip_suffix("Prototype").unwrap(),
                            Some(name) => name.as_str(),
                            None => c.name.as_str(),
                        };

                        camel_case(name)
                    };

                    CompletionItem::new_simple(name, "prototype".to_owned())
                })
                .collect::<Vec<_>>();

            return Ok(Some(CompletionResponse::Array(completions)));
        }
    }

    Ok(None)
}
