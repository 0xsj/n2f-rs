use crate::shared::secret::SecretString;
use std::collections::BTreeMap;
pub type Fields = BTreeMap<String, Value>;
/// Owned data only: arbitrary Debug/Display implementations never become fields.
#[derive(Clone, Debug)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    UInt(u64),
    Float(f64),
    String(String),
    Array(Vec<Value>),
    Object(Fields),
}
impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Self::String(v.into())
    }
}
impl From<String> for Value {
    fn from(v: String) -> Self {
        Self::String(v)
    }
}
impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Self::Int(v)
    }
}
impl From<u64> for Value {
    fn from(v: u64) -> Self {
        Self::UInt(v)
    }
}
impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}
impl From<&SecretString> for Value {
    fn from(_: &SecretString) -> Self {
        Self::String("[REDACTED]".into())
    }
}
pub(super) fn json(v: &Value) -> Result<serde_json::Value, ()> {
    use serde_json::Value as J;
    Ok(match v {
        Value::Null => J::Null,
        Value::Bool(v) => J::Bool(*v),
        Value::Int(v) => J::from(*v),
        Value::UInt(v) => J::from(*v),
        Value::Float(v) => J::Number(serde_json::Number::from_f64(*v).ok_or(())?),
        Value::String(v) => J::String(v.clone()),
        Value::Array(v) => J::Array(v.iter().map(json).collect::<Result<_, _>>()?),
        Value::Object(v) => J::Object(
            v.iter()
                .map(|(k, v)| Ok((k.clone(), json(v)?)))
                .collect::<Result<_, _>>()?,
        ),
    })
}
