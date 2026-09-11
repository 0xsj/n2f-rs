//! A refused registration projected for a caller without exposing its source.
use n2f_rs::shared::errors::{Classified, Context, Failure, Kind, public_info};

fn main() {
    let outcome: Result<(), Failure> =
        Err(Failure::new(Kind::Conflict, "email already registered")
            .with_type("account.email_taken")
            .with_field("email", "taken")
            .with_source(std::io::Error::other("PRIVATE database constraint")));
    let contextual = outcome.map_err(|e| Context::new("register account", e));
    let refused = contextual.expect_err("this example demonstrates a refusal");
    let view = public_info(refused.classification());
    println!("{}", view.kind.as_str());
    println!("{}", view.message);
    println!("{}", view.error_type.as_deref().unwrap_or_default());
    println!(
        "{}",
        view.fields
            .get("email")
            .map(String::as_str)
            .unwrap_or_default()
    );
}
