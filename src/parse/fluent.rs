use super::{
    common::{DefinitionIndex, ParseResult},
    structs::fluent::FluentKey,
};
use fluent_syntax::ast::{Entry, Expression, InlineExpression, PatternElement};
use std::{collections::HashSet, path::PathBuf};

pub async fn parse(path: PathBuf) -> ParseResult<Vec<FluentKey>> {
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let resource = fluent_syntax::parser::parse(content.as_ref()).or(Err(()))?;

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
                .collect::<HashSet<_>>();

            if msg.id.name.eq("wanted-list-history-table-reason-col") {
                tracing::trace!("span: {:#?}", msg.id.span);
            }

            let range = span_to_range(&content, &msg.id.span);
            let index = DefinitionIndex(path.clone(), Some(range));

            FluentKey::new(msg.id.name.to_string(), args, index)
        })
        .collect();

    Ok(keys)
}

fn span_to_range(src: &str, span: &fluent_syntax::ast::Span) -> tree_sitter::Range {
    let lines = std::iter::once(0)
        .chain(
            src.char_indices()
                .filter_map(|(i, c)| Some(i + 1).filter(|_| c == '\n')),
        )
        .collect::<Vec<_>>();

    let start_point = get_point(lines.clone(), span.start);
    let end_point = get_point(lines.clone(), span.end);

    tree_sitter::Range {
        start_byte: span.start,
        end_byte: span.end,
        start_point,
        end_point,
    }
}

fn get_point(lines: Vec<usize>, index: usize) -> tree_sitter::Point {
    let mut line_range = 0..lines.len();
    while line_range.end - line_range.start > 1 {
        let range_middle = line_range.start + (line_range.end - line_range.start) / 2;
        let (left, right) = (line_range.start..range_middle, range_middle..line_range.end);
        if (lines[left.start]..lines[left.end]).contains(&index) {
            line_range = left;
        } else {
            line_range = right;
        }
    }

    let line_start_index = lines[line_range.start];
    let line = line_range.start + 1;
    let col = index - line_start_index + 1;

    tree_sitter::Point {
        row: line - 1,
        column: col - 1,
    }
}
