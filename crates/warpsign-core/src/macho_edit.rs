//! Reading and editing Mach-O load commands: the dylibs a binary loads, its
//! rpaths and install name, as `otool -L`, `install_name_tool` and
//! `insert_dylib` do. Used to inject tweaks (and Python warpsign's own fix
//! dylibs), and to find and remove injected ones (docs/design/tweaks.md).
//!
//! Every edit is made in place, in each slice of a thin or FAT binary, by
//! rewriting the load commands within the space the linker left between
//! them and the first section (`-headerpad`). Nothing else in the file
//! moves, so offsets stay valid; an edit that doesn't fit fails rather than
//! moving segments. The code signature isn't touched: it's invalid after an
//! edit, and signing replaces it.

use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum MachoEditError {
    #[error("not a Mach-O binary")]
    NotMachO,
    #[error("truncated or malformed Mach-O: {0}")]
    Malformed(String),
    #[error("{0} slices aren't supported")]
    Unsupported(String),
    #[error("the {arch} slice is still FairPlay-encrypted: decrypt it first")]
    Encrypted { arch: String },
    #[error(
        "no room for the load commands in the {arch} slice: {needed} bytes needed, {available} available"
    )]
    NoRoom {
        arch: String,
        needed: usize,
        available: usize,
    },
    #[error(
        "{path} is used by the {arch} slice ({imports} imported symbols): removing it would break the binary"
    )]
    InUse {
        path: String,
        arch: String,
        imports: usize,
    },
    #[error(
        "removing {path} from the {arch} slice would renumber libraries the binary imports symbols from"
    )]
    OrdinalShift { path: String, arch: String },
    #[error("{path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
}

type Result<T> = std::result::Result<T, MachoEditError>;

const MH_MAGIC: u32 = 0xfeed_face;
const MH_MAGIC_64: u32 = 0xfeed_facf;
const FAT_MAGIC: u32 = 0xcafe_babe;
const FAT_MAGIC_64: u32 = 0xcafe_babf;

const LC_REQ_DYLD: u32 = 0x8000_0000;
const LC_SEGMENT: u32 = 0x1;
const LC_SYMTAB: u32 = 0x2;
const LC_LOAD_DYLIB: u32 = 0xc;
const LC_ID_DYLIB: u32 = 0xd;
const LC_SEGMENT_64: u32 = 0x19;
const LC_LOAD_WEAK_DYLIB: u32 = 0x18 | LC_REQ_DYLD;
const LC_RPATH: u32 = 0x1c | LC_REQ_DYLD;
const LC_REEXPORT_DYLIB: u32 = 0x1f | LC_REQ_DYLD;
const LC_LAZY_LOAD_DYLIB: u32 = 0x20;
const LC_ENCRYPTION_INFO: u32 = 0x21;
const LC_DYLD_INFO: u32 = 0x22;
const LC_DYLD_INFO_ONLY: u32 = 0x22 | LC_REQ_DYLD;
const LC_LOAD_UPWARD_DYLIB: u32 = 0x23 | LC_REQ_DYLD;
const LC_ENCRYPTION_INFO_64: u32 = 0x2c;
const LC_DYLD_CHAINED_FIXUPS: u32 = 0x34 | LC_REQ_DYLD;

const CPU_ARCH_ABI64: u32 = 0x0100_0000;
const CPU_TYPE_X86: u32 = 7;
const CPU_TYPE_ARM: u32 = 12;

/// Timestamp written in new dylib commands, as ld and insert_dylib write
const DEFAULT_TIMESTAMP: u32 = 2;

/// How a binary links a dylib
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DylibKind {
    /// `LC_LOAD_DYLIB`: the binary won't load without it
    Load,
    /// `LC_LOAD_WEAK_DYLIB`: loaded if it's there
    Weak,
    /// `LC_REEXPORT_DYLIB`
    Reexport,
    /// `LC_LAZY_LOAD_DYLIB`
    LazyLoad,
    /// `LC_LOAD_UPWARD_DYLIB`
    Upward,
}

impl DylibKind {
    fn from_cmd(cmd: u32) -> Option<Self> {
        match cmd {
            LC_LOAD_DYLIB => Some(Self::Load),
            LC_LOAD_WEAK_DYLIB => Some(Self::Weak),
            LC_REEXPORT_DYLIB => Some(Self::Reexport),
            LC_LAZY_LOAD_DYLIB => Some(Self::LazyLoad),
            LC_LOAD_UPWARD_DYLIB => Some(Self::Upward),
            _ => None,
        }
    }

