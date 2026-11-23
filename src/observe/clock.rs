#[cfg(all(feature = "observe", target_os = "linux"))]
use std::time::Instant;

use serde::Serialize;

use crate::acoustic::{ClockError, ClockInterval, SourceClockMapping};

pub const HORIZON_NS: u64 = 1_000_000_000;
pub const RATE_ERROR_PPM: u32 = 100;

pub struct KernelClockAnchor {
    mapping: SourceClockMapping,
    pub bracket_width_ns: u64,
}

impl KernelClockAnchor {
    pub fn from_bracket(source_ns: u64, before_ns: u64, after_ns: u64) -> Result<Self, ClockError> {
        Ok(Self {
            mapping: SourceClockMapping::new(
                source_ns,
                before_ns,
                after_ns,
                HORIZON_NS,
                RATE_ERROR_PPM,
            )?,
            bracket_width_ns: after_ns
                .checked_sub(before_ns)
                .ok_or(ClockError::InvalidAnchor)?,
        })
    }

    pub fn map(&self, source_ns: u64) -> Result<ClockInterval, ClockError> {
        self.mapping.map(source_ns)
    }
}

#[derive(Serialize)]
pub struct ClockCheck {
    pub schema_version: u16,
    pub source: &'static str,
    pub status: &'static str,
    pub clock_domain: &'static str,
    pub bracket_width_ns: Option<u64>,
    pub horizon_ns: u64,
    pub assumed_rate_error_ppm: u32,
    pub sensor_running: bool,
    pub load_test_performed: bool,
}

#[cfg(all(feature = "observe", target_os = "linux"))]
pub fn anchor(origin: Instant) -> Option<KernelClockAnchor> {
    let before = Instant::now().checked_duration_since(origin)?.as_nanos();
    let source = read_monotonic_ns()?;
    let after = Instant::now().checked_duration_since(origin)?.as_nanos();
    KernelClockAnchor::from_bracket(
        source,
        u64::try_from(before).ok()?,
        u64::try_from(after).ok()?,
    )
    .ok()
}

#[cfg(all(feature = "observe", target_os = "linux"))]
pub fn read_monotonic_ns() -> Option<u64> {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    let result = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) };
    if result != 0 || time.tv_sec < 0 || !(0..1_000_000_000).contains(&time.tv_nsec) {
        return None;
    }
    u64::try_from(
        u128::try_from(time.tv_sec).ok()? * 1_000_000_000 + u128::try_from(time.tv_nsec).ok()?,
    )
    .ok()
}

pub fn check() -> ClockCheck {
    #[cfg(all(feature = "observe", target_os = "linux"))]
    let (status, width) = match anchor(Instant::now()) {
        Some(anchor) => ("available", Some(anchor.bracket_width_ns)),
        None => ("unavailable", None),
    };
    #[cfg(all(not(feature = "observe"), target_os = "linux"))]
    let (status, width) = ("feature_disabled", None);
    #[cfg(not(target_os = "linux"))]
    let (status, width) = ("unsupported", None);
    ClockCheck {
        schema_version: 1,
        source: "local_kernel_clock_probe",
        status,
        clock_domain: intellicord_observe_common::CLOCK_DOMAIN,
        bracket_width_ns: width,
        horizon_ns: HORIZON_NS,
        assumed_rate_error_ppm: RATE_ERROR_PPM,
        sensor_running: false,
        load_test_performed: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_timestamps_keep_bracket_uncertainty_and_expire() {
        let anchor = KernelClockAnchor::from_bracket(10_000, 100, 120).unwrap();
        assert_eq!(anchor.bracket_width_ns, 20);
        assert_eq!(anchor.map(10_000).unwrap().earliest_ns, 100);
        assert_eq!(anchor.map(10_000).unwrap().latest_ns, 120);
        let mapped = anchor.map(20_001).unwrap();
        assert_eq!((mapped.earliest_ns, mapped.latest_ns), (10_099, 10_123));
        assert_eq!(anchor.map(9_999), Err(ClockError::BeforeAnchor));
        assert!(anchor.map(10_000 + HORIZON_NS).is_ok());
        assert_eq!(anchor.map(10_001 + HORIZON_NS), Err(ClockError::Expired));
        assert!(KernelClockAnchor::from_bracket(1, 2, 1).is_err());
        let overflow = KernelClockAnchor::from_bracket(0, u64::MAX - 1, u64::MAX).unwrap();
        assert_eq!(overflow.map(1), Err(ClockError::Overflow));
    }

    #[cfg(all(feature = "observe", target_os = "linux"))]
    #[test]
    fn actual_monotonic_read_maps_into_local_receipt_bracket() {
        let origin = Instant::now();
        assert!(super::anchor(origin + std::time::Duration::from_secs(60)).is_none());
        let anchor = anchor(origin).expect("Linux CLOCK_MONOTONIC");
        let before = origin.elapsed().as_nanos() as u64;
        let mut time = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        assert_eq!(
            unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) },
            0
        );
        let after = origin.elapsed().as_nanos() as u64;
        let source = time.tv_sec as u64 * 1_000_000_000 + time.tv_nsec as u64;
        let interval = anchor.map(source).unwrap();
        assert!(interval.earliest_ns <= after);
        assert!(interval.latest_ns >= before);
        assert!(interval.earliest_ns <= interval.latest_ns);
    }
}
