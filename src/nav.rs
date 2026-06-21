//! Parsers for the two EPUB table-of-contents formats:
//! - EPUB 3 navigation document (XHTML `<nav epub:type="toc">` with nested `<ol>`)
//! - EPUB 2 NCX (`<navMap>` of `<navPoint>`)

use quick_xml::events::Event;
use quick_xml::Reader;

use crate::container::local_name;
use crate::model::TocEntry;

/// Parse an EPUB 3 navigation document and return the entries of the `toc` nav.
///
/// Looks for `<nav ... epub:type="toc">` (or, failing that, the first `<nav>`),
/// then walks its nested `<ol>/<li>/<a>` structure. `<a href>` supplies the
/// link, the anchor text supplies the label, and `<ol>` nesting supplies depth.
pub fn parse_nav_xhtml(xml: &str) -> Vec<TocEntry> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    reader.config_mut().check_end_names = false;

    let mut entries = Vec::new();
    let mut buf = Vec::new();

    // Depth of <nav> nesting we care about. We collect from the toc nav; if no
    // nav advertises epub:type="toc" we fall back to the first nav encountered.
    let mut in_target_nav = false;
    let mut saw_target_nav = false;
    let mut nav_stack: u32 = 0; // ol nesting depth within the target nav
    let mut current_href: Option<String> = None;
    let mut label_buf = String::new();
    let mut in_anchor = false;

    // Two-phase isn't needed: prefer the toc nav by scanning first.
    let prefer_toc = xml.contains("epub:type") && xml.contains("toc");

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let local = local_name(e.name().as_ref()).to_vec();
                match local.as_slice() {
                    b"nav" => {
                        let is_toc = attr_contains(&e, b"epub:type", "toc")
                            || attr_contains(&e, b"type", "toc")
                            || attr_contains(&e, b"role", "doc-toc");
                        if is_toc {
                            in_target_nav = true;
                            saw_target_nav = true;
                        } else if !prefer_toc && !saw_target_nav {
                            // Fall back to first nav only when no toc nav exists.
                            in_target_nav = true;
                        }
                    }
                    b"ol" if in_target_nav => nav_stack += 1,
                    b"a" if in_target_nav => {
                        in_anchor = true;
                        label_buf.clear();
                        current_href = attr_value(&e, b"href");
                    }
                    b"span" if in_target_nav => {
                        // Some nav files use <span> for unlinked headings.
                        in_anchor = true;
                        label_buf.clear();
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(t)) => {
                if in_anchor {
                    label_buf.push_str(&t.unescape().unwrap_or_default());
                }
            }
            Ok(Event::End(e)) => {
                let local = local_name(e.name().as_ref()).to_vec();
                match local.as_slice() {
                    b"a" | b"span" if in_anchor => {
                        let label = collapse_ws(&label_buf);
                        if !label.is_empty() {
                            entries.push(TocEntry {
                                label,
                                href: current_href.take().unwrap_or_default(),
                                depth: nav_stack.saturating_sub(1) as usize,
                            });
                        }
                        in_anchor = false;
                        current_href = None;
                    }
                    b"ol" if in_target_nav => {
                        nav_stack = nav_stack.saturating_sub(1);
                    }
                    b"nav" if in_target_nav => {
                        in_target_nav = false;
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }

    entries
}

/// Parse an EPUB 2 NCX document's `<navMap>` into TOC entries.
///
/// Each `<navPoint>` contributes one entry: its `<navLabel><text>` is the label
/// and its `<content src>` is the href. `<navPoint>` nesting gives depth.
pub fn parse_ncx(xml: &str) -> Vec<TocEntry> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    reader.config_mut().check_end_names = false;

    let mut entries = Vec::new();
    let mut buf = Vec::new();

    let mut in_nav_map = false;
    let mut depth: i32 = -1; // navPoint nesting depth; first navPoint -> 0
    let mut in_text = false;
    let mut text_buf = String::new();
    // Pending entry for the navPoint currently being opened.
    let mut pending_label: Option<String> = None;
    let mut pending_href: Option<String> = None;
    let mut pending_depth: usize = 0;
    let mut have_pending = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let local = local_name(e.name().as_ref()).to_vec();
                match local.as_slice() {
                    b"navMap" => in_nav_map = true,
                    b"navPoint" if in_nav_map => {
                        // Flush any previous pending navPoint before descending.
                        flush_pending(
                            &mut entries,
                            &mut have_pending,
                            &mut pending_label,
                            &mut pending_href,
                            pending_depth,
                        );
                        depth += 1;
                        pending_depth = depth.max(0) as usize;
                        have_pending = true;
                        pending_label = None;
                        pending_href = None;
                    }
                    b"text" if in_nav_map => {
                        in_text = true;
                        text_buf.clear();
                    }
                    _ => {}
                }
            }
            Ok(Event::Empty(e)) => {
                let local = local_name(e.name().as_ref()).to_vec();
                if local.as_slice() == b"content" && in_nav_map && have_pending {
                    pending_href = attr_value(&e, b"src");
                }
            }
            Ok(Event::Text(t)) => {
                if in_text {
                    text_buf.push_str(&t.unescape().unwrap_or_default());
                }
            }
            Ok(Event::End(e)) => {
                let local = local_name(e.name().as_ref()).to_vec();
                match local.as_slice() {
                    b"text" if in_text => {
                        if have_pending && pending_label.is_none() {
                            pending_label = Some(collapse_ws(&text_buf));
                        }
                        in_text = false;
                    }
                    b"content" if in_nav_map && have_pending => {
                        // Handled via Empty in practice; kept for completeness.
                    }
                    b"navPoint" if in_nav_map => {
                        flush_pending(
                            &mut entries,
                            &mut have_pending,
                            &mut pending_label,
                            &mut pending_href,
                            pending_depth,
                        );
                        depth -= 1;
                    }
                    b"navMap" => in_nav_map = false,
                    _ => {}
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }

    entries
}

fn flush_pending(
    entries: &mut Vec<TocEntry>,
    have_pending: &mut bool,
    label: &mut Option<String>,
    href: &mut Option<String>,
    depth: usize,
) {
    if *have_pending {
        if let Some(l) = label.take() {
            if !l.is_empty() {
                entries.push(TocEntry {
                    label: l,
                    href: href.take().unwrap_or_default(),
                    depth,
                });
            }
        }
        *have_pending = false;
        *href = None;
    }
}

fn attr_value(e: &quick_xml::events::BytesStart, key: &[u8]) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == key)
        .map(|a| a.unescape_value().unwrap_or_default().into_owned())
}

