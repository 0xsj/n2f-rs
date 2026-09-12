use super::{
    Actor,
    actor::{identity_valid, invalid},
};
use crate::shared::errors::Failure;
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AttributionSpec {
    pub initiator: Option<Actor>,
    pub on_behalf_of: Option<Actor>,
    pub tenant: Option<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Attribution {
    data: AttributionSpec,
}
impl Attribution {
    pub fn new(data: AttributionSpec) -> Result<Self, Failure> {
        if let Some(represented) = &data.on_behalf_of {
            if !represented.named()
                || data
                    .initiator
                    .as_ref()
                    .is_none_or(|a| !a.named() || a == represented)
            {
                return Err(invalid("invalid_attribution"));
            }
        }
        if data.tenant.as_ref().is_some_and(|s| !identity_valid(s)) {
            return Err(invalid("invalid_attribution"));
        }
        Ok(Self { data })
    }
    pub fn snapshot(&self) -> AttributionSpec {
        self.data.clone()
    }
}
