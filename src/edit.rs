//! In-place editing of OPF Dublin Core metadata (`dc:title`, `dc:creator`).
//!
//! The strategy is a token-level rewrite of the original OPF bytes rather than a
//! full re-serialization: we replace the inner text of the first matching
//! `dc:*` element, or insert a fresh element just before `</metadata>` when the
//! field is absent. This preserves the rest of the package document verbatim
//! (namespaces, ordering, manifest, spine, formatting), which keeps the output a
//! valid EPUB with minimal churn.

use anyhow::{anyhow, Result};
use quick_xml::events::Event;
use quick_xml::Reader;

/// The set of Dublin Core fields this tool can rewrite.
#[derive(Debug, Default, Clone)]
pub struct MetadataEdit {
    pub title: Option<String>,
    pub author: Option<String>,
    pub language: Option<String>,
    pub publisher: Option<String>,
}

impl MetadataEdit {
    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.author.is_none()
            && self.language.is_none()
            && self.publisher.is_none()
    }
}

/// Apply the requested edits to the OPF XML, returning the new document text.
///
/// For each requested field: if a corresponding `dc:` element exists, its text
/// content is replaced; otherwise a new element is inserted before `</metadata>`.
pub fn apply_metadata_edit(opf_xml: &str, edit: &MetadataEdit) -> Result<String> {
    let mut xml = opf_xml.to_string();

    if let Some(title) = &edit.title {
        xml = set_or_insert(&xml, "title", title)?;
    }
    if let Some(author) = &edit.author {
        xml = set_or_insert(&xml, "creator", author)?;
    }
    if let Some(lang) = &edit.language {
        xml = set_or_insert(&xml, "language", lang)?;
    }
    if let Some(pubr) = &edit.publisher {
        xml = set_or_insert(&xml, "publisher", pubr)?;
    }

    Ok(xml)
}

/// Replace the text of the first `dc:<local>` element, or insert a new one.
///
/// Uses a string scan bounded to the `<metadata>...</metadata>` region (located
/// via a namespace-aware XML pass) so byte offsets are exact and the rest of the
/// document is untouched.
fn set_or_insert(xml: &str, local: &str, value: &str) -> Result<String> {
    let (meta_start, meta_end) = metadata_bounds(xml)?;
    let region = &xml[meta_start..meta_end];

    if let Some((text_start, text_end)) = find_dc_text_span(region, local) {
        // Offsets are relative to the metadata region; shift into the full doc.
        let abs_text_start = meta_start + text_start;
        let abs_text_end = meta_start + text_end;
        let mut result = String::with_capacity(xml.len() + value.len());
        result.push_str(&xml[..abs_text_start]);
        result.push_str(&escape_text(value));
        result.push_str(&xml[abs_text_end..]);
        Ok(result)
    } else {
        insert_before_metadata_close(xml, meta_end, local, value)
    }
}

/// Find the byte range `[start, end)` of the `<metadata>...</metadata>` block,
/// where `start` is just after the open tag's `>` and `end` is the index of the
/// closing `</metadata>`'s `<`. Tolerates an optional namespace prefix.
fn metadata_bounds(xml: &str) -> Result<(usize, usize)> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);

    let mut buf = Vec::new();
    let mut start: Option<usize> = None;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                if local_of(e.name().as_ref()) == b"metadata" {
                    // buffer_position() is just past the '>' of this start tag.
                    start = Some(reader.buffer_position() as usize);
                }
            }
            Ok(Event::End(e)) => {
                if local_of(e.name().as_ref()) == b"metadata" {
                    if let Some(s) = start {
                        let after = reader.buffer_position() as usize;
                        // Recover the '<' position of the close tag by scanning back.
                        let end = xml[..after].rfind('<').unwrap_or(after);
                        return Ok((s, end));
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(anyhow!("failed scanning OPF metadata block: {}", e)),
            _ => {}
        }
        buf.clear();
    }

    Err(anyhow!("no <metadata> element found in OPF"))
}

/// Within the metadata region text, find the inner-text span of the first
/// `<[ns:]LOCAL ...>...</[ns:]LOCAL>` element. Returns `(text_start, text_end)`
/// byte offsets relative to `region`. Self-closing matches return `None`.
fn find_dc_text_span(region: &str, local: &str) -> Option<(usize, usize)> {
    let bytes = region.as_bytes();
    let mut search_from = 0;

    while let Some(rel_lt) = region[search_from..].find('<') {
        let lt = search_from + rel_lt;
        // Skip closing tags, comments, processing instructions.
        let after_lt = &region[lt + 1..];
        if after_lt.starts_with('/') || after_lt.starts_with('!') || after_lt.starts_with('?') {
            search_from = lt + 1;
            continue;
        }

        // Read the tag name: chars up to whitespace, '>', or '/'.
        let name_start = lt + 1;
        let mut p = name_start;
        while p < bytes.len() {
            let c = bytes[p];
            if c == b' ' || c == b'\t' || c == b'\n' || c == b'\r' || c == b'>' || c == b'/' {
                break;
            }
            p += 1;
        }
        let raw_name = &region[name_start..p];
        let elem_local = local_of(raw_name.as_bytes());

        if elem_local == local.as_bytes() {
            // Find the '>' that closes this start tag.
            if let Some(rel_gt) = region[p..].find('>') {
                let gt = p + rel_gt;
                // Self-closing? Then there's no text content.
                if region[..gt].ends_with('/') {
                    return None;
                }
                let text_start = gt + 1;
                // Find the matching close tag `</[ns:]LOCAL`.
                if let Some(text_end) = find_close_tag(region, text_start, local) {
                    return Some((text_start, text_end));
                }
                return None;
            }
            return None;
        }

        // Not our element: advance past this '<' and keep scanning.
        search_from = lt + 1;
    }

    None
}

