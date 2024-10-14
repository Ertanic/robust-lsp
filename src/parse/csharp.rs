use std::collections::HashMap;

use super::common::{DefinitionIndex, Index};

#[derive(Default, Clone)]
pub struct CsharpClass {
    pub name: String,
    pub base: Vec<String>,
    pub attributes: Vec<CsharpAttribute>,
    pub fields: Vec<CsharpClassField>,

    index: DefinitionIndex,
}

impl Index for CsharpClass {
    fn index(&self) -> &super::common::DefinitionIndex {
        &self.index
    }
}

#[derive(Debug, Default, Clone)]
pub struct CsharpAttribute {
    pub name: String,
    pub arguments: HashMap<String, CsharpAttributeArgumentType>,
}

#[derive(Debug, Default, Clone)]
pub enum CsharpAttributeArgumentType {
    #[default]
    None,
    String(String),
    Bool(bool),
}

#[derive(Debug, Default, Clone)]
pub struct CsharpClassField {
    pub name: String,
    pub type_name: String,
    pub attributes: Vec<CsharpAttribute>,
}