    fn cmd(self) -> u32 {
        match self {
            Self::Load => LC_LOAD_DYLIB,
            Self::Weak => LC_LOAD_WEAK_DYLIB,
            Self::Reexport => LC_REEXPORT_DYLIB,
            Self::LazyLoad => LC_LAZY_LOAD_DYLIB,
            Self::Upward => LC_LOAD_UPWARD_DYLIB,
        }
    }
}

/// A dylib a slice loads, or its own install name (`LC_ID_DYLIB`)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dylib {
    pub path: String,
    pub timestamp: u32,
    pub current_version: u32,
    pub compatibility_version: u32,
}

/// A loaded dylib, as listed in the load commands
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedDylib {
    pub kind: DylibKind,
    pub dylib: Dylib,
    /// Its library ordinal: 1 for the first dylib loaded, and so on, as
    /// symbol imports refer to it
    pub ordinal: usize,
    /// Position among all the slice's load commands
    pub command_index: usize,
    /// Symbols the slice imports from it (two-level namespace binds,
    /// chained-fixup imports and undefined symbols, counted together).
    /// None when the binary has no import information to go by.
    pub imports: Option<usize>,
}

/// What one slice's load commands say
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SliceInfo {
    /// e.g. "arm64", "arm64e", "armv7"
    pub arch: String,
    pub cpu_type: u32,
    pub cpu_subtype: u32,
    /// Offset of the slice in the file
    pub offset: usize,
    pub size: usize,
    /// `LC_ID_DYLIB`, for a dylib
    pub id: Option<Dylib>,
    pub dylibs: Vec<LoadedDylib>,
    pub rpaths: Vec<String>,
    /// Command types in order, for telling linker-written from appended ones
    pub commands: Vec<u32>,
    /// FairPlay encryption still in place (cryptid != 0)
    pub encrypted: bool,
    /// Bytes free for more load commands
    pub free_space: usize,
}

/// One slice's bounds in the file
#[derive(Debug, Clone, Copy)]
struct SliceRange {
    offset: usize,
    size: usize,
    cpu_type: u32,
    cpu_subtype: u32,
}

/// A parsed slice: header fields and its load commands
struct Slice {
    is_64: bool,
    header_size: usize,
    ncmds_offset: usize,
    /// (cmd, offset within the slice, cmdsize)
    commands: Vec<(u32, usize, usize)>,
    /// Where load commands must end: the first section or segment content
    limit: usize,
    arch: String,
    range: SliceRange,
}

impl Slice {
    fn command_end(&self) -> usize {
        self.commands
            .last()
            .map(|&(_, offset, size)| offset + size)
            .unwrap_or(self.header_size)
    }

    fn alignment(&self) -> usize {
        if self.is_64 { 8 } else { 4 }
    }
}

fn u16_at(data: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        data.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn u32_at(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        data.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn u64_at(data: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        data.get(offset..offset + 8)?.try_into().ok()?,
    ))
}

fn u32_be(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        data.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn u64_be(data: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_be_bytes(
        data.get(offset..offset + 8)?.try_into().ok()?,
    ))
}

fn malformed(message: impl Into<String>) -> MachoEditError {
    MachoEditError::Malformed(message.into())
}

/// A NUL-terminated string at `offset` within `data`
fn c_string(data: &[u8], offset: usize) -> Option<String> {
    let bytes = data.get(offset..)?;
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    Some(String::from_utf8_lossy(&bytes[..end]).into_owned())
}

/// Architecture name, as `lipo` prints it
fn arch_name(cpu_type: u32, cpu_subtype: u32) -> String {
    let subtype = cpu_subtype & 0x00ff_ffff;
    match cpu_type {
        t if t == CPU_TYPE_ARM | CPU_ARCH_ABI64 => match subtype {
            2 => "arm64e".to_string(),
            _ => "arm64".to_string(),
        },
        CPU_TYPE_ARM => match subtype {
            9 => "armv7".to_string(),
            11 => "armv7s".to_string(),
            _ => format!("arm(subtype {subtype})"),
        },
        t if t == CPU_TYPE_X86 | CPU_ARCH_ABI64 => "x86_64".to_string(),
        CPU_TYPE_X86 => "i386".to_string(),
        _ => format!("cpu {cpu_type:#x}"),
    }
}

/// Whether the data starts like a Mach-O binary (thin or FAT)
pub fn is_macho(data: &[u8]) -> bool {
    matches!(u32_at(data, 0), Some(MH_MAGIC | MH_MAGIC_64))
        || matches!(u32_be(data, 0), Some(FAT_MAGIC | FAT_MAGIC_64))
}

