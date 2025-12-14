pub mod yaml;

#[async_trait::async_trait]
pub trait InlayHint {
    async fn inlay_hint(&self) -> Option<Vec<tower_lsp::lsp_types::InlayHint>>;
}