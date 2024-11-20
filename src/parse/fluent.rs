use super::{common::DefinitionIndex, structs::fluent::FluentKey, ParsedFiles, Result};
use crate::parse::ParseResult;
use fluent_syntax::ast::{Entry, Expression, InlineExpression, PatternElement};
use futures::{
    future::{ready, BoxFuture},
    FutureExt,
};
use rayon::join;
use std::{collections::HashSet, path::PathBuf, sync::Arc};

pub fn dispatch(
    result: ParseResult,
    context: Arc<crate::backend::Context>,
) -> BoxFuture<'static, ()> {
    let ParseResult::Fluent(keys) = result else {
        tracing::warn!("Failed to parse Fluent prototypes.");
        return ready(()).boxed();
    };

    Box::pin(async move {
        context.locales.write().await.extend(keys);
    })
}

pub(crate) fn parse(
    path: PathBuf,
    _parsed_files: ParsedFiles,
) -> BoxFuture<'static, Result<ParseResult>> {
    Box::pin(async move { p(path, ParsedFiles::default()).await })
}

async fn p(path: PathBuf, _parsed_files: ParsedFiles) -> Result<ParseResult> {
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

            let range = span_to_range(&content, &msg.id.span);
            let index = DefinitionIndex(path.clone(), Some(range));

            FluentKey::new(msg.id.name.to_string(), args, index)
        })
        .collect();

    Ok(ParseResult::Fluent(keys))
}

fn span_to_range(src: &str, span: &fluent_syntax::ast::Span) -> tree_sitter::Range {
    let lines = std::iter::once(0)
        .chain(
            src.char_indices()
                .filter_map(|(i, c)| Some(i + 1).filter(|_| c == '\n')),
        )
        .collect::<Vec<_>>();

    let (start_point, end_point) = join(
        || get_point(&lines, span.start),
        || get_point(&lines, span.end),
    );

    tree_sitter::Range {
        start_byte: span.start,
        end_byte: span.end,
        start_point,
        end_point,
    }
}

fn get_point(lines: &Vec<usize>, index: usize) -> tree_sitter::Point {
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
