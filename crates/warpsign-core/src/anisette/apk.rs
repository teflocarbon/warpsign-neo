//! Apple's ADI libraries, from Apple Music for Android.
//!
//! They're Apple's: never in the repository, the app or a release. Each
//! user's device fetches them from Apple. The APK is about 140 MB, but only
//! the two libraries are needed, so [`RangeReader`] reads just the ZIP
//! directory and those two entries with HTTP range requests.

use std::collections::HashMap;
use std::io::{self, Read, Seek, SeekFrom};

use super::AnisetteError;
use super::adi::{CORE_ADI, STORE_SERVICES_CORE};
use crate::portal::http::{Request, Transport};

pub const APPLE_MUSIC_APK: &str =
    "https://apps.mzstatic.com/content/android-apple-music-apk/applemusic.apk";

/// The two libraries, for arm64-v8a
#[derive(Clone)]
pub struct Libraries {
    pub store_services_core: Vec<u8>,
    pub core_adi: Vec<u8>,
}

impl std::fmt::Debug for Libraries {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Libraries")
            .field("store_services_core", &self.store_services_core.len())
            .field("core_adi", &self.core_adi.len())
            .finish()
    }
}

/// Take the libraries out of an APK (a ZIP)
pub fn extract(apk: impl Read + Seek) -> Result<Libraries, AnisetteError> {
    let mut archive =
        zip::ZipArchive::new(apk).map_err(|e| AnisetteError::Apk(format!("not a ZIP: {e}")))?;
    let mut take = |name: &str| {
        let path = format!("lib/arm64-v8a/{name}");
        let mut entry = archive
            .by_name(&path)
            .map_err(|_| AnisetteError::Apk(format!("no {path} in the APK")))?;
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry
            .read_to_end(&mut bytes)
            .map_err(|e| AnisetteError::Apk(format!("couldn't read {path}: {e}")))?;
        Ok::<_, AnisetteError>(bytes)
    };
    Ok(Libraries {
        store_services_core: take(STORE_SERVICES_CORE)?,
        core_adi: take(CORE_ADI)?,
    })
}

/// Fetch the libraries from Apple, reading only what's needed of the APK
pub fn download(transport: &dyn Transport) -> Result<Libraries, AnisetteError> {
    extract(RangeReader::new(transport, APPLE_MUSIC_APK)?)
}

const BLOCK: u64 = 1 << 20;

/// A remote file read with HTTP range requests, a block at a time
pub struct RangeReader<'a> {
    transport: &'a dyn Transport,
    url: String,
    length: u64,
    position: u64,
    blocks: HashMap<u64, Vec<u8>>,
    /// Bytes fetched, for reporting
    pub fetched: u64,
}

fn total_length(content_range: &str) -> Option<u64> {
    content_range.rsplit('/').next()?.trim().parse().ok()
}

impl<'a> RangeReader<'a> {
    pub fn new(transport: &'a dyn Transport, url: &str) -> Result<Self, AnisetteError> {
        let mut reader = Self {
            transport,
            url: url.to_string(),
            length: 0,
            position: 0,
            blocks: HashMap::new(),
            fetched: 0,
        };
        // The first block also says how long the file is
        let (block, length) = reader.fetch(0)?;
        reader.length = length;
        reader.blocks.insert(0, block);
        Ok(reader)
    }

    fn fetch(&mut self, index: u64) -> Result<(Vec<u8>, u64), AnisetteError> {
        let start = index * BLOCK;
        let end = start + BLOCK - 1;
        let request = Request::get(&self.url).header("Range", format!("bytes={start}-{end}"));
        let response = self
            .transport
            .send(&request)
            .map_err(|e| AnisetteError::Http(e.to_string()))?;
        if response.status != 206 {
            return Err(AnisetteError::Apk(format!(
                "the APK's server answered HTTP {} to a range request",
                response.status
            )));
        }
        let length = response
            .header("Content-Range")
            .and_then(total_length)
            .ok_or_else(|| AnisetteError::Apk("no Content-Range from the APK's server".into()))?;
        self.fetched += response.body.len() as u64;
        Ok((response.body, length))
    }

