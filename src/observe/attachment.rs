use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    ObjectUnreadable,
    ObjectRejected,
    MapSetupFailed,
    ProgramLoadFailed,
    TargetNotDisabled,
    AttachFailed,
    DetachFailed,
    Verified,
}

#[derive(Debug, Serialize)]
pub struct Verification {
    pub schema_version: u16,
    pub source: &'static str,
    pub outcome: Outcome,
    pub map_setup_attempted: bool,
    pub program_load_attempted: bool,
    pub program_loaded: bool,
    pub target_disabled_confirmed: bool,
    pub attachment_attempted: bool,
    pub attachment_confirmed: bool,
    pub detachment_attempted: bool,
    pub detachment_confirmed: bool,
    pub sensor_running: bool,
    pub owned_handles_dropped: bool,
}

fn report(outcome: Outcome) -> Verification {
    Verification {
        schema_version: 1,
        source: "explicit_kernel_attach_probe",
        outcome,
        map_setup_attempted: false,
        program_load_attempted: false,
        program_loaded: false,
        target_disabled_confirmed: false,
        attachment_attempted: false,
        attachment_confirmed: false,
        detachment_attempted: false,
        detachment_confirmed: false,
        sensor_running: false,
        owned_handles_dropped: false,
    }
}

trait Driver {
    fn setup(&mut self) -> bool;
    fn load(&mut self) -> bool;
    fn target_disabled(&mut self) -> bool;
    fn attach(&mut self) -> bool;
    fn detach(&mut self) -> bool;
}

fn exercise(mut driver: impl Driver) -> Verification {
    let mut result = report(Outcome::MapSetupFailed);
    result.map_setup_attempted = true;
    if driver.setup() {
        result.program_load_attempted = true;
        result.outcome = Outcome::ProgramLoadFailed;
        if driver.load() {
            result.program_loaded = true;
            result.outcome = Outcome::TargetNotDisabled;
            if driver.target_disabled() {
                result.target_disabled_confirmed = true;
                result.attachment_attempted = true;
                result.outcome = Outcome::AttachFailed;
                if driver.attach() {
                    result.attachment_confirmed = true;
                    result.detachment_attempted = true;
                    result.outcome = if driver.detach() {
                        result.detachment_confirmed = true;
                        Outcome::Verified
                    } else {
                        Outcome::DetachFailed
                    };
                }
            }
        }
    }
    drop(driver);
    result.owned_handles_dropped = true;
    result
}

#[cfg(all(feature = "observe", target_os = "linux"))]
fn verify_bytes(bytes: &[u8], run: impl FnOnce() -> Verification) -> Verification {
    if super::object::inspect_object(bytes).is_err() {
        return rejected(Outcome::ObjectRejected);
    }
    run()
}

#[cfg(all(feature = "observe", target_os = "linux"))]
fn rejected(outcome: Outcome) -> Verification {
    let mut result = report(outcome);
    result.owned_handles_dropped = true;
    result
}

#[cfg(all(feature = "observe", target_os = "linux"))]
pub fn verify_path(path: &std::path::Path) -> Verification {
    struct KernelDriver<'a> {
        bytes: &'a [u8],
        bpf: Option<aya::Ebpf>,
        link: Option<aya::programs::trace_point::TracePointLinkId>,
    }
    impl KernelDriver<'_> {
        fn program(&mut self) -> Option<&mut aya::programs::TracePoint> {
            self.bpf
                .as_mut()?
                .program_mut("process_exec")?
                .try_into()
                .ok()
        }
    }
    impl Driver for KernelDriver<'_> {
        fn setup(&mut self) -> bool {
            self.bpf = aya::Ebpf::load(self.bytes).ok();
            self.bpf.is_some()
        }
        fn load(&mut self) -> bool {
            self.program().is_some_and(|program| program.load().is_ok())
        }
        fn target_disabled(&mut self) -> bool {
            self.bpf
                .as_ref()
                .and_then(|bpf| bpf.map("TARGET"))
                .and_then(|map| aya::maps::Array::<_, u32>::try_from(map).ok())
                .is_some_and(|array| matches!(array.get(&0, 0), Ok(0)))
        }
        fn attach(&mut self) -> bool {
            self.link = self
                .program()
                .and_then(|program| program.attach("sched", "sched_process_exec").ok());
            self.link.is_some()
        }
        fn detach(&mut self) -> bool {
            let Some(link) = self.link.take() else {
                return false;
            };
            self.program()
                .is_some_and(|program| program.detach(link).is_ok())
        }
    }
    let Ok(bytes) = super::object::read_object(path) else {
        return rejected(Outcome::ObjectUnreadable);
    };
    verify_bytes(&bytes, || {
        exercise(KernelDriver {
            bytes: &bytes,
            bpf: None,
            link: None,
        })
    })
}

