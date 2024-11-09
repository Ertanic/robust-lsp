pub mod yml;

pub type GotoDefinitionResult = Option<tower_lsp::lsp_types::GotoDefinitionResponse>;

pub trait GotoDefinition {
    fn goto_definition(&self) -> GotoDefinitionResult;
}