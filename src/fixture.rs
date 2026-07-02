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
}

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

    // A tiny (1x1) PNG cover image.
    entries.push(("OEBPS/cover.png".to_string(), COVER_PNG.to_vec()));

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
    <meta name="cover" content="cover-img"/>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="cover-img" href="cover.png" media-type="image/png" properties="cover-image"/>
{manifest_chapters}  </manifest>
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

/// A minimal, valid 1x1 red PNG used as the fixture's cover image.
const COVER_PNG: [u8; 70] = [
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0,
    0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 240, 31, 0,
    5, 0, 1, 255, 137, 153, 61, 29, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

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
