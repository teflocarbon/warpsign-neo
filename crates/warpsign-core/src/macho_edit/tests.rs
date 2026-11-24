//! Mach-O binaries are built byte by byte here: no real binaries in the repo

use super::*;

const ARM64: u32 = CPU_TYPE_ARM | CPU_ARCH_ABI64;
/// Where the first section starts: the end of the room for load commands
const TEXT_OFFSET: usize = 0x400;
const LINKEDIT_OFFSET: usize = 0x800;
const SIZE: usize = 0x1000;

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn name16(bytes: &mut Vec<u8>, name: &str) {
    let mut field = [0u8; 16];
    field[..name.len()].copy_from_slice(name.as_bytes());
    bytes.extend_from_slice(&field);
}

fn segment_64(name: &str, fileoff: u64, filesize: u64, section_offset: Option<u32>) -> Vec<u8> {
    let nsects = u32::from(section_offset.is_some());
    let mut c = Vec::new();
    push_u32(&mut c, LC_SEGMENT_64);
    push_u32(&mut c, 72 + 80 * nsects);
    name16(&mut c, name);
    push_u64(&mut c, 0x1_0000_0000 + fileoff); // vmaddr
    push_u64(&mut c, filesize); // vmsize
    push_u64(&mut c, fileoff);
    push_u64(&mut c, filesize);
    push_u32(&mut c, 5);
    push_u32(&mut c, 5);
    push_u32(&mut c, nsects);
    push_u32(&mut c, 0);
    if let Some(offset) = section_offset {
        name16(&mut c, "__text");
        name16(&mut c, name);
        push_u64(&mut c, 0x1_0000_0000 + u64::from(offset));
        push_u64(&mut c, 0x10);
        push_u32(&mut c, offset);
        push_u32(&mut c, 2); // align
        push_u32(&mut c, 0); // reloff
        push_u32(&mut c, 0); // nreloc
        push_u32(&mut c, 0x8000_0400); // flags
        push_u32(&mut c, 0);
        push_u32(&mut c, 0);
        push_u32(&mut c, 0);
    }
    c
}

fn segment_32(name: &str, fileoff: u32, filesize: u32, section_offset: u32) -> Vec<u8> {
    let mut c = Vec::new();
    push_u32(&mut c, LC_SEGMENT);
    push_u32(&mut c, 56 + 68);
    name16(&mut c, name);
    push_u32(&mut c, 0x4000 + fileoff);
    push_u32(&mut c, filesize);
    push_u32(&mut c, fileoff);
    push_u32(&mut c, filesize);
    push_u32(&mut c, 5);
    push_u32(&mut c, 5);
    push_u32(&mut c, 1);
    push_u32(&mut c, 0);
    name16(&mut c, "__text");
    name16(&mut c, name);
    push_u32(&mut c, 0x4000 + section_offset);
    push_u32(&mut c, 0x10);
    push_u32(&mut c, section_offset);
    // align, reloff, nreloc, flags, reserved1, reserved2
    for _ in 0..6 {
        push_u32(&mut c, 0);
    }
    c
}

fn dylib(cmd: u32, path: &str, align: usize) -> Vec<u8> {
    dylib_command(
        cmd,
        &Dylib {
            path: path.to_string(),
            timestamp: 2,
            current_version: 0x10000,
            compatibility_version: 0x10000,
        },
        align,
    )
}

fn linkedit_command(cmd: u32, offset: u32, size: u32) -> Vec<u8> {
    let mut c = Vec::new();
    push_u32(&mut c, cmd);
    push_u32(&mut c, 16);
    push_u32(&mut c, offset);
    push_u32(&mut c, size);
    c
}

fn symtab(symoff: u32, nsyms: u32) -> Vec<u8> {
    let mut c = Vec::new();
    push_u32(&mut c, LC_SYMTAB);
    push_u32(&mut c, 24);
    push_u32(&mut c, symoff);
    push_u32(&mut c, nsyms);
    push_u32(&mut c, 0);
    push_u32(&mut c, 0);
    c
}

