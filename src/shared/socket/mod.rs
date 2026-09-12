//! Versioned bounded application envelope; domains own message meanings.
pub mod axum;
use crate::shared::errors::{Failure, Kind};
pub struct Message {
    pub id: String,
    pub message_type: String,
    pub payload: serde_json::Map<String, serde_json::Value>,
}
fn invalid() -> Failure {
    Failure::new(Kind::Invalid, "invalid socket message").with_type("socket.invalid")
}
pub fn decode(raw: &[u8]) -> Result<Message, Failure> {
    if raw.len() > 65536 {
        return Err(invalid());
    }
    let v: serde_json::Value = serde_json::from_slice(raw).map_err(|_| invalid())?;
    let o = v.as_object().ok_or_else(invalid)?;
    if o.len() != 4 || o.get("v").and_then(|v| v.as_f64()) != Some(1.0) {
        return Err(invalid());
    }
    let id = o.get("id").and_then(|v| v.as_str()).ok_or_else(invalid)?;
    let t = o.get("type").and_then(|v| v.as_str()).ok_or_else(invalid)?;
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        || t.is_empty()
        || t.len() > 64
        || !t
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.".contains(&b))
    {
        return Err(invalid());
    }
    let payload = o
        .get("payload")
        .and_then(|v| v.as_object())
        .ok_or_else(invalid)?
        .clone();
    Ok(Message {
        id: id.into(),
        message_type: t.into(),
        payload,
    })
}
pub fn encode(m: Message) -> Result<String, Failure> {
    let raw =
        serde_json::json!({"v":1,"id":m.id,"type":m.message_type,"payload":m.payload}).to_string();
    decode(raw.as_bytes())?;
    Ok(raw)
}
