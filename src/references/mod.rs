use tower_lsp::lsp_types::Location;

pub mod csharp;

pub type GetReferencesResult = Option<Vec<Location>>;

pub trait ReferencesProvider {
    fn get_references(&self) -> Option<Vec<Location>>;
}