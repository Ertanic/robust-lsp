use super::{
    index::{DefinitionIndex, IndexPosition, IndexRange},
    structs::fluent::FluentKey,
    ParsedFiles, Result,
};
use crate::parse::ParseResult;
use core::str;
use fluent_syntax::{
    ast::{Entry, Expression, InlineExpression, PatternElement, Span},
    parser::ParserError,
};
use futures::{
    future::{ready, BoxFuture},
    FutureExt,
};
use rayon::join;
use std::{collections::HashSet, path::PathBuf, sync::Arc};
use tower_lsp::{
    lsp_types::{self, CodeDescription, Diagnostic, DiagnosticSeverity, Url},
    Client,
};

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
    _client: Arc<Client>,
) -> BoxFuture<'static, Result<ParseResult>> {
    Box::pin(async move { p(path, ParsedFiles::default(), _client).await })
}

async fn p(path: PathBuf, _parsed_files: ParsedFiles, client: Arc<Client>) -> Result<ParseResult> {
    let content = std::fs::read_to_string(&path).unwrap_or_default();

    // Since the parser works with the string as a byte array,
    // it reads the Unicode BOM as an invalid beginning of the expression, i.e. as a syntax error
    let normalized_content = content
        .trim()
        .trim_start_matches(str::from_utf8(&[239u8, 187u8, 191u8]).unwrap());

    let resource = match fluent_syntax::parser::parse(normalized_content) {
        Ok(resource) => resource,
        Err((resource, errors)) => {
            report_errors(client, &normalized_content, errors, &path).await;
            resource
        }
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
                .collect::<HashSet<_>>();

            let range = span_to_range(&normalized_content, &msg.id.span);
            let index = DefinitionIndex(path.clone(), range);

            FluentKey::new(msg.id.name.to_string(), args, index)
        })
        .collect::<Vec<_>>();

    Ok(ParseResult::Fluent(keys))
}

fn span_to_range(src: &str, span: &Span) -> IndexRange {
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

    IndexRange(start_point, end_point, Some((span.start, span.end)))
}

fn get_point(lines: &Vec<usize>, index: usize) -> IndexPosition {
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

    IndexPosition(line - 1, col - 1)
}

async fn report_errors(
    client: Arc<Client>,
    src: &str,
    errors: Vec<ParserError>,
    path: &std::path::Path,
) {
    let uri = Url::from_file_path(path).expect("Unable to parse the path in uri.");
    let diagnostics = errors
        .into_iter()
        .map(|error| {
            let range = error.slice.unwrap_or(error.pos.clone());
            Diagnostic {
                severity: Some(DiagnosticSeverity::ERROR),
                code: Some(lsp_types::NumberOrString::String(
                    "syntax-error".to_string(),
                )),
                code_description: Some(CodeDescription {
                    href: Url::parse("https://projectfluent.org/fluent/guide/")
                        .expect("Unable to parse url."),
                }),
                message: error.kind.to_string(),
                range: span_to_range(src, &Span::new(range)).into(),
                source: Some(env!("CARGO_BIN_NAME").to_owned()),
                ..Default::default()
            }
        })
        .collect();

    client.publish_diagnostics(uri, diagnostics, None).await;
}
