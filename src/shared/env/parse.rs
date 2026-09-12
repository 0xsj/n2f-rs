pub const SAFE_INTEGER: i64 = 9007199254740991;
pub fn key_valid(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b[0].is_ascii_uppercase()
        && b.iter()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == b'_')
}
pub fn integer(s: &str) -> Option<i64> {
    let text = s.strip_prefix('-').unwrap_or(s);
    if text.is_empty()
        || !text.bytes().all(|c| c.is_ascii_digit())
        || (text.len() > 1 && text.starts_with('0'))
    {
        return None;
    }
    let n = s.parse::<i64>().ok()?;
    (-SAFE_INTEGER..=SAFE_INTEGER).contains(&n).then_some(n)
}
