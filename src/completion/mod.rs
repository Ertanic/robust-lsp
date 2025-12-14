pub mod yml;

pub type CompletionResult = Option<tower_lsp::lsp_types::CompletionResponse>;

#[async_trait::async_trait]
pub trait Completion {
    async fn completion(&self) -> CompletionResult;
}