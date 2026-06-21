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
/// ```
pub fn resolve_href(base_file: &str, href: &str) -> String {
    // Drop fragment and query from the href.
    let href = href.split(['#', '?']).next().unwrap_or(href);

    // Percent-decode is intentionally NOT done: zip entry names in practice
    // match the raw href for the EPUBs we target, and decoding risks mismatches.

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
    fn parent_dir_works() {
        assert_eq!(parent_dir("OEBPS/content.opf"), "OEBPS");
        assert_eq!(parent_dir("content.opf"), "");
    }
}