fn encryption_info(cryptid: u32) -> Vec<u8> {
    let mut c = Vec::new();
    push_u32(&mut c, LC_ENCRYPTION_INFO_64);
    push_u32(&mut c, 24);
    push_u32(&mut c, TEXT_OFFSET as u32);
    push_u32(&mut c, 0x10);
    push_u32(&mut c, cryptid);
    push_u32(&mut c, 0);
    c
}

/// Undefined external nlist_64 entries importing from these ordinals
fn undefined_symbols(ordinals: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for &ordinal in ordinals {
        push_u32(&mut bytes, 0); // n_strx
        bytes.push(0x01); // N_UNDF | N_EXT
        bytes.push(0); // n_sect
        bytes.extend_from_slice(&(u16::from(ordinal) << 8).to_le_bytes());
        push_u64(&mut bytes, 0);
    }
    bytes
}

/// A thin 64-bit binary with these load commands, after __TEXT (section at
/// TEXT_OFFSET) and __LINKEDIT (at LINKEDIT_OFFSET, holding `linkedit`)
fn thin(cpu_type: u32, cpu_subtype: u32, commands: &[Vec<u8>], linkedit: &[u8]) -> Vec<u8> {
    let mut all = vec![
        segment_64(
            "__TEXT",
            0,
            LINKEDIT_OFFSET as u64,
            Some(TEXT_OFFSET as u32),
        ),
        segment_64(
            "__LINKEDIT",
            LINKEDIT_OFFSET as u64,
            (SIZE - LINKEDIT_OFFSET) as u64,
            None,
        ),
    ];
    all.extend(commands.iter().cloned());
    let mut bytes = Vec::new();
    push_u32(&mut bytes, MH_MAGIC_64);
    push_u32(&mut bytes, cpu_type);
    push_u32(&mut bytes, cpu_subtype);
    push_u32(&mut bytes, 2); // MH_EXECUTE
    push_u32(&mut bytes, all.len() as u32);
    push_u32(&mut bytes, all.iter().map(Vec::len).sum::<usize>() as u32);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, 0);
    for command in &all {
        bytes.extend_from_slice(command);
    }
    assert!(bytes.len() <= TEXT_OFFSET);
    bytes.resize(SIZE, 0);
    bytes[TEXT_OFFSET..TEXT_OFFSET + 4].copy_from_slice(&[0x1f, 0x20, 0x03, 0xd5]); // nop
    bytes[LINKEDIT_OFFSET..LINKEDIT_OFFSET + linkedit.len()].copy_from_slice(linkedit);
    bytes
}

/// An app binary: libSystem and Foo.framework used, Tweak.dylib injected
fn app() -> Vec<u8> {
    let symbols = undefined_symbols(&[1, 2, 2]);
    thin(
        ARM64,
        0,
        &[
            dylib(LC_LOAD_DYLIB, "/usr/lib/libSystem.B.dylib", 8),
            dylib(LC_LOAD_DYLIB, "@rpath/Foo.framework/Foo", 8),
            rpath_command("@executable_path/Frameworks", 8),
            symtab(LINKEDIT_OFFSET as u32, 3),
            dylib(LC_LOAD_WEAK_DYLIB, "@rpath/Tweak.dylib", 8),
        ],
        &symbols,
    )
}

fn paths(info: &SliceInfo) -> Vec<(&str, DylibKind)> {
    info.dylibs
        .iter()
        .map(|d| (d.dylib.path.as_str(), d.kind))
        .collect()
}

fn sizeofcmds(data: &[u8]) -> usize {
    u32_at(data, 20).unwrap() as usize
}

