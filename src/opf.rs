//! Parser for the OPF package document: Dublin Core metadata, manifest, spine.

use anyhow::{Context, Result};
use quick_xml::events::Event;
use quick_xml::Reader;

use crate::container::local_name;
use crate::model::{ManifestItem, Metadata, Package, SpineItem};
use crate::util::resolve_href;

/// Which `<metadata>` Dublin Core element we are currently inside, so character
/// data lands in the right field.
#[derive(Clone, Copy, PartialEq)]
enum DcField {
    None,
    Title,
    Creator,
    Language,
    Identifier,
    Publisher,
    Date,
    Description,
    Rights,
}

fn dc_field(local: &[u8]) -> DcField {
    match local {
        b"title" => DcField::Title,
        b"creator" => DcField::Creator,
        b"language" => DcField::Language,
        b"identifier" => DcField::Identifier,
        b"publisher" => DcField::Publisher,
        b"date" => DcField::Date,
        b"description" => DcField::Description,
        b"rights" => DcField::Rights,
        _ => DcField::None,
    }
}

/// Parse the OPF document text. `opf_path` is the OPF's full archive path, used
/// both to record on the `Package` and to resolve manifest hrefs.
pub fn parse_opf(xml: &str, opf_path: &str) -> Result<Package> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false); // we trim DC text ourselves

    let mut pkg = Package {
        opf_path: opf_path.to_string(),
        ..Default::default()
    };

    let mut in_metadata = false;
    let mut current_dc = DcField::None;
    let mut dc_buf = String::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let local = local_name(e.name().as_ref()).to_vec();
                match local.as_slice() {
                    b"metadata" => in_metadata = true,
                    _ if in_metadata => {
                        let f = dc_field(&local);
                        if f != DcField::None {
                            current_dc = f;
                            dc_buf.clear();
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Empty(e)) => {
                let local = local_name(e.name().as_ref()).to_vec();
                match local.as_slice() {
                    b"item" => {
                        if let Some(item) = parse_manifest_item(&e, opf_path) {
                            pkg.manifest.push(item);
                        }
                    }
                    b"itemref" => {
                        if let Some(sp) = parse_spine_item(&e) {
                            pkg.spine.push(sp);
                        }
                    }
                    // A self-closing DC element (e.g. <dc:identifier .../>) carries
                    // no text but may still be meaningful; nothing to capture.
                    _ => {}
                }
            }
            Ok(Event::Text(t)) => {
                if in_metadata && current_dc != DcField::None {
                    dc_buf.push_str(&t.unescape().unwrap_or_default());
                }
            }
            Ok(Event::CData(t)) => {
                if in_metadata && current_dc != DcField::None {
                    dc_buf.push_str(&String::from_utf8_lossy(t.as_ref()));
                }
            }
            Ok(Event::End(e)) => {
                let local = local_name(e.name().as_ref()).to_vec();
                match local.as_slice() {
                    b"metadata" => {
                        in_metadata = false;
                        current_dc = DcField::None;
                    }
                    b"spine" => {}
                    _ if in_metadata && current_dc != DcField::None => {
                        // Closing a DC element: commit the accumulated text.
                        let value = dc_buf.trim().to_string();
                        if !value.is_empty() {
                            commit_dc(&mut pkg.metadata, current_dc, value);
                        }
                        current_dc = DcField::None;
                        dc_buf.clear();
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(e).context("malformed OPF package document"),
            _ => {}
        }
        buf.clear();
    }

    // The spine's `toc` attribute lives on the <spine> start tag; capture it in a
    // dedicated lightweight pass to avoid threading more state above.
    pkg.spine_toc = extract_spine_toc(xml);
    // Likewise the EPUB 2 `<meta name="cover" content="…">` reference.
    pkg.cover_id = extract_cover_meta(xml);

    Ok(pkg)
}

fn commit_dc(md: &mut Metadata, field: DcField, value: String) {
    match field {
        DcField::Title => {
            if md.title.is_none() {
                md.title = Some(value);
            }
        }
        DcField::Creator => md.creators.push(value),
        DcField::Language => md.languages.push(value),
        DcField::Identifier => {
            if md.identifier.is_none() {
                md.identifier = Some(value);
            }
        }
        DcField::Publisher => {
            if md.publisher.is_none() {
                md.publisher = Some(value);
            }
        }
        DcField::Date => {
            if md.date.is_none() {
                md.date = Some(value);
            }
        }
        DcField::Description => {
            if md.description.is_none() {
                md.description = Some(value);
            }
        }
        DcField::Rights => {
            if md.rights.is_none() {
                md.rights = Some(value);
            }
        }
        DcField::None => {}
    }
}

fn attr_value(e: &quick_xml::events::BytesStart, key: &[u8]) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == key)
        .map(|a| a.unescape_value().unwrap_or_default().into_owned())
}