#[derive(Serialize)]
pub struct Replay {
    pub schema_version: u16,
    pub source: &'static str,
    pub sensor_running: bool,
    pub load_test_performed: bool,
    pub attachment_attempted: bool,
    pub checks: Vec<super::SelectionCheck>,
}

pub fn generated_replay() -> Replay {
    use std::{cell::RefCell, rc::Rc};

    struct GeneratedDriver {
        failure: usize,
        calls: Rc<RefCell<Vec<usize>>>,
    }
    impl GeneratedDriver {
        fn step(&self, step: usize) -> bool {
            self.calls.borrow_mut().push(step);
            self.failure != step
        }
    }
    impl Drop for GeneratedDriver {
        fn drop(&mut self) {
            self.calls.borrow_mut().push(6);
        }
    }
    impl Driver for GeneratedDriver {
        fn setup(&mut self) -> bool {
            self.step(1)
        }
        fn load(&mut self) -> bool {
            self.step(2)
        }
        fn target_disabled(&mut self) -> bool {
            self.step(3)
        }
        fn attach(&mut self) -> bool {
            self.step(4)
        }
        fn detach(&mut self) -> bool {
            self.step(5)
        }
    }

    let cases = [
        ("setup_failure_stops", Outcome::MapSetupFailed),
        ("load_failure_stops", Outcome::ProgramLoadFailed),
        ("disabled_target_required", Outcome::TargetNotDisabled),
        ("attach_failure_stops", Outcome::AttachFailed),
        ("detach_failure_visible", Outcome::DetachFailed),
        ("successful_cleanup", Outcome::Verified),
    ];
    let checks = cases
        .into_iter()
        .enumerate()
        .map(|(index, (name, expected))| {
            let failure = index + 1;
            let calls = Rc::new(RefCell::new(Vec::new()));
            let result = exercise(GeneratedDriver {
                failure,
                calls: calls.clone(),
            });
            let mut expected_calls: Vec<_> = (1..=failure.min(5)).collect();
            expected_calls.push(6);
            super::SelectionCheck {
                name,
                passed: result.outcome == expected
                    && *calls.borrow() == expected_calls
                    && result.map_setup_attempted
                    && result.program_load_attempted == (failure > 1)
                    && result.program_loaded == (failure > 2)
                    && result.target_disabled_confirmed == (failure > 3)
                    && result.attachment_attempted == (failure > 3)
                    && result.attachment_confirmed == (failure > 4)
                    && result.detachment_attempted == (failure > 4)
                    && result.detachment_confirmed == (failure > 5)
                    && !result.sensor_running
                    && result.owned_handles_dropped,
            }
        })
        .collect();
    Replay {
        schema_version: 1,
        source: "generated_replay",
        sensor_running: false,
        load_test_performed: false,
        attachment_attempted: false,
        checks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_lifecycle_checks_every_failure_and_cleanup_boundary() {
        let replay = generated_replay();
        assert_eq!(replay.checks.len(), 6);
        for check in &replay.checks {
            assert!(check.passed, "{}", check.name);
        }
        let json = serde_json::to_string(&replay).unwrap();
        assert!(json.len() < 1024);
        assert!(!replay.sensor_running);
        assert!(!replay.load_test_performed);
        assert!(!replay.attachment_attempted);
    }

    #[cfg(all(feature = "observe", target_os = "linux"))]
    #[test]
    fn invalid_objects_never_reach_attach_driver() {
        for bytes in [&[][..], b"not an ELF"] {
            let result = verify_bytes(bytes, || panic!("invalid object reached driver"));
            assert_eq!(result.outcome, Outcome::ObjectRejected);
            assert!(!result.map_setup_attempted);
            assert!(!result.attachment_attempted);
            assert!(!result.detachment_confirmed);
            assert!(result.owned_handles_dropped);
            let json = serde_json::to_string(&result).unwrap();
            assert!(json.len() < 768);
            assert_eq!(
                serde_json::to_value(result)
                    .unwrap()
                    .as_object()
                    .unwrap()
                    .len(),
                13
            );
        }
    }
}