fn attr_contains(e: &quick_xml::events::BytesStart, key: &[u8], needle: &str) -> bool {
    attr_value(e, key)
        .map(|v| v.split_whitespace().any(|t| t == needle))
        .unwrap_or(false)
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_epub3_nav() {
        let xml = r#"<html xmlns:epub="http://www.idpf.org/2007/ops">
          <body>
            <nav epub:type="toc">
              <ol>
                <li><a href="ch1.xhtml">Chapter One</a></li>
                <li><a href="ch2.xhtml">Chapter Two</a>
                  <ol><li><a href="ch2.xhtml#s1">Section</a></li></ol>
                </li>
              </ol>
            </nav>
          </body>
        </html>"#;
        let toc = parse_nav_xhtml(xml);
        assert_eq!(toc.len(), 3);
        assert_eq!(toc[0].label, "Chapter One");
        assert_eq!(toc[0].href, "ch1.xhtml");
        assert_eq!(toc[0].depth, 0);
        assert_eq!(toc[1].label, "Chapter Two");
        assert_eq!(toc[2].label, "Section");
        assert_eq!(toc[2].depth, 1);
    }

    #[test]
    fn parses_ncx() {
        let xml = r#"<ncx><navMap>
            <navPoint id="n1" playOrder="1">
              <navLabel><text>Intro</text></navLabel>
              <content src="ch1.xhtml"/>
            </navPoint>
            <navPoint id="n2" playOrder="2">
              <navLabel><text>Body</text></navLabel>
              <content src="ch2.xhtml"/>
            </navPoint>
          </navMap></ncx>"#;
        let toc = parse_ncx(xml);
        assert_eq!(toc.len(), 2);
        assert_eq!(toc[0].label, "Intro");
        assert_eq!(toc[0].href, "ch1.xhtml");
        assert_eq!(toc[1].label, "Body");
        assert_eq!(toc[1].href, "ch2.xhtml");
    }

    #[test]
    fn parses_nested_ncx_depth() {
        let xml = r#"<ncx><navMap>
            <navPoint><navLabel><text>Part</text></navLabel><content src="p.xhtml"/>
              <navPoint><navLabel><text>Chapter</text></navLabel><content src="c.xhtml"/></navPoint>
            </navPoint>
          </navMap></ncx>"#;
        let toc = parse_ncx(xml);
        assert_eq!(toc.len(), 2);
        assert_eq!(toc[0].label, "Part");
        assert_eq!(toc[0].depth, 0);
        assert_eq!(toc[1].label, "Chapter");
        assert_eq!(toc[1].depth, 1);
    }
}
