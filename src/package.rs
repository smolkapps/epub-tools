//! Reading raw entries from an EPUB zip and writing a fresh, valid EPUB zip.
//!
//! The single most important EPUB packaging rule is honored here: the first
//! entry in the archive MUST be `mimetype`, it MUST be stored (no compression),
//! and it MUST contain exactly `application/epub+zip` with no extra fields. Some
//! readers and the EPUB OCF spec rely on this so the file can be sniffed as an
//! EPUB by reading bytes 30..58 of the zip.

use std::io::{Cursor, Read, Seek, Write};

use anyhow::{Context, Result};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

/// The exact mimetype string an EPUB must carry.
pub const EPUB_MIMETYPE: &str = "application/epub+zip";
/// The archive name of the mimetype entry.
pub const MIMETYPE_ENTRY: &str = "mimetype";

/// All entries read out of an EPUB zip, preserving their archive paths.
pub struct RawEpub {
    /// Ordered list of (path, bytes) for every entry except directories.
    pub entries: Vec<(String, Vec<u8>)>,
}

impl RawEpub {
    /// Read every file entry from a zip archive into memory.
    pub fn read<R: Read + Seek>(reader: R) -> Result<Self> {
        let mut archive = ZipArchive::new(reader).context("file is not a valid zip/EPUB")?;
        let mut entries = Vec::with_capacity(archive.len());
        for i in 0..archive.len() {
            let mut file = archive.by_index(i)?;
            if file.is_dir() {
                continue;
            }
            let name = file.name().to_string();
            let mut bytes = Vec::with_capacity(file.size() as usize);
            file.read_to_end(&mut bytes)
                .with_context(|| format!("reading zip entry {name}"))?;
            entries.push((name, bytes));
        }
        Ok(RawEpub { entries })
    }

    /// Fetch the bytes of a specific archive entry.
    pub fn get(&self, path: &str) -> Option<&[u8]> {
        self.entries
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, b)| b.as_slice())
    }

    /// Fetch an entry decoded as UTF-8 (lossy), for XML/text documents.
    pub fn get_str(&self, path: &str) -> Option<String> {
        self.get(path)
            .map(|b| String::from_utf8_lossy(b).into_owned())
    }

    /// Replace (or add) an entry's bytes, keeping overall order stable.
    pub fn set(&mut self, path: &str, bytes: Vec<u8>) {
        if let Some(slot) = self.entries.iter_mut().find(|(p, _)| p == path) {
            slot.1 = bytes;
        } else {
            self.entries.push((path.to_string(), bytes));
        }
    }
}

/// Write a valid EPUB zip to `out`, enforcing the mimetype-first-and-stored rule.
///
/// `entries` may or may not already contain a `mimetype` entry; any provided one
/// is dropped and a canonical stored `mimetype` is written first. Every other
/// entry is written with Deflate compression.
pub fn write_epub<W: Write + Seek>(out: W, entries: &[(String, Vec<u8>)]) -> Result<()> {
    let mut zip = ZipWriter::new(out);

    // 1. mimetype FIRST, STORED, no extra field.
    let stored = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .unix_permissions(0o644);
    zip.start_file(MIMETYPE_ENTRY, stored)
        .context("writing mimetype entry")?;
    zip.write_all(EPUB_MIMETYPE.as_bytes())
        .context("writing mimetype bytes")?;

    // 2. Everything else, deflated.
    let deflated = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o644);
    for (path, bytes) in entries {
        if path == MIMETYPE_ENTRY {
            continue; // never duplicate the mimetype
        }
        zip.start_file(path.as_str(), deflated)
            .with_context(|| format!("writing entry {path}"))?;
        zip.write_all(bytes)
            .with_context(|| format!("writing bytes for {path}"))?;
    }

    zip.finish().context("finalizing EPUB zip")?;
    Ok(())
}

/// Convenience: serialize an EPUB to an in-memory byte buffer.
pub fn write_epub_to_vec(entries: &[(String, Vec<u8>)]) -> Result<Vec<u8>> {
    let mut cursor = Cursor::new(Vec::new());
    write_epub(&mut cursor, entries)?;
    Ok(cursor.into_inner())
}

/// Report, for a serialized EPUB byte buffer, whether the first entry is
/// `mimetype` and whether it is stored (uncompressed). Used by tests and the
/// `verify`-style checks.
pub fn first_entry_info(epub_bytes: &[u8]) -> Result<(String, bool)> {
    let mut archive = ZipArchive::new(Cursor::new(epub_bytes))
        .context("re-reading written EPUB to inspect first entry")?;
    let first = archive.by_index(0).context("EPUB has no entries")?;
    let name = first.name().to_string();
    let stored = first.compression() == CompressionMethod::Stored;
    Ok((name, stored))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mimetype_is_first_and_stored() {
        let entries = vec![
            (
                "META-INF/container.xml".to_string(),
                b"<container/>".to_vec(),
            ),
            ("OEBPS/content.opf".to_string(), b"<package/>".to_vec()),
        ];
        let bytes = write_epub_to_vec(&entries).unwrap();
        let (name, stored) = first_entry_info(&bytes).unwrap();
        assert_eq!(name, "mimetype");
        assert!(stored, "mimetype entry must be stored (uncompressed)");
    }

    #[test]
    fn provided_mimetype_is_not_duplicated() {
        let entries = vec![
            ("mimetype".to_string(), b"application/epub+zip".to_vec()),
            ("OEBPS/content.opf".to_string(), b"<package/>".to_vec()),
        ];
        let bytes = write_epub_to_vec(&entries).unwrap();
        let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut count = 0;
        for i in 0..archive.len() {
            if archive.by_index(i).unwrap().name() == "mimetype" {
                count += 1;
            }
        }
        assert_eq!(count, 1, "mimetype must appear exactly once");
    }

    #[test]
    fn roundtrip_reads_back_entries() {
        let entries = vec![
            (
                "META-INF/container.xml".to_string(),
                b"<container/>".to_vec(),
            ),
            ("OEBPS/ch1.xhtml".to_string(), b"<html>hi</html>".to_vec()),
        ];
        let bytes = write_epub_to_vec(&entries).unwrap();
        let raw = RawEpub::read(Cursor::new(bytes)).unwrap();
        assert_eq!(raw.get_str("OEBPS/ch1.xhtml").unwrap(), "<html>hi</html>");
        // mimetype is present too.
        assert_eq!(raw.get_str("mimetype").unwrap(), "application/epub+zip");
    }
}
