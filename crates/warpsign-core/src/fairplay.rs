//! Offline FairPlay decryption for App Store IPAs you own.
//!
//! Given a 16-byte per-Apple-account credential and an app's own `SC_Info`
//! licence records (SINF and SUPF), this derives each encrypted page's key and
//! decrypts the arm64 Mach-O slices in place, clearing `cryptid`. No device, no
//! jailbreak, no FairPlay daemon: it's pure computation that runs anywhere.
//!
//! This is a Rust port of EthanArbuckle/fairplay-decrypt (`core.py`,
//! `decrypt.py`), GPL-3.0, used under the AGPL-3.0; see NOTICE. The credential
//! itself is produced elsewhere (e.g. UnfairPlay on an enrolled Mac, or
//! fairplay-decrypt's retrieval tool on a jailbroken device) — warpsign only
//! consumes it. It is a per-account secret: never log, print, or persist it.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use aes::Aes128;
use aes::cipher::{BlockCipherDecrypt, BlockCipherEncrypt, KeyInit};
use md5::{Digest, Md5};
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

const ARM64: u32 = 0x0100_000c;
const MH_MAGIC_64: u32 = 0xfeed_facf;
const FAT_MAGIC: [u8; 4] = [0xca, 0xfe, 0xba, 0xbe];
const FAT_MAGIC_64: [u8; 4] = [0xca, 0xfe, 0xba, 0xbf];
const THIN_MAGIC_64_LE: [u8; 4] = [0xcf, 0xfa, 0xed, 0xfe];
const LC_ENCRYPTION_INFO_64: u32 = 0x2c;
const PAGE_SIZE: usize = 4096;

// The page-key "finish" mixer (fairplay-decrypt core.py).
const ROUND_SELECTORS: [[(usize, usize); 4]; 4] = [
    [(0, 1), (2, 3), (4, 5), (6, 7)],
    [(2, 1), (0, 3), (4, 7), (6, 5)],
    [(2, 0), (4, 1), (6, 3), (7, 5)],
    [(1, 2), (4, 0), (7, 3), (5, 6)],
];
const ROUND_CONSTANTS: [u32; 4] = [0x6854_7c2e, 0x7e05_e1de, 0xd495_038b, 0x0ab2_4add];
const FINAL_SELECTORS: [[usize; 4]; 4] = [[0, 1, 2, 3], [0, 2, 4, 6], [0, 3, 5, 7], [4, 5, 6, 7]];
const FINAL_CONSTANTS: [u32; 4] = [0x5b71_98c7, 0xeb5d_e22c, 0x0f8a_a8fa, 0xf6e1_bb3f];

#[derive(Debug, thiserror::Error)]
pub enum FairplayError {
    #[error("{path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("{path}: {source}")]
    Zip {
        path: String,
        source: zip::result::ZipError,
    },
    #[error("credential must be exactly 16 bytes")]
    CredentialSize,
    #[error("{0}")]
    Macho(String),
    #[error("{0}")]
    Sinf(String),
    #[error("{0}")]
    Supf(String),
    #[error("the credential does not decode this app's licence (wrong account?)")]
    WrongCredential,
    #[error("no encrypted arm64 binaries with supported SINF/SUPF records")]
    NothingToDecrypt,
    #[error("{0} already exists")]
    OutputExists(String),
}

/// A FairPlay-encrypted region of one arm64 Mach-O slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MachoInfo {
    /// Slice offset within the (possibly FAT) file; 0 for a thin binary
    pub base: usize,
    /// Encrypted region offset within the slice
    pub cryptoff: usize,
    /// Encrypted region size (a multiple of the page size)
    pub cryptsize: usize,
    /// Absolute file offset of the `cryptid` u32, to zero after decrypting
    pub cryptid_offset: usize,
    /// arm64 cpu subtype, to match the right SUPF architecture record
    pub subtype: u32,
}

/// What `decrypt_ipa` decrypted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DecryptSummary {
    pub binaries: usize,
    pub pages: usize,
}

// ---- AES primitives (no padding; regions are exact block multiples) --------

fn aes_ecb_encrypt_block(key: &[u8; 16], block: &[u8; 16]) -> [u8; 16] {
    let cipher = Aes128::new(key.into());
    let mut b = (*block).into();
    cipher.encrypt_block(&mut b);
    b.into()
}

/// AES-128-CBC decrypt with no padding. `data.len()` must be a multiple of 16.
fn aes_cbc_decrypt_nopad(key: &[u8; 16], iv: &[u8; 16], data: &[u8]) -> Vec<u8> {
    let cipher = Aes128::new(key.into());
    let mut out = vec![0u8; data.len()];
    let mut prev = *iv;
    for (block_in, block_out) in data.chunks_exact(16).zip(out.chunks_exact_mut(16)) {
        let mut block: [u8; 16] = block_in.try_into().unwrap();
        let cipher_block = block;
        let mut b = block.into();
        cipher.decrypt_block(&mut b);
        block = b.into();
        for i in 0..16 {
            block_out[i] = block[i] ^ prev[i];
        }
        prev = cipher_block;
    }
    out
}

// ---- Mach-O parsing --------------------------------------------------------

