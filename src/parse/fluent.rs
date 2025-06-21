use super::{common::DefinitionIndex, structs::fluent::FluentKey, ParsedFiles};
use crate::parse::ParseResult;
use fluent_syntax::ast::{Entry, Expression, InlineExpression, PatternElement};
use std::{collections::HashSet, path::PathBuf};
use crate::utils::span_to_range;

pub async fn parse(path: PathBuf, _parsed_files: ParsedFiles) -> ParseResult {
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let Ok(resource) = fluent_syntax::parser::parse(content.as_ref()) else {
        return ParseResult::None;
    };

    let keys = resource
        .body
        .into_iter()
        .filter_map(|entry| match entry {
            Entry::Message(msg) if msg.value.is_some() => Some(msg),
            _ => None,
        })
        .map(|msg| {
            let args = msg
                .value
                .unwrap()
                .elements
                .into_iter()
                .filter_map(|v| match v {
                    PatternElement::Placeable { expression, .. } => Some(expression),
                    _ => None,
                })
                .filter_map(|expr| match expr {
                    // TODO: Get variables from functions calls
                    Expression::Inline(InlineExpression::VariableReference { id, .. }, ..) => {
                        Some(id.name.to_owned())
                    }
                    Expression::Select {
                        selector: InlineExpression::VariableReference { id, .. },
                        ..
                    } => Some(id.name.to_owned()),
                    _ => None,
                })
                .collect::<HashSet<String>>();

            let range = span_to_range(&content, &msg.id.span);
            let index = DefinitionIndex(path.clone(), Some(range));

            FluentKey::new(msg.id.name.to_string(), args, index)
        })
        .collect();

    ParseResult::Fluent(keys)
}
