#![cfg(all(feature = "observe", target_os = "linux"))]

use intellicord::observe::object::{ObjectError, inspect_object};
use intellicord_observe_common::RING_BYTES;
use object::{
    Architecture, BinaryFormat, Endianness, SectionKind, SymbolFlags, SymbolKind, SymbolScope,
    write::{Object, Symbol, SymbolSection},
};

fn fixture(
    architecture: Architecture,
    ring_bytes: u32,
    program: &str,
    extra_map: bool,
    pinning: u32,
    flags: u32,
    invalid_symbol: bool,
) -> Vec<u8> {
    fixture_object(
        architecture,
        ring_bytes,
        program,
        extra_map,
        pinning,
        flags,
        invalid_symbol,
    )
    .write()
    .unwrap()
}

fn fixture_object(
    architecture: Architecture,
    ring_bytes: u32,
    program: &str,
    extra_map: bool,
    pinning: u32,
    flags: u32,
    invalid_symbol: bool,
) -> Object<'static> {
    let mut elf = Object::new(
        BinaryFormat::Elf,
        architecture,
        if cfg!(target_endian = "little") {
            Endianness::Little
        } else {
            Endianness::Big
        },
    );
    let code = elf.add_section(Vec::new(), b"tracepoint".to_vec(), SectionKind::Text);
    elf.append_section_data(code, &[0x95, 0, 0, 0, 0, 0, 0, 0], 8);
    elf.add_symbol(Symbol {
        name: program.as_bytes().to_vec(),
        value: 0,
        size: if invalid_symbol { 16 } else { 8 },
        kind: SymbolKind::Text,
        scope: SymbolScope::Linkage,
        weak: false,
        section: SymbolSection::Section(code),
        flags: SymbolFlags::None,
    });
    let maps = elf.add_section(Vec::new(), b"maps".to_vec(), SectionKind::Data);
    let mut definitions = vec![
        ("TARGET", 2, 4, 4, 1),
        ("EVENTS", 27, 0, 0, ring_bytes),
        ("DROPPED", 6, 4, 8, 1),
    ];
    if extra_map {
        definitions.push(("EXTRA", 2, 4, 4, 1));
    }
    for (name, kind, key, value, entries) in definitions {
        let mut data = Vec::new();
        for field in [kind, key, value, entries, flags, 0, pinning] {
            data.extend_from_slice(&field.to_ne_bytes());
        }
        let offset = elf.append_section_data(maps, &data, 4);
        elf.add_symbol(Symbol {
            name: name.as_bytes().to_vec(),
            value: offset,
            size: data.len() as u64,
            kind: SymbolKind::Data,
            scope: SymbolScope::Linkage,
            weak: false,
            section: SymbolSection::Section(maps),
            flags: SymbolFlags::None,
        });
    }
    elf
}

#[test]
fn instruction_inventory_classifies_calls_without_treating_them_as_verifier_proof() {
    let mut elf = fixture_object(
        Architecture::Bpf,
        RING_BYTES,
        "process_exec",
        false,
        0,
        0,
        false,
    );
    let symbol = elf.symbol_id(b"process_exec").unwrap();
    let SymbolSection::Section(section) = elf.symbol_mut(symbol).section else {
        panic!("code section")
    };
    let mut instructions = Vec::new();
    for (opcode, source, immediate) in [
        (0x85, 0, 1_i32),
        (0x85, 0, 5),
        (0x85, 0, 14),
        (0x85, 0, 131),
        (0x85, 0, 132),
        (0x85, 0, 999),
        (0x85, 1, -1),
        (0x85, 2, 0),
        (0x8d, 0, 0),
        (0x95, 0, 0),
    ] {
        instructions.extend_from_slice(&[opcode, source << 4, 0, 0]);
        instructions.extend_from_slice(&immediate.to_ne_bytes());
    }
    elf.symbol_mut(symbol).size = instructions.len() as u64 + 8;
    elf.section_mut(section).append_data(&instructions, 8);
    let code = inspect_object(&elf.write().unwrap()).unwrap().code;
    assert!(code.valid());
    assert_eq!((code.functions, code.instruction_slots), (1, 11));
    assert_eq!(
        (code.map_lookup_calls, code.clock_calls, code.identity_calls),
        (1, 1, 1)
    );
    assert_eq!((code.ring_reserve_calls, code.ring_submit_calls), (1, 1));
    assert_eq!(
        (
            code.other_helper_calls,
            code.local_calls,
            code.indirect_calls
        ),
        (1, 1, 2)
    );
}

#[test]
#[ignore = "requires an explicitly built BPF artifact via INTELLICORD_TEST_BPF_OBJECT"]
fn real_bpf_artifact_has_expected_instruction_inventory() {
    let path = std::env::var_os("INTELLICORD_TEST_BPF_OBJECT")
        .expect("set INTELLICORD_TEST_BPF_OBJECT to the freshly built sensor object");
    let bytes = intellicord::observe::object::read_object(std::path::Path::new(&path)).unwrap();
    let layout = inspect_object(&bytes).unwrap();
    let code = layout.code;
    assert!(code.valid());
    assert!(code.instruction_slots >= 16 && code.instruction_slots <= 4096);
    assert!(code.map_lookup_calls >= 2);
    assert!(code.clock_calls >= 1);
    assert!(code.identity_calls >= 1);
    assert!(code.ring_reserve_calls >= 1);
    assert!(code.ring_submit_calls >= 1);
    assert_eq!(code.other_helper_calls, 0);
    assert_eq!(code.indirect_calls, 0);
    assert!(!layout.load_test_performed);
    assert!(!layout.sensor_running);
}

