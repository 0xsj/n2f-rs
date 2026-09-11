use std::collections::BTreeMap;

use super::{Classification, Kind};

/// An independent, safe projection for a transport to encode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicInfo {
    pub kind: Kind,
    pub message: String,
    pub error_type: Option<String>,
    pub fields: BTreeMap<String, String>,
}
/// Project an actual failure. None means unknown failure, not a successful Result.
pub fn public_info(classification: Option<Classification<'_>>) -> PublicInfo {
    match classification {
        Some(view) if view.kind != Kind::Internal => PublicInfo {
            kind: view.kind,
            message: if view.message.is_empty() {
                "request failed"
            } else {
                view.message
            }
            .into(),
            error_type: view
                .error_type
                .filter(|value| !value.is_empty())
                .map(str::to_owned),
            fields: view.fields.cloned().unwrap_or_default(),
        },
        _ => PublicInfo {
            kind: Kind::Internal,
            message: "internal error".into(),
            error_type: None,
            fields: BTreeMap::new(),
        },
    }
}
