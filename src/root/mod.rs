//! Composition and process ownership.
//!
//! Concrete adapters, application operations, and transports will be wired here
//! when the first workflow needs them. Process startup and shutdown also belong
//! here once an executable exists. Other layers must not depend on this module.
