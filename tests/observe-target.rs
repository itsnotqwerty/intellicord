#![cfg(all(feature = "observe", target_os = "linux"))]

use intellicord::observe::target::TargetGuard;
use std::{
    io::Write,
    process::{Child, Command, Stdio},
};

struct TestChild(Child);

impl Drop for TestChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn_child() -> TestChild {
    TestChild(
        Command::new("cat")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    )
}

#[test]
fn handle_tracks_one_child_through_exit_and_reaping() {
    let mut child = spawn_child();
    let guard = TargetGuard::open(child.0.id()).unwrap();
    assert!(guard.is_alive().unwrap());
    child
        .0
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"generated test input\n")
        .unwrap();
    child.0.stdin.take();
    assert!(child.0.wait().unwrap().success());
    for _ in 0..4 {
        assert!(!guard.is_alive().unwrap());
    }
    let replacement = spawn_child();
    assert!(
        TargetGuard::open(replacement.0.id())
            .unwrap()
            .is_alive()
            .unwrap()
    );
    assert!(!guard.is_alive().unwrap());
}

#[test]
fn killed_target_is_not_revived_by_other_process_activity() {
    let mut target = spawn_child();
    let guard = TargetGuard::open(target.0.id()).unwrap();
    assert!(guard.is_alive().unwrap());
    target.0.kill().unwrap();
    target.0.wait().unwrap();
    let other = spawn_child();
    assert!(TargetGuard::open(other.0.id()).unwrap().is_alive().unwrap());
    assert!(!guard.is_alive().unwrap());
}
