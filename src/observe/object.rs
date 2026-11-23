use aya_obj::{Object as AyaObject, ProgramSection};
use intellicord_observe_common::{EVENT_BYTES, RING_BYTES};
use object::{Architecture, BinaryFormat, Object, ObjectKind, ObjectSection, ObjectSymbol};
use serde::Serialize;

pub const OBJECT_LIMIT: usize = 4 * 1024 * 1024;

pub fn check_path(path: Option<&std::path::Path>) -> super::ObjectCheck {
    if let Some(path) = path {
        return match read_object(path) {
            Err(_) => super::ObjectCheck::new(true, "unreadable"),
            Ok(bytes) => match inspect_object(&bytes) {
                Ok(layout) => {
                    let mut check = super::ObjectCheck::new(true, "layout_matched");
                    check.code = Some(layout.code);
                    check
                }
                Err(_) => super::ObjectCheck::new(true, "rejected"),
            },
        };
    }
    super::ObjectCheck::new(true, "not_configured")
}

pub fn read_object(path: &std::path::Path) -> anyhow::Result<Vec<u8>> {
    use anyhow::ensure;
    use std::{fs::OpenOptions, io::Read, os::unix::fs::OpenOptionsExt};
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)?;
    ensure!(file.metadata()?.is_file(), "object must be a regular file");
    let mut bytes = Vec::new();
    file.take(OBJECT_LIMIT as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(
        !bytes.is_empty() && bytes.len() <= OBJECT_LIMIT,
        "object must be 1..4 MiB"
    );
    Ok(bytes)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectError {
    Size,
    InvalidElf,
    Architecture,
    Layout,
    Program,
    Maps,
}

impl std::fmt::Display for ObjectError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "sensor object rejected: {self:?}")
    }
}

impl std::error::Error for ObjectError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ObjectLayout {
    pub schema_version: u16,
    pub program_count: usize,
    pub map_count: usize,
    pub ring_bytes: u32,
    pub expected_event_bytes: usize,
    pub load_test_performed: bool,
    pub sensor_running: bool,
    pub code: super::CodeInventory,
}

pub fn inspect_object(bytes: &[u8]) -> Result<ObjectLayout, ObjectError> {
    if bytes.is_empty() || bytes.len() > OBJECT_LIMIT {
        return Err(ObjectError::Size);
    }
    let elf = object::File::parse(bytes).map_err(|_| ObjectError::InvalidElf)?;
    if elf.format() != BinaryFormat::Elf
        || elf.architecture() != Architecture::Bpf
        || elf.kind() != ObjectKind::Relocatable
        || !elf.is_64()
        || elf.is_little_endian() != cfg!(target_endian = "little")
    {
        return Err(ObjectError::Architecture);
    }
    if elf.sections().count() > 64
        || elf.symbols().count() > 4096
        || elf
            .sections()
            .any(|section| section.size() > OBJECT_LIMIT as u64 || section.data().is_err())
    {
        return Err(ObjectError::Layout);
    }
    for symbol in elf.symbols() {
        if let Some(index) = symbol.section_index() {
            let section = elf
                .section_by_index(index)
                .map_err(|_| ObjectError::Layout)?;
            let end = symbol
                .address()
                .checked_sub(section.address())
                .and_then(|offset| offset.checked_add(symbol.size()))
                .ok_or(ObjectError::Layout)?;
            if end > section.size() {
                return Err(ObjectError::Layout);
            }
        }
    }
    let parsed = AyaObject::parse(bytes).map_err(|_| ObjectError::InvalidElf)?;
    if parsed.programs.len() != 1
        || !parsed
            .programs
            .get("process_exec")
            .is_some_and(|program| matches!(program.section, ProgramSection::TracePoint))
    {
        return Err(ObjectError::Program);
    }
    if parsed.maps.len() != 3 {
        return Err(ObjectError::Maps);
    }
    for (name, kind, key, value, entries) in [
        ("TARGET", 2, 4, 4, 1),
        ("EVENTS", 27, 0, 0, RING_BYTES),
        ("DROPPED", 6, 4, 8, 1),
    ] {
        let map = parsed.maps.get(name).ok_or(ObjectError::Maps)?;
        if (
            map.map_type(),
            map.key_size(),
            map.value_size(),
            map.max_entries(),
        ) != (kind, key, value, entries)
            || map.map_flags() != 0
            || map.pinning() as u32 != 0
        {
            return Err(ObjectError::Maps);
        }
    }
    let mut code = super::CodeInventory {
        schema_version: 1,
        functions: u32::try_from(parsed.functions.len()).map_err(|_| ObjectError::Program)?,
        ..super::CodeInventory::default()
    };
    for function in parsed.functions.values() {
        if function.instructions.is_empty() {
            return Err(ObjectError::Program);
        }
        for instruction in &function.instructions {
            code.instruction_slots = code
                .instruction_slots
                .checked_add(1)
                .ok_or(ObjectError::Program)?;
            if instruction.code == 0x85 {
                match instruction.src_reg() {
                    0 => match instruction.imm {
                        1 => code.map_lookup_calls += 1,
                        5 => code.clock_calls += 1,
                        14 => code.identity_calls += 1,
                        131 => code.ring_reserve_calls += 1,
                        132 => code.ring_submit_calls += 1,
                        _ => code.other_helper_calls += 1,
                    },
                    1 => code.local_calls += 1,
                    _ => code.indirect_calls += 1,
                }
            } else if instruction.code == 0x8d {
                code.indirect_calls += 1;
            }
        }
    }
    if !code.valid() {
        return Err(ObjectError::Program);
    }
    Ok(ObjectLayout {
        schema_version: 1,
        program_count: 1,
        map_count: 3,
        ring_bytes: RING_BYTES,
        expected_event_bytes: EVENT_BYTES,
        load_test_performed: false,
        sensor_running: false,
        code,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_oversized_and_non_elf_input_are_rejected_without_kernel_access() {
        assert_eq!(inspect_object(&[]), Err(ObjectError::Size));
        assert_eq!(
            inspect_object(&vec![0; OBJECT_LIMIT + 1]),
            Err(ObjectError::Size)
        );
        assert_eq!(inspect_object(b"not an ELF"), Err(ObjectError::InvalidElf));
    }
}
