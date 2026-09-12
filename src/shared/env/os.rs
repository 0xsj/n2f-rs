use crate::shared::errors::{Failure, Kind};
use std::collections::BTreeMap;
pub fn os() -> Result<impl Fn(&str) -> Option<String>, Failure> {
    let mut values = BTreeMap::new();
    for (k, v) in std::env::vars_os() {
        let k = k.into_string().map_err(|_| source_error())?;
        let v = v.into_string().map_err(|_| source_error())?;
        values.insert(k, v);
    }
    Ok(super::map(values))
}
fn source_error() -> Failure {
    Failure::new(Kind::Invalid, "invalid environment source").with_type("env.source")
}
