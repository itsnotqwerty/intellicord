use intellicord_observe_common::ProcessEvent;
use serde::Serialize;

pub mod attachment;
pub mod clock;
pub mod collection;
pub mod delivery;
pub mod events;
pub mod export;
pub mod handshake;
pub mod health;
pub mod lifecycle;
pub mod preflight;
pub mod producer;
pub mod progress;
pub mod record;
pub mod stream;
pub mod supervisor;
pub mod timed;
#[cfg(all(feature = "observe", target_os = "linux"))]
pub mod verification;

#[cfg(feature = "observe")]
pub mod status;

#[cfg(all(feature = "observe", target_os = "linux"))]
pub mod object;

#[cfg(all(feature = "observe", target_os = "linux"))]
pub mod target;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetStatus {
    Available,
    Unavailable,
    FeatureDisabled,
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct TargetCheck {
    pub schema_version: u16,
    pub source: &'static str,
    pub status: TargetStatus,
    pub target_selected: bool,
    pub load_test_performed: bool,
    pub sensor_running: bool,
}

pub fn target_check() -> TargetCheck {
    #[cfg(all(feature = "observe", target_os = "linux"))]
    let status =
        match target::TargetGuard::open(std::process::id()).and_then(|guard| guard.is_alive()) {
            Ok(true) => TargetStatus::Available,
            _ => TargetStatus::Unavailable,
        };
    #[cfg(all(not(feature = "observe"), target_os = "linux"))]
    let status = TargetStatus::FeatureDisabled;
    #[cfg(not(target_os = "linux"))]
    let status = TargetStatus::Unsupported;
    TargetCheck {
        schema_version: 1,
        source: "self_pidfd_probe",
        status,
        target_selected: false,
        load_test_performed: false,
        sensor_running: false,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ObjectCheck {
    pub schema_version: u16,
    pub inspector_available: bool,
    pub status: &'static str,
    pub load_test_performed: bool,
    pub sensor_running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub load_probe_available: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeInventory>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeInventory {
    pub schema_version: u16,
    pub functions: u32,
    pub instruction_slots: u32,
    pub map_lookup_calls: u32,
    pub clock_calls: u32,
    pub identity_calls: u32,
    pub ring_reserve_calls: u32,
    pub ring_submit_calls: u32,
    pub other_helper_calls: u32,
    pub local_calls: u32,
    pub indirect_calls: u32,
}

impl CodeInventory {
    pub fn valid(&self) -> bool {
        let calls = [
            self.map_lookup_calls,
            self.clock_calls,
            self.identity_calls,
            self.ring_reserve_calls,
            self.ring_submit_calls,
            self.other_helper_calls,
            self.local_calls,
            self.indirect_calls,
        ];
        self.schema_version == 1
            && self.functions > 0
            && self.functions <= 4096
            && self.instruction_slots >= self.functions
            && self.instruction_slots <= 524288
            && calls.into_iter().map(u64::from).sum::<u64>() <= u64::from(self.instruction_slots)
    }
}

impl ObjectCheck {
    fn new(inspector_available: bool, status: &'static str) -> Self {
        Self {
            schema_version: 1,
            inspector_available,
            status,
            load_test_performed: false,
            sensor_running: false,
            load_probe_available: Some(inspector_available),
            code: None,
        }
    }
}

pub fn object_check() -> ObjectCheck {
    #[cfg(all(feature = "observe", target_os = "linux"))]
    {
        let path = std::env::var_os("INTELLICORD_OBSERVE_OBJECT");
        object::check_path(path.as_deref().map(std::path::Path::new))
    }
    #[cfg(all(not(feature = "observe"), target_os = "linux"))]
    {
        ObjectCheck::new(false, "feature_disabled")
    }
    #[cfg(not(target_os = "linux"))]
    {
        ObjectCheck::new(false, "unsupported")
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessHealth {
    pub schema_version: u16,
    pub accepted: u64,
    pub rejected_record: u64,
    pub rejected_clock: u64,
    pub ring_dropped: u64,
}

pub struct ProcessMonitor {
    process_id: u32,
    last_monotonic_ns: Option<u64>,
    health: ProcessHealth,
}

impl ProcessMonitor {
    pub fn new(process_id: u32) -> Option<Self> {
        (process_id > 0).then_some(Self {
            process_id,
            last_monotonic_ns: None,
            health: ProcessHealth {
                schema_version: 1,
                ..ProcessHealth::default()
            },
        })
    }

    pub fn accept(&mut self, bytes: &[u8]) -> bool {
        let Some(event) = ProcessEvent::decode(bytes, self.process_id) else {
            self.health.rejected_record = self.health.rejected_record.saturating_add(1);
            return false;
        };
        if self
            .last_monotonic_ns
            .is_some_and(|last| event.monotonic_ns < last)
        {
            self.health.rejected_clock = self.health.rejected_clock.saturating_add(1);
            return false;
        }
        self.last_monotonic_ns = Some(event.monotonic_ns);
        self.health.accepted = self.health.accepted.saturating_add(1);
        true
    }

    pub fn health(&self) -> ProcessHealth {
        self.health
    }

    pub fn update_ring_dropped(&mut self, dropped: u64) {
        self.health.ring_dropped = self.health.ring_dropped.max(dropped);
    }
}

#[derive(Serialize)]
pub struct SelectionCheck {
    pub name: &'static str,
    pub passed: bool,
}

pub fn generated_selection_checks() -> [SelectionCheck; 7] {
    use intellicord_observe_common::selected_identity;
    [
        ("disabled_target", 0, 42, 42, None),
        ("foreign_tgid", 42, 43, 42, None),
        ("matching_leader", 42, 42, 42, Some((42, 42))),
        ("matching_thread", 42, 42, 43, Some((42, 43))),
        ("zero_process", 42, 0, 42, None),
        ("zero_thread", 42, 42, 0, None),
        (
            "full_width_identity",
            u32::MAX,
            u32::MAX,
            u32::MAX,
            Some((u32::MAX, u32::MAX)),
        ),
    ]
    .map(|(name, target, process, thread, expected)| SelectionCheck {
        name,
        passed: selected_identity(target, (u64::from(process) << 32) | u64::from(thread))
            == expected,
    })
}

pub fn generated_replay() -> ProcessHealth {
    let mut monitor = ProcessMonitor::new(42).expect("fixed generated process");
    for (process, time) in [(42_u32, 10_u64), (43, 20), (42, 9), (42, 30)] {
        let bytes = [
            1_u32.to_ne_bytes().as_slice(),
            1_u32.to_ne_bytes().as_slice(),
            process.to_ne_bytes().as_slice(),
            process.to_ne_bytes().as_slice(),
            time.to_ne_bytes().as_slice(),
        ]
        .concat();
        monitor.accept(&bytes);
    }
    monitor.accept(&[0]);
    monitor.update_ring_dropped(3);
    monitor.health()
}