/// From `from`, find the `<` byte offset of the next `</[ns:]LOCAL ...>` close tag.
fn find_close_tag(region: &str, from: usize, local: &str) -> Option<usize> {
    let mut search_from = from;
    while let Some(rel) = region[search_from..].find("</") {
        let lt = search_from + rel;
        let name_start = lt + 2;
        let rest = &region[name_start..];
        let mut p = 0;
        let rb = rest.as_bytes();
        while p < rb.len() {
            let c = rb[p];
            if c == b' ' || c == b'\t' || c == b'\n' || c == b'\r' || c == b'>' {
                break;
            }
            p += 1;
        }
        let raw = &rest[..p];
        if local_of(raw.as_bytes()) == local.as_bytes() {
            return Some(lt);
        }
        search_from = lt + 2;
    }
    None
}

/// Insert `<dc:LOCAL>value</dc:LOCAL>` right before the `</metadata>` close tag,
/// whose `<` index in the full document is `meta_end`.
fn insert_before_metadata_close(
    xml: &str,
    meta_end: usize,
    local: &str,
    value: &str,
) -> Result<String> {
    let insertion = format!("  <dc:{0}>{1}</dc:{0}>\n", local, escape_text(value));
    let mut result = String::with_capacity(xml.len() + insertion.len());
    result.push_str(&xml[..meta_end]);
    result.push_str(&insertion);
    result.push_str(&xml[meta_end..]);
    Ok(result)
}

fn local_of(name: &[u8]) -> &[u8] {
    match name.iter().rposition(|&b| b == b':') {
        Some(i) => &name[i + 1..],
        None => name,
    }
}

/// Minimal XML text escaping for inserted/replaced values.
fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPF: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="bookid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Original Title</dc:title>
    <dc:creator>Original Author</dc:creator>
    <dc:language>en</dc:language>
    <dc:identifier id="bookid">urn:uuid:1234</dc:identifier>
  </metadata>
  <manifest>
    <item id="c1" href="c1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="c1"/>
  </spine>
</package>"#;

    #[test]
    fn replaces_existing_title() {
        let edit = MetadataEdit {
            title: Some("Brand New Title".to_string()),
            ..Default::default()
        };
        let out = apply_metadata_edit(OPF, &edit).unwrap();
        assert!(out.contains("<dc:title>Brand New Title</dc:title>"));
        assert!(!out.contains("Original Title"));
        // The rest of the document is preserved.
        assert!(out.contains(r#"<item id="c1" href="c1.xhtml""#));
        assert!(out.contains("<dc:creator>Original Author</dc:creator>"));
    }

    #[test]
    fn replaces_existing_author() {
        let edit = MetadataEdit {
            author: Some("New Author".to_string()),
            ..Default::default()
        };
        let out = apply_metadata_edit(OPF, &edit).unwrap();
        assert!(out.contains("<dc:creator>New Author</dc:creator>"));
        assert!(!out.contains("Original Author"));
    }

    #[test]
    fn inserts_missing_publisher() {
        let edit = MetadataEdit {
            publisher: Some("Acme Press".to_string()),
            ..Default::default()
        };
        let out = apply_metadata_edit(OPF, &edit).unwrap();
        assert!(out.contains("<dc:publisher>Acme Press</dc:publisher>"));
        // Inserted before </metadata>.
        let pub_idx = out.find("Acme Press").unwrap();
        let meta_close = out.find("</metadata>").unwrap();
        assert!(pub_idx < meta_close);
    }

    #[test]
    fn escapes_special_chars() {
        let edit = MetadataEdit {
            title: Some("Tom & Jerry <Best>".to_string()),
            ..Default::default()
        };
        let out = apply_metadata_edit(OPF, &edit).unwrap();
        assert!(out.contains("Tom &amp; Jerry &lt;Best&gt;"));
    }

    #[test]
    fn multiple_edits_at_once() {
        let edit = MetadataEdit {
            title: Some("T2".to_string()),
            author: Some("A2".to_string()),
            language: Some("fr".to_string()),
            publisher: Some("P2".to_string()),
        };
        let out = apply_metadata_edit(OPF, &edit).unwrap();
        assert!(out.contains("<dc:title>T2</dc:title>"));
        assert!(out.contains("<dc:creator>A2</dc:creator>"));
        assert!(out.contains("<dc:language>fr</dc:language>"));
        assert!(out.contains("<dc:publisher>P2</dc:publisher>"));
    }
}