fn read_u32_le(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        data.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn read_u32_be(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        data.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn read_u64_be(data: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_be_bytes(
        data.get(offset..offset + 8)?.try_into().ok()?,
    ))
}

/// Every FairPlay-encrypted arm64 slice in a Mach-O (thin or FAT).
pub fn arm64_encryption_infos(data: &[u8]) -> Result<Vec<MachoInfo>, FairplayError> {
    let err = |m: &str| FairplayError::Macho(m.to_string());
    // (base, slice_size, subtype) per arm64 slice
    let mut arm: Vec<(usize, usize, u32)> = Vec::new();

    if data.get(..4) == Some(&FAT_MAGIC) {
        let count = read_u32_be(data, 4).ok_or_else(|| err("truncated FAT header"))? as usize;
        if count == 0 || count > 16 || 8 + count * 20 > data.len() {
            return Err(err("invalid FAT32 architecture table"));
        }
        for i in 0..count {
            let base = 8 + i * 20;
            let cpu = read_u32_be(data, base).unwrap();
            let sub = read_u32_be(data, base + 4).unwrap();
            let off = read_u32_be(data, base + 8).unwrap() as usize;
            let size = read_u32_be(data, base + 12).unwrap() as usize;
            if cpu == ARM64 {
                arm.push((off, size, sub));
            }
        }
    } else if data.get(..4) == Some(&FAT_MAGIC_64) {
        let count = read_u32_be(data, 4).ok_or_else(|| err("truncated FAT header"))? as usize;
        if count == 0 || count > 16 || 8 + count * 32 > data.len() {
            return Err(err("invalid FAT64 architecture table"));
        }
        for i in 0..count {
            let base = 8 + i * 32;
            let cpu = read_u32_be(data, base).unwrap();
            let sub = read_u32_be(data, base + 4).unwrap();
            let off = read_u64_be(data, base + 8).unwrap() as usize;
            let size = read_u64_be(data, base + 16).unwrap() as usize;
            if cpu == ARM64 {
                arm.push((off, size, sub));
            }
        }
    } else if data.get(..4) == Some(&THIN_MAGIC_64_LE) {
        let sub = read_u32_le(data, 8).ok_or_else(|| err("truncated Mach-O header"))?;
        arm.push((0, data.len(), sub));
    } else {
        return Err(err("not a supported arm64 Mach-O"));
    }

    if arm.is_empty() {
        return Err(err("no arm64 slices found"));
    }
    let mut infos = Vec::new();
    for (base, size, subtype) in arm {
        if let Some(info) = arm64_slice_info(data, base, size, subtype)? {
            infos.push(info);
        }
    }
    Ok(infos)
}

fn arm64_slice_info(
    data: &[u8],
    base: usize,
    slice_size: usize,
    subtype: u32,
) -> Result<Option<MachoInfo>, FairplayError> {
    let err = |m: &str| FairplayError::Macho(m.to_string());
    if base
        .checked_add(slice_size)
        .is_none_or(|end| end > data.len())
        || base + 32 > data.len()
    {
        return Err(err("arm64 slice exceeds file"));
    }
    let magic = read_u32_le(data, base).unwrap();
    let cpu = read_u32_le(data, base + 4).unwrap();
    let ncmds = read_u32_le(data, base + 16).unwrap();
    let sizeofcmds = read_u32_le(data, base + 20).unwrap() as usize;
    if magic != MH_MAGIC_64 || cpu != ARM64 || ncmds > 1000 {
        return Err(err("invalid arm64 Mach-O header"));
    }
    let mut pos = base + 32;
    let commands_end = pos + sizeofcmds;
    if commands_end > base + slice_size {
        return Err(err("load commands exceed arm64 slice"));
    }
    let mut found: Vec<(usize, usize, u32, usize)> = Vec::new();
    for _ in 0..ncmds {
        if pos + 8 > commands_end {
            return Err(err("truncated load command"));
        }
        let cmd = read_u32_le(data, pos).unwrap();
        let size = read_u32_le(data, pos + 4).unwrap() as usize;
        if size < 8 || pos + size > commands_end {
            return Err(err("invalid load command"));
        }
        if cmd == LC_ENCRYPTION_INFO_64 {
            if size < 24 {
                return Err(err("short encryption info command"));
            }
            let cryptoff = read_u32_le(data, pos + 8).unwrap() as usize;
            let cryptsize = read_u32_le(data, pos + 12).unwrap() as usize;
            let cryptid = read_u32_le(data, pos + 16).unwrap();
            found.push((cryptoff, cryptsize, cryptid, pos + 16));
        }
        pos += size;
    }
    if pos != commands_end {
        return Err(err("load command sizes do not match Mach-O header"));
    }
    let (cryptoff, cryptsize, cryptid, cryptid_offset) = match found.as_slice() {
        [] => return Ok(None),
        [one] => *one,
        _ => return Err(err("expected one encryption command")),
    };
    if cryptid == 0 {
        return Ok(None);
    }
    if cryptid != 1 || cryptsize == 0 || cryptsize % PAGE_SIZE != 0 {
        return Err(err("unsupported cryptid/size"));
    }
    if cryptoff + cryptsize > slice_size || cryptid_offset >= base + cryptoff {
        return Err(err("invalid encrypted region bounds"));
    }
    Ok(Some(MachoInfo {
        base,
        cryptoff,
        cryptsize,
        cryptid_offset,
        subtype,
    }))
}

// ---- SINF (licence) parsing ------------------------------------------------

/// Yields `(fourcc, body_start, body_end)` for each atom in `data[start..end]`.
fn sinf_children(
    data: &[u8],
    start: usize,
    end: usize,
) -> Result<Vec<([u8; 4], usize, usize)>, FairplayError> {
    let err = || FairplayError::Sinf("invalid SINF atom".to_string());
    let mut out = Vec::new();
    let mut offset = start;
    while offset < end {
        if offset + 8 > end {
            return Err(err());
        }
        let size = u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
        if size < 8 || offset + size > end {
            return Err(err());
        }
        let fourcc: [u8; 4] = data[offset + 4..offset + 8].try_into().unwrap();
        out.push((fourcc, offset + 8, offset + size));
        offset += size;
    }
    Ok(out)
}

/// The bytes of the single `schi` child field named `wanted` (e.g. `b"priv"`).
fn sinf_field<'a>(sinf: &'a [u8], wanted: &[u8; 4]) -> Result<&'a [u8], FairplayError> {
    let err = |m: &str| FairplayError::Sinf(m.to_string());
    let roots = sinf_children(sinf, 0, sinf.len())?;
    let [(fourcc, start, end)] = roots.as_slice() else {
        return Err(err("expected outer sinf atom"));
    };
    if fourcc != b"sinf" {
        return Err(err("expected outer sinf atom"));
    }
    let schi: Vec<_> = sinf_children(sinf, *start, *end)?
        .into_iter()
        .filter(|(f, _, _)| f == b"schi")
        .collect();
    let [(_, schi_start, schi_end)] = schi.as_slice() else {
        return Err(err("expected one schi container"));
    };
    let fields: Vec<_> = sinf_children(sinf, *schi_start, *schi_end)?
        .into_iter()
        .filter(|(f, _, _)| f == wanted)
        .collect();
    let [(_, fstart, fend)] = fields.as_slice() else {
        return Err(err("expected one matching sinf field"));
    };
    Ok(&sinf[*fstart..*fend])
}

