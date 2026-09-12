use n2f_rs::shared::logger::*;
use std::{
    io::{self, Write},
    sync::{Arc, Mutex},
    time::{Duration, UNIX_EPOCH},
};
struct Output(Arc<Mutex<Vec<u8>>>);
impl Write for Output {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
#[test]
fn l09_maximum_time_and_invalid_float() {
    let out = Arc::new(Mutex::new(Vec::new()));
    let mut c = Config::new(
        Resource {
            name: "test".into(),
            ..Resource::default()
        },
        Arc::new(|| UNIX_EPOCH + Duration::from_millis(281474976710655)),
        Box::new(Output(out.clone())),
    );
    c.color = "never".into();
    let r = Runtime::new(c).unwrap();
    r.log.info("boundary", Fields::new());
    r.close(Duration::from_secs(1)).unwrap();
    assert!(
        String::from_utf8(out.lock().unwrap().clone())
            .unwrap()
            .starts_with("05:31:50.655 ")
    );
    let c = Config::new(
        Resource {
            name: "test".into(),
            ..Resource::default()
        },
        Arc::new(|| UNIX_EPOCH),
        Box::new(io::sink()),
    );
    let r = Runtime::new(c).unwrap();
    r.log.info(
        "invalid",
        Fields::from([("value".into(), Value::Float(f64::NAN))]),
    );
    assert!(r.close(Duration::from_secs(1)).is_err());
    assert_eq!(r.stats().failed, 1);
}
struct FlushFailure;
impl Write for FlushFailure {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::other("private flush detail"))
    }
}
#[test]
fn l07_flush_failure() {
    let c = Config::new(
        Resource {
            name: "test".into(),
            ..Resource::default()
        },
        Arc::new(|| UNIX_EPOCH),
        Box::new(FlushFailure),
    );
    let r = Runtime::new(c).unwrap();
    assert!(r.close(Duration::from_secs(1)).is_err());
    assert_eq!(r.stats().failed, 1);
}
