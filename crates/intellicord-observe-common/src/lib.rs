#![no_std]

pub const SCHEMA_VERSION: u32 = 1;
pub const PROCESS_EXEC: u32 = 1;
pub const EVENT_BYTES: usize = 24;
pub const RING_BYTES: u32 = 65536;
pub const CLOCK_DOMAIN: &str = "linux_monotonic";

#[inline(always)]
pub const fn next_ring_drop_count(current: u64) -> u64 {
    current.saturating_add(1)
}

pub fn ring_drop_total(counters: impl IntoIterator<Item = u64>) -> u64 {
    counters.into_iter().fold(0_u64, u64::saturating_add)
}

#[inline(always)]
pub fn selected_identity(target: u32, pid_tgid: u64) -> Option<(u32, u32)> {
    let process_id = (pid_tgid >> 32) as u32;
    let thread_id = pid_tgid as u32;
    if target == 0 || process_id != target || thread_id == 0 {
        return None;
    }
    Some((process_id, thread_id))
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ProcessEvent {
    pub schema_version: u32,
    pub kind: u32,
    pub process_id: u32,
    pub thread_id: u32,
    pub monotonic_ns: u64,
}

pub trait ProcessEventSink {
    fn try_submit(&mut self, process_id: u32, thread_id: u32) -> bool;
    fn record_drop(&mut self);
}

#[inline(always)]
pub fn submit_reserved_exec<Reservation>(
    reservation: Option<Reservation>,
    process_id: u32,
    thread_id: u32,
    clock: impl FnOnce() -> u64,
    submit: impl FnOnce(Reservation, ProcessEvent),
) -> bool {
    let Some(reservation) = reservation else {
        return false;
    };
    let event = ProcessEvent::exec(process_id, thread_id, clock());
    submit(reservation, event);
    true
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublishOutcome {
    Filtered,
    Submitted,
    Dropped,
}

#[inline(always)]
pub fn publish_exec(
    target: u32,
    pid_tgid: u64,
    sink: &mut impl ProcessEventSink,
) -> PublishOutcome {
    let Some((process_id, thread_id)) = selected_identity(target, pid_tgid) else {
        return PublishOutcome::Filtered;
    };
    if sink.try_submit(process_id, thread_id) {
        PublishOutcome::Submitted
    } else {
        sink.record_drop();
        PublishOutcome::Dropped
    }
}

const _: () = assert!(core::mem::size_of::<ProcessEvent>() == EVENT_BYTES);
const _: () = assert!(core::mem::offset_of!(ProcessEvent, schema_version) == 0);
const _: () = assert!(core::mem::offset_of!(ProcessEvent, kind) == 4);
const _: () = assert!(core::mem::offset_of!(ProcessEvent, process_id) == 8);
const _: () = assert!(core::mem::offset_of!(ProcessEvent, thread_id) == 12);
const _: () = assert!(core::mem::offset_of!(ProcessEvent, monotonic_ns) == 16);

impl ProcessEvent {
    #[inline(always)]
    pub const fn exec(process_id: u32, thread_id: u32, monotonic_ns: u64) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            kind: PROCESS_EXEC,
            process_id,
            thread_id,
            monotonic_ns,
        }
    }

    pub fn to_ne_bytes(self) -> [u8; EVENT_BYTES] {
        let mut bytes = [0; EVENT_BYTES];
        bytes[0..4].copy_from_slice(&self.schema_version.to_ne_bytes());
        bytes[4..8].copy_from_slice(&self.kind.to_ne_bytes());
        bytes[8..12].copy_from_slice(&self.process_id.to_ne_bytes());
        bytes[12..16].copy_from_slice(&self.thread_id.to_ne_bytes());
        bytes[16..24].copy_from_slice(&self.monotonic_ns.to_ne_bytes());
        bytes
    }

    pub fn decode(bytes: &[u8], expected_process: u32) -> Option<Self> {
        if bytes.len() != EVENT_BYTES || expected_process == 0 {
            return None;
        }
        let event = Self {
            schema_version: u32::from_ne_bytes(bytes[0..4].try_into().ok()?),
            kind: u32::from_ne_bytes(bytes[4..8].try_into().ok()?),
            process_id: u32::from_ne_bytes(bytes[8..12].try_into().ok()?),
            thread_id: u32::from_ne_bytes(bytes[12..16].try_into().ok()?),
            monotonic_ns: u64::from_ne_bytes(bytes[16..24].try_into().ok()?),
        };
        (event.schema_version == SCHEMA_VERSION
            && event.kind == PROCESS_EXEC
            && event.process_id == expected_process
            && event.thread_id != 0)
            .then_some(event)
    }
}
