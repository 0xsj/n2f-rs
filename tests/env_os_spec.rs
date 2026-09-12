use n2f_rs::shared::env;
#[test]
fn v01_os_capture_matches_child_environment() {
    if std::env::var("N2F_ENV_CHILD").as_deref() == Ok("yes") {
        let captured = env::os().unwrap();
        assert_eq!(
            captured("N2F_ENV_SNAPSHOT_FIXTURE").as_deref(),
            Some("before")
        );
        return;
    }
    // Rust 2024 marks process environment mutation unsafe. Run an isolated child
    // with Command::env instead of mutating a multithreaded test process.
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("v01_os_capture_matches_child_environment")
        .env("N2F_ENV_CHILD", "yes")
        .env("N2F_ENV_SNAPSHOT_FIXTURE", "before")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}
