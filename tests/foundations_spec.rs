use n2f_rs::{root::run, shared::env};
use std::{
    collections::BTreeMap,
    io::{self, Write},
    sync::{Arc, Mutex},
};
#[derive(Clone, Default)]
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
impl Output {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}
fn vars(v: &[(&str, &str)]) -> BTreeMap<String, String> {
    v.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect()
}
#[test]
fn f01_f04_invalid_before_boot() {
    let out = Output::default();
    let mut diagnostic = Vec::new();
    let code = run(
        env::map(vars(&[
            ("DEMO_TOKEN", ""),
            ("LOG_LEVEL", "private-SENTINEL"),
            ("LOG_CAPACITY", "-1"),
        ])),
        Box::new(out.clone()),
        &mut diagnostic,
        false,
    );
    assert_eq!(code, 2);
    assert!(out.text().is_empty());
    let s = String::from_utf8(diagnostic).unwrap();
    assert!(!s.contains("private-SENTINEL"));
    for k in ["DEMO_TOKEN", "LOG_LEVEL", "LOG_CAPACITY"] {
        assert!(s.contains(k));
    }
}
#[test]
fn f02_f03_modes_and_identities() {
    for mode in ["json", "console", "none"] {
        let out = Output::default();
        let mut diagnostic = Vec::new();
        let code = run(
            env::map(vars(&[
                ("DEMO_TOKEN", "private-SENTINEL"),
                ("LOG_FORMAT", mode),
                ("LOG_COLOR", "never"),
            ])),
            Box::new(out.clone()),
            &mut diagnostic,
            false,
        );
        assert_eq!(code, 0);
        assert!(diagnostic.is_empty());
        let text = out.text();
        assert!(!text.contains("private-SENTINEL"));
        if mode == "none" {
            assert!(text.is_empty());
            continue;
        }
        assert!(text.contains("foundations.complete"));
        if mode != "json" {
            continue;
        }
        let events: BTreeMap<String, serde_json::Value> = text
            .lines()
            .map(|s| {
                let v: serde_json::Value = serde_json::from_str(s).unwrap();
                (v["message"].as_str().unwrap().into(), v)
            })
            .collect();
        let first = &events["dependency.unavailable"]["scope"];
        let second = &events["work.completed"]["scope"];
        assert_eq!(first["work_id"], second["work_id"]);
        assert_eq!(first["correlation_id"], second["correlation_id"]);
        assert_ne!(first["scope_id"], second["scope_id"]);
        assert_eq!(second["attempt"], 2);
        assert_eq!(second["previous_attempt"], first["scope_id"]);
        assert_eq!(events["foundations.complete"]["fields"]["attempts"], 2);
    }
}
