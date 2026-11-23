#![no_std]
#![no_main]

use aya_ebpf::{
    helpers::{bpf_get_current_pid_tgid, bpf_ktime_get_ns},
    macros::{map, tracepoint},
    maps::{Array, PerCpuArray, RingBuf},
    programs::TracePointContext,
};
use intellicord_observe_common::{
    ProcessEvent, ProcessEventSink, RING_BYTES, next_ring_drop_count, publish_exec,
    submit_reserved_exec,
};

#[map]
static TARGET: Array<u32> = Array::with_max_entries(1, 0);

#[map]
static EVENTS: RingBuf = RingBuf::with_byte_size(RING_BYTES, 0);

#[map]
static DROPPED: PerCpuArray<u64> = PerCpuArray::with_max_entries(1, 0);

#[tracepoint]
pub fn process_exec(_context: TracePointContext) -> u32 {
    let identity = bpf_get_current_pid_tgid();
    let Some(target) = TARGET.get(0) else {
        return 0;
    };
    publish_exec(*target, identity, &mut KernelSink);
    0
}

struct KernelSink;

impl ProcessEventSink for KernelSink {
    #[inline(always)]
    fn try_submit(&mut self, process_id: u32, thread_id: u32) -> bool {
        submit_reserved_exec(
            EVENTS.reserve::<ProcessEvent>(0),
            process_id,
            thread_id,
            || unsafe { bpf_ktime_get_ns() },
            |mut slot, event| {
                slot.write(event);
                slot.submit(0);
            },
        )
    }

    #[inline(always)]
    fn record_drop(&mut self) {
        if let Some(counter) = DROPPED.get_ptr_mut(0) {
            unsafe {
                *counter = next_ring_drop_count(*counter);
            }
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
