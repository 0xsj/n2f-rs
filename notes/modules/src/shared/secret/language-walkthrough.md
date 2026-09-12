# Secret: implementation walkthrough

The secret value owns presentation, not requiredness. Empty is valid and reveal
returns the exact input; env rejects empty credentials at its boundary. A public
neighbor in each nested test prevents a false fix that redacts the entire object.
Logger tests exercise the real adapter, not only standalone formatting hooks.

Redaction is explicit type behavior. It cannot recognize a credential after reveal
has returned an ordinary string, erase process memory, or authorize provider access.

## Rust mechanics

SecretString owns String. Moving it does not require Clone; reveal borrows &str
and neither Display nor Debug grants the raw content. A containing #[derive(Debug)]
struct uses the field's safe Debug implementation. Compile-fail doctests verify
private-field access and implicit conversion are refused. No Serialize derive is
added: the logger explicitly converts &SecretString to its owned redacted Value.

