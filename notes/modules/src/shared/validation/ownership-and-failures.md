# Safe issues are values, not rejected input

Origin: comparable scalar/decimal/report tests in this slice. An issue code is
safe public data; storing an input value as the issue would bypass error redaction.
The report keeps the first issue per field, bounds distinct fields, and copies
snapshots. The builder is operation-local, not a concurrent collector.

Rust &str already guarantees valid UTF-8. chars counts Unicode scalars, not grapheme clusters. The report returns owned Vec snapshots; Result distinguishes an empty report from invalid input.

No normalization or domain invariant belongs here. For example a combining mark
counts separately; an identity module must decide its own normalized identifier
policy. Used in the sibling pagination source directory. See CONTRACT.md there.
