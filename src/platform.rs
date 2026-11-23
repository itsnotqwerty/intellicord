use std::{fs::File, io::Read, path::Path};

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Prerequisite {
    Present,
    Missing,
    Unreadable,
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnprivilegedBpf {
    Enabled,
    DisabledLocked,
    DisabledConfigurable,
    Unknown,
    Unsupported,
}

#[derive(Clone, Debug, Serialize)]
pub struct EffectiveCapabilities {
    pub bpf: bool,
    pub perfmon: bool,
    pub sys_admin: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Lockdown {
    None,
    Integrity,
    Confidentiality,
    Unknown,
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Seccomp {
    Disabled,
    Strict,
    Filter,
    Unknown,
    Unsupported,
}

#[derive(Clone, Debug, Serialize)]
pub struct LinuxReadiness {
    pub schema_version: u16,
    pub platform: &'static str,
    pub kernel_btf: Prerequisite,
    pub cgroup_v2: Prerequisite,
    pub unprivileged_bpf: UnprivilegedBpf,
    pub perf_event_paranoid: Option<i32>,
    pub effective_capabilities: Option<EffectiveCapabilities>,
    pub sched_process_exec: Prerequisite,
    pub lockdown: Lockdown,
    pub no_new_privileges: Option<bool>,
    pub seccomp: Seccomp,
    pub sensor_running: bool,
    pub load_test_performed: bool,
}

impl LinuxReadiness {
    pub fn check() -> Self {
        if cfg!(target_os = "linux") {
            probe(Path::new("/"))
        } else {
            Self {
                schema_version: 3,
                platform: "unsupported",
                kernel_btf: Prerequisite::Unsupported,
                cgroup_v2: Prerequisite::Unsupported,
                unprivileged_bpf: UnprivilegedBpf::Unsupported,
                perf_event_paranoid: None,
                effective_capabilities: None,
                sched_process_exec: Prerequisite::Unsupported,
                lockdown: Lockdown::Unsupported,
                no_new_privileges: None,
                seccomp: Seccomp::Unsupported,
                sensor_running: false,
                load_test_performed: false,
            }
        }
    }
}

fn probe(root: &Path) -> LinuxReadiness {
    let kernel_btf = match std::fs::metadata(root.join("sys/kernel/btf/vmlinux")) {
        Ok(metadata) if metadata.is_file() && metadata.len() > 0 => Prerequisite::Present,
        Ok(_) => Prerequisite::Unreadable,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Prerequisite::Missing,
        Err(_) => Prerequisite::Unreadable,
    };
    let cgroup_v2 = match read_bounded(&root.join("sys/fs/cgroup/cgroup.controllers")) {
        Ok(value)
            if value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || byte == b'_' || byte.is_ascii_whitespace()
            }) =>
        {
            Prerequisite::Present
        }
        Ok(_) => Prerequisite::Unreadable,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Prerequisite::Missing,
        Err(_) => Prerequisite::Unreadable,
    };
    let unprivileged_bpf =
        match read_bounded(&root.join("proc/sys/kernel/unprivileged_bpf_disabled")) {
            Ok(value) => match value.trim() {
                "0" => UnprivilegedBpf::Enabled,
                "1" => UnprivilegedBpf::DisabledLocked,
                "2" => UnprivilegedBpf::DisabledConfigurable,
                _ => UnprivilegedBpf::Unknown,
            },
            Err(_) => UnprivilegedBpf::Unknown,
        };
    let perf_event_paranoid = read_bounded(&root.join("proc/sys/kernel/perf_event_paranoid"))
        .ok()
        .and_then(|value| value.trim().parse::<i32>().ok());
    let status = read_limited(&root.join("proc/self/status"), 65536).ok();
    let effective_capabilities = status.as_deref().and_then(capabilities);
    let no_new_privileges =
        status
            .as_deref()
            .and_then(|status| match status_field(status, "NoNewPrivs:")? {
                "0" => Some(false),
                "1" => Some(true),
                _ => None,
            });
    let seccomp = match status
        .as_deref()
        .and_then(|status| status_field(status, "Seccomp:"))
    {
        Some("0") => Seccomp::Disabled,
        Some("1") => Seccomp::Strict,
        Some("2") => Seccomp::Filter,
        _ => Seccomp::Unknown,
    };
    let sched_process_exec = tracepoint(root);
    let lockdown = read_bounded(&root.join("sys/kernel/security/lockdown"))
        .ok()
        .map_or(Lockdown::Unknown, |value| match value.trim() {
            "[none] integrity confidentiality" => Lockdown::None,
            "none [integrity] confidentiality" => Lockdown::Integrity,
            "none integrity [confidentiality]" => Lockdown::Confidentiality,
            _ => Lockdown::Unknown,
        });
    LinuxReadiness {
        schema_version: 3,
        platform: "linux",
        kernel_btf,
        cgroup_v2,
        unprivileged_bpf,
        perf_event_paranoid,
        effective_capabilities,
        sched_process_exec,
        lockdown,
        no_new_privileges,
        seccomp,
        sensor_running: false,
        load_test_performed: false,
    }
}

fn read_bounded(file: &Path) -> std::io::Result<String> {
    read_limited(file, 4096)
}

fn read_limited(file: &Path, limit: u64) -> std::io::Result<String> {
    let mut value = String::new();
    File::open(file)?
        .take(limit + 1)
        .read_to_string(&mut value)?;
    if value.len() as u64 > limit {
        return Err(std::io::Error::other("prerequisite metadata exceeds limit"));
    }
    Ok(value)
}

fn status_field<'status>(status: &'status str, field: &str) -> Option<&'status str> {
    let mut entries = status.lines().filter_map(|line| line.strip_prefix(field));
    let value = entries.next()?.trim();
    entries.next().is_none().then_some(value)
}

