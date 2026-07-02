//! `epub-tools` library core.
//!
//! Opens an EPUB (a zip), reads `META-INF/container.xml` to locate the OPF
//! package document, parses its Dublin Core metadata / manifest / spine, extracts
//! readable text from the spine documents, reads the TOC (EPUB 3 nav or EPUB 2
//! NCX), and repackages an edited EPUB while honoring the mimetype-first-and-
//! stored OCF rule.

pub mod container;
pub mod edit;
pub mod fixture;
pub mod model;
pub mod nav;
pub mod opf;
pub mod package;
pub mod text;
pub mod util;

use std::io::Cursor;
use std::path::Path;

use anyhow::{anyhow, Context, Result};

pub use edit::MetadataEdit;
pub use model::{ManifestItem, Metadata, Package, SpineItem, TocEntry};

/// A borrowed view of an EPUB's cover image: where it lives in the archive, its
/// media type, and its raw bytes.
pub struct Cover<'a> {
    /// Full archive path of the cover resource.
    pub resolved_path: &'a str,
    /// The manifest media type, e.g. `image/jpeg`.
    pub media_type: &'a str,
    /// The raw image bytes.
    pub bytes: &'a [u8],
}

impl Cover<'_> {
    /// A sensible lowercase file extension (no dot) for saving the cover,
    /// derived from the media type and falling back to the archive path's own
    /// extension, or `"img"` if nothing better is known.
    pub fn extension(&self) -> String {
        match self.media_type.trim().to_ascii_lowercase().as_str() {
            "image/jpeg" | "image/jpg" => return "jpg".to_string(),
            "image/png" => return "png".to_string(),
            "image/gif" => return "gif".to_string(),
            "image/svg+xml" => return "svg".to_string(),
            "image/webp" => return "webp".to_string(),
            "image/tiff" => return "tiff".to_string(),
            "image/bmp" => return "bmp".to_string(),
            _ => {}
        }
        // Fall back to the extension of the resource's own filename, lowercased
        // to match the documented behavior.
        self.resolved_path
            .rsplit('/')
            .next()
            .and_then(|name| name.rsplit_once('.').map(|(_, ext)| ext))
            .filter(|ext| !ext.is_empty() && ext.len() <= 5)
            .map(|ext| ext.to_ascii_lowercase())
            .unwrap_or_else(|| "img".to_string())
    }
}

/// A loaded EPUB: the raw zip entries plus the parsed package document.
pub struct Epub {
    raw: package::RawEpub,
    package: Package,
}

