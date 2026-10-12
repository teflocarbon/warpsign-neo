//! Unpacking and repacking IPA files.
//!
//! Unlike Python warpsign (zipfile + shutil.make_archive), this keeps unix
//! permissions and symlinks both ways, and only packs `Payload/`.

use std::fs::File;
#[cfg(not(unix))]
use std::io::Read;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

#[derive(Debug, thiserror::Error)]
pub enum IpaError {
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
    #[error("{0} has no Payload/*.app")]
    NoApp(String),
    #[error("{0} has more than one app in Payload/")]
    MultipleApps(String),
    #[error("{0} is not an .app directory")]
    NotAnApp(String),
    #[error("{0} is neither an IPA nor a zip with one inside")]
    NoIpa(String),
    #[error("{0} has more than one IPA inside")]
    MultipleIpas(String),
}

fn io_error(path: &Path) -> impl FnOnce(std::io::Error) -> IpaError + '_ {
    move |source| IpaError::Io {
        path: path.display().to_string(),
        source,
    }
}

fn zip_error(path: &Path) -> impl FnOnce(zip::result::ZipError) -> IpaError + '_ {
    move |source| IpaError::Zip {
        path: path.display().to_string(),
        source,
    }
}

/// Extract an IPA into `dest` and return the path of the .app inside it.
///
/// Entry paths that would escape `dest` are rejected and symlinks pointing
/// outside it aren't created. Permissions stored in the IPA are restored, then
/// everything is made writable by the owner so it can be patched and signed.
pub fn extract_ipa(ipa: &Path, dest: &Path) -> Result<PathBuf, IpaError> {
    let file = File::open(ipa).map_err(io_error(ipa))?;
    let mut archive = ZipArchive::new(file).map_err(zip_error(ipa))?;
    archive.extract(dest).map_err(zip_error(ipa))?;
    make_owner_writable(dest)?;

    let payload = dest.join("Payload");
    let mut apps = Vec::new();
    if payload.is_dir() {
        for entry in std::fs::read_dir(&payload).map_err(io_error(&payload))? {
            let path = entry.map_err(io_error(&payload))?.path();
            if path.is_dir() && path.extension().is_some_and(|e| e == "app") {
                apps.push(path);
            }
        }
    }
    match apps.len() {
        0 => Err(IpaError::NoApp(ipa.display().to_string())),
        1 => Ok(apps.remove(0)),
        _ => Err(IpaError::MultipleApps(ipa.display().to_string())),
    }
}

/// If `path` is a zip wrapping an IPA (as GitHub's artifact downloads are),
/// write that IPA to `output` and return true. Returns false, writing
/// nothing, when `path` is an IPA itself.
pub fn unwrap_ipa(path: &Path, output: &Path) -> Result<bool, IpaError> {
    let file = File::open(path).map_err(io_error(path))?;
    let mut archive = ZipArchive::new(file).map_err(zip_error(path))?;
    if archive
        .file_names()
        .any(|name| name.starts_with("Payload/"))
    {
        return Ok(false);
    }
    // Finder adds __MACOSX/._name resource forks next to each file
    let ipas: Vec<String> = archive
        .file_names()
        .filter(|name| {
            !name.starts_with("__MACOSX/")
                && name.rsplit('/').next().is_some_and(|base| {
                    !base.starts_with("._") && base.to_ascii_lowercase().ends_with(".ipa")
                })
        })
        .map(String::from)
        .collect();
    let name = match ipas.as_slice() {
        [] => return Err(IpaError::NoIpa(path.display().to_string())),
        [name] => name,
        _ => return Err(IpaError::MultipleIpas(path.display().to_string())),
    };
    let mut entry = archive.by_name(name).map_err(zip_error(path))?;
    let mut out = BufWriter::new(File::create(output).map_err(io_error(output))?);
    std::io::copy(&mut entry, &mut out).map_err(io_error(output))?;
    out.flush().map_err(io_error(output))?;
    Ok(true)
}

