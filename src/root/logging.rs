use super::config::Config;
use crate::shared::{
    clock::SystemClock,
    errors::Failure,
    logger::{self, Runtime},
};
use std::{io::Write, sync::Arc};
pub(super) fn logging(
    c: &Config,
    clock: Arc<SystemClock>,
    output: Box<dyn Write + Send>,
    terminal: bool,
) -> Result<Runtime, Failure> {
    Runtime::new(logger::Config {
        format: c.format.clone(),
        level: c.level.clone(),
        color: c.color.clone(),
        terminal,
        no_color: c.no_color,
        resource: c.resource.clone(),
        clock: Arc::new(move || clock.now()),
        output,
        capacity: c.capacity,
        max_record_bytes: c.max_bytes,
    })
}
