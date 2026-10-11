//! What's in an IPA, read straight from the zip without extracting it: for
//! listing apps in a library before anything is signed.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use plist::{Dictionary, Value};
use zip::ZipArchive;

use crate::ipa::IpaError;

#[derive(Debug, thiserror::Error)]
pub enum MetadataError {
    #[error(transparent)]
    Ipa(#[from] IpaError),
    #[error("{path}: {source}")]
    Zip {
        path: String,
        source: zip::result::ZipError,
    },
    #[error("{path}: invalid Info.plist: {source}")]
    InfoPlist { path: String, source: plist::Error },
    #[error("{0}: Info.plist has no CFBundleIdentifier")]
    NoBundleId(String),
}

/// Whether an app's main executable is FairPlay-encrypted
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encryption {
    /// Decrypted (or never encrypted): can be re-signed
    Decrypted,
    /// Still encrypted as downloaded from the App Store. Re-signed, it
    /// installs but crashes on launch.
    Encrypted,
    /// Couldn't tell (missing or unusual executable)
    Unknown,
}

#[derive(Debug, Clone)]
pub struct IpaMetadata {
    /// The .app's name inside Payload/, e.g. "Test.app"
    pub app_name: String,
    /// CFBundleDisplayName, CFBundleName, or the .app's name
    pub display_name: String,
    pub bundle_id: String,
    /// CFBundleShortVersionString
    pub version: Option<String>,
    /// CFBundleVersion
    pub build: Option<String>,
    pub minimum_os: Option<String>,
    /// Size of the IPA file
    pub file_size: u64,
    /// Total uncompressed size
    pub installed_size: u64,
    /// App extensions in PlugIns/ and Extensions/
    pub extension_count: u32,
    pub encryption: Encryption,
    /// The largest app icon PNG, as stored. Usually Apple's CgBI variant,
    /// which iOS decodes but most other PNG decoders don't.
    pub icon: Option<Vec<u8>>,
    pub info_plist: Dictionary,
}

/// Read an IPA's app metadata and icon
pub fn read_ipa_metadata(ipa: &Path) -> Result<IpaMetadata, MetadataError> {
    let display = || ipa.display().to_string();
    let zip_error = |source| MetadataError::Zip {
        path: display(),
        source,
    };
    let file = File::open(ipa).map_err(|source| IpaError::Io {
        path: display(),
        source,
    })?;
    let file_size = file.metadata().map(|m| m.len()).unwrap_or(0);
    let mut archive = ZipArchive::new(BufReader::new(file)).map_err(zip_error)?;

    let names: Vec<String> = archive.file_names().map(String::from).collect();
    let mut apps: Vec<&str> = names
        .iter()
        .filter_map(|name| {
            let parts: Vec<&str> = name.split('/').collect();
            match parts.as_slice() {
                ["Payload", app, "Info.plist"] if app.ends_with(".app") => Some(*app),
                _ => None,
            }
        })
        .collect();
    apps.sort_unstable();
    apps.dedup();
    let app_name = match apps.as_slice() {
        [] => return Err(IpaError::NoApp(display()).into()),
        [app] => app.to_string(),
        _ => return Err(IpaError::MultipleApps(display()).into()),
    };
    let app_prefix = format!("Payload/{app_name}/");

    let info_data = read_entry(&mut archive, &format!("{app_prefix}Info.plist"), u64::MAX)
        .map_err(zip_error)?;
    let info_plist: Dictionary =
        plist::from_bytes(&info_data).map_err(|source| MetadataError::InfoPlist {
            path: display(),
            source,
        })?;
    let info_string = |key: &str| {
        info_plist
            .get(key)
            .and_then(Value::as_string)
            .filter(|s| !s.is_empty())
            .map(String::from)
    };
    let bundle_id =
        info_string("CFBundleIdentifier").ok_or_else(|| MetadataError::NoBundleId(display()))?;
    let display_name = info_string("CFBundleDisplayName")
        .or_else(|| info_string("CFBundleName"))
        .unwrap_or_else(|| app_name.trim_end_matches(".app").to_string());

    let mut installed_size = 0;
    for index in 0..archive.len() {
        if let Ok(entry) = archive.by_index_raw(index) {
            installed_size += entry.size();
        }
    }
    let extension_count = names
        .iter()
        .filter(|name| {
            let Some(rest) = name.strip_prefix(&app_prefix) else {
                return false;
            };
            let parts: Vec<&str> = rest.split('/').collect();
            matches!(
                parts.as_slice(),
                ["PlugIns" | "Extensions", appex, "Info.plist"] if appex.ends_with(".appex")
            )
        })
        .count() as u32;

    let encryption = match info_string("CFBundleExecutable") {
        Some(executable) => {
            executable_encryption(&mut archive, &format!("{app_prefix}{executable}"))
        }
        None => Encryption::Unknown,
    };
    let icon = best_icon(&mut archive, &names, &app_prefix, &info_plist);

    Ok(IpaMetadata {
        app_name,
        display_name,
        bundle_id,
        version: info_string("CFBundleShortVersionString"),
        build: info_string("CFBundleVersion"),
        minimum_os: info_string("MinimumOSVersion"),
        file_size,
        installed_size,
        extension_count,
        encryption,
        icon,
        info_plist,
    })
}

/// Read up to `limit` bytes of an entry
fn read_entry<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    name: &str,
    limit: u64,
) -> Result<Vec<u8>, zip::result::ZipError> {
    let entry = archive.by_name(name)?;
    let mut data = Vec::new();
    entry.take(limit).read_to_end(&mut data)?;
    Ok(data)
}