fn slice_ranges(data: &[u8]) -> Result<Vec<SliceRange>> {
    let magic_be = u32_be(data, 0).ok_or(MachoEditError::NotMachO)?;
    let (entry_size, is_64) = match magic_be {
        FAT_MAGIC => (20, false),
        FAT_MAGIC_64 => (32, true),
        _ => {
            return match u32_at(data, 0) {
                Some(MH_MAGIC | MH_MAGIC_64) => Ok(vec![SliceRange {
                    offset: 0,
                    size: data.len(),
                    cpu_type: u32_at(data, 4).ok_or_else(|| malformed("truncated header"))?,
                    cpu_subtype: u32_at(data, 8).ok_or_else(|| malformed("truncated header"))?,
                }]),
                // Big-endian thin binaries (PowerPC) never reach iOS
                _ if matches!(magic_be, MH_MAGIC | MH_MAGIC_64) => {
                    Err(MachoEditError::Unsupported("big-endian".into()))
                }
                _ => Err(MachoEditError::NotMachO),
            };
        }
    };
    let count = u32_be(data, 4).ok_or_else(|| malformed("truncated FAT header"))? as usize;
    // Java class files share FAT_MAGIC; they have a large "count"
    if count == 0 || count > 32 || 8 + count * entry_size > data.len() {
        return Err(MachoEditError::NotMachO);
    }
    let mut ranges = Vec::with_capacity(count);
    for i in 0..count {
        let base = 8 + i * entry_size;
        let cpu_type = u32_be(data, base).unwrap_or(0);
        let cpu_subtype = u32_be(data, base + 4).unwrap_or(0);
        let (offset, size) = if is_64 {
            (
                u64_be(data, base + 8).unwrap_or(0) as usize,
                u64_be(data, base + 16).unwrap_or(0) as usize,
            )
        } else {
            (
                u32_be(data, base + 8).unwrap_or(0) as usize,
                u32_be(data, base + 12).unwrap_or(0) as usize,
            )
        };
        if offset.checked_add(size).is_none_or(|end| end > data.len()) {
            return Err(malformed(format!(
                "FAT slice {i} runs past the end of the file"
            )));
        }
        ranges.push(SliceRange {
            offset,
            size,
            cpu_type,
            cpu_subtype,
        });
    }
    Ok(ranges)
}

fn parse_slice(data: &[u8], range: SliceRange) -> Result<Slice> {
    let s = &data[range.offset..range.offset + range.size];
    let arch = arch_name(range.cpu_type, range.cpu_subtype);
    let is_64 = match u32_at(s, 0) {
        Some(MH_MAGIC_64) => true,
        Some(MH_MAGIC) => false,
        _ => return Err(malformed(format!("{arch} slice has no Mach-O header"))),
    };
    let header_size = if is_64 { 32 } else { 28 };
    let ncmds = u32_at(s, 16).ok_or_else(|| malformed("truncated header"))? as usize;
    let sizeofcmds = u32_at(s, 20).ok_or_else(|| malformed("truncated header"))? as usize;
    if header_size + sizeofcmds > s.len() {
        return Err(malformed(format!(
            "{arch} load commands run past the slice"
        )));
    }

    let mut commands = Vec::with_capacity(ncmds);
    let mut limit = s.len();
    let mut offset = header_size;
    for _ in 0..ncmds {
        let cmd = u32_at(s, offset).ok_or_else(|| malformed("truncated load command"))?;
        let size =
            u32_at(s, offset + 4).ok_or_else(|| malformed("truncated load command"))? as usize;
        if size < 8 || offset + size > header_size + sizeofcmds {
            return Err(malformed(format!(
                "{arch} load command {cmd:#x} has a bad size"
            )));
        }
        if cmd == LC_SEGMENT_64 || cmd == LC_SEGMENT {
            // Segment content (other than the header's own, at file offset
            // 0) and section content bound the space for load commands
            let (fileoff, filesize, nsects, first_section, section_size, sect_offset, sect_flags) =
                if cmd == LC_SEGMENT_64 {
                    (
                        u64_at(s, offset + 40).unwrap_or(0) as usize,
                        u64_at(s, offset + 48).unwrap_or(0) as usize,
                        u32_at(s, offset + 64).unwrap_or(0) as usize,
                        72,
                        80,
                        48,
                        64,
                    )
                } else {
                    (
                        u32_at(s, offset + 32).unwrap_or(0) as usize,
                        u32_at(s, offset + 36).unwrap_or(0) as usize,
                        u32_at(s, offset + 48).unwrap_or(0) as usize,
                        56,
                        68,
                        40,
                        56,
                    )
                };
            if fileoff > 0 && filesize > 0 {
                limit = limit.min(fileoff);
            }
            for n in 0..nsects {
                let section = offset + first_section + n * section_size;
                if section + section_size > offset + size {
                    break;
                }
                let sect_off = u32_at(s, section + sect_offset).unwrap_or(0) as usize;
                let flags = u32_at(s, section + sect_flags).unwrap_or(0);
                // S_ZEROFILL, S_GB_ZEROFILL, S_THREAD_LOCAL_ZEROFILL have
                // no content in the file
                let zerofill = matches!(flags & 0xff, 0x1 | 0xc | 0x12);
                if sect_off > 0 && !zerofill {
                    limit = limit.min(sect_off);
                }
            }
        }
        commands.push((cmd, offset, size));
        offset += size;
    }
    if offset > limit {
        return Err(malformed(format!(
            "{arch} load commands overlap the first section"
        )));
    }
    Ok(Slice {
        is_64,
        header_size,
        ncmds_offset: 16,
        commands,
        limit,
        arch,
        range,
    })
}

