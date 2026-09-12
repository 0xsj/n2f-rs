use std::collections::BTreeMap;
pub fn map(values: BTreeMap<String, String>) -> impl Fn(&str) -> Option<String> {
    move |key| values.get(key).cloned()
}
