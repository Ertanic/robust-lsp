use super::{
    common::{DefinitionIndex, ParseFromNode},
    structs::csharp::{
        CsharpAttribute, CsharpAttributeArgument, CsharpAttributeArgumentType,
        CsharpAttributeCollection, CsharpClassField, CsharpObject,
    },
    ParseResult,
};
use crate::backend::ParsedFiles;
use ropey::Rope;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};
use tree_sitter::Node;

static PROTOTYPE_ATTR_ARGS: &[&str] = &["type", "loadPriority"];
static DATA_FIELD_ATTR_ARGS: &[&str] = &[
    "tag",
    "readOnly",
    "priority",
    "required",
    "serverOnly",
    "customTypeSerializer",
];
static ID_DATA_FIELD_ATTR_ARGS: &[&str] = &["priority", "customTypeSerializer"];

type Result<T, E = ()> = std::result::Result<T, E>;

pub async fn parse(path: PathBuf, parsed_files: ParsedFiles) -> ParseResult {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_c_sharp::LANGUAGE.into())
        .expect("Failed to load C# grammer");

    let rope = Rope::from_reader(std::fs::File::open(&path).unwrap()).unwrap();

    let mut lock = parsed_files.write().await;
    let old_tree = lock.get_mut(&path);

    let tree = parser.parse(rope.to_string(), old_tree.as_deref());
    if let Some(tree) = tree {
        if let Some(old_tree) = old_tree {
            *old_tree = tree.clone();
            drop(lock);
        }

        let root_node = tree.root_node();
        let src = Arc::new(rope);
        let mut stack = vec![root_node];

        let mut objects = vec![];

        while let Some(node) = stack.pop() {
            if node.kind() == "class_declaration" || node.kind() == "interface_declaration" {
                let src = src.clone();

                if let Ok(result) = CsharpObject::get(node, src, &path) {
                    objects.push(result);
                }
            }

            for i in 0..node.named_child_count() {
                stack.push(node.named_child(i).unwrap());
            }
        }

        return ParseResult::Csharp(objects);
    }

    ParseResult::None
}

