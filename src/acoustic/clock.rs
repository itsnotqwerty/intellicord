use serde::Serialize;

use super::Format;

const PARTS_PER_MILLION: u64 = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClockError {
    InvalidAnchor,
    BeforeAnchor,
    Expired,
    Overflow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ClockInterval {
    pub schema_version: u16,
    pub earliest_ns: u64,
    pub latest_ns: u64,
}

pub struct SourceClockMapping {
    source_anchor_ns: u64,
    local_before_ns: u64,
    local_after_ns: u64,
    max_elapsed_ns: u64,
    rate_error_ppm: u32,
}

pub struct SampleClockMapping {
    source: SourceClockMapping,
    source_origin_ns: u64,
    format: Format,
}

impl SampleClockMapping {
    pub(super) fn format(&self) -> Format {
        self.format
    }

    pub fn new(
        source: SourceClockMapping,
        format: Format,
        source_origin_ns: u64,
    ) -> Result<Self, ClockError> {
        source.map(source_origin_ns)?;
        Ok(Self {
            source,
            source_origin_ns,
            format,
        })
    }

    pub fn map(&self, sample_per_channel: u64) -> Result<ClockInterval, ClockError> {
        let numerator = u128::from(sample_per_channel) * 1_000_000_000;
        let rate = u128::from(self.format.sample_rate);
        let origin = u128::from(self.source_origin_ns);
        let floor = u64::try_from(origin + numerator / rate).map_err(|_| ClockError::Overflow)?;
        let ceil =
            u64::try_from(origin + numerator.div_ceil(rate)).map_err(|_| ClockError::Overflow)?;
        let earliest = self.source.map(floor)?;
        let latest = self.source.map(ceil)?;
        Ok(ClockInterval {
            schema_version: 1,
            earliest_ns: earliest.earliest_ns,
            latest_ns: latest.latest_ns,
        })
    }
}

impl SourceClockMapping {
    pub fn new(
        source_anchor_ns: u64,
        local_before_ns: u64,
        local_after_ns: u64,
        max_elapsed_ns: u64,
        rate_error_ppm: u32,
    ) -> Result<Self, ClockError> {
        if local_before_ns > local_after_ns
            || max_elapsed_ns == 0
            || u64::from(rate_error_ppm) > PARTS_PER_MILLION
        {
            return Err(ClockError::InvalidAnchor);
        }
        Ok(Self {
            source_anchor_ns,
            local_before_ns,
            local_after_ns,
            max_elapsed_ns,
            rate_error_ppm,
        })
    }

    pub fn map(&self, source_ns: u64) -> Result<ClockInterval, ClockError> {
        let elapsed = source_ns
            .checked_sub(self.source_anchor_ns)
            .ok_or(ClockError::BeforeAnchor)?;
        if elapsed > self.max_elapsed_ns {
            return Err(ClockError::Expired);
        }
        let elapsed = u128::from(elapsed);
        let drift =
            (elapsed * u128::from(self.rate_error_ppm)).div_ceil(u128::from(PARTS_PER_MILLION));
        let earliest = u128::from(self.local_before_ns) + elapsed - drift;
        let latest = u128::from(self.local_after_ns) + elapsed + drift;
        Ok(ClockInterval {
            schema_version: 1,
            earliest_ns: u64::try_from(earliest).map_err(|_| ClockError::Overflow)?,
            latest_ns: u64::try_from(latest).map_err(|_| ClockError::Overflow)?,
        })
    }
}
