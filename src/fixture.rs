//! Build a minimal, valid EPUB entirely in memory for tests and demos.
//!
//! The generated book has: a stored `mimetype`, `META-INF/container.xml`,
//! `OEBPS/content.opf` (title / two creators / language / identifier / publisher
//! / date, a manifest of two chapters + a nav doc, and a spine listing the two
//! chapters in order), an EPUB 3 `OEBPS/nav.xhtml`, and two XHTML chapters.
//!
//! This module is part of the library (not a test-only file) so that both the
//! unit tests, the integration tests, and the binary's demo path can construct a
//! known-good fixture without duplicating the markup.

use anyhow::Result;

use crate::package::write_epub_to_vec;

/// Knobs for the generated fixture so tests can assert on specific values.
#[derive(Debug, Clone)]
pub struct FixtureSpec {
    pub title: String,
    pub authors: Vec<String>,
    pub language: String,
    pub identifier: String,
    pub publisher: String,
    pub date: String,
    /// (chapter title, body paragraphs) for each of the chapters.
    pub chapters: Vec<(String, Vec<String>)>,
    /// Optional cover image as (filename, media-type, bytes). When present it is
    /// added to the manifest with `properties="cover-image"` and referenced by an
    /// EPUB 2 `<meta name="cover" .../>` for good measure.
    pub cover: Option<(String, String, Vec<u8>)>,
}

/// A minimal but valid 1×1 transparent PNG, used as the sample cover image.
pub const SAMPLE_COVER_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x62, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];

impl Default for FixtureSpec {
    fn default() -> Self {
        FixtureSpec {
            title: "The Sample Book".to_string(),
            authors: vec!["Ada Lovelace".to_string(), "Charles Babbage".to_string()],
            language: "en".to_string(),
            identifier: "urn:uuid:11111111-2222-3333-4444-555555555555".to_string(),
            publisher: "Analytical Press".to_string(),
            date: "2026-06-21".to_string(),
            chapters: vec![
                (
                    "Chapter One".to_string(),
                    vec![
                        "It was the best of times.".to_string(),
                        "It was the worst of times.".to_string(),
                    ],
                ),
                (
                    "Chapter Two".to_string(),
                    vec!["Call me Ishmael.".to_string()],
                ),
            ],
            cover: Some((
                "cover.png".to_string(),
                "image/png".to_string(),
                SAMPLE_COVER_PNG.to_vec(),
            )),
        }
    }
}

/// Build the fixture EPUB and return its raw bytes.
pub fn build_epub_bytes(spec: &FixtureSpec) -> Result<Vec<u8>> {
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();

    // container.xml -> points at OEBPS/content.opf
    entries.push((
        "META-INF/container.xml".to_string(),
        CONTAINER_XML.as_bytes().to_vec(),
    ));

    // Two chapter XHTML files.
    for (i, (ch_title, paras)) in spec.chapters.iter().enumerate() {
        let n = i + 1;
        let body: String = paras
            .iter()
            .map(|p| format!("    <p>{}</p>\n", xml_escape(p)))
            .collect();
        let xhtml = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml">
  <head><title>{title}</title></head>
  <body>
    <h1>{title}</h1>
{body}  </body>
</html>
"#,
            title = xml_escape(ch_title),
            body = body,
        );
        entries.push((format!("OEBPS/chapter{n}.xhtml"), xhtml.into_bytes()));
    }

    // EPUB 3 navigation document with a toc nav listing both chapters.
    let nav_items: String = spec
        .chapters
        .iter()
        .enumerate()
        .map(|(i, (ch_title, _))| {
            format!(
                "        <li><a href=\"chapter{}.xhtml\">{}</a></li>\n",
                i + 1,
                xml_escape(ch_title)
            )
        })
        .collect();
    let nav = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
  <head><title>Table of Contents</title></head>
  <body>
    <nav epub:type="toc" id="toc">
      <h1>Contents</h1>
      <ol>
{nav_items}      </ol>
    </nav>
  </body>
</html>
"#,
        nav_items = nav_items,
    );
    entries.push(("OEBPS/nav.xhtml".to_string(), nav.into_bytes()));

    // Optional cover image: raw bytes entry, manifest item, and EPUB 2 meta.
    let (cover_manifest, cover_meta) = match &spec.cover {
        Some((filename, media_type, bytes)) => {
            entries.push((format!("OEBPS/{filename}"), bytes.clone()));
            (
                format!(
                    "    <item id=\"cover-image\" href=\"{}\" media-type=\"{}\" properties=\"cover-image\"/>\n",
                    xml_escape(filename),
                    xml_escape(media_type),
                ),
                "    <meta name=\"cover\" content=\"cover-image\"/>\n".to_string(),
            )
        }
        None => (String::new(), String::new()),
    };

    // OPF package document.
    let creators: String = spec
        .authors
        .iter()
        .map(|a| format!("    <dc:creator>{}</dc:creator>\n", xml_escape(a)))
        .collect();
    let manifest_chapters: String = spec
        .chapters
        .iter()
        .enumerate()
        .map(|(i, _)| {
            let n = i + 1;
            format!(
                "    <item id=\"ch{n}\" href=\"chapter{n}.xhtml\" media-type=\"application/xhtml+xml\"/>\n"
            )
        })
        .collect();
    let spine_chapters: String = spec
        .chapters
        .iter()
        .enumerate()
        .map(|(i, _)| format!("    <itemref idref=\"ch{}\"/>\n", i + 1))
        .collect();

    let opf = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="bookid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
    <dc:title>{title}</dc:title>
{creators}    <dc:language>{language}</dc:language>
    <dc:identifier id="bookid">{identifier}</dc:identifier>
    <dc:publisher>{publisher}</dc:publisher>
    <dc:date>{date}</dc:date>
{cover_meta}  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
{cover_manifest}{manifest_chapters}  </manifest>
  <spine>
{spine_chapters}  </spine>
</package>
"#,
        title = xml_escape(&spec.title),
        creators = creators,
        language = xml_escape(&spec.language),
        identifier = xml_escape(&spec.identifier),
        publisher = xml_escape(&spec.publisher),
        date = xml_escape(&spec.date),
        cover_meta = cover_meta,
        cover_manifest = cover_manifest,
        manifest_chapters = manifest_chapters,
        spine_chapters = spine_chapters,
    );
    entries.push(("OEBPS/content.opf".to_string(), opf.into_bytes()));

    write_epub_to_vec(&entries)
}

/// Build the default fixture EPUB bytes.
pub fn build_default_epub_bytes() -> Result<Vec<u8>> {
    build_epub_bytes(&FixtureSpec::default())
}

const CONTAINER_XML: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>
"#;

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Epub;

    #[test]
    fn fixture_is_parseable() {
        let bytes = build_default_epub_bytes().unwrap();
        let epub = Epub::from_bytes(bytes).unwrap();
        assert_eq!(epub.metadata().title.as_deref(), Some("The Sample Book"));
        assert_eq!(epub.metadata().creators.len(), 2);
        assert_eq!(epub.chapter_count(), 2);
    }
}
