//! Safe field reports and strict scalar checks. See CONTRACT.md.
use crate::shared::errors::{Failure, Kind};

fn invalid(code: &str) -> Failure {
    Failure::new(Kind::Invalid, "invalid input").with_type(format!("validation.{code}"))
}
pub fn text(value: &str, min: usize, max: usize, required: bool) -> Result<(), Failure> {
    if max < min {
        return Err(invalid("policy"));
    }
    let n = value.chars().count();
    if n < min || n > max || (required && value.trim_matches([' ', '\t', '\r', '\n']).is_empty()) {
        return Err(invalid("text"));
    }
    Ok(())
}
pub fn decimal(value: &str, min: u32, max: u32) -> Result<u32, Failure> {
    if max < min || max > 2147483647 {
        return Err(invalid("policy"));
    }
    if value.is_empty()
        || value.len() > 10
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(invalid("decimal"));
    }
    let n = value.parse::<u32>().map_err(|_| invalid("decimal"))?;
    if n < min || n > max {
        return Err(invalid("decimal"));
    }
    Ok(n)
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub field: String,
    pub code: String,
}
#[derive(Default)]
pub struct Report {
    issues: Vec<Issue>,
    truncated: bool,
}
impl Report {
    pub fn add(&mut self, field: &str, code: &str) -> Result<(), Failure> {
        if field.is_empty()
            || field.len() > 128
            || !field
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.-[]".contains(&b))
            || code.is_empty()
            || code.len() > 64
            || !code
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.".contains(&b))
        {
            return Err(invalid("issue"));
        }
        if self.issues.iter().any(|i| i.field == field) {
            return Ok(());
        }
        if self.issues.len() == 32 {
            self.truncated = true;
        } else {
            self.issues.push(Issue {
                field: field.into(),
                code: code.into(),
            });
        }
        Ok(())
    }
    pub fn issues(&self) -> Vec<Issue> {
        self.issues.clone()
    }
    pub fn truncated(&self) -> bool {
        self.truncated
    }
    pub fn result(&self) -> Result<(), Failure> {
        if self.issues.is_empty() {
            return Ok(());
        }
        Err(Failure::new(Kind::Invalid, "validation failed")
            .with_type("validation.failed")
            .with_fields(
                self.issues
                    .iter()
                    .map(|i| (i.field.clone(), i.code.clone()))
                    .collect(),
            ))
    }
}
