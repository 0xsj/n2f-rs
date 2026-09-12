# Env: implementation walkthrough

Lookup is captured at the process boundary, then parsing is deterministic.
Presence is a separate question from truthiness: an explicitly empty ordinary string
survives, while required/secret readers refuse it. Strict decimal and boolean syntax
avoid different coercion rules between runtimes.

Reader collects fixed key/reason problems, and a manifest is unavailable until
validation succeeds. Returned manifests own their storage and redact only marked
secrets. Raw source values never enter invalid-config diagnostics. Root owns settings
and cross-field rules; the shared module does not own a universal application config.

## Rust mechanics

Reader<L> uses an Fn lookup and owned BTreeMap fixtures. BTreeMap gives stable
ordering, but manifests still explicitly sort their copied records. Option<String>
distinguishes absent from empty. Parsing returns Option for syntax and the reader
collects a Failure with safe fields.

Rust 2024 return-position impl Trait captures lifetimes more broadly. The test
fixture's owned returned closure required + use<> to avoid unnecessarily borrowing
its temporary input slice (E0716). Process env mutation is unsafe in this edition;
the OS boundary test uses Command::env on an isolated child instead.