/// Icon base names declared in Info.plist, e.g. "AppIcon60x60"
fn icon_names(info: &Dictionary) -> Vec<String> {
    let mut names = Vec::new();
    let mut add_files = |value: Option<&Value>| {
        for name in value.and_then(Value::as_array).into_iter().flatten() {
            if let Some(name) = name.as_string() {
                names.push(name.trim_end_matches(".png").to_string());
            }
        }
    };
    for key in ["CFBundleIcons", "CFBundleIcons~ipad"] {
        let primary = info
            .get(key)
            .and_then(Value::as_dictionary)
            .and_then(|icons| icons.get("CFBundlePrimaryIcon"))
            .and_then(Value::as_dictionary);
        add_files(primary.and_then(|p| p.get("CFBundleIconFiles")));
    }
    add_files(info.get("CFBundleIconFiles"));
    if let Some(name) = info.get("CFBundleIconFile").and_then(Value::as_string) {
        names.push(name.trim_end_matches(".png").to_string());
    }
    names.retain(|name| !name.is_empty());
    names
}

/// The highest-resolution icon PNG at the top of the app bundle
fn best_icon<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    names: &[String],
    app_prefix: &str,
    info: &Dictionary,
) -> Option<Vec<u8>> {
    let mut bases = icon_names(info);
    if bases.is_empty() {
        // Undeclared, but this is what Xcode names them
        bases = vec!["AppIcon".into(), "Icon".into()];
    }
    let candidates: Vec<&String> = names
        .iter()
        .filter(|name| {
            let Some(file) = name.strip_prefix(app_prefix) else {
                return false;
            };
            !file.contains('/')
                && file.to_ascii_lowercase().ends_with(".png")
                && bases.iter().any(|base| file.starts_with(base.as_str()))
        })
        .collect();

    let mut best: Option<(u32, Vec<u8>)> = None;
    for name in candidates {
        // Icons are small; anything huge isn't one
        let Ok(data) = read_entry(archive, name, 4 * 1024 * 1024) else {
            continue;
        };
        let Some(width) = png_width(&data) else {
            continue;
        };
        if best
            .as_ref()
            .is_none_or(|(best_width, _)| width > *best_width)
        {
            best = Some((width, data));
        }
    }
    best.map(|(_, data)| data)
}

