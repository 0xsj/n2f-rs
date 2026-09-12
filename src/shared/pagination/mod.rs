//! Query-bound opaque cursors and bounded pages. Encoding is not authorization.
use crate::shared::{
    errors::{Failure, Kind},
    validation,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
fn invalid() -> Failure {
    Failure::new(Kind::Invalid, "invalid pagination").with_type("pagination.invalid")
}
pub fn size(raw: Option<&str>) -> Result<u32, Failure> {
    raw.map_or(Ok(25), |v| validation::decimal(v, 1, 100))
}
fn valid_part(v: &str, max: usize) -> bool {
    !v.is_empty() && v.len() <= max && !v.chars().any(|c| c < '\u{20}' || c == '\u{7f}')
}
pub fn encode(scope: &str, position: &str) -> Result<String, Failure> {
    if !valid_part(scope, 128) || !valid_part(position, 256) {
        return Err(invalid());
    }
    Ok(URL_SAFE_NO_PAD.encode(serde_json::json!([1, scope, position]).to_string()))
}
pub fn decode(wire: &str, scope: &str) -> Result<String, Failure> {
    if wire.is_empty() || wire.len() > 1024 || !valid_part(scope, 128) {
        return Err(invalid());
    }
    let bytes = URL_SAFE_NO_PAD.decode(wire).map_err(|_| invalid())?;
    if URL_SAFE_NO_PAD.encode(&bytes) != wire {
        return Err(invalid());
    }
    let v: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    let a = v.as_array().ok_or_else(invalid)?;
    if a.len() != 3 || a[0].as_f64() != Some(1.0) || a[1].as_str() != Some(scope) {
        return Err(invalid());
    }
    let position = a[2].as_str().ok_or_else(invalid)?;
    if !valid_part(position, 256) {
        return Err(invalid());
    }
    Ok(position.to_owned())
}
#[derive(Debug)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub has_more: bool,
    pub next_cursor: Option<String>,
}
pub fn window<T>(
    mut rows: Vec<T>,
    limit: u32,
    scope: &str,
    position: impl FnOnce(&T) -> String,
) -> Result<Page<T>, Failure> {
    if !(1..=100).contains(&limit) || rows.len() > limit as usize + 1 {
        return Err(invalid());
    }
    let has_more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next_cursor = if has_more {
        Some(encode(
            scope,
            &position(rows.last().expect("nonempty bounded page")),
        )?)
    } else {
        None
    };
    Ok(Page {
        items: rows,
        has_more,
        next_cursor,
    })
}
