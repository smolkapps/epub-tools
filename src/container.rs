//! Parser for `META-INF/container.xml`, which points at the OPF package document.

use anyhow::{anyhow, Context, Result};
use quick_xml::events::Event;
use quick_xml::Reader;

/// Path inside an EPUB at which the container file always lives.
pub const CONTAINER_PATH: &str = "META-INF/container.xml";

/// Parse `container.xml` and return the `full-path` of the first rootfile whose
/// media-type is the OPF package type (falling back to the first rootfile).
///
/// The relevant XML looks like:
/// ```xml
/// <container ...>
///   <rootfiles>
///     <rootfile full-path="OEBPS/content.opf"
///               media-type="application/oebps-package+xml"/>
///   </rootfiles>
/// </container>
/// ```
pub fn parse_container(xml: &str) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut first_rootfile: Option<String> = None;
    let mut opf_rootfile: Option<String> = None;
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(e)) | Ok(Event::Start(e)) => {
                if local_name(e.name().as_ref()) == b"rootfile" {
                    let mut full_path: Option<String> = None;
                    let mut media_type: Option<String> = None;
                    for attr in e.attributes().flatten() {
                        match attr.key.as_ref() {
                            b"full-path" => {
                                full_path =
                                    Some(attr.unescape_value().unwrap_or_default().into_owned());
                            }
                            b"media-type" => {
                                media_type =
                                    Some(attr.unescape_value().unwrap_or_default().into_owned());
                            }
                            _ => {}
                        }
                    }
                    if let Some(fp) = full_path {
                        if first_rootfile.is_none() {
                            first_rootfile = Some(fp.clone());
                        }
                        if media_type.as_deref() == Some("application/oebps-package+xml")
                            && opf_rootfile.is_none()
                        {
                            opf_rootfile = Some(fp);
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(e).context("malformed container.xml");
            }
            _ => {}
        }
        buf.clear();
    }

    opf_rootfile
        .or(first_rootfile)
        .ok_or_else(|| anyhow!("no <rootfile> with a full-path found in {}", CONTAINER_PATH))
}

/// Strip any XML namespace prefix, returning the local element name.
pub(crate) fn local_name(name: &[u8]) -> &[u8] {
    match name.iter().rposition(|&b| b == b':') {
        Some(i) => &name[i + 1..],
        None => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_opf_path() {
        let xml = r#"<?xml version="1.0"?>
        <container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
          <rootfiles>
            <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
          </rootfiles>
        </container>"#;
        assert_eq!(parse_container(xml).unwrap(), "OEBPS/content.opf");
    }

    #[test]
    fn prefers_oebps_media_type() {
        let xml = r#"<container>
          <rootfiles>
            <rootfile full-path="other.xml" media-type="application/something"/>
            <rootfile full-path="content.opf" media-type="application/oebps-package+xml"/>
          </rootfiles>
        </container>"#;
        assert_eq!(parse_container(xml).unwrap(), "content.opf");
    }

    #[test]
    fn falls_back_to_first_rootfile() {
        let xml = r#"<container><rootfiles>
            <rootfile full-path="weird.opf"/>
          </rootfiles></container>"#;
        assert_eq!(parse_container(xml).unwrap(), "weird.opf");
    }
}