#[test]
fn reads_dylibs_ordinals_imports_and_rpaths() {
    let slices = read(&app()).unwrap();
    assert_eq!(slices.len(), 1);
    let info = &slices[0];
    assert_eq!(info.arch, "arm64");
    assert_eq!(
        paths(info),
        [
            ("/usr/lib/libSystem.B.dylib", DylibKind::Load),
            ("@rpath/Foo.framework/Foo", DylibKind::Load),
            ("@rpath/Tweak.dylib", DylibKind::Weak),
        ]
    );
    let ordinals: Vec<_> = info.dylibs.iter().map(|d| d.ordinal).collect();
    assert_eq!(ordinals, [1, 2, 3]);
    let imports: Vec<_> = info.dylibs.iter().map(|d| d.imports).collect();
    assert_eq!(imports, [Some(1), Some(2), Some(0)]);
    assert_eq!(info.dylibs[2].command_index, 6);
    assert_eq!(info.rpaths, ["@executable_path/Frameworks"]);
    assert!(!info.encrypted);
    assert_eq!(info.free_space, TEXT_OFFSET - 32 - sizeofcmds(&app()));
    assert_eq!(info.dylibs[0].dylib.current_version, 0x10000);
}

#[test]
fn adds_a_dylib_after_the_others() {
    let mut data = app();
    assert!(add_dylib(&mut data, "@rpath/New.dylib", DylibKind::Weak).unwrap());
    let info = &read(&data).unwrap()[0];
    let last = info.dylibs.last().unwrap();
    assert_eq!(last.dylib.path, "@rpath/New.dylib");
    assert_eq!(last.kind, DylibKind::Weak);
    assert_eq!(last.ordinal, 4);
    assert_eq!(last.dylib.timestamp, 2);
    // Appended as the last command; the old ones are untouched
    assert_eq!(last.command_index, info.commands.len() - 1);
    assert_eq!(
        data[..32 + sizeofcmds(&app())][32..],
        app()[32..32 + sizeofcmds(&app())]
    );
    // Commands are 8-byte aligned, and the rest of the file didn't move
    assert_eq!(sizeofcmds(&data) % 8, 0);
    assert_eq!(data[TEXT_OFFSET..], app()[TEXT_OFFSET..]);

    // Already there: nothing to do
    assert!(!add_dylib(&mut data, "@rpath/New.dylib", DylibKind::Weak).unwrap());
}

#[test]
fn adding_an_existing_dylib_changes_its_kind() {
    let mut data = app();
    assert!(add_dylib(&mut data, "@rpath/Tweak.dylib", DylibKind::Load).unwrap());
    let info = &read(&data).unwrap()[0];
    assert_eq!(info.dylibs.len(), 3);
    assert_eq!(info.dylibs[2].kind, DylibKind::Load);
    assert_eq!(info.dylibs[2].command_index, 6);
}

#[test]
fn refuses_when_there_is_no_room() {
    let mut data = app();
    let long = format!("@rpath/{}.dylib", "x".repeat(TEXT_OFFSET));
    let error = add_dylib(&mut data, &long, DylibKind::Weak).unwrap_err();
    assert!(matches!(error, MachoEditError::NoRoom { .. }), "{error}");
    assert_eq!(data, app());
}

#[test]
fn fills_the_free_space_exactly() {
    let mut data = app();
    let free = read(&data).unwrap()[0].free_space;
    // A command of exactly the free space: 24-byte header + path + NUL
    let path = format!("@rpath/{}", "y".repeat(free - 24 - 7 - 1));
    assert!(add_dylib(&mut data, &path, DylibKind::Weak).unwrap());
    assert_eq!(read(&data).unwrap()[0].free_space, 0);
}

#[test]
fn refuses_encrypted_binaries() {
    let mut data = thin(
        ARM64,
        0,
        &[
            encryption_info(1),
            dylib(LC_LOAD_DYLIB, "/usr/lib/libSystem.B.dylib", 8),
        ],
        &[],
    );
    assert!(read(&data).unwrap()[0].encrypted);
    let error = add_dylib(&mut data, "@rpath/Tweak.dylib", DylibKind::Weak).unwrap_err();
    assert!(matches!(error, MachoEditError::Encrypted { .. }), "{error}");

    // cryptid 0: decrypted, fine to edit
    let mut data = thin(ARM64, 0, &[encryption_info(0)], &[]);
    assert!(!read(&data).unwrap()[0].encrypted);
    assert!(add_dylib(&mut data, "@rpath/Tweak.dylib", DylibKind::Weak).unwrap());
}

