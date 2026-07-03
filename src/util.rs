//! Small path helpers for resolving OPF-relative hrefs inside the zip archive.

/// Resolve a (possibly relative) href against the directory of a base file,
/// both expressed as forward-slash zip-archive paths.
///
/// EPUB hrefs are URLs relative to the document that contains them. This handles
/// `.` and `..` segments and strips any URL fragment / query. Absolute-looking
/// hrefs (those beginning with `/`) are returned without the leading slash so
/// they map onto zip entry names.
///
/// # Examples
/// ```
/// use epub_tools::util::resolve_href;
/// assert_eq!(resolve_href("OEBPS/content.opf", "chap1.xhtml"), "OEBPS/chap1.xhtml");
/// assert_eq!(resolve_href("OEBPS/content.opf", "../images/x.png"), "images/x.png");
/// assert_eq!(resolve_href("content.opf", "text/c.xhtml"), "text/c.xhtml");
/// assert_eq!(resolve_href("OEBPS/content.opf", "cover%20art.jpg"), "OEBPS/cover art.jpg");
/// ```
pub fn resolve_href(base_file: &str, href: &str) -> String {
    // Percent-decode so a URL-escaped href like `cover%20art.jpg` maps onto the
    // literal zip entry name `cover art.jpg`. This is the PRIMARY resolution:
    // conforming EPUBs percent-encode reserved characters (notably spaces) in
    // hrefs, so the decoded form is what usually matches the archive entry.
    resolve_href_impl(base_file, href, true)
}

/// Like [`resolve_href`] but WITHOUT percent-decoding: the href is treated as a
/// literal path. Used as a fallback for the rare archive whose zip entry names
/// literally contain a percent sequence (e.g. a stored entry genuinely named
/// `cover%20art.png`, where `%` is a real character rather than an encoding of a
/// space). Path segment logic (`.`/`..`, fragment/query stripping, leading `/`)
/// is identical to the decoding form.
///
/// # Examples
/// ```
/// use epub_tools::util::resolve_href_raw;
/// // No decoding: the literal %20 is preserved in the resolved entry name.
/// assert_eq!(
///     resolve_href_raw("OEBPS/content.opf", "cover%20art.png"),
///     "OEBPS/cover%20art.png"
/// );
/// ```
pub fn resolve_href_raw(base_file: &str, href: &str) -> String {
    resolve_href_impl(base_file, href, false)
}

/// Shared resolution: strip fragment/query, optionally percent-decode, then
/// normalize `.`/`..` segments against the base file's directory.
fn resolve_href_impl(base_file: &str, href: &str, decode: bool) -> String {
    // Drop fragment and query from the href.
    let href = href.split(['#', '?']).next().unwrap_or(href);

    let decoded;
    let href = if decode {
        decoded = percent_decode(href);
        decoded.as_str()
    } else {
        href
    };

    let mut stack: Vec<&str> = Vec::new();

    if href.starts_with('/') {
        // Absolute path within the container root.
        for seg in href.split('/') {
            push_segment(&mut stack, seg);
        }
    } else {
        // Start from the base file's directory.
        let base_dir = match base_file.rfind('/') {
            Some(i) => &base_file[..i],
            None => "",
        };
        if !base_dir.is_empty() {
            for seg in base_dir.split('/') {
                push_segment(&mut stack, seg);
            }
        }
        for seg in href.split('/') {
            push_segment(&mut stack, seg);
        }
    }

    stack.join("/")
}

/// Decode `%XX` percent-escapes in a URL path. Invalid or truncated escapes are
/// left as-is. Decoded bytes are interpreted as UTF-8 (lossily).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push(hi * 16 + lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn push_segment<'a>(stack: &mut Vec<&'a str>, seg: &'a str) {
    match seg {
        "" | "." => {}
        ".." => {
            stack.pop();
        }
        other => stack.push(other),
    }
}

/// The directory portion of an archive path (no trailing slash), or "" if none.
pub fn parent_dir(path: &str) -> &str {
    match path.rfind('/') {
        Some(i) => &path[..i],
        None => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_sibling() {
        assert_eq!(
            resolve_href("OEBPS/content.opf", "ch1.xhtml"),
            "OEBPS/ch1.xhtml"
        );
    }

    #[test]
    fn resolves_parent() {
        assert_eq!(
            resolve_href("OEBPS/text/content.opf", "../images/cover.png"),
            "OEBPS/images/cover.png"
        );
    }

    #[test]
    fn resolves_root_opf() {
        assert_eq!(resolve_href("content.opf", "nav.xhtml"), "nav.xhtml");
    }

    #[test]
    fn strips_fragment() {
        assert_eq!(
            resolve_href("OEBPS/content.opf", "ch1.xhtml#sec2"),
            "OEBPS/ch1.xhtml"
        );
    }

    #[test]
    fn handles_dot_segments() {
        assert_eq!(
            resolve_href("OEBPS/content.opf", "./sub/./a.xhtml"),
            "OEBPS/sub/a.xhtml"
        );
    }

    #[test]
    fn decodes_percent_encoding() {
        assert_eq!(
            resolve_href("OEBPS/content.opf", "cover%20art.jpg"),
            "OEBPS/cover art.jpg"
        );
        // A stray, malformed percent escape is left untouched.
        assert_eq!(
            resolve_href("OEBPS/content.opf", "50%off.png"),
            "OEBPS/50%off.png"
        );
    }

    #[test]
    fn raw_resolution_does_not_decode() {
        // The raw form leaves percent sequences intact so it can match an entry
        // whose name literally contains `%20`.
        assert_eq!(
            resolve_href_raw("OEBPS/content.opf", "cover%20art.png"),
            "OEBPS/cover%20art.png"
        );
        // Fragment/query stripping and `..` normalization still apply.
        assert_eq!(
            resolve_href_raw("OEBPS/text/x.opf", "../img/c%20a.png#f"),
            "OEBPS/img/c%20a.png"
        );
    }

    #[test]
    fn parent_dir_works() {
        assert_eq!(parent_dir("OEBPS/content.opf"), "OEBPS");
        assert_eq!(parent_dir("content.opf"), "");
    }
}