/// The 16-byte work key for this app, from the credential and the SINF.
pub fn derive_work16(credential: &[u8; 16], sinf: &[u8]) -> Result<[u8; 16], FairplayError> {
    let name = sinf_field(sinf, b"name")?;
    let name = name.split(|&b| b == 0).next().unwrap_or(name);
    let iviv = sinf_field(sinf, b"iviv")?;
    let mut raw_priv = sinf_field(sinf, b"priv")?.to_vec();
    match raw_priv.len() {
        448 => {}
        440 => {
            if raw_priv[432..] != [0u8; 8] {
                return Err(FairplayError::Sinf(
                    "invalid SINF priv-record trailer".into(),
                ));
            }
            raw_priv.truncate(432);
        }
        _ => {
            return Err(FairplayError::Sinf(
                "unsupported SINF priv-record length".into(),
            ));
        }
    }
    if iviv.len() != 16 {
        return Err(FairplayError::Sinf("invalid SINF iviv length".into()));
    }

    let mut md5 = Md5::new();
    md5.update(name);
    md5.update(iviv);
    let iv: [u8; 16] = md5.finalize().into();

    let mut key = *credential;
    for b in &mut key {
        *b ^= b'0';
    }
    let clear = aes_cbc_decrypt_nopad(&key, &iv, &raw_priv);
    if clear[0..4] != *b"itun" || clear[20..24] != *b"key " || clear[44..48] != *b"iviv" {
        return Err(FairplayError::WrongCredential);
    }
    Ok(clear[24..40].try_into().unwrap())
}

// ---- SUPF (page records) parsing -------------------------------------------

/// Offset of the first arm64 page record in the SUPF for `wanted_subtype`.
pub fn arm64_supf_record_offset(
    supf: &[u8],
    required_pages: usize,
    wanted_subtype: u32,
) -> Result<usize, FairplayError> {
    let err = |m: &str| FairplayError::Supf(m.to_string());
    if supf.len() < 12
        || (supf[0] != 3 && supf[0] != 5)
        || !matches!(&supf[1..4], b"507" | b"510" | b"612")
    {
        return Err(err("unsupported SUPF format"));
    }
    let header_size = if supf[0] == 3 { 32 } else { 12 };
    let count = read_u32_be(supf, 8).ok_or_else(|| err("truncated SUPF"))? as usize;
    if count == 0 || count > 16 {
        return Err(err("invalid SUPF architecture count"));
    }
    let mut pos = 12;
    for _ in 0..count {
        if pos + 4 + header_size > supf.len() {
            return Err(err("truncated SUPF architecture header"));
        }
        let length = read_u32_be(supf, pos).unwrap() as usize;
        let cpu = read_u32_be(supf, pos + 4).unwrap();
        let subtype = read_u32_be(supf, pos + 8).unwrap();
        if length < header_size || pos + 4 + length > supf.len() {
            return Err(err("invalid SUPF architecture length"));
        }
        let pages = read_u32_be(supf, pos + header_size).unwrap() as usize;
        if cpu == ARM64 && subtype == wanted_subtype {
            if pages < required_pages || length < header_size + pages * 32 {
                return Err(err("SUPF arm64 page records are incomplete"));
            }
            return Ok(pos + 4 + header_size);
        }
        pos += length + 4;
    }
    Err(err("arm64 SUPF record missing"))
}

// ---- Page-key derivation ---------------------------------------------------

fn be_u16x8(block: &[u8; 16]) -> [u16; 8] {
    let mut out = [0u16; 8];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = u16::from_be_bytes([block[2 * i], block[2 * i + 1]]);
    }
    out
}

fn u16x8_be(state: &[u16; 8]) -> [u8; 16] {
    let mut out = [0u8; 16];
    for (i, &v) in state.iter().enumerate() {
        out[2 * i..2 * i + 2].copy_from_slice(&v.to_be_bytes());
    }
    out
}

fn xor8(a: &[u16; 8], b: &[u16; 8]) -> [u16; 8] {
    let mut out = [0u16; 8];
    for i in 0..8 {
        out[i] = a[i] ^ b[i];
    }
    out
}

fn finish(state: &[u16; 8]) -> [u16; 8] {
    let mut src = *state;
    for selectors in ROUND_SELECTORS {
        let mut dst = [0u16; 8];
        for (i, &(p0, p1)) in selectors.iter().enumerate() {
            let mixed = src[p0] as u32 + src[p1] as u32; // carry kept (up to 17 bits)
            let square = mixed as u64 * mixed as u64;
            let x = (square as u32) ^ (mixed >> 16) ^ ROUND_CONSTANTS[i];
            dst[2 * i] = (x & 0xffff) as u16;
            dst[2 * i + 1] = (x >> 16) as u16;
        }
        src = dst;
    }
    let mut dst = [0u16; 8];
    for (i, q) in FINAL_SELECTORS.iter().enumerate() {
        let a = ((src[q[1]] as u32) << 16) | src[q[0]] as u32;
        let b = ((src[q[3]] as u32) << 16) | src[q[2]] as u32;
        let mixed = a.wrapping_add(b); // carry dropped
        let square = mixed as u64 * mixed as u64;
        let x = (square as u32) ^ ((square >> 32) as u32) ^ FINAL_CONSTANTS[i];
        dst[2 * i] = (x & 0xffff) as u16;
        dst[2 * i + 1] = (x >> 16) as u16;
    }
    xor8(state, &dst)
}