#[test]
fn removes_an_unused_dylib_and_zeroes_the_space() {
    let mut data = app();
    assert!(remove_dylib(&mut data, "@rpath/Tweak.dylib", RemoveOptions::default()).unwrap());
    let info = &read(&data).unwrap()[0];
    assert_eq!(info.dylibs.len(), 2);
    let end = 32 + sizeofcmds(&data);
    assert!(end < 32 + sizeofcmds(&app()));
    assert!(data[end..TEXT_OFFSET].iter().all(|&b| b == 0));
    assert!(!remove_dylib(&mut data, "@rpath/Tweak.dylib", RemoveOptions::default()).unwrap());
}

#[test]
fn refuses_to_remove_a_dylib_in_use() {
    let mut data = app();
    let error = remove_dylib(
        &mut data,
        "@rpath/Foo.framework/Foo",
        RemoveOptions::default(),
    )
    .unwrap_err();
    assert!(
        matches!(error, MachoEditError::InUse { imports: 2, .. }),
        "{error}"
    );
    assert_eq!(data, app());

    let options = RemoveOptions {
        allow_in_use: true,
        ..Default::default()
    };
    assert!(remove_dylib(&mut data, "@rpath/Foo.framework/Foo", options).unwrap());
}

#[test]
fn refuses_to_renumber_used_libraries() {
    // An unused fix-up dylib loaded first, before libraries in use
    let symbols = undefined_symbols(&[2]);
    let binary = thin(
        ARM64,
        0,
        &[
            dylib(LC_LOAD_DYLIB, "@rpath/sideloadFixerLol.dylib", 8),
            dylib(LC_LOAD_DYLIB, "/usr/lib/libSystem.B.dylib", 8),
            symtab(LINKEDIT_OFFSET as u32, 1),
        ],
        &symbols,
    );
    let mut data = binary.clone();
    let error = remove_dylib(
        &mut data,
        "@rpath/sideloadFixerLol.dylib",
        RemoveOptions::default(),
    )
    .unwrap_err();
    assert!(
        matches!(error, MachoEditError::OrdinalShift { .. }),
        "{error}"
    );
    assert_eq!(data, binary);

    let options = RemoveOptions {
        allow_ordinal_shift: true,
        ..Default::default()
    };
    assert!(
        remove_dylibs_where(&mut data, options, |d| d.path.contains("sideloadFixerLol")).unwrap()
    );
    assert_eq!(read(&data).unwrap()[0].dylibs.len(), 1);
}

#[test]
fn changes_a_dylib_path() {
    let mut data = app();
    let old = "@rpath/Tweak.dylib";
    let new = "@executable_path/Frameworks/A-much-longer-name-for-the-tweak.dylib";
    assert_eq!(change_dylib(&mut data, old, new).unwrap(), 1);
    let info = &read(&data).unwrap()[0];
    assert_eq!(info.dylibs[2].dylib.path, new);
    assert_eq!(info.dylibs[2].kind, DylibKind::Weak);
    assert_eq!(info.dylibs[2].command_index, 6);
    assert_eq!(change_dylib(&mut data, old, new).unwrap(), 0);
}

#[test]
fn adds_and_removes_rpaths() {
    let mut data = app();
    assert!(!add_rpath(&mut data, "@executable_path/Frameworks").unwrap());
    assert!(add_rpath(&mut data, "@loader_path").unwrap());
    assert_eq!(
        read(&data).unwrap()[0].rpaths,
        ["@executable_path/Frameworks", "@loader_path"]
    );
    assert!(remove_rpath(&mut data, "@executable_path/Frameworks").unwrap());
    assert_eq!(read(&data).unwrap()[0].rpaths, ["@loader_path"]);
}

#[test]
fn sets_the_install_name() {
    let mut data = thin(
        ARM64,
        0,
        &[dylib(
            LC_ID_DYLIB,
            "/Library/MobileSubstrate/DynamicLibraries/Tweak.dylib",
            8,
        )],
        &[],
    );
    assert!(set_id(&mut data, "@rpath/Tweak.dylib").unwrap());
    let info = &read(&data).unwrap()[0];
    assert_eq!(info.id.as_ref().unwrap().path, "@rpath/Tweak.dylib");
    assert!(info.dylibs.is_empty());
    assert!(!set_id(&mut data, "@rpath/Tweak.dylib").unwrap());
    // A binary without an install name is left alone
    let mut data = app();
    assert!(!set_id(&mut data, "@rpath/Tweak.dylib").unwrap());
}

