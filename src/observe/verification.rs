use serde::Serialize;

use super::object::{inspect_object, read_object};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    ObjectUnreadable,
    ObjectRejected,
    MapSetupFailed,
    ProgramUnavailable,
    ProgramLoadFailed,
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
    pub attachment_attempted: bool,
    pub sensor_running: bool,
    pub owned_handles_dropped: bool,
}

fn report(outcome: Outcome) -> Verification {
    Verification {
        schema_version: 1,
        source: "explicit_kernel_load_probe",
        outcome,
        map_setup_attempted: !matches!(
            outcome,
            Outcome::ObjectUnreadable | Outcome::ObjectRejected
        ),
        program_load_attempted: matches!(outcome, Outcome::ProgramLoadFailed | Outcome::Verified),
        program_loaded: outcome == Outcome::Verified,
        attachment_attempted: false,
        sensor_running: false,
        owned_handles_dropped: true,
    }
}

fn verify_bytes(bytes: &[u8], load: impl FnOnce(&[u8]) -> Outcome) -> Verification {
    if inspect_object(bytes).is_err() {
        return report(Outcome::ObjectRejected);
    }
    report(load(bytes))
}

pub fn verify_path(path: &std::path::Path) -> Verification {
    let Ok(bytes) = read_object(path) else {
        return report(Outcome::ObjectUnreadable);
    };
    verify_bytes(&bytes, |bytes| {
        let Ok(mut bpf) = aya::Ebpf::load(bytes) else {
            return Outcome::MapSetupFailed;
        };
        let outcome = match bpf.program_mut("process_exec") {
            None => Outcome::ProgramUnavailable,
            Some(program) => match <&mut aya::programs::TracePoint>::try_from(program) {
                Err(_) => Outcome::ProgramUnavailable,
                Ok(program) => match program.load() {
                    Ok(()) => Outcome::Verified,
                    Err(_) => Outcome::ProgramLoadFailed,
                },
            },
        };
        drop(bpf);
        outcome
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires an explicitly built BPF artifact; injected loader performs no syscalls"]
    fn real_bpf_artifact_exercises_load_outcomes_without_syscalls() {
        let path = std::env::var_os("INTELLICORD_TEST_BPF_OBJECT")
            .expect("set INTELLICORD_TEST_BPF_OBJECT to the freshly built sensor object");
        let bytes = read_object(std::path::Path::new(&path)).unwrap();
        for outcome in [
            Outcome::MapSetupFailed,
            Outcome::ProgramUnavailable,
            Outcome::ProgramLoadFailed,
            Outcome::Verified,
        ] {
            let calls = std::cell::Cell::new(0);
            let result = verify_bytes(&bytes, |supplied| {
                assert_eq!(supplied, bytes);
                calls.set(calls.get() + 1);
                outcome
            });
            assert_eq!(calls.get(), 1);
            assert_eq!(result.outcome, outcome);
            assert_eq!(result.program_loaded, outcome == Outcome::Verified);
            assert_eq!(
                result.program_load_attempted,
                matches!(outcome, Outcome::ProgramLoadFailed | Outcome::Verified)
            );
            assert!(!result.attachment_attempted);
            assert!(!result.sensor_running);
        }
    }

    #[test]
    fn rejected_objects_never_reach_kernel_loader() {
        for bytes in [&[][..], b"not an ELF"] {
            let result = verify_bytes(bytes, |_| panic!("invalid object reached loader"));
            assert_eq!(result.outcome, Outcome::ObjectRejected);
            assert!(!result.map_setup_attempted);
            assert!(!result.program_load_attempted);
        }
    }

    #[test]
    fn typed_outcomes_preserve_operation_boundaries_and_bounded_privacy() {
        for outcome in [
            Outcome::ObjectUnreadable,
            Outcome::ObjectRejected,
            Outcome::MapSetupFailed,
            Outcome::ProgramUnavailable,
            Outcome::ProgramLoadFailed,
            Outcome::Verified,
        ] {
            let result = report(outcome);
            assert_eq!(result.program_loaded, outcome == Outcome::Verified);
            assert_eq!(
                result.program_load_attempted,
                matches!(outcome, Outcome::ProgramLoadFailed | Outcome::Verified)
            );
            assert_eq!(
                result.map_setup_attempted,
                !matches!(outcome, Outcome::ObjectUnreadable | Outcome::ObjectRejected)
            );
            assert!(!result.attachment_attempted);
            assert!(!result.sensor_running);
            assert!(result.owned_handles_dropped);
            let json = serde_json::to_value(result).unwrap();
            assert_eq!(json.as_object().unwrap().len(), 9);
            let json = serde_json::to_string(&json).unwrap();
            assert!(json.len() < 512);
            for forbidden in ["path", "process_id", "thread_id", "verifier_log", "token"] {
                assert!(!json.contains(forbidden));
            }
        }
    }
}