fn source_blocks(wrapped: &[u8; 16], page_offset: &[u8; 8]) -> [[u8; 16]; 3] {
    let mut first = [0u8; 16];
    first[..8].copy_from_slice(&wrapped[8..16]);
    first[8..].copy_from_slice(page_offset);
    let second = *wrapped;
    let mut third = [0u8; 16];
    third[..8].copy_from_slice(page_offset);
    third[8..].copy_from_slice(&wrapped[..8]);
    [first, second, third]
}

/// The page key, keyed by the 8-byte page offset (`absolute_index << 12`).
fn page_key_with_offset(wrapped: &[u8; 16], page_offset: &[u8; 8], work: &[u8; 16]) -> [u8; 16] {
    let [b0, b1, b2] =
        source_blocks(wrapped, page_offset).map(|s| be_u16x8(&aes_ecb_encrypt_block(work, &s)));
    let mixed = finish(&xor8(&finish(&xor8(&finish(&b0), &b1)), &b2));
    u16x8_be(&mixed)
}

fn derive_page_key(wrapped: &[u8; 16], absolute_index: u64, work: &[u8; 16]) -> [u8; 16] {
    page_key_with_offset(wrapped, &(absolute_index << 12).to_be_bytes(), work)
}

/// Decrypt every encrypted arm64 slice in `source`, returning the decrypted
/// binary (with `cryptid` cleared) and the number of pages decrypted.
pub fn decrypt_binary(
    source: &[u8],
    sinf: &[u8],
    supf: &[u8],
    credential: &[u8; 16],
) -> Result<(Vec<u8>, usize), FairplayError> {
    let infos = arm64_encryption_infos(source)?;
    let work = derive_work16(credential, sinf)?;
    let mut result = source.to_vec();
    let mut total_pages = 0;
    for info in infos {
        let page_count = info.cryptsize / PAGE_SIZE;
        let record_start = arm64_supf_record_offset(supf, page_count, info.subtype)?;
        total_pages += page_count;
        for local_page in 0..page_count {
            let rec = supf
                .get(record_start + 32 * local_page..record_start + 32 * (local_page + 1))
                .ok_or_else(|| FairplayError::Supf("truncated SUPF page record".into()))?;
            let wrapped: &[u8; 16] = rec[..16].try_into().unwrap();
            let iv: &[u8; 16] = rec[16..32].try_into().unwrap();
            let absolute_index = ((info.base + info.cryptoff) / PAGE_SIZE + local_page) as u64;
            let key = derive_page_key(wrapped, absolute_index, &work);
            let start = info.base + info.cryptoff + PAGE_SIZE * local_page;
            let plain = aes_cbc_decrypt_nopad(&key, iv, &source[start..start + PAGE_SIZE]);
            result[start..start + PAGE_SIZE].copy_from_slice(&plain);
        }
        result[info.cryptid_offset..info.cryptid_offset + 4].copy_from_slice(&0u32.to_le_bytes());
    }
    Ok((result, total_pages))
}

// ---- IPA-level orchestration (port of decrypt.py) --------------------------

/// Prefer `<binary>.v5.supf` over `<binary>.supf` for each binary.
fn supf_paths(names: &[String]) -> Vec<String> {
    use std::collections::BTreeMap;
    let mut by_binary: BTreeMap<String, String> = BTreeMap::new();
    let mut sorted = names.to_vec();
    sorted.sort();
    for name in sorted {
        if let Some(stem) = name.strip_suffix(".v5.supf") {
            by_binary.insert(stem.to_string(), name.clone());
        } else if let Some(stem) = name.strip_suffix(".supf") {
            by_binary
                .entry(stem.to_string())
                .or_insert_with(|| name.clone());
        }
    }
    let mut out: Vec<String> = by_binary.into_values().collect();
    out.sort();
    out
}

/// The SINF for a binary with no adjacent SINF: the enclosing `.app`'s own.
fn enclosing_app_sinf(names: &[String], bundle: &str) -> Option<String> {
    let parts: Vec<&str> = bundle.split('/').collect();
    for i in (1..=parts.len()).rev() {
        let prefix = parts[..i].join("/");
        if !prefix.ends_with(".app") {
            continue;
        }
        let sc_info = format!("{prefix}/SC_Info/");
        let stem = Path::new(&prefix)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let depth = sc_info.matches('/').count();
        let mut sinfs: Vec<&String> = names
            .iter()
            .filter(|n| {
                n.starts_with(&sc_info) && n.ends_with(".sinf") && n.matches('/').count() == depth
            })
            .collect();
        if sinfs.is_empty() {
            continue;
        }
        sinfs.sort();
        for suffix in [".sinf", ".v4.sinf"] {
            let preferred = format!("{sc_info}{stem}{suffix}");
            if sinfs.iter().any(|n| **n == preferred) {
                return Some(preferred);
            }
        }
        if let Some(unversioned) = sinfs.iter().find(|n| !n.ends_with(".v4.sinf")) {
            return Some((*unversioned).clone());
        }
        return Some(sinfs[0].clone());
    }
    None
}