/// A 32-bit armv7 slice, with 4-byte-aligned commands
fn armv7() -> Vec<u8> {
    let commands = [
        segment_32("__TEXT", 0, 0x800, 0x300),
        dylib(LC_LOAD_DYLIB, "/usr/lib/libSystem.B.dylib", 4),
    ];
    let mut bytes = Vec::new();
    push_u32(&mut bytes, MH_MAGIC);
    push_u32(&mut bytes, CPU_TYPE_ARM);
    push_u32(&mut bytes, 9);
    push_u32(&mut bytes, 2);
    push_u32(&mut bytes, commands.len() as u32);
    push_u32(
        &mut bytes,
        commands.iter().map(Vec::len).sum::<usize>() as u32,
    );
    push_u32(&mut bytes, 0);
    for command in &commands {
        bytes.extend_from_slice(command);
    }
    bytes.resize(0x800, 0);
    bytes
}

fn fat(slices: &[(u32, u32, Vec<u8>)]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&FAT_MAGIC.to_be_bytes());
    bytes.extend_from_slice(&(slices.len() as u32).to_be_bytes());
    let mut offset = 0x4000;
    let mut bodies = Vec::new();
    for (cpu_type, cpu_subtype, body) in slices {
        for value in [*cpu_type, *cpu_subtype, offset, body.len() as u32, 14] {
            bytes.extend_from_slice(&value.to_be_bytes());
        }
        bodies.push((offset as usize, body));
        offset = (offset + body.len() as u32).next_multiple_of(0x4000);
    }
    for (offset, body) in bodies {
        bytes.resize(offset, 0);
        bytes.extend_from_slice(body);
    }
    bytes
}

#[test]
fn edits_every_slice_of_a_fat_binary() {
    let mut data = fat(&[(CPU_TYPE_ARM, 9, armv7()), (ARM64, 2, app())]);
    let before = read(&data).unwrap();
    assert_eq!(
        before.iter().map(|s| s.arch.as_str()).collect::<Vec<_>>(),
        ["armv7", "arm64e"]
    );
    assert_eq!(before[0].dylibs.len(), 1);
    // No import information in the armv7 slice
    assert_eq!(before[0].dylibs[0].imports, None);

    assert!(add_dylib(&mut data, "@rpath/Tweak.dylib", DylibKind::Weak).unwrap());
    let after = read(&data).unwrap();
    assert_eq!(
        after[0].dylibs.last().unwrap().dylib.path,
        "@rpath/Tweak.dylib"
    );
    assert_eq!(after[0].dylibs.len(), 2);
    // arm64e already had it, weak: unchanged
    assert_eq!(after[1].dylibs.len(), 3);
    let armv7_cmds = u32_at(&data[0x4000..], 20).unwrap() as usize;
    assert_eq!(armv7_cmds % 4, 0);
}

#[test]
fn a_failing_slice_leaves_the_whole_file_alone() {
    // The armv7 slice has room for little; the edit must not half-apply
    let mut data = fat(&[(ARM64, 0, app()), (CPU_TYPE_ARM, 9, armv7())]);
    let original = data.clone();
    let path = format!("@rpath/{}.dylib", "z".repeat(0x2c0));
    assert!(add_dylib(&mut data, &path, DylibKind::Weak).is_err());
    assert_eq!(data, original);
}

