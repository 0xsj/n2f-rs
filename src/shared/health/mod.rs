//! Readiness owns a finite evaluation budget and irreversible process draining.
use crate::shared::errors::{Failure, Kind};
use futures_util::{FutureExt, future::BoxFuture};
use std::{
    panic::AssertUnwindSafe,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
};
pub type Check = Arc<dyn Fn() -> BoxFuture<'static, Result<(), Failure>> + Send + Sync>;
pub struct Gate {
    state: AtomicU8,
    busy: tokio::sync::Mutex<()>,
    checks: Vec<Check>,
    budget: Duration,
}
impl Gate {
    pub fn new(budget: Duration, checks: Vec<Check>) -> Result<Self, Failure> {
        if budget < Duration::from_millis(1) || budget > Duration::from_secs(5) || checks.len() > 16
        {
            return Err(Failure::new(Kind::Invalid, "invalid health configuration"));
        }
        Ok(Self {
            state: AtomicU8::new(0),
            busy: tokio::sync::Mutex::new(()),
            checks,
            budget,
        })
    }
    pub fn start(&self) -> bool {
        self.state
            .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }
    pub fn drain(&self) {
        self.state.store(2, Ordering::SeqCst);
    }
    pub async fn ready(&self) -> bool {
        if self.state.load(Ordering::SeqCst) != 1 {
            return false;
        }
        let Ok(_guard) = self.busy.try_lock() else {
            return false;
        };
        let check = async {
            for c in &self.checks {
                c().await?;
            }
            Ok::<(), Failure>(())
        };
        matches!(
            tokio::time::timeout(self.budget, AssertUnwindSafe(check).catch_unwind()).await,
            Ok(Ok(Ok(())))
        ) && self.state.load(Ordering::SeqCst) == 1
    }
}
