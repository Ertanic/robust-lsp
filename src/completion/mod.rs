pub mod yml;

pub(self) type CompletionResult = Option<tower_lsp::lsp_types::CompletionResponse>;

pub trait Completion {
    fn completion(&self) -> CompletionResult;
}