    fn block(&mut self, index: u64) -> io::Result<&[u8]> {
        if !self.blocks.contains_key(&index) {
            let (block, _) = self.fetch(index).map_err(io::Error::other)?;
            self.blocks.insert(index, block);
        }
        Ok(&self.blocks[&index])
    }
}

impl Read for RangeReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.position >= self.length || buf.is_empty() {
            return Ok(0);
        }
        let index = self.position / BLOCK;
        let offset = (self.position % BLOCK) as usize;
        let position = self.position;
        let block = self.block(index)?;
        if offset >= block.len() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("short block at {position}"),
            ));
        }
        let count = buf.len().min(block.len() - offset);
        buf[..count].copy_from_slice(&block[offset..offset + count]);
        self.position += count as u64;
        Ok(count)
    }
}

impl Seek for RangeReader<'_> {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let position = match to {
            SeekFrom::Start(at) => at as i128,
            SeekFrom::End(delta) => self.length as i128 + delta as i128,
            SeekFrom::Current(delta) => self.position as i128 + delta as i128,
        };
        if position < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek before the start",
            ));
        }
        self.position = position as u64;
        Ok(self.position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal::http::{Response, TransportError};
    use std::io::Write;
    use std::sync::Mutex;

    /// Serves `file` with range requests, counting them
    struct Ranges {
        file: Vec<u8>,
        requests: Mutex<u32>,
    }

    impl Transport for Ranges {
        fn send(&self, request: &Request) -> Result<Response, TransportError> {
            *self.requests.lock().unwrap() += 1;
            let range = request.header_value("Range").unwrap();
            let (start, end) = range.trim_start_matches("bytes=").split_once('-').unwrap();
            let start: usize = start.parse().unwrap();
            let end = (end.parse::<usize>().unwrap() + 1).min(self.file.len());
            Ok(Response {
                status: 206,
                headers: vec![(
                    "Content-Range".into(),
                    format!("bytes {start}-{}/{}", end - 1, self.file.len()),
                )],
                body: self.file[start..end].to_vec(),
            })
        }
    }

    fn apk(padding: usize) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut zip = zip::ZipWriter::new(io::Cursor::new(&mut bytes));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("assets/big.bin", options).unwrap();
        zip.write_all(&vec![7u8; padding]).unwrap();
        zip.start_file("lib/arm64-v8a/libstoreservicescore.so", options)
            .unwrap();
        zip.write_all(b"ssc").unwrap();
        zip.start_file("lib/arm64-v8a/libCoreADI.so", options)
            .unwrap();
        zip.write_all(b"adi").unwrap();
        zip.finish().unwrap();
        bytes
    }

    #[test]
    fn extracts_both_libraries() {
        let libraries = extract(io::Cursor::new(apk(10))).unwrap();
        assert_eq!(libraries.store_services_core, b"ssc");
        assert_eq!(libraries.core_adi, b"adi");
    }

    #[test]
    fn range_reads_skip_the_rest_of_the_apk() {
        let transport = Ranges {
            file: apk(5 * BLOCK as usize),
            requests: Mutex::new(0),
        };
        let libraries = download_from(&transport).unwrap();
        assert_eq!(libraries.core_adi, b"adi");
        // The first block, then the last one with the directory and the
        // libraries: none of the padding in between
        assert!(*transport.requests.lock().unwrap() <= 3);
    }

    fn download_from(transport: &dyn Transport) -> Result<Libraries, AnisetteError> {
        extract(RangeReader::new(
            transport,
            "https://example.invalid/a.apk",
        )?)
    }

    #[test]
    fn reads_total_length() {
        assert_eq!(total_length("bytes 0-99/1234"), Some(1234));
        assert_eq!(total_length("bytes */"), None);
    }
}