fn capabilities(status: &str) -> Option<EffectiveCapabilities> {
    let mut entries = status
        .lines()
        .filter_map(|line| line.strip_prefix("CapEff:"));
    let value = entries.next()?.trim();
    if entries.next().is_some()
        || value.len() != 16
        || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return None;
    }
    let mask = u64::from_str_radix(value, 16).ok()?;
    Some(EffectiveCapabilities {
        bpf: mask & (1_u64 << 39) != 0,
        perfmon: mask & (1_u64 << 38) != 0,
        sys_admin: mask & (1_u64 << 21) != 0,
    })
}

fn tracepoint(root: &Path) -> Prerequisite {
    let mut unreadable = false;
    for location in ["sys/kernel/tracing", "sys/kernel/debug/tracing"] {
        match read_bounded(
            &root
                .join(location)
                .join("events/sched/sched_process_exec/id"),
        ) {
            Ok(value) if value.trim().parse::<u32>().is_ok_and(|id| id > 0) => {
                return Prerequisite::Present;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => unreadable = true,
        }
    }
    if unreadable {
        Prerequisite::Unreadable
    } else {
        Prerequisite::Missing
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Fixture(std::path::PathBuf);

    impl Fixture {
        fn new() -> Self {
            let directory = std::env::temp_dir().join(format!(
                "intellicord-platform-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&directory).unwrap();
            Self(directory)
        }

        fn write(&self, file: &str, value: &str) {
            let destination = self.0.join(file);
            std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
            std::fs::write(destination, value).unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn prerequisites_do_not_claim_a_running_or_loadable_sensor() {
        let fixture = Fixture::new();
        fixture.write("sys/kernel/btf/vmlinux", "fixture BTF presence only");
        fixture.write("sys/fs/cgroup/cgroup.controllers", "cpu memory pids\n");
        for (setting, expected) in [
            ("0\n", UnprivilegedBpf::Enabled),
            ("1", UnprivilegedBpf::DisabledLocked),
            ("2", UnprivilegedBpf::DisabledConfigurable),
        ] {
            fixture.write("proc/sys/kernel/unprivileged_bpf_disabled", setting);
            let report = probe(&fixture.0);
            assert_eq!(report.kernel_btf, Prerequisite::Present);
            assert_eq!(report.cgroup_v2, Prerequisite::Present);
            assert_eq!(report.unprivileged_bpf, expected);
            assert!(!report.sensor_running);
            assert!(!report.load_test_performed);
        }
    }

    #[test]
    fn missing_metadata_is_not_a_positive_capability_verdict() {
        let fixture = Fixture::new();
        let report = probe(&fixture.0);
        assert_eq!(report.kernel_btf, Prerequisite::Missing);
        assert_eq!(report.cgroup_v2, Prerequisite::Missing);
        assert_eq!(report.unprivileged_bpf, UnprivilegedBpf::Unknown);
    }

    #[test]
    fn malformed_or_oversized_metadata_fails_conservatively() {
        let fixture = Fixture::new();
        fixture.write("sys/kernel/btf/vmlinux", "");
        fixture.write("sys/fs/cgroup/cgroup.controllers", "<invalid>");
        fixture.write("proc/sys/kernel/unprivileged_bpf_disabled", "3");
        let report = probe(&fixture.0);
        assert_eq!(report.kernel_btf, Prerequisite::Unreadable);
        assert_eq!(report.cgroup_v2, Prerequisite::Unreadable);
        assert_eq!(report.unprivileged_bpf, UnprivilegedBpf::Unknown);
        fixture.write("sys/fs/cgroup/cgroup.controllers", &" ".repeat(4097));
        fixture.write(
            "proc/sys/kernel/unprivileged_bpf_disabled",
            &"0".repeat(4097),
        );
        let report = probe(&fixture.0);
        assert_eq!(report.cgroup_v2, Prerequisite::Unreadable);
        assert_eq!(report.unprivileged_bpf, UnprivilegedBpf::Unknown);
    }

    #[test]
    fn report_serializes_only_fixed_prerequisite_fields() {
        let fixture = Fixture::new();
        let value = serde_json::to_value(probe(&fixture.0)).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"schema_version":3,"platform":"linux","kernel_btf":"missing","cgroup_v2":"missing","unprivileged_bpf":"unknown","perf_event_paranoid":null,"effective_capabilities":null,"sched_process_exec":"missing","lockdown":"unknown","no_new_privileges":null,"seccomp":"unknown","sensor_running":false,"load_test_performed":false})
        );
    }

    #[test]
    fn execution_restrictions_are_strict_independent_context_not_load_verdicts() {
        let fixture = Fixture::new();
        for (setting, expected) in [("0", false), ("1", true)] {
            for (mode, expected_mode) in [("0", "disabled"), ("1", "strict"), ("2", "filter")] {
                fixture.write(
                    "proc/self/status",
                    &format!(
                        "CapEff:\t0000000000000000\nNoNewPrivs:\t{setting}\nSeccomp:\t{mode}\n"
                    ),
                );
                let report = probe(&fixture.0);
                assert_eq!(report.no_new_privileges, Some(expected));
                assert_eq!(serde_json::to_value(report.seccomp).unwrap(), expected_mode);
                assert!(!report.sensor_running);
                assert!(!report.load_test_performed);
            }
        }
        for fields in [
            "",
            "NoNewPrivs: 2\nSeccomp: 3",
            "NoNewPrivs: 01\nSeccomp: 02",
            "NoNewPrivs: 0 extra\nSeccomp: 2 extra",
            "NoNewPrivs: 0\nNoNewPrivs: 1\nSeccomp: 0\nSeccomp: 2",
        ] {
            fixture.write(
                "proc/self/status",
                &format!("CapEff: 0000000000000000\n{fields}\n"),
            );
            let report = probe(&fixture.0);
            assert!(report.no_new_privileges.is_none());
            assert_eq!(serde_json::to_value(report.seccomp).unwrap(), "unknown");
            assert!(report.effective_capabilities.is_some());
        }
        fixture.write(
            "proc/self/status",
            &format!("NoNewPrivs: 1\nSeccomp: 2\n{}", " ".repeat(65537)),
        );
        let report = probe(&fixture.0);
        assert!(report.no_new_privileges.is_none());
        assert_eq!(serde_json::to_value(report.seccomp).unwrap(), "unknown");
        fixture.write(
            "proc/self/status",
            "CapEff: malformed\nNoNewPrivs: 0\nSeccomp: 2\n",
        );
        let report = probe(&fixture.0);
        assert!(report.effective_capabilities.is_none());
        assert_eq!(report.no_new_privileges, Some(false));
        assert_eq!(report.seccomp, Seccomp::Filter);
        fixture.write(
            "proc/self/status",
            "NoNewPrivs: 1\nSeccomp: 2\nSeccomp: 2\n",
        );
        let report = probe(&fixture.0);
        assert_eq!(report.no_new_privileges, Some(true));
        assert_eq!(report.seccomp, Seccomp::Unknown);
    }

    #[test]
    fn tracing_permissions_are_reported_without_claiming_attach_support() {
        let fixture = Fixture::new();
        fixture.write(
            "proc/self/status",
            "Name:\tfixture\nCapEff:\t000000c000200000\n",
        );
        fixture.write(
            "sys/kernel/debug/tracing/events/sched/sched_process_exec/id",
            "123\n",
        );
        for policy in [-1, 0, 1, 2, 3, i32::MAX] {
            fixture.write("proc/sys/kernel/perf_event_paranoid", &policy.to_string());
            let report = probe(&fixture.0);
            assert_eq!(report.perf_event_paranoid, Some(policy));
            let capabilities = report.effective_capabilities.unwrap();
            assert!(capabilities.bpf && capabilities.perfmon && capabilities.sys_admin);
            assert_eq!(report.sched_process_exec, Prerequisite::Present);
            assert!(!report.sensor_running);
            assert!(!report.load_test_performed);
        }
        for (setting, expected) in [
            ("[none] integrity confidentiality", Lockdown::None),
            ("none [integrity] confidentiality", Lockdown::Integrity),
            (
                "none integrity [confidentiality]",
                Lockdown::Confidentiality,
            ),
            ("[none] [integrity]", Lockdown::Unknown),
        ] {
            fixture.write("sys/kernel/security/lockdown", setting);
            assert_eq!(probe(&fixture.0).lockdown, expected);
        }
    }

    #[test]
    fn malformed_permission_metadata_remains_unknown() {
        let fixture = Fixture::new();
        for status in [
            "CapEff: xyz",
            "CapEff: 0000000000000000\nCapEff: 000000c000200000",
            "CapEff: 10000000000000000",
            &" ".repeat(65537),
        ] {
            fixture.write("proc/self/status", status);
            assert!(probe(&fixture.0).effective_capabilities.is_none());
        }
        fixture.write("proc/self/status", "CapEff:\t0000000000000000\n");
        let report = probe(&fixture.0).effective_capabilities.unwrap();
        assert!(!report.bpf && !report.perfmon && !report.sys_admin);
        for policy in ["2147483648", "2 extra", "", &"0".repeat(4097)] {
            fixture.write("proc/sys/kernel/perf_event_paranoid", policy);
            assert!(probe(&fixture.0).perf_event_paranoid.is_none());
        }
        fixture.write("sys/kernel/tracing/events/sched/sched_process_exec/id", "0");
        assert_eq!(
            probe(&fixture.0).sched_process_exec,
            Prerequisite::Unreadable
        );
    }
}