/// Width from a PNG's IHDR chunk, allowing for Apple's CgBI chunk before it
fn png_width(data: &[u8]) -> Option<u32> {
    if !data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return None;
    }
    let mut offset = 8;
    while offset + 16 <= data.len() {
        let length = u32::from_be_bytes(data[offset..offset + 4].try_into().ok()?) as usize;
        let kind = &data[offset + 4..offset + 8];
        if kind == b"IHDR" {
            return Some(u32::from_be_bytes(
                data[offset + 8..offset + 12].try_into().ok()?,
            ));
        }
        offset += 12 + length;
    }
    None
}

fn executable_encryption<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    name: &str,
) -> Encryption {
    let Ok(entry) = archive.by_name(name) else {
        return Encryption::Unknown;
    };
    macho_encryption(entry).unwrap_or(Encryption::Unknown)
}

const FAT_MAGIC: u32 = 0xcafe_babe;
const FAT_MAGIC_64: u32 = 0xcafe_babf;
const MH_MAGIC: u32 = 0xfeed_face;
const MH_MAGIC_64: u32 = 0xfeed_facf;
const CPU_TYPE_ARM64: u32 = 0x0100_000c;
const LC_ENCRYPTION_INFO: u32 = 0x21;
const LC_ENCRYPTION_INFO_64: u32 = 0x2c;