#[test]
fn counts_chained_fixup_imports() {
    // dyld_chained_fixups_header, then DYLD_CHAINED_IMPORT entries
    let mut fixups = Vec::new();
    for value in [0, 0, 28, 0, 4, 1, 0] {
        push_u32(&mut fixups, value);
    }
    for ordinal in [1u32, 2, 2, 0xff] {
        push_u32(&mut fixups, ordinal | (7 << 9));
    }
    let data = thin(
        ARM64,
        0,
        &[
            dylib(LC_LOAD_DYLIB, "/usr/lib/libSystem.B.dylib", 8),
            dylib(LC_LOAD_DYLIB, "@rpath/Foo.framework/Foo", 8),
            dylib(LC_LOAD_WEAK_DYLIB, "@rpath/Tweak.dylib", 8),
            linkedit_command(
                LC_DYLD_CHAINED_FIXUPS,
                LINKEDIT_OFFSET as u32,
                fixups.len() as u32,
            ),
        ],
        &fixups,
    );
    let info = &read(&data).unwrap()[0];
    let imports: Vec<_> = info.dylibs.iter().map(|d| d.imports).collect();
    assert_eq!(imports, [Some(1), Some(2), Some(0)]);
}

#[test]
fn counts_bind_opcodes() {
    let ops = [
        0x11, // SET_DYLIB_ORDINAL_IMM 1
        0x40, b'_', b'f', 0x00, // SET_SYMBOL_TRAILING_FLAGS_IMM "_f"
        0x51, // SET_TYPE_IMM pointer
        0x72, 0x10, // SET_SEGMENT_AND_OFFSET_ULEB seg 2, 0x10
        0x90, // DO_BIND
        0x20, 0x83, 0x01, // SET_DYLIB_ORDINAL_ULEB 131
        0x40, b'_', b'g', 0x00, 0xa0, 0x08, // DO_BIND_ADD_ADDR_ULEB
        0x3e, // SET_DYLIB_SPECIAL_IMM -2 (flat lookup): not a library
        0x90, 0x00, // DONE
    ];
    let mut counts = BTreeMap::new();
    assert!(count_bind_opcodes(&ops, &mut counts).is_some());
    assert_eq!(counts, BTreeMap::from([(1, 1), (131, 1)]));
}

#[test]
fn bind_opcodes_feed_dylib_imports() {
    let ops = [0x12, 0x40, b'_', b'x', 0x00, 0x90, 0x00];
    let mut info = Vec::new();
    push_u32(&mut info, LC_DYLD_INFO_ONLY);
    push_u32(&mut info, 48);
    for value in [
        0,
        0,
        LINKEDIT_OFFSET as u32,
        ops.len() as u32,
        0,
        0,
        0,
        0,
        0,
        0,
    ] {
        push_u32(&mut info, value);
    }
    let data = thin(
        ARM64,
        0,
        &[
            dylib(LC_LOAD_DYLIB, "/usr/lib/libSystem.B.dylib", 8),
            dylib(LC_LOAD_DYLIB, "@rpath/Foo.framework/Foo", 8),
            info,
        ],
        &ops,
    );
    let imports: Vec<_> = read(&data).unwrap()[0]
        .dylibs
        .iter()
        .map(|d| d.imports)
        .collect();
    assert_eq!(imports, [Some(0), Some(1)]);
}

#[test]
fn rejects_what_isnt_mach_o() {
    assert!(matches!(read(b"PK\x03\x04"), Err(MachoEditError::NotMachO)));
    assert!(matches!(read(&[]), Err(MachoEditError::NotMachO)));
    // A Java class file shares the FAT magic
    let class = [0xca, 0xfe, 0xba, 0xbe, 0x00, 0x00, 0x00, 0x34];
    assert!(matches!(read(&class), Err(MachoEditError::NotMachO)));
    assert!(is_macho(&app()));
    assert!(!is_macho(b"#!/bin/sh"));
}

#[test]
fn edits_files_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("App");
    std::fs::write(&path, app()).unwrap();
    assert!(
        edit_file(&path, |data| add_dylib(
            data,
            "@rpath/X.dylib",
            DylibKind::Weak
        ))
        .unwrap()
    );
    let info = &read_file(&path).unwrap()[0];
    assert_eq!(info.dylibs.last().unwrap().dylib.path, "@rpath/X.dylib");
    let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
    // No change: the file isn't rewritten
    assert!(
        !edit_file(&path, |data| add_dylib(
            data,
            "@rpath/X.dylib",
            DylibKind::Weak
        ))
        .unwrap()
    );
    assert_eq!(
        std::fs::metadata(&path).unwrap().modified().unwrap(),
        modified
    );
}