#[test]
fn generated_elf_matches_layout_without_claiming_live_compatibility() {
    let report = inspect_object(&fixture(
        Architecture::Bpf,
        RING_BYTES,
        "process_exec",
        false,
        0,
        0,
        false,
    ))
    .unwrap();
    assert_eq!(report.map_count, 3);
    assert_eq!(report.program_count, 1);
    assert_eq!(report.ring_bytes, RING_BYTES);
    assert_eq!(report.expected_event_bytes, 24);
    assert!(report.code.valid());
    assert_eq!(report.code.functions, 1);
    assert_eq!(report.code.instruction_slots, 1);
    assert_eq!(report.code.clock_calls, 0);
    assert_eq!(report.code.other_helper_calls, 0);
    assert!(!report.sensor_running);
    assert!(!report.load_test_performed);
}

#[test]
fn wrong_architecture_program_resources_pinning_and_symbol_ranges_are_rejected() {
    for (bytes, error) in [
        (
            fixture(
                Architecture::X86_64,
                RING_BYTES,
                "process_exec",
                false,
                0,
                0,
                false,
            ),
            ObjectError::Architecture,
        ),
        (
            fixture(
                Architecture::Bpf,
                RING_BYTES,
                "unexpected",
                false,
                0,
                0,
                false,
            ),
            ObjectError::Program,
        ),
        (
            fixture(
                Architecture::Bpf,
                RING_BYTES * 2,
                "process_exec",
                false,
                0,
                0,
                false,
            ),
            ObjectError::Maps,
        ),
        (
            fixture(
                Architecture::Bpf,
                RING_BYTES,
                "process_exec",
                true,
                0,
                0,
                false,
            ),
            ObjectError::Maps,
        ),
        (
            fixture(
                Architecture::Bpf,
                RING_BYTES,
                "process_exec",
                false,
                1,
                0,
                false,
            ),
            ObjectError::Maps,
        ),
        (
            fixture(
                Architecture::Bpf,
                RING_BYTES,
                "process_exec",
                false,
                0,
                1,
                false,
            ),
            ObjectError::Maps,
        ),
        (
            fixture(
                Architecture::Bpf,
                RING_BYTES,
                "process_exec",
                false,
                0,
                0,
                true,
            ),
            ObjectError::Layout,
        ),
    ] {
        assert_eq!(inspect_object(&bytes), Err(error));
    }
}

#[test]
fn inspection_cli_and_capture_rejection_need_no_bpf_privileges() {
    use std::{
        io::{Seek, Write},
        process::Command,
    };
    let path = std::env::temp_dir().join(format!(
        "intellicord-object-{}-{}.elf",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    struct Remove(std::path::PathBuf);
    impl Drop for Remove {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _remove = Remove(path.clone());
    let mut file = std::fs::File::create_new(&path).unwrap();
    file.write_all(&fixture(
        Architecture::Bpf,
        RING_BYTES,
        "process_exec",
        false,
        0,
        0,
        false,
    ))
    .unwrap();
    let checked = intellicord::observe::object::check_path(Some(&path));
    assert_eq!(checked.status, "layout_matched");
    assert!(!checked.load_test_performed);
    let output = Command::new(env!("CARGO_BIN_EXE_intellicord-observe"))
        .args(["inspect", "--object"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    assert!(output.stdout.len() < 1024);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["source"], "object_metadata");
    assert_eq!(report["layout"]["map_count"], 3);
    assert_eq!(report["layout"]["sensor_running"], false);
    assert_eq!(report["layout"]["load_test_performed"], false);
    let check_helper = |expected: &str| {
        let output = Command::new(env!("CARGO_BIN_EXE_intellicord"))
            .arg("check-object")
            .env("INTELLICORD_OBSERVE_OBJECT", &path)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        assert!(output.stdout.len() < 2048);
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["outcome"], "completed");
        assert_eq!(report["helper_reaped"], true);
        assert_eq!(report["object"]["status"], expected);
        if expected == "layout_matched" {
            assert_eq!(report["object"]["code"]["instruction_slots"], 1);
            assert_eq!(report["object"]["code"]["functions"], 1);
        } else {
            assert!(report["object"].get("code").is_none());
        }
        assert_eq!(report["sensor_running"], false);
        assert_eq!(report["load_test_performed"], false);
        assert!(!String::from_utf8_lossy(&output.stdout).contains(path.to_str().unwrap()));
    };
    check_helper("layout_matched");
    file.set_len(0).unwrap();
    file.rewind().unwrap();
    file.write_all(&fixture(
        Architecture::Bpf,
        RING_BYTES * 2,
        "process_exec",
        false,
        0,
        0,
        false,
    ))
    .unwrap();
    assert_eq!(
        intellicord::observe::object::check_path(Some(&path)).status,
        "rejected"
    );
    check_helper("rejected");
    for arguments in [vec!["inspect", "--object"], vec!["capture", "--object"]] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_intellicord-observe"));
        command.args(&arguments).arg(&path);
        if arguments[0] == "capture" {
            command.args([
                "--pid",
                "1",
                "--seconds",
                "1",
                "--acknowledge-process-observation",
            ]);
        }
        let rejected = command.output().unwrap();
        assert!(!rejected.status.success());
        assert!(rejected.stdout.is_empty());
        let error = String::from_utf8_lossy(&rejected.stderr);
        assert!(error.contains("sensor object rejected: Maps"));
        assert!(!error.contains("OBSERVATION REQUESTED"));
        assert!(!error.contains("load trusted sensor object"));
    }
    assert_eq!(
        intellicord::observe::object::check_path(None).status,
        "not_configured"
    );
    assert_eq!(
        intellicord::observe::object::check_path(Some(std::path::Path::new("/dev/null"))).status,
        "unreadable"
    );
}