/// Read a Mach-O's encryption state from a stream, reading only as far as
/// the (arm64) load commands
pub(crate) fn macho_encryption(mut reader: impl Read) -> Option<Encryption> {
    let mut header = [0u8; 8];
    reader.read_exact(&mut header).ok()?;
    let mut position = 8u64;
    let magic = u32::from_be_bytes(header[..4].try_into().ok()?);

    let mut start = Vec::from(header);
    if magic == FAT_MAGIC || magic == FAT_MAGIC_64 {
        let count = u32::from_be_bytes(header[4..8].try_into().ok()?) as usize;
        let entry_size = if magic == FAT_MAGIC { 20 } else { 32 };
        let mut table = vec![0u8; count.min(32) * entry_size];
        reader.read_exact(&mut table).ok()?;
        position += table.len() as u64;
        let mut slices: Vec<(u32, u64)> = table
            .chunks(entry_size)
            .map(|arch| {
                let cpu = u32::from_be_bytes(arch[..4].try_into().unwrap());
                let offset = if magic == FAT_MAGIC {
                    u64::from(u32::from_be_bytes(arch[8..12].try_into().unwrap()))
                } else {
                    u64::from_be_bytes(arch[8..16].try_into().unwrap())
                };
                (cpu, offset)
            })
            .collect();
        // arm64 is what runs on current devices; otherwise the first slice
        slices.sort_by_key(|(cpu, _)| *cpu != CPU_TYPE_ARM64);
        let offset = slices.first()?.1;
        std::io::copy(
            &mut (&mut reader).take(offset.checked_sub(position)?),
            &mut std::io::sink(),
        )
        .ok()?;
        start = vec![0u8; 8];
        reader.read_exact(&mut start).ok()?;
    }

    let magic = u32::from_le_bytes(start[..4].try_into().ok()?);
    let header_size = match magic {
        MH_MAGIC => 28,
        MH_MAGIC_64 => 32,
        _ => return None,
    };
    let mut rest = vec![0u8; header_size - 8];
    reader.read_exact(&mut rest).ok()?;
    let command_count = u32::from_le_bytes(rest[8..12].try_into().ok()?);
    let commands_size = u32::from_le_bytes(rest[12..16].try_into().ok()?) as usize;
    let mut commands = vec![0u8; commands_size.min(16 * 1024 * 1024)];
    reader.read_exact(&mut commands).ok()?;

    let mut offset = 0;
    for _ in 0..command_count {
        let command = commands.get(offset..offset + 8)?;
        let kind = u32::from_le_bytes(command[..4].try_into().ok()?);
        let size = u32::from_le_bytes(command[4..8].try_into().ok()?) as usize;
        if kind == LC_ENCRYPTION_INFO || kind == LC_ENCRYPTION_INFO_64 {
            let crypt_id = commands.get(offset + 16..offset + 20)?;
            return Some(if u32::from_le_bytes(crypt_id.try_into().ok()?) == 0 {
                Encryption::Decrypted
            } else {
                Encryption::Encrypted
            });
        }
        if size < 8 {
            return None;
        }
        offset += size;
    }
    // No encryption info at all: never encrypted
    Some(Encryption::Decrypted)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal 64-bit Mach-O with the given load commands
    fn thin(commands: &[(u32, Vec<u8>)]) -> Vec<u8> {
        let mut body = Vec::new();
        for (kind, payload) in commands {
            body.extend(kind.to_le_bytes());
            body.extend((8 + payload.len() as u32).to_le_bytes());
            body.extend(payload);
        }
        let mut data = Vec::new();
        data.extend(MH_MAGIC_64.to_le_bytes());
        data.extend(CPU_TYPE_ARM64.to_le_bytes());
        data.extend(0u32.to_le_bytes()); // cpusubtype
        data.extend(2u32.to_le_bytes()); // MH_EXECUTE
        data.extend((commands.len() as u32).to_le_bytes());
        data.extend((body.len() as u32).to_le_bytes());
        data.extend(0u32.to_le_bytes()); // flags
        data.extend(0u32.to_le_bytes()); // reserved
        data.extend(body);
        data.extend([0u8; 64]);
        data
    }

    fn encryption_info(crypt_id: u32) -> (u32, Vec<u8>) {
        let mut payload = Vec::new();
        payload.extend(0x4000u32.to_le_bytes()); // cryptoff
        payload.extend(0x1000u32.to_le_bytes()); // cryptsize
        payload.extend(crypt_id.to_le_bytes());
        payload.extend(0u32.to_le_bytes()); // pad
        (LC_ENCRYPTION_INFO_64, payload)
    }

    fn fat(slices: &[(u32, Vec<u8>)]) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend(FAT_MAGIC.to_be_bytes());
        data.extend((slices.len() as u32).to_be_bytes());
        let mut offset = 0x1000u32;
        let mut bodies = Vec::new();
        for (cpu, slice) in slices {
            data.extend(cpu.to_be_bytes());
            data.extend(0u32.to_be_bytes());
            data.extend(offset.to_be_bytes());
            data.extend((slice.len() as u32).to_be_bytes());
            data.extend(12u32.to_be_bytes());
            bodies.push((offset, slice));
            offset += 0x1000 * (1 + slice.len() as u32 / 0x1000);
        }
        for (offset, slice) in bodies {
            data.resize(offset as usize, 0);
            data.extend(slice.iter());
        }
        data
    }

    #[test]
    fn reads_encryption_state() {
        let uuid = (0x1b, vec![0u8; 16]);
        assert_eq!(
            macho_encryption(&thin(&[uuid.clone(), encryption_info(1)])[..]),
            Some(Encryption::Encrypted)
        );
        assert_eq!(
            macho_encryption(&thin(&[uuid.clone(), encryption_info(0)])[..]),
            Some(Encryption::Decrypted)
        );
        assert_eq!(
            macho_encryption(&thin(&[uuid])[..]),
            Some(Encryption::Decrypted)
        );
        assert_eq!(macho_encryption(&b"#!/bin/sh\n"[..]), None);
    }

    #[test]
    fn reads_arm64_slice_of_fat_binaries() {
        let armv7 = 12;
        let binary = fat(&[
            (armv7, thin(&[encryption_info(0)])),
            (CPU_TYPE_ARM64, thin(&[encryption_info(1)])),
        ]);
        assert_eq!(macho_encryption(&binary[..]), Some(Encryption::Encrypted));
    }

    #[test]
    fn reads_png_width_past_cgbi() {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend(4u32.to_be_bytes());
        png.extend(b"CgBI");
        png.extend([0u8; 8]); // data and CRC
        png.extend(13u32.to_be_bytes());
        png.extend(b"IHDR");
        png.extend(180u32.to_be_bytes());
        png.extend(180u32.to_be_bytes());
        png.extend([0u8; 9]);
        assert_eq!(png_width(&png), Some(180));
        assert_eq!(png_width(b"GIF89a"), None);
    }
}
