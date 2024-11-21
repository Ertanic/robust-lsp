use super::{common::DefinitionIndex, structs::yaml::YamlPrototype, ParsedFiles, Result};
use crate::parse::ParseResult;
use futures::{
    future::{ready, BoxFuture},
    FutureExt,
};
use ropey::Rope;
use std::{path::PathBuf, sync::Arc};
use tree_sitter::Node;

pub fn dispatch(
    result: ParseResult,
    context: Arc<crate::backend::Context>,
) -> BoxFuture<'static, ()> {
    let ParseResult::YamlPrototypes(protos) = result else {
        tracing::warn!("Failed to parse YAML prototypes.");
        return ready(()).boxed();
    };

    Box::pin(async move {
        context.prototypes.write().await.extend(protos);
    })
}

pub(crate) fn parse(
    path: PathBuf,
    _parsed_files: ParsedFiles,
) -> BoxFuture<'static, Result<ParseResult>> {
    Box::pin(async move { p(path, _parsed_files).await })
}

async fn p(path: PathBuf, parsed_files: ParsedFiles) -> Result<ParseResult> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_yaml::language())
        .expect("Failed to load YAML grammer");

    let rope = Rope::from_reader(std::fs::File::open(&path).unwrap()).unwrap();

    let mut lock = parsed_files.write().await;
    let old_tree = lock.get_mut(&path);

    let src = rope.to_string();

    let tree = parser.parse(&src, old_tree.as_deref());
    if let Some(tree) = tree {
        if let Some(old_tree) = old_tree {
            *old_tree = tree.clone();
            drop(lock);
        }

        let root_node = tree.root_node();
        if let Some(block_sequence_node) = get_block_sequence_node(&root_node) {
            if block_sequence_node.kind() != "block_sequence" {
                return Err(());
            }

            let mut protos = vec![];
            for i in 0..block_sequence_node.named_child_count() {
                let block_sequence_item_node = block_sequence_node.named_child(i).unwrap();
                if let Some(prototype) = get_yaml_prototype(block_sequence_item_node, &src, &path) {
                    protos.push(prototype);
                }
            }
            return Ok(ParseResult::YamlPrototypes(protos));
        }
    }

    Err(())
}

fn get_yaml_prototype(
    block_sequence_item_node: Node,
    src: &str,
    path: &PathBuf,
) -> Option<YamlPrototype> {
    if let Some(block_mapping_node) = get_block_mapping(block_sequence_item_node) {
        let mut prototype = None;
        let mut id = None;
        let mut id_range = None;
        let mut parents = vec![];

        for i in 0..block_mapping_node.named_child_count() {
            let mapping_pair_node = block_mapping_node.named_child(i).unwrap();

            let key_node = match mapping_pair_node.child_by_field_name("key") {
                Some(n) => n,
                None => continue,
            };
            let key_name = key_node.utf8_text(src.as_bytes()).unwrap();

            let value_node = match mapping_pair_node.child_by_field_name("value") {
                Some(n) => n,
                None => continue,
            };

            match key_name {
                "type" => {
                    prototype = Some(value_node.utf8_text(src.as_bytes()).unwrap().to_owned())
                }
                "id" => {
                    id = Some(value_node.utf8_text(src.as_bytes()).unwrap().to_owned());
                    id_range = Some(value_node.range());
                }
                "parent" => match value_node.kind() {
                    "block_node" | "flow_node" => {
                        let sequence_node = match value_node.named_child(0) {
                            Some(n) => n,
                            None => continue,
                        };

                        match sequence_node.kind() {
                            "flow_sequence" | "block_sequence" => {
                                for i in 0..sequence_node.named_child_count() {
                                    let sequence_item_node = sequence_node.named_child(i).unwrap();
                                    match sequence_item_node.named_child(0) {
                                        Some(content_node) => parents.push(
                                            content_node
                                                .utf8_text(src.as_bytes())
                                                .unwrap()
                                                .to_owned(),
                                        ),
                                        None => continue,
                                    }
                                }
                            }
                            _ => {
                                parents.push(
                                    sequence_node.utf8_text(src.as_bytes()).unwrap().to_owned(),
                                );
                                continue;
                            }
                        }
                    }
                    _ => continue,
                },
                _ => continue,
            }
        }

        match (prototype, id) {
            (Some(prototype), Some(id)) => {
                return Some(YamlPrototype::new(
                    prototype,
                    id,
                    DefinitionIndex(path.clone(), id_range),
                ))
            }
            _ => return None,
        }
    }

    None
}

fn get_block_sequence_node<'a>(root_node: &'a Node<'a>) -> Option<Node<'a>> {
    let document = find_child_node(*root_node, "document")?;
    let block_node = find_child_node(document, "block_node")?;
    find_child_node(block_node, "block_sequence")
}

fn find_child_node<'a>(node: Node<'a>, name: &'a str) -> Option<Node<'a>> {
    let mut n = None;
    for i in 0..node.named_child_count() {
        let document_node = node.named_child(i).unwrap();
        if document_node.kind() == name {
            n = Some(document_node);
            break;
        }
    }
    n
}

fn get_block_mapping<'a>(block_sequence_item_node: Node<'a>) -> Option<Node<'a>> {
    let block_node = block_sequence_item_node.named_child(0)?;
    let block_mapping_node = block_node.named_child(0)?;

    if block_mapping_node.kind() == "block_mapping" {
        Some(block_mapping_node)
    } else {
        None
    }
}