/// Pair each SUPF with its binary and SINF. `read` fetches a member's bytes by
/// name, used only to skip binaries that aren't actually encrypted. Returns
/// `(binary, sinf, supf)` name triples.
fn plan_decryption(
    names: &[String],
    mut read: impl FnMut(&str) -> Result<Vec<u8>, FairplayError>,
) -> Result<Vec<(String, String, String)>, FairplayError> {
    let mut plan = Vec::new();
    for supf_path in supf_paths(names) {
        let (bundle, filename) = supf_path
            .rsplit_once("/SC_Info/")
            .ok_or_else(|| FairplayError::Supf(format!("SUPF outside SC_Info: {supf_path}")))?;
        let is_v5 = filename.ends_with(".v5.supf");
        let stem = filename
            .strip_suffix(".v5.supf")
            .or_else(|| filename.strip_suffix(".supf"))
            .unwrap();
        let binary_path = format!("{bundle}/{stem}");
        if !names.iter().any(|n| n == &binary_path) {
            return Err(FairplayError::Supf(format!(
                "SUPF lacks binary: {supf_path}"
            )));
        }
        let source = read(&binary_path)?;
        if arm64_encryption_infos(&source)?.is_empty() {
            continue;
        }
        let sinf_name = if is_v5 {
            format!("{stem}.v4.sinf")
        } else {
            format!("{stem}.sinf")
        };
        let mut sinf_path = format!("{bundle}/SC_Info/{sinf_name}");
        if !names.iter().any(|n| n == &sinf_path) && is_v5 {
            sinf_path = format!("{bundle}/SC_Info/{stem}.sinf");
        }
        if !names.iter().any(|n| n == &sinf_path) {
            sinf_path = enclosing_app_sinf(names, bundle)
                .ok_or_else(|| FairplayError::Sinf(format!("SUPF lacks SINF: {supf_path}")))?;
        }
        plan.push((binary_path, sinf_path, supf_path));
    }
    Ok(plan)
}

/// Every file under `dir`, as `"/"`-joined names below `prefix`.
fn collect_names(dir: &Path, prefix: &str, out: &mut Vec<String>) -> Result<(), FairplayError> {
    for entry in std::fs::read_dir(dir).map_err(io_err(dir))? {
        let entry = entry.map_err(io_err(dir))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel = format!("{prefix}/{name}");
        if entry.file_type().map_err(io_err(&path))?.is_dir() {
            collect_names(&path, &rel, out)?;
        } else {
            out.push(rel);
        }
    }
    Ok(())
}

/// Decrypt the FairPlay-encrypted binaries of an already-extracted `.app`, in
/// place, using the `SC_Info` records still present in the bundle. Returns a
/// zero summary (not an error) when nothing is encrypted, so it's safe to run
/// unconditionally before signing.
pub fn decrypt_app_dir(
    app_dir: &Path,
    credential: &[u8; 16],
    mut on_binary: impl FnMut(usize, usize, &str, usize),
) -> Result<DecryptSummary, FairplayError> {
    let root = app_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut names = Vec::new();
    collect_names(app_dir, &root, &mut names)?;
    let prefix = format!("{root}/");
    let to_abs = |name: &str| app_dir.join(name.strip_prefix(&prefix).unwrap_or(name));

    let plan = plan_decryption(&names, |name| {
        std::fs::read(to_abs(name)).map_err(io_err(&to_abs(name)))
    })?;

    let mut summary = DecryptSummary::default();
    let total = plan.len();
    for (ordinal, (binary_path, sinf_path, supf_path)) in plan.iter().enumerate() {
        let source = std::fs::read(to_abs(binary_path)).map_err(io_err(&to_abs(binary_path)))?;
        let sinf = std::fs::read(to_abs(sinf_path)).map_err(io_err(&to_abs(sinf_path)))?;
        let supf = std::fs::read(to_abs(supf_path)).map_err(io_err(&to_abs(supf_path)))?;
        let (result, pages) = decrypt_binary(&source, &sinf, &supf, credential)?;
        std::fs::write(to_abs(binary_path), &result).map_err(io_err(&to_abs(binary_path)))?;
        on_binary(ordinal + 1, total, binary_path, pages);
        summary.binaries += 1;
        summary.pages += pages;
    }
    Ok(summary)
}

fn io_err(path: &Path) -> impl FnOnce(std::io::Error) -> FairplayError + '_ {
    move |source| FairplayError::Io {
        path: path.display().to_string(),
        source,
    }
}

fn zip_err(path: &Path) -> impl FnOnce(zip::result::ZipError) -> FairplayError + '_ {
    move |source| FairplayError::Zip {
        path: path.display().to_string(),
        source,
    }
}

/// Decrypt an owned, encrypted IPA to `output` using a 16-byte credential.
///
/// `on_binary(ordinal, total, path, pages)` is called as each binary is
/// decrypted, for progress reporting. The output keeps `SC_Info` and
/// `iTunesMetadata.plist` (it is still a licensed app, now decrypted).
pub fn decrypt_ipa(
    ipa: &Path,
    output: &Path,
    credential: &[u8; 16],
    mut on_binary: impl FnMut(usize, usize, &str, usize),
) -> Result<DecryptSummary, FairplayError> {
    if output.exists() {
        return Err(FairplayError::OutputExists(output.display().to_string()));
    }
    let file = File::open(ipa).map_err(io_err(ipa))?;
    let mut archive = ZipArchive::new(file).map_err(zip_err(ipa))?;
    let names: Vec<String> = archive.file_names().map(String::from).collect();

    let plan = plan_decryption(&names, |name| read_entry(&mut archive, name))?;
    if plan.is_empty() {
        return Err(FairplayError::NothingToDecrypt);
    }

    // Decrypt each planned binary, collecting replacement bytes by path.
    let mut replacements: std::collections::HashMap<String, Vec<u8>> =
        std::collections::HashMap::new();
    let mut summary = DecryptSummary::default();
    let total = plan.len();
    for (ordinal, (binary_path, sinf_path, supf_path)) in plan.iter().enumerate() {
        let source = read_entry(&mut archive, binary_path)?;
        let sinf = read_entry(&mut archive, sinf_path)?;
        let supf = read_entry(&mut archive, supf_path)?;
        let (result, pages) = decrypt_binary(&source, &sinf, &supf, credential)?;
        on_binary(ordinal + 1, total, binary_path, pages);
        summary.binaries += 1;
        summary.pages += pages;
        replacements.insert(binary_path.clone(), result);
    }

    // Rewrite the zip to `output.partial`, substituting decrypted binaries and
    // copying everything else verbatim, then rename into place.
    let partial = with_extension_suffix(output, "partial");
    {
        let out_file = File::create(&partial).map_err(io_err(&partial))?;
        let mut writer = ZipWriter::new(out_file);
        for index in 0..archive.len() {
            let entry = archive.by_index_raw(index).map_err(zip_err(ipa))?;
            let name = entry.name().to_string();
            if let Some(data) = replacements.get(&name) {
                let options = SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated);
                drop(entry);
                writer
                    .start_file(&name, options)
                    .map_err(zip_err(&partial))?;
                writer.write_all(data).map_err(io_err(&partial))?;
            } else {
                writer.raw_copy_file(entry).map_err(zip_err(&partial))?;
            }
        }
        writer.finish().map_err(zip_err(&partial))?;
    }
    std::fs::rename(&partial, output).map_err(io_err(output))?;
    Ok(summary)
}

