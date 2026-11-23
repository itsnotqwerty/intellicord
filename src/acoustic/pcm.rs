use super::{Format, FrameError, MAX_CHUNK_FRAMES};

pub struct PcmPlane<'buffer> {
    pub bytes: &'buffer [u8],
    pub offset_bytes: usize,
    pub stride_bytes: usize,
}

pub fn decode_pcm_planes(
    format: Format,
    encoding: PcmEncoding,
    samples_per_channel: usize,
    planes: &[PcmPlane<'_>],
) -> Result<Vec<f32>, FrameError> {
    if samples_per_channel == 0
        || samples_per_channel > format.sample_count() / format.channels * MAX_CHUNK_FRAMES
        || planes.len() != format.channels
    {
        return Err(FrameError::InvalidLength);
    }
    let width = encoding.sample_bytes();
    for plane in planes {
        let end = (samples_per_channel - 1)
            .checked_mul(plane.stride_bytes)
            .and_then(|distance| plane.offset_bytes.checked_add(distance))
            .and_then(|last| last.checked_add(width));
        if plane.stride_bytes < width || end.is_none_or(|end| end > plane.bytes.len()) {
            return Err(FrameError::InvalidLength);
        }
    }
    let mut samples = Vec::with_capacity(samples_per_channel * format.channels);
    for index in 0..samples_per_channel {
        for plane in planes {
            let start = plane.offset_bytes + index * plane.stride_bytes;
            let sample = encoding.decode(&plane.bytes[start..start + width]);
            if !sample.is_finite() || sample.abs() > 1.0 {
                return Err(FrameError::InvalidSample);
            }
            samples.push(sample);
        }
    }
    Ok(samples)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PcmEncoding {
    Signed16Le,
    Signed24Le,
    Signed32Le,
    Float32Le,
}

impl PcmEncoding {
    fn sample_bytes(self) -> usize {
        match self {
            Self::Signed16Le => 2,
            Self::Signed24Le => 3,
            Self::Signed32Le | Self::Float32Le => 4,
        }
    }

    fn decode(self, bytes: &[u8]) -> f32 {
        match self {
            Self::Signed16Le => f32::from(i16::from_le_bytes([bytes[0], bytes[1]])) / 32_768.0,
            Self::Signed24Le => {
                i32::from_le_bytes([0, bytes[0], bytes[1], bytes[2]]) as f32 / 2_147_483_648.0
            }
            Self::Signed32Le => {
                i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f32
                    / 2_147_483_648.0
            }
            Self::Float32Le => f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        }
    }
}

pub fn decode_pcm_chunk(
    format: Format,
    encoding: PcmEncoding,
    bytes: &[u8],
) -> Result<Vec<f32>, FrameError> {
    let sample_bytes = encoding.sample_bytes();
    let channel_group_bytes = sample_bytes * format.channels;
    if bytes.is_empty()
        || !bytes.len().is_multiple_of(channel_group_bytes)
        || bytes.len() / sample_bytes > format.sample_count() * MAX_CHUNK_FRAMES
    {
        return Err(FrameError::InvalidLength);
    }
    let mut samples = Vec::with_capacity(bytes.len() / sample_bytes);
    for encoded in bytes.chunks_exact(sample_bytes) {
        let sample = encoding.decode(encoded);
        if !sample.is_finite() || sample.abs() > 1.0 {
            return Err(FrameError::InvalidSample);
        }
        samples.push(sample);
    }
    Ok(samples)
}
