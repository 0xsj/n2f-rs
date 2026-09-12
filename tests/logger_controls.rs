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
fn l06_unicode_controls_cannot_split_console_records() {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let mut c = Config::new(
        Resource {
            name: "test".into(),
            ..Resource::default()
        },
        Arc::new(|| UNIX_EPOCH),
        Box::new(Output(bytes.clone())),
    );
    c.color = "never".into();
    let r = Runtime::new(c).unwrap();
    let text = "text\u{7f}\u{85}\u{9b}\u{2028}\u{2029}";
    r.log
        .info(text, Fields::from([("field".into(), text.into())]));
    r.close(Duration::from_secs(1)).unwrap();
    let output = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
    for control in ['\u{7f}', '\u{85}', '\u{9b}', '\u{2028}', '\u{2029}'] {
        assert!(!output.contains(control), "raw control {control:?}");
    }
}
