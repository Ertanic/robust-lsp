use super::{common::ParseResult, structs::fluent::FluentKey};
use fluent_syntax::ast::{Entry, Expression, InlineExpression, PatternElement};
use std::{collections::HashSet, path::PathBuf};

pub async fn parse(path: PathBuf) -> ParseResult<Vec<FluentKey>> {
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let resource = fluent_syntax::parser::parse(content).or(Err(()))?;

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
                    PatternElement::Placeable { expression } => Some(expression),
                    _ => None,
                })
                .filter_map(|expr| match expr {
                    // TODO: Get variables from functions calls
                    Expression::Inline(InlineExpression::VariableReference { id }) => Some(id.name),
                    Expression::Select {
                        selector: InlineExpression::VariableReference { id },
                        ..
                    } => Some(id.name),
                    _ => None,
                })
                .collect::<HashSet<_>>();

            FluentKey {
                key: msg.id.name.to_string(),
                args,
            }
        })
        .collect();

    Ok(keys)
}
