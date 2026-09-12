use crate::shared::errors::{Failure, Kind};
// Refusing scaffold for red executable specs. No process uses these auth APIs.
pub(super) fn pending() -> Failure {
    Failure::new(Kind::Internal, "auth leaf not implemented")
        .with_type("identity.auth_not_implemented")
}