#[cfg(unix)]
fn make_owner_writable(path: &Path) -> Result<(), IpaError> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = std::fs::symlink_metadata(path).map_err(io_error(path))?;
    if metadata.is_symlink() {
        return Ok(());
    }
    let mode = metadata.permissions().mode();
    let wanted = if metadata.is_dir() {
        mode | 0o700
    } else {
        mode | 0o600
    };
    if wanted != mode {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(wanted))
            .map_err(io_error(path))?;
    }
    if metadata.is_dir() {
        for entry in std::fs::read_dir(path).map_err(io_error(path))? {
            make_owner_writable(&entry.map_err(io_error(path))?.path())?;
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn make_owner_writable(_path: &Path) -> Result<(), IpaError> {
    Ok(())
}

/// Pack an .app into an IPA at `output` (as `Payload/<name>.app/...`).
///
/// Unix permissions and symlinks are stored. On platforms without unix
/// permissions, Mach-O files are stored as executable (0755) and other files
/// as 0644. The IPA is written to a temporary file and renamed into place.
pub fn package_ipa(app_dir: &Path, output: &Path) -> Result<(), IpaError> {
    let app_name = app_dir
        .file_name()
        .filter(|_| app_dir.extension().is_some_and(|e| e == "app"))
        .ok_or_else(|| IpaError::NotAnApp(app_dir.display().to_string()))?
        .to_string_lossy()
        .into_owned();

    let partial = output.with_extension("ipa.partial");
    let file = File::create(&partial).map_err(io_error(&partial))?;
    let mut zip = ZipWriter::new(BufWriter::new(file));

    let result = (|| {
        let directory = SimpleFileOptions::default().unix_permissions(0o755);
        zip.add_directory("Payload/", directory)
            .map_err(zip_error(output))?;
        add_to_zip(&mut zip, app_dir, &format!("Payload/{app_name}"), output)?;
        zip.finish().map_err(zip_error(output))?;
        Ok(())
    })();

    match result {
        Ok(()) => std::fs::rename(&partial, output).map_err(io_error(output)),
        Err(e) => {
            let _ = std::fs::remove_file(&partial);
            Err(e)
        }
    }
}

fn add_to_zip<W: Write + std::io::Seek>(
    zip: &mut ZipWriter<W>,
    path: &Path,
    name: &str,
    output: &Path,
) -> Result<(), IpaError> {
    let metadata = std::fs::symlink_metadata(path).map_err(io_error(path))?;

    if metadata.is_symlink() {
        let target = std::fs::read_link(path).map_err(io_error(path))?;
        zip.add_symlink(
            name,
            target.to_string_lossy(),
            SimpleFileOptions::default().unix_permissions(0o755),
        )
        .map_err(zip_error(output))?;
    } else if metadata.is_dir() {
        zip.add_directory(
            format!("{name}/"),
            SimpleFileOptions::default().unix_permissions(0o755),
        )
        .map_err(zip_error(output))?;
        let mut entries = std::fs::read_dir(path)
            .map_err(io_error(path))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(io_error(path))?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let child = format!("{name}/{}", entry.file_name().to_string_lossy());
            add_to_zip(zip, &entry.path(), &child, output)?;
        }
    } else {
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .unix_permissions(file_mode(path, &metadata)?)
            .large_file(metadata.len() >= u32::MAX as u64);
        zip.start_file(name, options).map_err(zip_error(output))?;
        // Streamed, so large apps don't have to fit in memory (matters on iOS)
        let mut file = File::open(path).map_err(io_error(path))?;
        std::io::copy(&mut file, zip).map_err(io_error(path))?;
    }
    Ok(())
}

#[cfg(unix)]
fn file_mode(_path: &Path, metadata: &std::fs::Metadata) -> Result<u32, IpaError> {
    use std::os::unix::fs::PermissionsExt;
    Ok(metadata.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn file_mode(path: &Path, _metadata: &std::fs::Metadata) -> Result<u32, IpaError> {
    let mut magic = [0u8; 4];
    let mut file = File::open(path).map_err(io_error(path))?;
    let read = file.read(&mut magic).map_err(io_error(path))?;
    Ok(if is_macho(&magic[..read]) {
        0o755
    } else {
        0o644
    })
}

/// Whether `contents` starts with a Mach-O or universal binary magic number
#[cfg_attr(unix, allow(dead_code))]
fn is_macho(contents: &[u8]) -> bool {
    const MAGICS: [[u8; 4]; 6] = [
        [0xfe, 0xed, 0xfa, 0xce],
        [0xce, 0xfa, 0xed, 0xfe],
        [0xfe, 0xed, 0xfa, 0xcf],
        [0xcf, 0xfa, 0xed, 0xfe],
        [0xca, 0xfe, 0xba, 0xbe],
        [0xbe, 0xba, 0xfe, 0xca],
    ];
    contents.len() >= 4 && MAGICS.iter().any(|magic| contents[..4] == *magic)
}
