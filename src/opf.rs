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
                    b"meta" if in_metadata => {
                        if let Some(id) = parse_cover_meta(&e) {
                            pkg.cover_id.get_or_insert(id);
                        }
                    }
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
                    b"meta" if in_metadata => {
                        if let Some(id) = parse_cover_meta(&e) {
                            pkg.cover_id.get_or_insert(id);
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

/// Extract the referenced manifest id from an EPUB 2 cover meta element,
/// i.e. `<meta name="cover" content="ID"/>`. Returns `None` for any other meta.
fn parse_cover_meta(e: &quick_xml::events::BytesStart) -> Option<String> {
    let name = attr_value(e, b"name")?;
    if !name.eq_ignore_ascii_case("cover") {
        return None;
    }
    attr_value(e, b"content").filter(|c| !c.is_empty())
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
