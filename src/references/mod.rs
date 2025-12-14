use tower_lsp::lsp_types::Location;

pub mod csharp;

pub type GetReferencesResult = Option<Vec<Location>>;

#[async_trait::async_trait]
pub trait ReferencesProvider {
    async fn get_references(&self) -> Option<Vec<Location>>;
}
