use intellicord_observe_common::{EVENT_BYTES, ProcessEvent};
use serde::Serialize;

#[derive(Serialize)]
pub struct Replay {
    pub schema_version: u16,
    pub source: &'static str,
    pub sensor_running: bool,
    pub load_test_performed: bool,
    pub checks: [super::producer::Check; 4],
}

pub fn generated_replay() -> Replay {
    let event = ProcessEvent::exec(0x0403_0201, 0x0807_0605, 0x100f_0e0d_0c0b_0a09);
    let mut expected = [
        1, 0, 0, 0, 1, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
    ];
    if cfg!(target_endian = "big") {
        for field in expected[..16].as_chunks_mut::<4>().0 {
            field.reverse();
        }
        expected[16..].reverse();
    }
    let native = event.to_ne_bytes() == expected;
    let boundary = ProcessEvent::exec(u32::MAX, u32::MAX - 1, u64::MAX).to_ne_bytes();
    let full_width = ProcessEvent::decode(&boundary, u32::MAX).is_some_and(|decoded| {
        decoded.process_id == u32::MAX
            && decoded.thread_id == u32::MAX - 1
            && decoded.monotonic_ns == u64::MAX
    });
    let mut unaligned = [0; EVENT_BYTES + 1];
    unaligned[1..].copy_from_slice(&expected);
    let unaligned =
        ProcessEvent::decode(&unaligned[1..], event.process_id).is_some_and(|decoded| {
            decoded.thread_id == event.thread_id && decoded.monotonic_ns == event.monotonic_ns
        });
    let mut wrong_schema = expected;
    wrong_schema[..4].copy_from_slice(&2_u32.to_ne_bytes());
    let mut wrong_kind = expected;
    wrong_kind[4..8].copy_from_slice(&2_u32.to_ne_bytes());
    let rejection = [&wrong_schema[..], &wrong_kind[..], &expected[..23]]
        .into_iter()
        .all(|bytes| ProcessEvent::decode(bytes, event.process_id).is_none())
        && ProcessEvent::decode(&expected, event.process_id + 1).is_none()
        && ProcessEvent::decode(&ProcessEvent::exec(0, 1, 0).to_ne_bytes(), 0).is_none()
        && ProcessEvent::decode(&ProcessEvent::exec(1, 0, 0).to_ne_bytes(), 1).is_none();
    Replay {
        schema_version: 1,
        source: "generated_replay",
        sensor_running: false,
        load_test_performed: false,
        checks: [
            ("native_layout", native),
            ("full_width_fields", full_width),
            ("unaligned_decode", unaligned),
            ("invalid_record_rejection", rejection),
        ]
        .map(|(name, passed)| super::producer::Check { name, passed }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_report_is_bounded_inactive_and_deterministic() {
        let report = generated_replay();
        assert!(report.checks.iter().all(|check| check.passed));
        assert_eq!(
            report.checks.each_ref().map(|check| check.name),
            [
                "native_layout",
                "full_width_fields",
                "unaligned_decode",
                "invalid_record_rejection"
            ]
        );
        let value = serde_json::to_value(report).unwrap();
        assert_eq!(value, serde_json::to_value(generated_replay()).unwrap());
        assert_eq!(value.as_object().unwrap().len(), 5);
        assert_eq!(value["sensor_running"], false);
        assert_eq!(value["load_test_performed"], false);
        let bytes = serde_json::to_string(&value).unwrap();
        assert!(bytes.len() < 512);
        for forbidden in [
            "process_id",
            "thread_id",
            "monotonic_ns",
            "18446744073709551615",
        ] {
            assert!(!bytes.contains(forbidden));
        }
    }
}
