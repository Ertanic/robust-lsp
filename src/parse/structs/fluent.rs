#[derive(Debug, PartialEq, Eq, Clone, Default)]
pub struct FluentKey {
    pub key: String,
    pub args: std::collections::HashSet<String>,
}

impl Ord for FluentKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key.cmp(&other.key)
    }
}

impl PartialOrd for FluentKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.key.partial_cmp(&other.key)
    }
}

impl std::hash::Hash for FluentKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.key.hash(state);
    }
}