impl ParseFromNode for CsharpObject {
    fn get(node: Node, src: Arc<Rope>, path: &Path) -> Result<Self> {
        let mut cursor = node.walk();

        let mut modifiers = HashSet::new();
        let mut base = vec![];
        let mut attributes = CsharpAttributeCollection::new();
        let mut fields = vec![];
        let mut name = None;
        let mut name_range = None;

        for node in node.named_children(&mut cursor) {
            let source = src.clone().to_string();
            match node.kind() {
                "modifier" => {
                    let modifier = node.utf8_text(source.as_bytes()).unwrap().to_owned();
                    modifiers.insert(modifier);
                }
                "identifier" => {
                    let indent = node.utf8_text(source.as_bytes()).unwrap().to_owned();
                    name = Some(indent);
                    name_range = Some(node.range());
                }
                "base_list" => {
                    let mut cursor = node.walk();
                    for parent_node in node.named_children(&mut cursor) {
                        let parent = parent_node.utf8_text(source.as_bytes()).unwrap().to_owned();
                        base.push(parent);
                    }
                }
                "attribute_list" => {
                    attributes
                        .extend(Vec::<CsharpAttribute>::get(node, src.clone(), path)?.into_iter());
                }
                "declaration_list" => {
                    // class body
                    let mut cursor = node.walk();
                    for node in node.named_children(&mut cursor) {
                        match node.kind() {
                            "field_declaration" | "property_declaration" => {
                                if let Ok(field) = CsharpClassField::get(node, src.clone(), path) {
                                    fields.push(field);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }

        match name {
            Some(name) => Ok(CsharpObject::new(
                name,
                base,
                attributes,
                fields,
                modifiers,
                DefinitionIndex(path.to_path_buf(), name_range),
            )),
            _ => Err(()),
        }
    }
}

impl ParseFromNode for CsharpClassField {
    fn get(node: Node, src: Arc<Rope>, path: &Path) -> Result<Self> {
        let mut cursor = node.walk();
        let source = src.clone().to_string();

        let mut modifiers = HashSet::new();
        let mut attributes = CsharpAttributeCollection::new();
        let mut type_name = None;
        let mut field_name = None;
        let mut name_range = None;

        if node.kind() == "field_declaration" {
            for node in node.named_children(&mut cursor) {
                match node.kind() {
                    "attribute_list" => attributes
                        .extend(Vec::<CsharpAttribute>::get(node, src.clone(), path)?.into_iter()),
                    "modifier" => {
                        let modifier = node.utf8_text(source.as_bytes()).unwrap().to_owned();
                        modifiers.insert(modifier);
                    }
                    "variable_declaration" => {
                        let type_node = node.child_by_field_name("type");
                        if let Some(type_node) = type_node {
                            type_name =
                                Some(type_node.utf8_text(source.as_bytes()).unwrap().to_owned());
                            let declarator_node = type_node.next_named_sibling();
                            if let Some(declarator_node) = declarator_node {
                                let name_node = declarator_node.child_by_field_name("name");
                                if let Some(name_node) = name_node {
                                    field_name = Some(
                                        name_node.utf8_text(source.as_bytes()).unwrap().to_owned(),
                                    );
                                    name_range = Some(name_node.range());
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        } else if node.kind() == "property_declaration" {
            match (
                node.child_by_field_name("type"),
                node.child_by_field_name("name"),
            ) {
                (Some(type_node), Some(name_node)) => {
                    field_name = Some(name_node.utf8_text(source.as_bytes()).unwrap().to_owned());
                    name_range = Some(name_node.range());
                    type_name = Some(type_node.utf8_text(source.as_bytes()).unwrap().to_owned());
                }
                _ => return Err(()),
            }

            for prop_node in node.named_children(&mut cursor) {
                match prop_node.kind() {
                    "attribute_list" => attributes.extend(
                        Vec::<CsharpAttribute>::get(prop_node, src.clone(), path)?.into_iter(),
                    ),
                    "modifier" => {
                        let modifier = prop_node.utf8_text(source.as_bytes()).unwrap().to_owned();
                        modifiers.insert(modifier);
                    }
                    _ => {}
                }
            }
        }

        match (field_name, type_name) {
            (Some(field_name), Some(type_name)) => Ok(CsharpClassField::new(
                field_name,
                type_name,
                attributes,
                modifiers,
                DefinitionIndex(path.to_path_buf(), name_range),
            )),
            _ => Err(()),
        }
    }
}

impl ParseFromNode for Vec<CsharpAttribute> {
    fn get(node: Node, src: Arc<Rope>, _path: &Path) -> Result<Self> {
        let mut cursor = node.walk();

        let mut attributes = vec![];
        let src = src.to_string();

        for node in node.named_children(&mut cursor) {
            // attributes traversal
            let mut cursor = node.walk();

            let mut attr_name = None;
            let mut args = HashMap::new();

            for attr_within_node in node.named_children(&mut cursor) {
                // attribute name and argument_list traversal
                let mut cursor = attr_within_node.walk();

                match attr_within_node.kind() {
                    "identifier" => {
                        let name = attr_within_node
                            .utf8_text(src.as_bytes())
                            .unwrap()
                            .to_owned();

                        attr_name =
                            Some(if let Some(normalized) = name.strip_suffix("Attribute") {
                                normalized.to_owned()
                            } else {
                                name
                            });
                    }
                    "attribute_argument_list" => {
                        let mut arg_index = 0;

                        for node in attr_within_node.named_children(&mut cursor) {
                            // attribute argument traversal
                            let mut cursor = node.walk();

                            let mut arg_name = None;
                            let mut arg_value = None;

                            for arg_within_node in node.named_children(&mut cursor) {
                                // attribute argument name and value traversal
                                let mut cursor = arg_within_node.walk();

                                match arg_within_node.kind() {
                                    "identifier" => {
                                        let name = arg_within_node
                                            .utf8_text(src.as_bytes())
                                            .unwrap()
                                            .to_owned();

                                        arg_name = Some(
                                            if arg_within_node.next_sibling().is_none()
                                                && name == "ProtoName"
                                            {
                                                "audioMetadata".to_owned()
                                            } else {
                                                name
                                            },
                                        );
                                    }
                                    "string_literal" => {
                                        if cursor.goto_first_child() {
                                            // string_literal_content
                                            let value = arg_within_node
                                                .utf8_text(src.as_bytes())
                                                .unwrap()
                                                .to_owned();
                                            arg_value =
                                                Some(CsharpAttributeArgumentType::String(value));

                                            cursor.goto_first_child();
                                        }
                                    }
                                    "boolean_literal" => {
                                        let value = arg_within_node
                                            .utf8_text(src.as_bytes())
                                            .unwrap()
                                            .to_owned();
                                        arg_value = Some(CsharpAttributeArgumentType::Bool(
                                            value.parse().unwrap(),
                                        ));
                                    }
                                    "real_literal" => {
                                        let value = arg_within_node
                                            .utf8_text(src.as_bytes())
                                            .unwrap()
                                            .to_owned();
                                        arg_value = Some(CsharpAttributeArgumentType::Real(
                                            value.parse().unwrap(),
                                        ));
                                    }
                                    "integer_literal" => {
                                        let value = arg_within_node
                                            .utf8_text(src.as_bytes())
                                            .unwrap()
                                            .to_owned();
                                        arg_value = Some(CsharpAttributeArgumentType::Int(
                                            value.parse().unwrap(),
                                        ));
                                    }
                                    "prefix_unary_expression" => {
                                        let unary_val_node = cursor.node();

                                        if cursor.goto_first_child() {
                                            // prefix_unary_operator
                                            match cursor.node().kind() {
                                                "integer_literal" => {
                                                    let value = unary_val_node
                                                        .utf8_text(src.as_bytes())
                                                        .unwrap()
                                                        .to_owned();
                                                    arg_value =
                                                        Some(CsharpAttributeArgumentType::Int(
                                                            value.parse().unwrap(),
                                                        ));
                                                }
                                                "real_literal" => {
                                                    let value = unary_val_node
                                                        .utf8_text(src.as_bytes())
                                                        .unwrap()
                                                        .to_owned();
                                                    arg_value =
                                                        Some(CsharpAttributeArgumentType::Real(
                                                            value.parse().unwrap(),
                                                        ));
                                                }
                                                _ => {}
                                            }

                                            cursor.goto_parent();
                                        }
                                    }
                                    "typeof_expression" => {
                                        let mut cursor = arg_within_node.walk();
                                        for node in arg_within_node.named_children(&mut cursor) {
                                            match node.kind() {
                                                "identifier" => {
                                                    let value = node
                                                        .utf8_text(src.as_bytes())
                                                        .unwrap()
                                                        .to_owned();
                                                    arg_value =
                                                        Some(CsharpAttributeArgumentType::TypeOf(
                                                            Box::new(
                                                                CsharpAttributeArgumentType::String(
                                                                    value,
                                                                ),
                                                            ),
                                                        ));
                                                }
                                                "generic_name" => {
                                                    // TODO
                                                }
                                                _ => {}
                                            }
                                        }
                                    }
                                    _ => {}
                                }

                                cursor.goto_parent();
                            }

                            if let Some(arg_value) = arg_value {
                                let name = if let Some(arg_name1) = arg_name {
                                    // an argument of attribute may contain a name
                                    arg_name1
                                } else if attr_name.is_some() {
                                    // if an attribute name has been found
                                    let attr_name = attr_name.clone().unwrap();
                                    match attr_name.as_str() {
                                        // check if an argument is valid
                                        "Prototype" => {
                                            // otherwise skip it
                                            if PROTOTYPE_ATTR_ARGS.len() > arg_index
                                                && !args
                                                    .contains_key(PROTOTYPE_ATTR_ARGS[arg_index])
                                            {
                                                PROTOTYPE_ATTR_ARGS[arg_index].to_owned()
                                            } else {
                                                cursor.goto_parent();
                                                continue;
                                            }
                                        }
                                        "DataField" => {
                                            if DATA_FIELD_ATTR_ARGS.len() > arg_index
                                                && !args
                                                    .contains_key(DATA_FIELD_ATTR_ARGS[arg_index])
                                            {
                                                DATA_FIELD_ATTR_ARGS[arg_index].to_owned()
                                            } else {
                                                cursor.goto_parent();
                                                continue;
                                            }
                                        }
                                        "IdDataField" => {
                                            if ID_DATA_FIELD_ATTR_ARGS.len() > arg_index
                                                && !args.contains_key(
                                                    ID_DATA_FIELD_ATTR_ARGS[arg_index],
                                                )
                                            {
                                                ID_DATA_FIELD_ATTR_ARGS[arg_index].to_owned()
                                            } else {
                                                cursor.goto_parent();
                                                continue;
                                            }
                                        }
                                        _ => arg_index.to_string(),
                                    }
                                } else {
                                    arg_index.to_string()
                                };

                                let arg = CsharpAttributeArgument {
                                    index: arg_index,
                                    name: name.clone(),
                                    value: arg_value,
                                };

                                // Just why hashmap? What was I thinking then? it needs to be reworked..
                                args.insert(name, arg);
                            }

                            arg_index += 1;

                            cursor.goto_parent();
                        }
                    }
                    _ => {}
                }

                cursor.goto_parent();
            }

            if let Some(attr_name) = attr_name {
                attributes.push(CsharpAttribute {
                    name: attr_name,
                    arguments: args,
                });
            }

            cursor.goto_parent();
        }

        Ok(attributes)
    }
}
