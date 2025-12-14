use super::{common::DefinitionIndex, structs::fluent::FluentKey};
use crate::{
    cache::{CacheContent, CacheContext, CacheKey, ProjectCache},
    parse::ParseResult,
    utils::{read_file, span_to_range, FileContent},
};
use fluent_syntax::ast::{Entry, Expression, InlineExpression, PatternElement};
use std::{collections::HashSet, path::PathBuf, sync::Arc};
use tokio::sync::RwLock;

pub async fn parse(path: PathBuf, cache: Arc<RwLock<ProjectCache>>) -> ParseResult {
    let FileContent { hash, content } = match read_file(&path).await {
        Some(content) => content,
        None => return ParseResult::None,
    };

    let key = CacheKey::new(hash);
    if let CacheContent::Fluent(cache) = cache.read().await.get(&key, CacheContext::Fluent) {
        return ParseResult::Fluent(cache.clone());
    }

    let content = Arc::new(content);
    let Ok(resource) = fluent_syntax::parser::parse(&**content)
    else {
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
                    Expression::Inline(InlineExpression::VariableReference { id, .. }, ..) => Some(id.name.to_owned()),
                    Expression::Select {
                        selector: InlineExpression::VariableReference { id, .. },
                        ..
                    } => Some(id.name.to_owned()),
                    _ => None,
                })
                .collect::<HashSet<String>>();

            let range = span_to_range(&content, &msg.id.span);
            let index = DefinitionIndex(path.clone(), Some(range.into()));

            FluentKey::new(msg.id.name.to_string(), args, index)
        })
        .collect();

    cache.write().await.insert(key, CacheContent::Fluent(&keys));

    ParseResult::Fluent(keys)
}
