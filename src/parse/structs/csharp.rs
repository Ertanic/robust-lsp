#![allow(unused)]

use super::*;
use crate::backend::CsharpObjects;
use common::{DefinitionIndex, Index};
use std::sync::Arc;
use tree_sitter::Range;

pub struct ReflectionManager {
    classes: CsharpObjects,
}

impl ReflectionManager {
    pub fn new(classes: CsharpObjects) -> Self {
        Self { classes }
    }

    pub async fn get_fields(&self, class: Arc<CsharpObject>) -> Vec<CsharpClassField> {
        let lock = self.classes.read().await;
        let bases = class
            .base
            .iter()
            .map(|b| lock.par_iter().find_any(|c| c.name == *b))
            .filter_map(|c| c)
            .chain([&class])
            .collect::<Vec<_>>();

        let mut fields = Vec::with_capacity(bases.len());
        for base in bases {
            fields.extend(base.fields.clone());
        }

        fields
    }

    pub async fn get_prototype_by_name(&self, name: impl AsRef<str>) -> Option<Arc<Prototype>> {
        let name = name.as_ref();
        let normalized_name = stringcase::pascal_case(name);

        let lock = self.classes.read().await;
        let class = lock.par_iter().find_any(|c| {
            c.name == normalized_name || c.name == format!("{normalized_name}Prototype") || {
                if let Some(attr) = c.attributes.get("Prototype") {
                    let arg = attr.arguments.get("type");
                    if let Some(arg) = arg {
                        match arg.value {
                            CsharpAttributeArgumentType::String(ref tag) => {
                                tag.trim_matches('"') == name
                            }
                            _ => false,
                        }
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
        });

        Prototype::try_from(Arc::clone(class?)).ok().map(Arc::new)
    }

    pub async fn get_component_by_name(&self, name: impl AsRef<str>) -> Option<Arc<Component>> {
        let name = name.as_ref();

        let lock = self.classes.read().await;
        let class = lock.par_iter().find_any(|c| {
            let name = c.name == name || c.name == format!("{name}Component");
            let attr = c.attributes.contains("RegisterComponent");
            let base = c.base.contains(&String::from("Component"))
                || c.base.contains(&String::from("IComponent"));

            name && attr && base
        });

        Component::try_from(Arc::clone(class?)).ok().map(Arc::new)
    }
}

#[derive(Default, Clone, Debug)]
pub struct CsharpAttributeCollection {
    pub attributes: Vec<CsharpAttribute>,
}

impl FromIterator<CsharpAttribute> for CsharpAttributeCollection {
    fn from_iter<T: IntoIterator<Item = CsharpAttribute>>(iter: T) -> Self {
        Self {
            attributes: iter.into_iter().collect(),
        }
    }
}

impl CsharpAttributeCollection {
    pub fn new() -> Self {
        Self { attributes: vec![] }
    }

    pub fn push(&mut self, attr: CsharpAttribute) {
        self.attributes.push(attr);
    }

    pub fn get(&self, name: impl AsRef<str> + Sync) -> Option<&CsharpAttribute> {
        self.attributes
            .par_iter()
            .find_any(|attr| attr.name == name.as_ref())
    }

    pub fn get_mut(&mut self, name: impl AsRef<str> + Sync) -> Option<&mut CsharpAttribute> {
        self.attributes
            .par_iter_mut()
            .find_any(|attr| attr.name == name.as_ref())
    }

    pub fn contains(&self, name: impl AsRef<str> + Sync) -> bool {
        self.attributes
            .par_iter()
            .any(|attr| attr.name == name.as_ref())
    }

    pub fn len(&self) -> usize {
        self.attributes.len()
    }
}

impl Iterator for CsharpAttributeCollection {
    type Item = CsharpAttribute;
    fn next(&mut self) -> Option<Self::Item> {
        self.attributes.pop()
    }
}

impl Extend<CsharpAttribute> for CsharpAttributeCollection {
    fn extend<T: IntoIterator<Item = CsharpAttribute>>(&mut self, iter: T) {
        self.attributes.extend(iter)
    }
}

pub struct Component {
    class: Arc<CsharpObject>,
}

impl Component {
    pub fn get_component_name(&self) -> String {
        let name = self
            .class
            .name
            .strip_suffix("Component")
            .unwrap_or(self.class.name.as_str());

        stringcase::pascal_case(name)
    }
}

impl TryFrom<Arc<CsharpObject>> for Component {
    type Error = ();

    fn try_from(class: Arc<CsharpObject>) -> Result<Self, Self::Error> {
        if class.attributes.contains("RegisterComponent")
            && class.base.contains(&"Component".to_owned())
            || class.base.contains(&"IComponent".to_owned())
        {
            Ok(Component { class })
        } else {
            Err(())
        }
    }
}

impl Deref for Component {
    type Target = Arc<CsharpObject>;

    fn deref(&self) -> &Self::Target {
        &self.class
    }
}

#[derive(Debug)]
pub struct Prototype {
    class: Arc<CsharpObject>,
}

impl Prototype {
    pub fn get_prototype_name(&self) -> String {
        let mut name = None;

        if let Some(attr) = self.class.attributes.get("Prototype") {
            if let Some(type_name) = attr.arguments.get("type") {
                if let CsharpAttributeArgumentType::String(type_name) = &type_name.value {
                    let type_name = stringcase::pascal_case(type_name.trim_matches('\"'));
                    name = Some(type_name);
                }
            }
        }

        if name.is_none() {
            name = Some(self.class.name.clone());
        }

        let name = name.unwrap();
        if name.ends_with("Prototype") {
            name.strip_suffix("Prototype").unwrap().to_owned()
        } else {
            name
        }
    }
}

impl TryFrom<Arc<CsharpObject>> for Prototype {
    type Error = ();

    fn try_from(class: Arc<CsharpObject>) -> Result<Self, Self::Error> {
        if class.base.contains(&"IPrototype".into()) && class.attributes.contains("Prototype") {
            Ok(Self { class })
        } else {
            Err(())
        }
    }
}

impl Deref for Prototype {
    type Target = Arc<CsharpObject>;

    fn deref(&self) -> &Self::Target {
        &self.class
    }
}

#[derive(Default, Clone, Debug)]
pub struct CsharpObject {
    pub name: String,
    pub base: Vec<String>,
    pub attributes: CsharpAttributeCollection,
    pub fields: Vec<CsharpClassField>,
    pub modifiers: HashSet<String>,

    index: DefinitionIndex,
}

impl CsharpObject {
    pub fn new(
        name: String,
        base: Vec<String>,
        attributes: CsharpAttributeCollection,
        fields: Vec<CsharpClassField>,
        modifiers: HashSet<String>,
        index: DefinitionIndex,
    ) -> Self {
        Self {
            name,
            base,
            attributes,
            fields,
            modifiers,
            index,
        }
    }

    pub fn set_file(&mut self, file: PathBuf) {
        self.index.0 = file;
    }
}

impl From<&str> for CsharpObject {
    fn from(value: &str) -> Self {
        Self {
            name: value.to_string(),
            ..Default::default()
        }
    }
}

impl PartialEq for CsharpObject {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Eq for CsharpObject {}

impl Hash for CsharpObject {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.name.hash(state);
    }
}

impl Index for CsharpObject {
    fn index(&self) -> &DefinitionIndex {
        &self.index
    }
}

#[derive(Debug, Default, Clone)]
pub struct CsharpAttribute {
    pub name: String,
    pub arguments: HashMap<String, CsharpAttributeArgument>,
}

#[derive(Debug, Default, Clone)]
pub struct CsharpAttributeArgument {
    pub index: usize,
    pub name: String,
    pub value: CsharpAttributeArgumentType,
}

#[derive(Debug, Default, Clone)]
pub enum CsharpAttributeArgumentType {
    #[default]
    None,
    String(String),
    Bool(bool),
    Real(f64),
    Int(i64),

    TypeOf(Box<CsharpAttributeArgumentType>),
    GenericType {
        indent: String,
        types: Vec<CsharpAttributeArgumentType>,
    },
}

#[derive(Debug, Default, Clone)]
pub struct CsharpClassField {
    pub name: String,
    pub type_name: String,
    pub attributes: CsharpAttributeCollection,
    pub modifiers: HashSet<String>,

    index: DefinitionIndex,
}

impl CsharpClassField {
    pub fn new(
        name: String,
        type_name: String,
        attributes: CsharpAttributeCollection,
        modifiers: HashSet<String>,
        index: DefinitionIndex,
    ) -> Self {
        Self {
            name,
            type_name,
            attributes,
            modifiers,
            index,
        }
    }

    pub fn new_empty<T: ToString>(name: T, type_name: T) -> Self {
        Self {
            name: name.to_string(),
            type_name: type_name.to_string(),
            ..Default::default()
        }
    }

    pub fn get_data_field_name(&self) -> String {
        if let Some(attr) = self.attributes.get("DataField") {
            if let Some(name) = attr.arguments.get("tag") {
                if let CsharpAttributeArgumentType::String(ref name) = name.value {
                    return name.trim_matches('"').to_owned();
                }
            }
        } else if self.attributes.contains("IncludeDataField") {
            match self.type_name.trim_end_matches('?') {
                "SpriteSpecifier.Rsi" | "SpriteSpecifier" => return "sprite".to_owned(),
                _ => {}
            }
        }

        stringcase::camel_case(&self.name)
    }
}

impl Index for CsharpClassField {
    fn index(&self) -> &DefinitionIndex {
        &self.index
    }
}