fn read_entry<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    name: &str,
) -> Result<Vec<u8>, FairplayError> {
    let mut entry = archive.by_name(name).map_err(|source| FairplayError::Zip {
        path: name.to_string(),
        source,
    })?;
    let mut data = Vec::new();
    entry
        .read_to_end(&mut data)
        .map_err(|source| FairplayError::Io {
            path: name.to_string(),
            source,
        })?;
    Ok(data)
}

fn with_extension_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".");
    name.push(suffix);
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AES-128-CBC encrypt, no padding (test-only; the module only decrypts).
    fn aes_cbc_encrypt_nopad(key: &[u8; 16], iv: &[u8; 16], data: &[u8]) -> Vec<u8> {
        let cipher = Aes128::new(key.into());
        let mut out = vec![0u8; data.len()];
        let mut prev = *iv;
        for (chunk, slot) in data.chunks_exact(16).zip(out.chunks_exact_mut(16)) {
            let mut block = [0u8; 16];
            for j in 0..16 {
                block[j] = chunk[j] ^ prev[j];
            }
            let mut b = block.into();
            cipher.encrypt_block(&mut b);
            let enc: [u8; 16] = b.into();
            slot.copy_from_slice(&enc);
            prev = enc;
        }
        out
    }

    fn atom(fourcc: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = ((8 + body.len()) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(fourcc);
        v.extend_from_slice(body);
        v
    }

    /// A SINF whose priv record decodes under `credential` to `work`.
    fn build_sinf(credential: &[u8; 16], work: &[u8; 16]) -> Vec<u8> {
        let name = b"TestApp";
        let iviv: [u8; 16] = std::array::from_fn(|i| (i as u8).wrapping_mul(37).wrapping_add(5));
        let mut clear = vec![0u8; 448];
        for (i, b) in clear.iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(11).wrapping_add(3);
        }
        clear[0..4].copy_from_slice(b"itun");
        clear[20..24].copy_from_slice(b"key ");
        clear[24..40].copy_from_slice(work);
        clear[44..48].copy_from_slice(b"iviv");
        let mut md5 = Md5::new();
        md5.update(name);
        md5.update(iviv);
        let iv: [u8; 16] = md5.finalize().into();
        let mut key = *credential;
        for b in &mut key {
            *b ^= b'0';
        }
        let priv_rec = aes_cbc_encrypt_nopad(&key, &iv, &clear);
        let schi = [
            atom(b"name", name),
            atom(b"iviv", &iviv),
            atom(b"priv", &priv_rec),
        ]
        .concat();
        atom(b"sinf", &atom(b"schi", &schi))
    }

    /// A thin arm64 Mach-O with one LC_ENCRYPTION_INFO_64; returns the buffer
    /// and the file offset of its `cryptid` field.
    fn build_macho(cryptoff: usize, cryptsize: usize, subtype: u32) -> (Vec<u8>, usize) {
        let mut enc = Vec::new();
        enc.extend(LC_ENCRYPTION_INFO_64.to_le_bytes());
        enc.extend(0x18u32.to_le_bytes()); // cmdsize (24: the struct has a pad word)
        enc.extend((cryptoff as u32).to_le_bytes());
        enc.extend((cryptsize as u32).to_le_bytes());
        enc.extend(1u32.to_le_bytes()); // cryptid
        enc.extend(0u32.to_le_bytes()); // pad
        let mut buf = Vec::new();
        buf.extend(MH_MAGIC_64.to_le_bytes());
        buf.extend(ARM64.to_le_bytes());
        buf.extend(subtype.to_le_bytes());
        buf.extend(2u32.to_le_bytes()); // MH_EXECUTE
        buf.extend(1u32.to_le_bytes()); // ncmds
        buf.extend((enc.len() as u32).to_le_bytes()); // sizeofcmds
        buf.extend(0u32.to_le_bytes()); // flags
        buf.extend(0u32.to_le_bytes()); // reserved
        let cryptid_offset = buf.len() + 16;
        buf.extend(&enc);
        buf.resize(cryptoff + cryptsize, 0);
        (buf, cryptid_offset)
    }

    fn build_supf_v5(subtype: u32, records: &[[u8; 32]]) -> Vec<u8> {
        let pages = records.len();
        let header_size = 12usize;
        let length = header_size + pages * 32;
        let mut supf = Vec::new();
        supf.push(5u8);
        supf.extend(b"510");
        supf.extend([0u8; 4]);
        supf.extend(1u32.to_be_bytes()); // arch count
        supf.extend((length as u32).to_be_bytes());
        supf.extend(ARM64.to_be_bytes());
        supf.extend(subtype.to_be_bytes());
        supf.extend((pages as u32).to_be_bytes());
        for rec in records {
            supf.extend(rec);
        }
        supf
    }

    // FIPS-197 AES-128 known-answer vector, to prove the ECB block primitive.
    #[test]
    fn aes_ecb_matches_fips197() {
        let key: [u8; 16] = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
            0x0e, 0x0f,
        ];
        let plain: [u8; 16] = [
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff,
        ];
        let want: [u8; 16] = [
            0x69, 0xc4, 0xe0, 0xd8, 0x6a, 0x7b, 0x04, 0x30, 0xd8, 0xcd, 0xb7, 0x80, 0x70, 0xb4,
            0xc5, 0x5a,
        ];
        assert_eq!(aes_ecb_encrypt_block(&key, &plain), want);
    }

    // Cross-implementation known-answer vector from DrWhax/unfairplay
    // (pkg/fairplay/pagekey_test.go): a different author, in Go, deriving the
    // same page key. Anchors the whole AES + source_blocks + finish path.
    #[test]
    fn page_key_matches_unfairplay_kat() {
        // UnfairPlay's effective AES key = fixedPageAESKey XOR appMask.
        let fixed: [u8; 16] = [
            0xf5, 0x88, 0xff, 0xe1, 0x89, 0xf9, 0x89, 0xf8, 0xe1, 0xf8, 0x89, 0xfe, 0x8a, 0xe1,
            0xf4, 0xfe,
        ];
        let mask: [u8; 16] = [
            0x79, 0x89, 0x1e, 0xea, 0x60, 0xad, 0x2e, 0x7e, 0xa7, 0x5f, 0xe7, 0xff, 0x29, 0x1b,
            0xc0, 0x82,
        ];
        let work: [u8; 16] = std::array::from_fn(|i| fixed[i] ^ mask[i]);
        let wrapped: [u8; 16] = std::array::from_fn(|i| 0xf0 + i as u8);
        let page_offset: [u8; 8] = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        let want: [u8; 16] = [
            0x45, 0xf8, 0x9e, 0xa4, 0xa3, 0xa1, 0xe9, 0xb9, 0x7e, 0x03, 0x8b, 0xcb, 0x27, 0xf2,
            0x69, 0xc9,
        ];
        assert_eq!(page_key_with_offset(&wrapped, &page_offset, &work), want);
    }

    #[test]
    fn derive_work16_round_trips_through_sinf() {
        let credential: [u8; 16] =
            std::array::from_fn(|i| (i as u8).wrapping_mul(7).wrapping_add(1));
        let work: [u8; 16] = std::array::from_fn(|i| (i as u8).wrapping_mul(13).wrapping_add(9));
        let sinf = build_sinf(&credential, &work);
        assert_eq!(derive_work16(&credential, &sinf).unwrap(), work);
    }

    #[test]
    fn derive_work16_rejects_wrong_credential() {
        let credential: [u8; 16] = [1; 16];
        let work: [u8; 16] = [2; 16];
        let sinf = build_sinf(&credential, &work);
        let wrong: [u8; 16] = [9; 16];
        assert!(matches!(
            derive_work16(&wrong, &sinf),
            Err(FairplayError::WrongCredential)
        ));
    }

    // Full forward/reverse round-trip: encrypt pages with the derived keys, then
    // decrypt_binary must recover the plaintext and clear cryptid. Exercises
    // the Mach-O, SINF and SUPF parsing and the whole page pipeline.
    #[test]
    fn decrypt_binary_round_trips() {
        for npages in [1usize, 3] {
            let credential: [u8; 16] =
                std::array::from_fn(|i| (i as u8).wrapping_mul(5).wrapping_add(2));
            let work: [u8; 16] =
                std::array::from_fn(|i| (i as u8).wrapping_mul(17).wrapping_add(4));
            let subtype = 0u32;
            let cryptoff = 0x4000usize;
            let cryptsize = npages * PAGE_SIZE;

            let sinf = build_sinf(&credential, &work);
            let (mut plaintext, cryptid_off) = build_macho(cryptoff, cryptsize, subtype);
            for p in 0..npages {
                let start = cryptoff + p * PAGE_SIZE;
                for i in 0..PAGE_SIZE {
                    plaintext[start + i] = ((p * 7 + i) & 0xff) as u8;
                }
            }

            let mut encrypted = plaintext.clone();
            let mut records: Vec<[u8; 32]> = Vec::new();
            for p in 0..npages {
                let wrapped: [u8; 16] =
                    std::array::from_fn(|i| (p as u8).wrapping_mul(31).wrapping_add(i as u8));
                let iv: [u8; 16] =
                    std::array::from_fn(|i| (p as u8).wrapping_mul(19).wrapping_add(i as u8 + 1));
                let mut rec = [0u8; 32];
                rec[..16].copy_from_slice(&wrapped);
                rec[16..].copy_from_slice(&iv);
                records.push(rec);
                let absolute_index = (cryptoff / PAGE_SIZE + p) as u64;
                let key = derive_page_key(&wrapped, absolute_index, &work);
                let start = cryptoff + p * PAGE_SIZE;
                let ct = aes_cbc_encrypt_nopad(&key, &iv, &plaintext[start..start + PAGE_SIZE]);
                encrypted[start..start + PAGE_SIZE].copy_from_slice(&ct);
            }
            let supf = build_supf_v5(subtype, &records);

            let (result, pages) = decrypt_binary(&encrypted, &sinf, &supf, &credential).unwrap();
            assert_eq!(pages, npages);
            let mut expected = plaintext.clone();
            expected[cryptid_off..cryptid_off + 4].copy_from_slice(&0u32.to_le_bytes());
            assert_eq!(result, expected);
        }
    }

    /// Build an encrypted binary + its SINF/SUPF, returning (encrypted, sinf,
    /// supf, decrypted-with-cryptid-cleared).
    fn encrypted_sample(npages: usize) -> (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) {
        let credential: [u8; 16] =
            std::array::from_fn(|i| (i as u8).wrapping_mul(5).wrapping_add(2));
        let work: [u8; 16] = std::array::from_fn(|i| (i as u8).wrapping_mul(17).wrapping_add(4));
        let cryptoff = 0x4000usize;
        let sinf = build_sinf(&credential, &work);
        let (mut plaintext, cryptid_off) = build_macho(cryptoff, npages * PAGE_SIZE, 0);
        for p in 0..npages {
            let start = cryptoff + p * PAGE_SIZE;
            for i in 0..PAGE_SIZE {
                plaintext[start + i] = ((p * 7 + i) & 0xff) as u8;
            }
        }
        let mut encrypted = plaintext.clone();
        let mut records: Vec<[u8; 32]> = Vec::new();
        for p in 0..npages {
            let wrapped: [u8; 16] =
                std::array::from_fn(|i| (p as u8).wrapping_mul(31).wrapping_add(i as u8));
            let iv: [u8; 16] =
                std::array::from_fn(|i| (p as u8).wrapping_mul(19).wrapping_add(i as u8 + 1));
            let mut rec = [0u8; 32];
            rec[..16].copy_from_slice(&wrapped);
            rec[16..].copy_from_slice(&iv);
            records.push(rec);
            let key = derive_page_key(&wrapped, (cryptoff / PAGE_SIZE + p) as u64, &work);
            let start = cryptoff + p * PAGE_SIZE;
            let ct = aes_cbc_encrypt_nopad(&key, &iv, &plaintext[start..start + PAGE_SIZE]);
            encrypted[start..start + PAGE_SIZE].copy_from_slice(&ct);
        }
        let supf = build_supf_v5(0, &records);
        let mut expected = plaintext;
        expected[cryptid_off..cryptid_off + 4].copy_from_slice(&0u32.to_le_bytes());
        (encrypted, sinf, supf, expected)
    }

    #[test]
    fn decrypt_ipa_round_trips() {
        use std::io::Read as _;
        let (encrypted, sinf, supf, expected) = encrypted_sample(2);
        let credential: [u8; 16] =
            std::array::from_fn(|i| (i as u8).wrapping_mul(5).wrapping_add(2));

        let dir = tempfile::tempdir().unwrap();
        let ipa_path = dir.path().join("app.ipa");
        {
            let mut zip = ZipWriter::new(File::create(&ipa_path).unwrap());
            let opt = SimpleFileOptions::default();
            let write = |zip: &mut ZipWriter<File>, name: &str, data: &[u8]| {
                zip.start_file(name, opt).unwrap();
                zip.write_all(data).unwrap();
            };
            write(&mut zip, "Payload/Test.app/Test", &encrypted);
            write(&mut zip, "Payload/Test.app/SC_Info/Test.sinf", &sinf);
            write(&mut zip, "Payload/Test.app/SC_Info/Test.supf", &supf);
            write(&mut zip, "Payload/Test.app/Info.plist", b"<plist/>");
            zip.finish().unwrap();
        }

        let out_path = dir.path().join("out.ipa");
        let summary = decrypt_ipa(&ipa_path, &out_path, &credential, |_, _, _, _| {}).unwrap();
        assert_eq!(summary.binaries, 1);
        assert_eq!(summary.pages, 2);

        let mut out = ZipArchive::new(File::open(&out_path).unwrap()).unwrap();
        let mut got = Vec::new();
        out.by_name("Payload/Test.app/Test")
            .unwrap()
            .read_to_end(&mut got)
            .unwrap();
        assert_eq!(got, expected);
        // Untouched entries are preserved.
        let mut info = Vec::new();
        out.by_name("Payload/Test.app/Info.plist")
            .unwrap()
            .read_to_end(&mut info)
            .unwrap();
        assert_eq!(info, b"<plist/>");
    }

    #[test]
    fn decrypt_app_dir_decrypts_in_place() {
        let (encrypted, sinf, supf, expected) = encrypted_sample(2);
        let credential: [u8; 16] =
            std::array::from_fn(|i| (i as u8).wrapping_mul(5).wrapping_add(2));

        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("Test.app");
        std::fs::create_dir_all(app.join("SC_Info")).unwrap();
        std::fs::write(app.join("Test"), &encrypted).unwrap();
        std::fs::write(app.join("SC_Info/Test.sinf"), &sinf).unwrap();
        std::fs::write(app.join("SC_Info/Test.supf"), &supf).unwrap();

        let summary = decrypt_app_dir(&app, &credential, |_, _, _, _| {}).unwrap();
        assert_eq!(summary.binaries, 1);
        assert_eq!(summary.pages, 2);
        assert_eq!(std::fs::read(app.join("Test")).unwrap(), expected);
    }

    #[test]
    fn decrypt_app_dir_is_noop_without_encryption() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("Plain.app");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(app.join("Plain"), b"not a mach-o").unwrap();
        let summary = decrypt_app_dir(&app, &[0u8; 16], |_, _, _, _| {}).unwrap();
        assert_eq!(summary.binaries, 0);
    }

    #[test]
    fn decrypt_ipa_refuses_existing_output() {
        let dir = tempfile::tempdir().unwrap();
        let ipa = dir.path().join("a.ipa");
        std::fs::write(&ipa, b"x").unwrap();
        let out = dir.path().join("out.ipa");
        std::fs::write(&out, b"existing").unwrap();
        assert!(matches!(
            decrypt_ipa(&ipa, &out, &[0u8; 16], |_, _, _, _| {}),
            Err(FairplayError::OutputExists(_))
        ));
    }

    #[test]
    fn decrypted_binary_reports_no_encryption() {
        let credential: [u8; 16] = [3; 16];
        let work: [u8; 16] = [4; 16];
        let sinf = build_sinf(&credential, &work);
        let (plaintext, _) = build_macho(0x4000, PAGE_SIZE, 0);
        let wrapped = [7u8; 16];
        let iv = [8u8; 16];
        let mut rec = [0u8; 32];
        rec[..16].copy_from_slice(&wrapped);
        rec[16..].copy_from_slice(&iv);
        let key = derive_page_key(&wrapped, (0x4000 / PAGE_SIZE) as u64, &work);
        let mut encrypted = plaintext.clone();
        let ct = aes_cbc_encrypt_nopad(&key, &iv, &plaintext[0x4000..0x4000 + PAGE_SIZE]);
        encrypted[0x4000..0x4000 + PAGE_SIZE].copy_from_slice(&ct);
        let supf = build_supf_v5(0, &[rec]);
        let (result, _) = decrypt_binary(&encrypted, &sinf, &supf, &credential).unwrap();
        // The decrypted binary no longer advertises encryption.
        assert!(arm64_encryption_infos(&result).unwrap().is_empty());
    }
}
