use crate::shared::errors::{Failure, Kind};
use std::{fmt, str::FromStr};

/// Validated standard-variant UUID. No public nil or unchecked constructor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Id([u8; 16]);

fn invalid() -> Failure {
    Failure::new(Kind::Invalid, "invalid ID").with_type("id.invalid")
}
fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
impl Id {
    pub fn parse(text: &str) -> Result<Self, Failure> {
        let text = text.as_bytes();
        if text.len() != 36 || [8, 13, 18, 23].iter().any(|&i| text[i] != b'-') {
            return Err(invalid());
        }
        let mut compact = [0u8; 32];
        let mut n = 0;
        for (i, &b) in text.iter().enumerate() {
            if [8, 13, 18, 23].contains(&i) {
                continue;
            }
            compact[n] = b;
            n += 1;
        }
        let mut bytes = [0u8; 16];
        for (out, pair) in bytes.iter_mut().zip(compact.chunks_exact(2)) {
            *out = hex(pair[0]).ok_or_else(invalid)? << 4 | hex(pair[1]).ok_or_else(invalid)?;
        }
        if bytes[8] & 0xc0 != 0x80 {
            return Err(invalid());
        }
        Ok(Self(bytes))
    }
    pub fn version(self) -> u8 {
        self.0[6] >> 4
    }
    pub fn unix_millis(self) -> Option<u64> {
        if self.version() != 7 {
            return None;
        }
        Some(
            self.0[..6]
                .iter()
                .fold(0u64, |ms, &b| ms << 8 | u64::from(b)),
        )
    }
    pub(super) fn v7(ms: u64, counter: u16, suffix: &[u8]) -> Self {
        let mut bytes = [0u8; 16];
        bytes[..6].copy_from_slice(&ms.to_be_bytes()[2..]);
        bytes[6] = 0x70 | ((counter >> 8) as u8);
        bytes[7] = counter as u8;
        bytes[8..].copy_from_slice(suffix);
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        Self(bytes)
    }
}
impl FromStr for Id {
    type Err = Failure;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}
impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, b) in self.0.iter().enumerate() {
            if [4, 6, 8, 10].contains(&i) {
                f.write_str("-")?;
            }
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}
