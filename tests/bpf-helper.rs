#![cfg(all(target_os = "linux", feature = "observe"))]

use std::process::Command;

#[test]
fn helper_rejects_every_general_observer_surface() {
    for arguments in [
        vec![],
        vec!["verify-load", "--object", "/tmp/arbitrary"],
        vec![
            "capture-socket-progress",
            "--object",
            "/tmp/arbitrary",
            "--pid",
            "1",
            "--seconds",
            "1",
            "--session",
            "0123456789abcdef0123456789abcdef",
            "--socket",
            "/tmp/progress.sock",
            "--parent-pid",
            "1",
            "--acknowledge-socket-progress",
        ],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_intellicord-bpf-helper"))
            .args(arguments)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("Usage:"));
    }
}

#[test]
fn helper_checks_parent_before_system_artifacts_or_capabilities() {
    let output = Command::new(env!("CARGO_BIN_EXE_intellicord-bpf-helper"))
        .args([
            "capture-socket-progress",
            "--pid",
            &std::process::id().to_string(),
            "--seconds",
            "1",
            "--session",
            "0123456789abcdef0123456789abcdef",
            "--socket",
            "/tmp/progress.sock",
            "--parent-pid",
            "1",
            "--acknowledge-privileged-observation",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("parent mismatch"));
}