fn parse_dylib(s: &[u8], offset: usize, size: usize) -> Option<Dylib> {
    let name_offset = u32_at(s, offset + 8)? as usize;
    if name_offset >= size {
        return None;
    }
    Some(Dylib {
        path: c_string(&s[offset..offset + size], name_offset)?,
        timestamp: u32_at(s, offset + 12)?,
        current_version: u32_at(s, offset + 16)?,
        compatibility_version: u32_at(s, offset + 20)?,
    })
}

fn parse_rpath(s: &[u8], offset: usize, size: usize) -> Option<String> {
    let path_offset = u32_at(s, offset + 8)? as usize;
    if path_offset >= size {
        return None;
    }
    c_string(&s[offset..offset + size], path_offset)
}

fn uleb(data: &[u8], pos: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    let mut shift = 0;
    loop {
        let byte = *data.get(*pos)?;
        *pos += 1;
        if shift < 64 {
            value |= u64::from(byte & 0x7f) << shift;
        }
        shift += 7;
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
}

/// Count binds per (positive) library ordinal in dyld bind opcodes
fn count_bind_opcodes(ops: &[u8], counts: &mut BTreeMap<usize, usize>) -> Option<()> {
    let mut pos = 0;
    let mut ordinal: i64 = 0;
    let record = |ordinal: i64, counts: &mut BTreeMap<usize, usize>| {
        if ordinal > 0 {
            *counts.entry(ordinal as usize).or_default() += 1;
        }
    };
    while pos < ops.len() {
        let byte = ops[pos];
        pos += 1;
        let imm = byte & 0x0f;
        match byte & 0xf0 {
            // DONE: ends an entry in lazy binds, so keep going
            0x00 => {}
            0x10 => ordinal = i64::from(imm),
            0x20 => ordinal = uleb(ops, &mut pos)? as i64,
            // SPECIAL_IMM: 0 or a small negative (self, main, flat, weak)
            0x30 => {
                ordinal = if imm == 0 {
                    0
                } else {
                    i64::from((0xf0 | imm) as i8)
                }
            }
            0x40 => {
                // Symbol name: skip the C string
                let end = ops[pos..].iter().position(|&b| b == 0)?;
                pos += end + 1;
            }
            0x50 => {}
            0x60 | 0x70 | 0x80 => {
                uleb(ops, &mut pos)?;
            }
            0x90 | 0xb0 => record(ordinal, counts),
            0xa0 => {
                uleb(ops, &mut pos)?;
                record(ordinal, counts);
            }
            0xc0 => {
                uleb(ops, &mut pos)?;
                uleb(ops, &mut pos)?;
                record(ordinal, counts);
            }
            // THREADED: SET_BIND_ORDINAL_TABLE_SIZE_ULEB (0) or APPLY (1)
            0xd0 => {
                if imm == 0 {
                    uleb(ops, &mut pos)?;
                }
            }
            _ => return None,
        }
    }
    Some(())
}

/// Symbols imported per library ordinal, from every source the slice has.
/// None when it has none of them.
fn imports_by_ordinal(s: &[u8], slice: &Slice) -> Option<BTreeMap<usize, usize>> {
    let mut counts = BTreeMap::new();
    let mut found = false;
    for &(cmd, offset, _) in &slice.commands {
        match cmd {
            LC_DYLD_INFO | LC_DYLD_INFO_ONLY => {
                found = true;
                // bind (16) and lazy bind (32); weak binds have no ordinal
                for field in [16, 32] {
                    let (Some(off), Some(size)) =
                        (u32_at(s, offset + field), u32_at(s, offset + field + 4))
                    else {
                        continue;
                    };
                    if let Some(ops) = s.get(off as usize..off as usize + size as usize) {
                        count_bind_opcodes(ops, &mut counts);
                    }
                }
            }
            LC_DYLD_CHAINED_FIXUPS => {
                found = true;
                let (Some(off), Some(size)) = (u32_at(s, offset + 8), u32_at(s, offset + 12))
                else {
                    continue;
                };
                let Some(fixups) = s.get(off as usize..off as usize + size as usize) else {
                    continue;
                };
                let imports_offset = u32_at(fixups, 8).unwrap_or(0) as usize;
                let imports_count = u32_at(fixups, 16).unwrap_or(0) as usize;
                let format = u32_at(fixups, 20).unwrap_or(0);
                for i in 0..imports_count {
                    let ordinal = match format {
                        // DYLD_CHAINED_IMPORT: lib_ordinal is the low 8 bits
                        1 => u32_at(fixups, imports_offset + i * 4).map(|v| u64::from(v & 0xff)),
                        // DYLD_CHAINED_IMPORT_ADDEND: same, then a 32-bit addend
                        2 => u32_at(fixups, imports_offset + i * 8).map(|v| u64::from(v & 0xff)),
                        // DYLD_CHAINED_IMPORT_ADDEND64: 16-bit ordinal
                        3 => u64_at(fixups, imports_offset + i * 16).map(|v| v & 0xffff),
                        _ => None,
                    };
                    let Some(ordinal) = ordinal else { break };
                    let special = if format == 3 { 0xfff0 } else { 0xf0 };
                    if ordinal > 0 && ordinal < special {
                        *counts.entry(ordinal as usize).or_default() += 1;
                    }
                }
            }
            LC_SYMTAB => {
                found = true;
                let (Some(symoff), Some(nsyms)) = (u32_at(s, offset + 8), u32_at(s, offset + 12))
                else {
                    continue;
                };
                let entry = if slice.is_64 { 16 } else { 12 };
                for i in 0..nsyms as usize {
                    let base = symoff as usize + i * entry;
                    let (Some(&n_type), Some(n_desc)) = (s.get(base + 4), u16_at(s, base + 6))
                    else {
                        break;
                    };
                    // Undefined (N_UNDF), external (N_EXT), not a debug entry
                    if n_type & 0xe0 == 0 && n_type & 0x0e == 0 && n_type & 0x01 != 0 {
                        let ordinal = usize::from(n_desc >> 8);
                        // 0xfe: dynamic lookup, 0xff: main executable
                        if ordinal > 0 && ordinal < 0xfe {
                            *counts.entry(ordinal).or_default() += 1;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    found.then_some(counts)
}

fn slice_info(data: &[u8], slice: &Slice) -> SliceInfo {
    let s = &data[slice.range.offset..slice.range.offset + slice.range.size];
    let imports = imports_by_ordinal(s, slice);
    let mut info = SliceInfo {
        arch: slice.arch.clone(),
        cpu_type: slice.range.cpu_type,
        cpu_subtype: slice.range.cpu_subtype,
        offset: slice.range.offset,
        size: slice.range.size,
        id: None,
        dylibs: Vec::new(),
        rpaths: Vec::new(),
        commands: slice.commands.iter().map(|&(cmd, _, _)| cmd).collect(),
        encrypted: false,
        free_space: slice.limit - slice.command_end(),
    };
    for (index, &(cmd, offset, size)) in slice.commands.iter().enumerate() {
        if let Some(kind) = DylibKind::from_cmd(cmd) {
            if let Some(dylib) = parse_dylib(s, offset, size) {
                let ordinal = info.dylibs.len() + 1;
                info.dylibs.push(LoadedDylib {
                    kind,
                    dylib,
                    ordinal,
                    command_index: index,
                    imports: imports
                        .as_ref()
                        .map(|counts| counts.get(&ordinal).copied().unwrap_or(0)),
                });
            }
        } else if cmd == LC_ID_DYLIB {
            info.id = parse_dylib(s, offset, size);
        } else if cmd == LC_RPATH {
            if let Some(path) = parse_rpath(s, offset, size) {
                info.rpaths.push(path);
            }
        } else if cmd == LC_ENCRYPTION_INFO || cmd == LC_ENCRYPTION_INFO_64 {
            info.encrypted |= u32_at(s, offset + 16).unwrap_or(0) != 0;
        }
    }
    info
}

/// Read every slice's load commands
pub fn read(data: &[u8]) -> Result<Vec<SliceInfo>> {
    slice_ranges(data)?
        .into_iter()
        .map(|range| Ok(slice_info(data, &parse_slice(data, range)?)))
        .collect()
}

/// Read a file's load commands
pub fn read_file(path: &Path) -> Result<Vec<SliceInfo>> {
    let data = std::fs::read(path).map_err(|source| MachoEditError::Io {
        path: path.display().to_string(),
        source,
    })?;
    read(&data)
}

/// A dylib-style load command: header, then the path, NUL-padded
fn dylib_command(cmd: u32, dylib: &Dylib, align: usize) -> Vec<u8> {
    let size = (24 + dylib.path.len() + 1).next_multiple_of(align);
    let mut bytes = Vec::with_capacity(size);
    bytes.extend_from_slice(&cmd.to_le_bytes());
    bytes.extend_from_slice(&(size as u32).to_le_bytes());
    bytes.extend_from_slice(&24u32.to_le_bytes());
    bytes.extend_from_slice(&dylib.timestamp.to_le_bytes());
    bytes.extend_from_slice(&dylib.current_version.to_le_bytes());
    bytes.extend_from_slice(&dylib.compatibility_version.to_le_bytes());
    bytes.extend_from_slice(dylib.path.as_bytes());
    bytes.resize(size, 0);
    bytes
}

fn rpath_command(path: &str, align: usize) -> Vec<u8> {
    let size = (12 + path.len() + 1).next_multiple_of(align);
    let mut bytes = Vec::with_capacity(size);
    bytes.extend_from_slice(&LC_RPATH.to_le_bytes());
    bytes.extend_from_slice(&(size as u32).to_le_bytes());
    bytes.extend_from_slice(&12u32.to_le_bytes());
    bytes.extend_from_slice(path.as_bytes());
    bytes.resize(size, 0);
    bytes
}

/// A load command, as an edit sees it: kept as it was, or new bytes
enum Command {
    Keep(usize),
    New(Vec<u8>),
}

/// Write `commands` as the slice's load commands, within its free space.
/// Leftover space from the old commands is zeroed.
fn rewrite(data: &mut [u8], slice: &Slice, commands: &[Command]) -> Result<()> {
    let s = &data[slice.range.offset..slice.range.offset + slice.range.size];
    let mut bytes = Vec::new();
    for command in commands {
        match command {
            Command::Keep(index) => {
                let (_, offset, size) = slice.commands[*index];
                bytes.extend_from_slice(&s[offset..offset + size]);
            }
            Command::New(new) => bytes.extend_from_slice(new),
        }
    }
    let available = slice.limit - slice.header_size;
    if bytes.len() > available {
        return Err(MachoEditError::NoRoom {
            arch: slice.arch.clone(),
            needed: bytes.len() - (slice.command_end() - slice.header_size),
            available: slice.limit - slice.command_end(),
        });
    }
    let old_end = slice.command_end();
    let s = &mut data[slice.range.offset..slice.range.offset + slice.range.size];
    let start = slice.header_size;
    s[start..start + bytes.len()].copy_from_slice(&bytes);
    let new_end = start + bytes.len();
    if new_end < old_end {
        s[new_end..old_end].fill(0);
    }
    let ncmds = slice.ncmds_offset;
    s[ncmds..ncmds + 4].copy_from_slice(&(commands.len() as u32).to_le_bytes());
    s[ncmds + 4..ncmds + 8].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
    Ok(())
}

/// Run `edit` on every slice of a copy of `data`, and keep the copy only if
/// every slice succeeded. `edit` returns the new command list, or None for
/// no change. Returns how many slices changed.
fn edit_slices(
    data: &mut [u8],
    refuse_encrypted: bool,
    mut edit: impl FnMut(&[u8], &Slice) -> Result<Option<Vec<Command>>>,
) -> Result<usize> {
    let ranges = slice_ranges(data)?;
    let mut copy = data.to_vec();
    let mut changed = 0;
    for range in ranges {
        let slice = parse_slice(&copy, range)?;
        let s = &copy[range.offset..range.offset + range.size];
        if refuse_encrypted && slice_info(&copy, &slice).encrypted {
            return Err(MachoEditError::Encrypted {
                arch: slice.arch.clone(),
            });
        }
        if let Some(commands) = edit(s, &slice)? {
            rewrite(&mut copy, &slice, &commands)?;
            changed += 1;
        }
    }
    data.copy_from_slice(&copy);
    Ok(changed)
}

fn keep_all(slice: &Slice) -> Vec<Command> {
    (0..slice.commands.len()).map(Command::Keep).collect()
}

/// Add a dylib load to every slice, after its existing load commands (as
/// insert_dylib and Zsign do), so existing library ordinals don't change.
/// A slice already loading `path` keeps it, with its kind set to `kind`.
/// Returns whether anything changed. Refuses encrypted slices.
pub fn add_dylib(data: &mut [u8], path: &str, kind: DylibKind) -> Result<bool> {
    add_dylib_with(
        data,
        &Dylib {
            path: path.to_string(),
            timestamp: DEFAULT_TIMESTAMP,
            current_version: 0,
            compatibility_version: 0,
        },
        kind,
    )
}

/// `add_dylib`, with the command's timestamp and versions given
pub fn add_dylib_with(data: &mut [u8], dylib: &Dylib, kind: DylibKind) -> Result<bool> {
    let changed = edit_slices(data, true, |s, slice| {
        let mut commands = keep_all(slice);
        for (index, &(cmd, offset, size)) in slice.commands.iter().enumerate() {
            let Some(existing_kind) = DylibKind::from_cmd(cmd) else {
                continue;
            };
            let Some(existing) = parse_dylib(s, offset, size) else {
                continue;
            };
            if existing.path == dylib.path {
                if existing_kind == kind {
                    return Ok(None);
                }
                commands[index] =
                    Command::New(dylib_command(kind.cmd(), &existing, slice.alignment()));
                return Ok(Some(commands));
            }
        }
        commands.push(Command::New(dylib_command(
            kind.cmd(),
            dylib,
            slice.alignment(),
        )));
        Ok(Some(commands))
    })?;
    Ok(changed > 0)
}

/// Append a dylib load to every slice, even one already loading the same
/// path, as LIEF's `add_library` does (Python warpsign's injection).
/// Refuses encrypted slices.
pub fn append_dylib(data: &mut [u8], dylib: &Dylib, kind: DylibKind) -> Result<()> {
    edit_slices(data, true, |_, slice| {
        let mut commands = keep_all(slice);
        commands.push(Command::New(dylib_command(
            kind.cmd(),
            dylib,
            slice.alignment(),
        )));
        Ok(Some(commands))
    })?;
    Ok(())
}

/// Every dylib command in each slice, in order: loads of each kind, and
/// the install name (`LC_ID_DYLIB`) where it falls. With the raw command
/// type, as LIEF lists `DylibCommand`s.
pub fn dylib_commands(data: &[u8]) -> Result<Vec<Vec<(u32, Dylib)>>> {
    slice_ranges(data)?
        .into_iter()
        .map(|range| {
            let slice = parse_slice(data, range)?;
            let s = &data[range.offset..range.offset + range.size];
            Ok(slice
                .commands
                .iter()
                .filter(|&&(cmd, _, _)| cmd == LC_ID_DYLIB || DylibKind::from_cmd(cmd).is_some())
                .filter_map(|&(cmd, offset, size)| Some((cmd, parse_dylib(s, offset, size)?)))
                .collect())
        })
        .collect()
}

/// In each slice, remove the first dylib command `matches` picks, the
/// install name (`LC_ID_DYLIB`) included, without the checks
/// `remove_dylib` makes. Python warpsign removes conflicting dylibs this
/// way, with LIEF; encrypted slices are edited too, as LIEF does (only the
/// header changes). Returns whether anything changed.
pub fn remove_first_dylib_command_where(
    data: &mut [u8],
    mut matches: impl FnMut(&Dylib) -> bool,
) -> Result<bool> {
    let changed = edit_slices(data, false, |s, slice| {
        let found = slice.commands.iter().position(|&(cmd, offset, size)| {
            (cmd == LC_ID_DYLIB || DylibKind::from_cmd(cmd).is_some())
                && parse_dylib(s, offset, size).is_some_and(|dylib| matches(&dylib))
        });
        Ok(found.map(|found| {
            (0..slice.commands.len())
                .filter(|&index| index != found)
                .map(Command::Keep)
                .collect()
        }))
    })?;
    Ok(changed > 0)
}

/// What `remove_dylib` may do that would otherwise make it refuse
#[derive(Debug, Clone, Copy, Default)]
pub struct RemoveOptions {
    /// Remove a dylib the binary imports symbols from
    pub allow_in_use: bool,
    /// Remove a dylib before others the binary imports symbols from, which
    /// renumbers them. Python warpsign removes conflicting dylibs this way.
    pub allow_ordinal_shift: bool,
}

/// Remove every load of `path` from every slice. Refuses (unless `options`
/// allow it) when the binary imports symbols from it, or from a library
/// after it, whose ordinal would change. Returns whether anything changed.
pub fn remove_dylib(data: &mut [u8], path: &str, options: RemoveOptions) -> Result<bool> {
    remove_dylibs_where(data, options, |dylib| dylib.path == path)
}

/// Remove, from every slice, the loads `matches` picks
pub fn remove_dylibs_where(
    data: &mut [u8],
    options: RemoveOptions,
    mut matches: impl FnMut(&Dylib) -> bool,
) -> Result<bool> {
    let changed = edit_slices(data, true, |s, slice| {
        let imports = imports_by_ordinal(s, slice).unwrap_or_default();
        let mut commands = Vec::with_capacity(slice.commands.len());
        let mut ordinal = 0;
        let mut first_removed: Option<String> = None;
        for (index, &(cmd, offset, size)) in slice.commands.iter().enumerate() {
            if DylibKind::from_cmd(cmd).is_some() {
                ordinal += 1;
                if let Some(dylib) = parse_dylib(s, offset, size)
                    && matches(&dylib)
                {
                    let used = imports.get(&ordinal).copied().unwrap_or(0);
                    if used > 0 && !options.allow_in_use {
                        return Err(MachoEditError::InUse {
                            path: dylib.path,
                            arch: slice.arch.clone(),
                            imports: used,
                        });
                    }
                    first_removed.get_or_insert(dylib.path);
                    continue;
                }
                if let Some(path) = &first_removed
                    && imports.get(&ordinal).copied().unwrap_or(0) > 0
                    && !options.allow_ordinal_shift
                {
                    return Err(MachoEditError::OrdinalShift {
                        path: path.clone(),
                        arch: slice.arch.clone(),
                    });
                }
            }
            commands.push(Command::Keep(index));
        }
        Ok(first_removed.map(|_| commands))
    })?;
    Ok(changed > 0)
}

/// Point every load of `old` at `new` (`install_name_tool -change`).
/// Returns how many slices changed.
pub fn change_dylib(data: &mut [u8], old: &str, new: &str) -> Result<usize> {
    edit_slices(data, true, |s, slice| {
        let mut commands = keep_all(slice);
        let mut changed = false;
        for (index, &(cmd, offset, size)) in slice.commands.iter().enumerate() {
            let Some(kind) = DylibKind::from_cmd(cmd) else {
                continue;
            };
            if let Some(mut dylib) = parse_dylib(s, offset, size)
                && dylib.path == old
            {
                dylib.path = new.to_string();
                commands[index] =
                    Command::New(dylib_command(kind.cmd(), &dylib, slice.alignment()));
                changed = true;
            }
        }
        Ok(changed.then_some(commands))
    })
}

/// Set the install name of a dylib (`install_name_tool -id`). Returns
/// whether anything changed; a binary without one is left alone.
pub fn set_id(data: &mut [u8], path: &str) -> Result<bool> {
    let changed = edit_slices(data, true, |s, slice| {
        let mut commands = keep_all(slice);
        for (index, &(cmd, offset, size)) in slice.commands.iter().enumerate() {
            if cmd != LC_ID_DYLIB {
                continue;
            }
            let Some(mut dylib) = parse_dylib(s, offset, size) else {
                continue;
            };
            if dylib.path == path {
                return Ok(None);
            }
            dylib.path = path.to_string();
            commands[index] = Command::New(dylib_command(LC_ID_DYLIB, &dylib, slice.alignment()));
            return Ok(Some(commands));
        }
        Ok(None)
    })?;
    Ok(changed > 0)
}

/// Add an rpath to every slice that doesn't have it. Returns whether
/// anything changed.
pub fn add_rpath(data: &mut [u8], path: &str) -> Result<bool> {
    let changed = edit_slices(data, true, |s, slice| {
        let present = slice.commands.iter().any(|&(cmd, offset, size)| {
            cmd == LC_RPATH && parse_rpath(s, offset, size).as_deref() == Some(path)
        });
        if present {
            return Ok(None);
        }
        let mut commands = keep_all(slice);
        commands.push(Command::New(rpath_command(path, slice.alignment())));
        Ok(Some(commands))
    })?;
    Ok(changed > 0)
}

/// Remove an rpath from every slice. Returns whether anything changed.
pub fn remove_rpath(data: &mut [u8], path: &str) -> Result<bool> {
    let changed = edit_slices(data, true, |s, slice| {
        let commands: Vec<Command> = slice
            .commands
            .iter()
            .enumerate()
            .filter(|&(_, &(cmd, offset, size))| {
                !(cmd == LC_RPATH && parse_rpath(s, offset, size).as_deref() == Some(path))
            })
            .map(|(index, _)| Command::Keep(index))
            .collect();
        Ok((commands.len() != slice.commands.len()).then_some(commands))
    })?;
    Ok(changed > 0)
}

/// Edit a file in place: read it, run `edit`, and write it back if it
/// returns true
pub fn edit_file<T>(path: &Path, edit: impl FnOnce(&mut [u8]) -> Result<T>) -> Result<T>
where
    T: Changed,
{
    let io = |source| MachoEditError::Io {
        path: path.display().to_string(),
        source,
    };
    let mut data = std::fs::read(path).map_err(io)?;
    let result = edit(&mut data)?;
    if result.changed() {
        std::fs::write(path, &data).map_err(io)?;
    }
    Ok(result)
}

/// Whether an edit's result means the file changed
pub trait Changed {
    fn changed(&self) -> bool;
}

impl Changed for bool {
    fn changed(&self) -> bool {
        *self
    }
}

impl Changed for usize {
    fn changed(&self) -> bool {
        *self > 0
    }
}

#[cfg(test)]
mod tests;
