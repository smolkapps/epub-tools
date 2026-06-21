//! Convert XHTML content documents into readable plain text.

use quick_xml::events::Event;
use quick_xml::Reader;

use crate::container::local_name;

/// Block-level / line-breaking elements: emitting a newline after them keeps the
/// extracted text readable instead of one run-on paragraph.
fn is_block(local: &[u8]) -> bool {
    matches!(
        local,
        b"p" | b"div"
            | b"br"
            | b"li"
            | b"tr"
            | b"h1"
            | b"h2"
            | b"h3"
            | b"h4"
            | b"h5"
            | b"h6"
            | b"section"
            | b"article"
            | b"header"
            | b"footer"
            | b"blockquote"
            | b"figcaption"
            | b"table"
            | b"ul"
            | b"ol"
            | b"hr"
            | b"pre"
    )
}

/// Elements whose textual content should be discarded entirely.
fn is_skipped(local: &[u8]) -> bool {
    matches!(local, b"script" | b"style" | b"head" | b"title")
}

/// Extract plain text from an XHTML (or HTML) document.
///
/// Tags are removed, entities are decoded, `<script>`/`<style>`/`<head>` content
/// is dropped, and block elements introduce line breaks. Runs of blank lines are
/// collapsed and surrounding whitespace trimmed.
pub fn xhtml_to_text(xml: &str) -> String {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    reader.config_mut().check_end_names = false;

    let mut out = String::new();
    let mut skip_depth: u32 = 0;
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = e.name();
                let local = local_name(name.as_ref());
                if is_skipped(local) {
                    skip_depth += 1;
                } else if is_block(local) {
                    push_break(&mut out);
                }
            }
            Ok(Event::Empty(e)) => {
                let name = e.name();
                let local = local_name(name.as_ref());
                if is_block(local) {
                    push_break(&mut out);
                }
            }
            Ok(Event::End(e)) => {
                let name = e.name();
                let local = local_name(name.as_ref());
                if is_skipped(local) {
                    skip_depth = skip_depth.saturating_sub(1);
                } else if is_block(local) {
                    push_break(&mut out);
                }
            }
            Ok(Event::Text(t)) => {
                if skip_depth == 0 {
                    let decoded = t.unescape().unwrap_or_default();
                    push_text(&mut out, &decoded);
                }
            }
            Ok(Event::CData(t)) => {
                if skip_depth == 0 {
                    push_text(&mut out, &String::from_utf8_lossy(t.as_ref()));
                }
            }
            Ok(Event::Eof) => break,
            // Be lenient: malformed markup should not abort text extraction.
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }

    normalize(&out)
}

/// Append text, collapsing internal runs of ASCII whitespace to single spaces.
fn push_text(out: &mut String, s: &str) {
    let mut last_ws = out.ends_with(['\n', ' ']) || out.is_empty();
    for ch in s.chars() {
        if ch.is_whitespace() {
            if !last_ws {
                out.push(' ');
                last_ws = true;
            }
        } else {
            out.push(ch);
            last_ws = false;
        }
    }
}

/// Record a line break, avoiding more than one trailing newline at a time.
fn push_break(out: &mut String) {
    // Trim a trailing space before the break.
    while out.ends_with(' ') {
        out.pop();
    }
    if !out.ends_with('\n') && !out.is_empty() {
        out.push('\n');
    }
}

/// Trim leading/trailing whitespace and collapse 3+ newlines into 2.
fn normalize(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut newline_run = 0;
    for ch in s.chars() {
        if ch == '\n' {
            newline_run += 1;
            if newline_run <= 2 {
                result.push('\n');
            }
        } else {
            newline_run = 0;
            result.push(ch);
        }
    }
    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_tags() {
        let html = "<p>Hello <b>world</b></p>";
        assert_eq!(xhtml_to_text(html), "Hello world");
    }

    #[test]
    fn decodes_entities() {
        let html = "<p>Tom &amp; Jerry &lt;3</p>";
        assert_eq!(xhtml_to_text(html), "Tom & Jerry <3");
    }

    #[test]
    fn drops_script_and_style() {
        let html =
            "<html><head><style>p{color:red}</style></head><body><script>alert(1)</script><p>Keep</p></body></html>";
        assert_eq!(xhtml_to_text(html), "Keep");
    }

    #[test]
    fn block_elements_break_lines() {
        let html = "<p>First</p><p>Second</p>";
        assert_eq!(xhtml_to_text(html), "First\nSecond");
    }

    #[test]
    fn collapses_whitespace() {
        let html = "<p>a    b\n\t c</p>";
        assert_eq!(xhtml_to_text(html), "a b c");
    }
}