impl Epub {
    /// Open and parse an EPUB from a filesystem path.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let bytes = std::fs::read(path)
            .with_context(|| format!("reading EPUB file {}", path.display()))?;
        Self::from_bytes(bytes)
    }

    /// Open and parse an EPUB from raw bytes already in memory.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        let raw = package::RawEpub::read(Cursor::new(bytes))?;

        let container_xml = raw
            .get_str(container::CONTAINER_PATH)
            .ok_or_else(|| anyhow!("missing {} — not a valid EPUB", container::CONTAINER_PATH))?;
        let opf_path = container::parse_container(&container_xml)?;

        let opf_xml = raw
            .get_str(&opf_path)
            .ok_or_else(|| anyhow!("OPF package document not found at '{}'", opf_path))?;
        let package = opf::parse_opf(&opf_xml, &opf_path)?;

        Ok(Epub { raw, package })
    }

    /// The parsed package document.
    pub fn package(&self) -> &Package {
        &self.package
    }

    /// The book's Dublin Core metadata.
    pub fn metadata(&self) -> &Metadata {
        &self.package.metadata
    }

    /// Manifest items in spine reading order.
    pub fn spine_items(&self) -> Vec<&ManifestItem> {
        self.package.spine_items()
    }

    /// Raw bytes of an archive entry by its full path.
    pub fn entry_bytes(&self, path: &str) -> Option<&[u8]> {
        self.raw.get(path)
    }

    /// Extract plain text of every spine document, in reading order, joined by
    /// blank lines.
    pub fn full_text(&self) -> String {
        let mut parts = Vec::new();
        for item in self.spine_items() {
            if let Some(t) = self.chapter_text_for(item) {
                if !t.is_empty() {
                    parts.push(t);
                }
            }
        }
        parts.join("\n\n")
    }

    /// Number of documents in the spine (chapters addressable by `--chapter`).
    pub fn chapter_count(&self) -> usize {
        self.package.spine.len()
    }

    /// Extract plain text of a single spine document by 1-based index.
    pub fn chapter_text(&self, one_based: usize) -> Result<String> {
        let items = self.spine_items();
        if one_based == 0 || one_based > items.len() {
            return Err(anyhow!(
                "chapter {} out of range (book has {} spine documents)",
                one_based,
                items.len()
            ));
        }
        Ok(self.chapter_text_for(items[one_based - 1]).unwrap_or_default())
    }

    fn chapter_text_for(&self, item: &ManifestItem) -> Option<String> {
        let bytes = self.raw.get(&item.resolved_path)?;
        let xml = String::from_utf8_lossy(bytes);
        Some(text::xhtml_to_text(&xml))
    }

    /// Read the table of contents, preferring the EPUB 3 nav document and
    /// falling back to the EPUB 2 NCX. Returns an empty vector if neither exists.
    pub fn toc(&self) -> Vec<TocEntry> {
        if let Some(nav) = self.package.nav_item() {
            if let Some(bytes) = self.raw.get(&nav.resolved_path) {
                let xml = String::from_utf8_lossy(bytes);
                let entries = nav::parse_nav_xhtml(&xml);
                if !entries.is_empty() {
                    return entries;
                }
            }
        }
        if let Some(ncx) = self.package.ncx_item() {
            if let Some(bytes) = self.raw.get(&ncx.resolved_path) {
                let xml = String::from_utf8_lossy(bytes);
                return nav::parse_ncx(&xml);
            }
        }
        Vec::new()
    }

    /// The cover image, if the book declares one and the resource is present in
    /// the archive. Prefers the EPUB 3 `properties="cover-image"` manifest item,
    /// falling back to the EPUB 2 `<meta name="cover">` convention.
    pub fn cover(&self) -> Option<Cover<'_>> {
        let item = self.package.cover_item()?;
        let bytes = self.raw.get(&item.resolved_path)?;
        Some(Cover {
            resolved_path: &item.resolved_path,
            media_type: &item.media_type,
            bytes,
        })
    }

    /// Apply metadata edits to the OPF and write a fresh, valid EPUB to `out_path`.
    ///
    /// The mimetype entry is rewritten first and stored; all other entries
    /// (including the edited OPF) are deflated.
    pub fn write_with_metadata<P: AsRef<Path>>(
        &self,
        edit: &MetadataEdit,
        out_path: P,
    ) -> Result<()> {
        let bytes = self.serialize_with_metadata(edit)?;
        std::fs::write(out_path.as_ref(), bytes)
            .with_context(|| format!("writing output EPUB {}", out_path.as_ref().display()))?;
        Ok(())
    }

    /// Like [`write_with_metadata`](Self::write_with_metadata) but returns the
    /// serialized EPUB bytes instead of writing to disk.
    pub fn serialize_with_metadata(&self, edit: &MetadataEdit) -> Result<Vec<u8>> {
        let opf_xml = self
            .raw
            .get_str(&self.package.opf_path)
            .ok_or_else(|| anyhow!("OPF disappeared from archive"))?;
        let new_opf = edit::apply_metadata_edit(&opf_xml, edit)?;

        // Clone entries, swap in the edited OPF, then write with the packaging
        // rules enforced (mimetype first & stored).
        let mut entries: Vec<(String, Vec<u8>)> = self
            .raw
            .entries
            .iter()
            .map(|(p, b)| (p.clone(), b.clone()))
            .collect();
        for (p, b) in entries.iter_mut() {
            if *p == self.package.opf_path {
                *b = new_opf.clone().into_bytes();
            }
        }

        package::write_epub_to_vec(&entries)
    }
}
