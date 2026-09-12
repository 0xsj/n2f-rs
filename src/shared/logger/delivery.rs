use crate::shared::errors::{Failure, Kind};
use std::{
    io::{self, Write},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, SyncSender},
    },
    time::{Duration, Instant},
};
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub accepted: u64,
    pub written: u64,
    pub dropped: u64,
    pub failed: u64,
}
#[derive(Default)]
pub(super) struct Counts {
    pub accepted: AtomicU64,
    pub written: AtomicU64,
    pub dropped: AtomicU64,
    pub failed: AtomicU64,
}
pub(super) struct Delivery {
    sender: Mutex<Option<SyncSender<Vec<u8>>>>,
    done: Mutex<bool>,
    wake: Condvar,
    pub counts: Counts,
    max: usize,
    pub noop: bool,
}
impl Delivery {
    pub fn start(
        mut output: Box<dyn Write + Send>,
        capacity: usize,
        max: usize,
        noop: bool,
    ) -> Arc<Self> {
        let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(capacity);
        let d = Arc::new(Self {
            sender: Mutex::new(if noop { None } else { Some(tx) }),
            done: Mutex::new(noop),
            wake: Condvar::new(),
            counts: Counts::default(),
            max,
            noop,
        });
        if !noop {
            let worker = d.clone();
            std::thread::spawn(move || {
                for b in rx {
                    if output.write_all(&b).is_ok() {
                        worker.counts.written.fetch_add(1, Ordering::Relaxed);
                    } else {
                        worker.fail();
                    }
                }
                if output.flush().is_err() {
                    worker.fail();
                }
                *worker.done.lock().expect("delivery lock") = true;
                worker.wake.notify_all();
            });
        }
        d
    }
    pub fn fail(&self) {
        self.counts.failed.fetch_add(1, Ordering::Relaxed);
    }
    pub fn closed(&self) -> bool {
        self.sender.lock().expect("delivery lock").is_none()
    }
    pub fn stop(&self) {
        self.sender.lock().expect("delivery lock").take();
    }
    pub fn drop_record(&self) {
        self.counts.dropped.fetch_add(1, Ordering::Relaxed);
    }
    pub fn write(&self, b: Vec<u8>) {
        if self.noop {
            return;
        }
        let sender = self.sender.lock().expect("delivery lock");
        if b.len() > self.max {
            self.drop_record();
            return;
        }
        if let Some(tx) = sender.as_ref() {
            if tx.try_send(b).is_ok() {
                self.counts.accepted.fetch_add(1, Ordering::Relaxed);
            } else {
                self.drop_record();
            }
        } else {
            self.drop_record();
        }
    }
    pub fn stats(&self) -> Stats {
        Stats {
            accepted: self.counts.accepted.load(Ordering::Relaxed),
            written: self.counts.written.load(Ordering::Relaxed),
            dropped: self.counts.dropped.load(Ordering::Relaxed),
            failed: self.counts.failed.load(Ordering::Relaxed),
        }
    }
    pub fn close(&self, timeout: Duration) -> Result<(), Failure> {
        self.stop();
        let start = Instant::now();
        let mut done = self.done.lock().expect("delivery lock");
        while !*done {
            let remaining = timeout.saturating_sub(start.elapsed());
            if remaining.is_zero() {
                return Err(Failure::new(Kind::Timeout, "log flush deadline exceeded")
                    .with_type("logger.flush_timeout"));
            }
            done = self
                .wake
                .wait_timeout(done, remaining)
                .expect("delivery lock")
                .0;
        }
        if self.stats().failed > 0 {
            Err(Failure::new(Kind::Unavailable, "log sink failed").with_type("logger.sink"))
        } else {
            Ok(())
        }
    }
}
pub(super) struct RecordWriter {
    delivery: Arc<Delivery>,
    bytes: Vec<u8>,
}
impl Write for RecordWriter {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.bytes.extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Drop for RecordWriter {
    fn drop(&mut self) {
        if !self.bytes.is_empty() {
            self.delivery.write(std::mem::take(&mut self.bytes));
        }
    }
}
#[derive(Clone)]
pub(super) struct Writer(pub Arc<Delivery>);
impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Writer {
    type Writer = RecordWriter;
    fn make_writer(&'a self) -> Self::Writer {
        RecordWriter {
            delivery: self.0.clone(),
            bytes: Vec::new(),
        }
    }
}
