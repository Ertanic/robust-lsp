pub mod yml;

pub type GotoDefinitionResult = Option<tower_lsp::lsp_types::GotoDefinitionResponse>;

#[async_trait::async_trait]
pub trait GotoDefinition {
    async fn goto_definition(&self) -> GotoDefinitionResult;
}