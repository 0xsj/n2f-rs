use super::parse::{SAFE_INTEGER, integer, key_valid};
use crate::shared::{
    errors::{Failure, Kind},
    secret::SecretString,
};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Var {
    pub key: String,
    pub value: String,
    pub source: String,
    pub secret: bool,
}
pub struct Reader<L> {
    lookup: L,
    seen: BTreeSet<String>,
    problems: BTreeMap<String, String>,
    resolved: Vec<Var>,
}
impl<L: Fn(&str) -> Option<String>> Reader<L> {
    pub fn new(lookup: L) -> Self {
        Self {
            lookup,
            seen: BTreeSet::new(),
            problems: BTreeMap::new(),
            resolved: Vec::new(),
        }
    }
    fn problem(&mut self, k: &str, why: &str) {
        self.problems
            .insert(if key_valid(k) { k } else { "<key>" }.into(), why.into());
    }
    fn read(&mut self, k: &str) -> Result<Option<String>, ()> {
        if !key_valid(k) {
            self.problem(k, "invalid_key");
            return Err(());
        }
        if !self.seen.insert(k.into()) {
            self.problem(k, "duplicate_key");
            return Err(());
        }
        Ok((self.lookup)(k))
    }
    fn record(&mut self, k: &str, v: &str, p: bool, s: bool) {
        self.resolved.push(Var {
            key: k.into(),
            value: if s { "[REDACTED]" } else { v }.into(),
            source: if p { "environment" } else { "default" }.into(),
            secret: s,
        });
    }
    pub fn string(&mut self, k: &str, fallback: &str) -> String {
        let Ok(raw) = self.read(k) else {
            return String::new();
        };
        let p = raw.is_some();
        let v = raw.unwrap_or_else(|| fallback.into());
        self.record(k, &v, p, false);
        v
    }
    pub fn required(&mut self, k: &str) -> String {
        let Ok(raw) = self.read(k) else {
            return String::new();
        };
        match raw {
            Some(v) if !v.is_empty() => {
                self.record(k, &v, true, false);
                v
            }
            _ => {
                self.problem(k, "required");
                String::new()
            }
        }
    }
    pub fn secret(&mut self, k: &str) -> SecretString {
        let Ok(raw) = self.read(k) else {
            return SecretString::new(String::new());
        };
        match raw {
            Some(v) if !v.is_empty() => {
                self.record(k, &v, true, true);
                SecretString::new(v)
            }
            _ => {
                self.problem(k, "required");
                SecretString::new(String::new())
            }
        }
    }
    pub fn int(&mut self, k: &str, fallback: i64, min: i64, max: i64) -> i64 {
        if min > max
            || min < -SAFE_INTEGER
            || max > SAFE_INTEGER
            || fallback < min
            || fallback > max
        {
            self.problem(k, "invalid_definition");
            return 0;
        }
        let Ok(raw) = self.read(k) else { return 0 };
        let p = raw.is_some();
        let v = match raw {
            None => fallback,
            Some(s) => match integer(&s) {
                Some(n) => n,
                None => {
                    self.problem(k, "invalid_integer");
                    return 0;
                }
            },
        };
        if v < min || v > max {
            self.problem(k, "out_of_range");
            return 0;
        }
        self.record(k, &v.to_string(), p, false);
        v
    }
    pub fn boolean(&mut self, k: &str, fallback: bool) -> bool {
        let Ok(raw) = self.read(k) else { return false };
        let p = raw.is_some();
        let v = match raw.as_deref() {
            None => fallback,
            Some("true") => true,
            Some("false") => false,
            _ => {
                self.problem(k, "invalid_boolean");
                return false;
            }
        };
        self.record(k, if v { "true" } else { "false" }, p, false);
        v
    }
    pub fn enumeration(&mut self, k: &str, fallback: &str, allowed: &[&str]) -> String {
        let unique: BTreeSet<_> = allowed.iter().copied().collect();
        if unique.len() != allowed.len() || unique.contains("") || !unique.contains(fallback) {
            self.problem(k, "invalid_definition");
            return String::new();
        }
        let Ok(raw) = self.read(k) else {
            return String::new();
        };
        let p = raw.is_some();
        let v = raw.unwrap_or_else(|| fallback.into());
        if !unique.contains(v.as_str()) {
            self.problem(k, "invalid_choice");
            return String::new();
        }
        self.record(k, &v, p, false);
        v
    }
    pub fn check(&self) -> Result<(), Failure> {
        if self.problems.is_empty() {
            Ok(())
        } else {
            Err(Failure::new(Kind::Invalid, "invalid configuration")
                .with_type("env.invalid")
                .with_fields(self.problems.clone()))
        }
    }
    pub fn manifest(&self) -> Result<Vec<Var>, Failure> {
        self.check()?;
        let mut v = self.resolved.clone();
        v.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(v)
    }
}