fn parse_manifest_item(e: &quick_xml::events::BytesStart, opf_path: &str) -> Option<ManifestItem> {
    let id = attr_value(e, b"id")?;
    let href = attr_value(e, b"href")?;
    let media_type = attr_value(e, b"media-type").unwrap_or_default();
    let properties = attr_value(e, b"properties");
    let resolved_path = resolve_href(opf_path, &href);
    Some(ManifestItem {
        id,
        href,
        media_type,
        resolved_path,
        properties,
    })
}

fn parse_spine_item(e: &quick_xml::events::BytesStart) -> Option<SpineItem> {
    let idref = attr_value(e, b"idref")?;
    let linear = attr_value(e, b"linear")
        .map(|v| v.eq_ignore_ascii_case("yes") || v.is_empty())
        .unwrap_or(true);
    Some(SpineItem { idref, linear })
}

/// Find the `toc` attribute on the (possibly start) `<spine>` element.
fn extract_spine_toc(xml: &str) -> Option<String> {
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                if local_name(e.name().as_ref()) == b"spine" {
                    return attr_value(&e, b"toc");
                }
            }
            Ok(Event::Eof) | Err(_) => return None,
            _ => {}
        }
        buf.clear();
    }
}

/// Find the EPUB 2 `<meta name="cover" content="…">` reference, if present. The
/// `content` value is a manifest item id pointing at the cover image resource.
fn extract_cover_meta(xml: &str) -> Option<String> {
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                if local_name(e.name().as_ref()) == b"meta"
                    && attr_value(&e, b"name").as_deref() == Some("cover")
                {
                    if let Some(content) = attr_value(&e, b"content") {
                        return Some(content);
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => return None,
            _ => {}
        }
        buf.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // An EPUB 2 style package: no `properties="cover-image"`, only a legacy
    // `<meta name="cover" content="…">` pointing at the manifest item.
    const EPUB2_OPF: &str = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="bookid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Legacy Book</dc:title>
    <meta name="cover" content="cover-img"/>
  </metadata>
  <manifest>
    <item id="cover-img" href="images/cover.jpg" media-type="image/jpeg"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>
"#;

    #[test]
    fn cover_item_falls_back_to_meta_name_cover() {
        let pkg = parse_opf(EPUB2_OPF, "OEBPS/content.opf").unwrap();
        assert_eq!(pkg.cover_id.as_deref(), Some("cover-img"));
        let cover = pkg.cover_item().expect("cover resolved via meta");
        assert_eq!(cover.id, "cover-img");
        assert_eq!(cover.resolved_path, "OEBPS/images/cover.jpg");
        assert_eq!(cover.media_type, "image/jpeg");
    }

    #[test]
    fn cover_item_prefers_properties_over_meta() {
        // properties="cover-image" wins even when a stale meta points elsewhere.
        let opf = r#"<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata><meta name="cover" content="old"/></metadata>
  <manifest>
    <item id="old" href="old.png" media-type="image/png"/>
    <item id="c" href="cover.png" media-type="image/png" properties="cover-image"/>
  </manifest>
  <spine/>
</package>"#;
        let pkg = parse_opf(opf, "content.opf").unwrap();
        assert_eq!(pkg.cover_item().unwrap().id, "c");
    }

    #[test]
    fn no_cover_declared_yields_none() {
        let opf = r#"<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata><dc:title xmlns:dc="http://purl.org/dc/elements/1.1/">X</dc:title></metadata>
  <manifest><item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest>
  <spine><itemref idref="ch1"/></spine>
</package>"#;
        let pkg = parse_opf(opf, "content.opf").unwrap();
        assert!(pkg.cover_item().is_none());
    }
}
