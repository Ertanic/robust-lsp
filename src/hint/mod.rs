pub mod yaml;

pub trait InlayHint {
    fn inlay_hint(&self) -> Option<Vec<tower_lsp::lsp_types::InlayHint>>;
}