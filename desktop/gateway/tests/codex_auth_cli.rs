#[cfg(unix)]
#[test]
fn non_utf8_codex_argument_is_redacted_structured_error() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::process::Command;

    let output = Command::new(env!("CARGO_BIN_EXE_csswitch-gateway"))
        .arg("codex-auth")
        .arg(OsString::from_vec(vec![0xff]))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1);
    let value: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(value["schema_version"], 3);
    assert_eq!(value["ok"], false);
    assert_eq!(value["error"]["code"], "invalid_arguments");
}

#[cfg(target_os = "linux")]
#[test]
fn linux_status_uses_csswitch_owned_state() {
    let root =
        std::env::temp_dir().join(format!("csswitch-linux-status-test-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_csswitch-gateway"))
        .arg("codex-auth")
        .arg("status")
        .env("HOME", &root)
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&root);

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let value: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(value["command"], "status");
    assert_eq!(value["status"]["authenticated"], false);
    assert_ne!(value["error"]["code"], "unsupported_platform");
}
