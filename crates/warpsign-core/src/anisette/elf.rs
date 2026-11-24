//! Just enough of ELF to load Apple's two Android libraries: 64-bit,
//! little-endian AArch64 shared objects with plain RELA relocations.

use std::collections::HashMap;

pub const EM_AARCH64: u16 = 183;
const PT_LOAD: u32 = 1;
const SHT_RELA: u32 = 4;
const SHT_DYNSYM: u32 = 11;
const SHN_UNDEF: u16 = 0;

pub const R_AARCH64_ABS64: u32 = 257;
pub const R_AARCH64_GLOB_DAT: u32 = 1025;
pub const R_AARCH64_JUMP_SLOT: u32 = 1026;
pub const R_AARCH64_RELATIVE: u32 = 1027;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub vaddr: u64,
    pub memsz: u64,
    /// File bytes, `filesz` of them; the rest of `memsz` is zero
    pub offset: u64,
    pub filesz: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relocation {
    pub offset: u64,
    pub kind: u32,
    /// Index into `symbols`; 0 for none
    pub symbol: u32,
    pub addend: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    /// None: undefined here, imported
    pub value: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct SharedObject {
    pub segments: Vec<Segment>,
    pub symbols: Vec<Symbol>,
    pub relocations: Vec<Relocation>,
    exports: HashMap<String, u64>,
}

fn u16_at(data: &[u8], at: usize) -> Result<u16, String> {
    data.get(at..at + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or_else(|| format!("ELF truncated at {at:#x}"))
}

fn u32_at(data: &[u8], at: usize) -> Result<u32, String> {
    data.get(at..at + 4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .ok_or_else(|| format!("ELF truncated at {at:#x}"))
}

fn u64_at(data: &[u8], at: usize) -> Result<u64, String> {
    data.get(at..at + 8)
        .map(|b| u64::from_le_bytes(b.try_into().unwrap()))
        .ok_or_else(|| format!("ELF truncated at {at:#x}"))
}

fn c_string(data: &[u8], at: usize) -> Result<String, String> {
    let rest = data
        .get(at..)
        .ok_or_else(|| format!("string out of range at {at:#x}"))?;
    let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
    Ok(String::from_utf8_lossy(&rest[..end]).into_owned())
}

struct Section {
    kind: u32,
    offset: usize,
    size: usize,
    link: usize,
    entsize: usize,
}

impl SharedObject {
    pub fn parse(data: &[u8]) -> Result<Self, String> {
        if data.get(..4) != Some(b"\x7fELF") {
            return Err("not an ELF file".into());
        }
        if data.get(4) != Some(&2) || data.get(5) != Some(&1) {
            return Err("not a 64-bit little-endian ELF file".into());
        }
        let machine = u16_at(data, 18)?;
        if machine != EM_AARCH64 {
            return Err(format!("not an AArch64 library (machine {machine})"));
        }

        let phoff = u64_at(data, 32)? as usize;
        let shoff = u64_at(data, 40)? as usize;
        let phentsize = u16_at(data, 54)? as usize;
        let phnum = u16_at(data, 56)? as usize;
        let shentsize = u16_at(data, 58)? as usize;
        let shnum = u16_at(data, 60)? as usize;

        let mut segments = Vec::new();
        for i in 0..phnum {
            let at = phoff + i * phentsize;
            if u32_at(data, at)? != PT_LOAD {
                continue;
            }
            let segment = Segment {
                offset: u64_at(data, at + 8)?,
                vaddr: u64_at(data, at + 16)?,
                filesz: u64_at(data, at + 32)?,
                memsz: u64_at(data, at + 40)?,
            };
            if segment
                .offset
                .checked_add(segment.filesz)
                .is_none_or(|end| end > data.len() as u64)
                || segment.filesz > segment.memsz
            {
                return Err("a segment runs past the end of the file".into());
            }
            segments.push(segment);
        }
        if segments.is_empty() {
            return Err("no loadable segments".into());
        }

        let mut sections = Vec::with_capacity(shnum);
        for i in 0..shnum {
            let at = shoff + i * shentsize;
            sections.push(Section {
                kind: u32_at(data, at + 4)?,
                offset: u64_at(data, at + 24)? as usize,
                size: u64_at(data, at + 32)? as usize,
                link: u32_at(data, at + 40)? as usize,
                entsize: u64_at(data, at + 56)? as usize,
            });
        }

        let dynsym = sections
            .iter()
            .find(|s| s.kind == SHT_DYNSYM)
            .ok_or("no dynamic symbol table")?;
        let strtab = sections
            .get(dynsym.link)
            .ok_or("dynamic symbol table has no string table")?;
        let strings = data
            .get(strtab.offset..strtab.offset + strtab.size)
            .ok_or("string table out of range")?;
        let entsize = if dynsym.entsize == 0 {
            24
        } else {
            dynsym.entsize
        };
        let mut symbols = Vec::with_capacity(dynsym.size / entsize);
        let mut exports = HashMap::new();
        for i in 0..dynsym.size / entsize {
            let at = dynsym.offset + i * entsize;
            let name = c_string(strings, u32_at(data, at)? as usize)?;
            let shndx = u16_at(data, at + 6)?;
            let value = u64_at(data, at + 8)?;
            let value = (shndx != SHN_UNDEF).then_some(value);
            if let (Some(value), false) = (value, name.is_empty()) {
                exports.insert(name.clone(), value);
            }
            symbols.push(Symbol { name, value });
        }

        let mut relocations = Vec::new();
        for section in sections.iter().filter(|s| s.kind == SHT_RELA) {
            let entsize = if section.entsize == 0 {
                24
            } else {
                section.entsize
            };
            for i in 0..section.size / entsize {
                let at = section.offset + i * entsize;
                let info = u64_at(data, at + 8)?;
                relocations.push(Relocation {
                    offset: u64_at(data, at)?,
                    kind: info as u32,
                    symbol: (info >> 32) as u32,
                    addend: u64_at(data, at + 16)? as i64,
                });
            }
        }

        Ok(Self {
            segments,
            symbols,
            relocations,
            exports,
        })
    }

    /// The address range the segments cover, relative to the load address
    pub fn extent(&self) -> (u64, u64) {
        let start = self.segments.iter().map(|s| s.vaddr).min().unwrap_or(0);
        let end = self
            .segments
            .iter()
            .map(|s| s.vaddr + s.memsz)
            .max()
            .unwrap_or(0);
        (start, end)
    }

    /// An exported symbol's address, relative to the load address
    pub fn export(&self, name: &str) -> Option<u64> {
        self.exports.get(name).copied()
    }

    /// Names this library needs from elsewhere
    pub fn imports(&self) -> impl Iterator<Item = &str> {
        self.symbols
            .iter()
            .filter(|s| s.value.is_none() && !s.name.is_empty())
            .map(|s| s.name.as_str())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A tiny AArch64 shared object: one PT_LOAD segment holding `code`,
    /// symbols `exports` (defined at the given offsets into `code`) and
    /// `imports` (undefined), and one JUMP_SLOT relocation per import, at
    /// `slots` + 8 × index, plus one RELATIVE relocation at `relative`
    /// pointing at the start of `code`.
    pub(crate) fn build(
        code: &[u8],
        exports: &[(&str, u64)],
        imports: &[&str],
        slots: u64,
        relative: u64,
    ) -> Vec<u8> {
        let mut strings = vec![0u8];
        let mut name_at = |name: &str| {
            let at = strings.len() as u32;
            strings.extend_from_slice(name.as_bytes());
            strings.push(0);
            at
        };
        // Symbol 0 is the null symbol
        let mut syms = vec![[0u8; 24]];
        for (name, value) in exports {
            let mut sym = [0u8; 24];
            sym[0..4].copy_from_slice(&name_at(name).to_le_bytes());
            sym[4] = 0x12; // global function
            sym[6..8].copy_from_slice(&1u16.to_le_bytes());
            sym[8..16].copy_from_slice(&(0x1000 + value).to_le_bytes());
            syms.push(sym);
        }
        let first_import = syms.len();
        for name in imports {
            let mut sym = [0u8; 24];
            sym[0..4].copy_from_slice(&name_at(name).to_le_bytes());
            sym[4] = 0x12;
            syms.push(sym);
        }
        let mut relas = Vec::new();
        for (i, _) in imports.iter().enumerate() {
            let mut rela = [0u8; 24];
            rela[0..8].copy_from_slice(&(slots + 8 * i as u64).to_le_bytes());
            let info = ((first_import + i) as u64) << 32 | R_AARCH64_JUMP_SLOT as u64;
            rela[8..16].copy_from_slice(&info.to_le_bytes());
            relas.push(rela);
        }
        let mut rela = [0u8; 24];
        rela[0..8].copy_from_slice(&relative.to_le_bytes());
        rela[8..16].copy_from_slice(&(R_AARCH64_RELATIVE as u64).to_le_bytes());
        rela[16..24].copy_from_slice(&0x1000i64.to_le_bytes());
        relas.push(rela);

        // Layout: header, 1 phdr, code at 0x1000, then tables, then shdrs
        let mut file = vec![0u8; 0x1000];
        file.extend_from_slice(code);
        let code_size = code.len() as u64;
        let align = |file: &mut Vec<u8>| file.resize(file.len().div_ceil(8) * 8, 0);
        align(&mut file);
        let dynsym_at = file.len();
        for sym in &syms {
            file.extend_from_slice(sym);
        }
        let strtab_at = file.len();
        file.extend_from_slice(&strings);
        align(&mut file);
        let rela_at = file.len();
        for rela in &relas {
            file.extend_from_slice(rela);
        }
        align(&mut file);
        let shoff = file.len();
        // Sections: null, dynsym (link 2), dynstr, rela
        let section = |kind: u32, offset: usize, size: usize, link: u32, entsize: u64| {
            let mut sh = [0u8; 64];
            sh[4..8].copy_from_slice(&kind.to_le_bytes());
            sh[24..32].copy_from_slice(&(offset as u64).to_le_bytes());
            sh[32..40].copy_from_slice(&(size as u64).to_le_bytes());
            sh[40..44].copy_from_slice(&link.to_le_bytes());
            sh[56..64].copy_from_slice(&entsize.to_le_bytes());
            sh
        };
        file.extend_from_slice(&[0u8; 64]);
        file.extend_from_slice(&section(SHT_DYNSYM, dynsym_at, syms.len() * 24, 2, 24));
        file.extend_from_slice(&section(3, strtab_at, strings.len(), 0, 0));
        file.extend_from_slice(&section(SHT_RELA, rela_at, relas.len() * 24, 1, 24));

        file[0..4].copy_from_slice(b"\x7fELF");
        file[4] = 2;
        file[5] = 1;
        file[6] = 1;
        file[16..18].copy_from_slice(&3u16.to_le_bytes()); // ET_DYN
        file[18..20].copy_from_slice(&EM_AARCH64.to_le_bytes());
        file[32..40].copy_from_slice(&64u64.to_le_bytes()); // phoff
        file[40..48].copy_from_slice(&(shoff as u64).to_le_bytes());
        file[54..56].copy_from_slice(&56u16.to_le_bytes());
        file[56..58].copy_from_slice(&1u16.to_le_bytes());
        file[58..60].copy_from_slice(&64u16.to_le_bytes());
        file[60..62].copy_from_slice(&4u16.to_le_bytes());
        // PT_LOAD: offset 0x1000 → vaddr 0x1000, plus room for slots
        let ph = 64;
        file[ph..ph + 4].copy_from_slice(&PT_LOAD.to_le_bytes());
        file[ph + 8..ph + 16].copy_from_slice(&0x1000u64.to_le_bytes());
        file[ph + 16..ph + 24].copy_from_slice(&0x1000u64.to_le_bytes());
        file[ph + 32..ph + 40].copy_from_slice(&code_size.to_le_bytes());
        file[ph + 40..ph + 48].copy_from_slice(&0x2000u64.to_le_bytes());
        file
    }

    #[test]
    fn parses_a_built_library() {
        let file = build(
            &[0xC0, 0x03, 0x5F, 0xD6],
            &[("f", 0)],
            &["malloc"],
            0x2000,
            0x2100,
        );
        let elf = SharedObject::parse(&file).unwrap();
        assert_eq!(elf.segments.len(), 1);
        assert_eq!(elf.extent(), (0x1000, 0x3000));
        assert_eq!(elf.export("f"), Some(0x1000));
        assert_eq!(elf.imports().collect::<Vec<_>>(), ["malloc"]);
        assert_eq!(elf.relocations.len(), 2);
        assert_eq!(elf.relocations[0].kind, R_AARCH64_JUMP_SLOT);
        assert_eq!(elf.relocations[1].kind, R_AARCH64_RELATIVE);
    }

    #[test]
    fn refuses_other_files() {
        assert!(SharedObject::parse(b"not an elf").is_err());
        let mut file = build(&[0; 4], &[], &[], 0x2000, 0x2100);
        file[18] = 62; // x86-64
        assert!(SharedObject::parse(&file).unwrap_err().contains("AArch64"));
    }
}
