//! Core data structures describing the contents of an EPUB package document.

/// Dublin Core metadata pulled from the OPF `<metadata>` element.
///
/// All fields are optional except the vectors, which may be empty. EPUB allows
/// repeated `dc:creator` / `dc:language` etc., so those are kept as `Vec`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Metadata {
    pub title: Option<String>,
    pub creators: Vec<String>,
    pub languages: Vec<String>,
    pub identifier: Option<String>,
    pub publisher: Option<String>,
    pub date: Option<String>,
    pub description: Option<String>,
    pub rights: Option<String>,
}

impl Metadata {
    /// First creator, if any. Convenience for single-author display.
    pub fn first_creator(&self) -> Option<&str> {
        self.creators.first().map(|s| s.as_str())
    }

    /// First language code, if any.
    pub fn first_language(&self) -> Option<&str> {
        self.languages.first().map(|s| s.as_str())
    }
}

/// A single `<item>` from the OPF `<manifest>`.
///
/// `href` is stored exactly as written in the OPF (relative to the OPF's own
/// directory). `resolved_path` is the full zip-archive path of the resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestItem {
    pub id: String,
    pub href: String,
    pub media_type: String,
    /// Full path of the resource inside the zip archive.
    pub resolved_path: String,
    /// Value of the `properties` attribute (e.g. "nav", "cover-image"), if present.
    pub properties: Option<String>,
}

impl ManifestItem {
    /// True if this manifest item is the EPUB 3 navigation document.
    pub fn is_nav(&self) -> bool {
        self.properties
            .as_deref()
            .map(|p| p.split_whitespace().any(|t| t == "nav"))
            .unwrap_or(false)
    }

    /// True if this manifest item is the EPUB 3 cover image
    /// (`properties="cover-image"`).
    pub fn is_cover_image(&self) -> bool {
        self.properties
            .as_deref()
            .map(|p| p.split_whitespace().any(|t| t == "cover-image"))
            .unwrap_or(false)
    }
}

/// A single `<itemref>` from the OPF `<spine>`, in reading order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpineItem {
    /// The `idref` pointing at a manifest item id.
    pub idref: String,
    /// `false` when `linear="no"`; defaults to `true`.
    pub linear: bool,
}

/// The fully parsed OPF package document.
#[derive(Debug, Clone, Default)]
pub struct Package {
    pub metadata: Metadata,
    pub manifest: Vec<ManifestItem>,
    pub spine: Vec<SpineItem>,
    /// The `toc` attribute of the spine: an idref to the NCX manifest item (EPUB 2).
    pub spine_toc: Option<String>,
    /// The EPUB 2 cover id from `<meta name="cover" content="ID"/>`, if present.
    pub cover_id: Option<String>,
    /// Full archive path of the OPF document itself.
    pub opf_path: String,
}

impl Package {
    /// Look up a manifest item by its id.
    pub fn manifest_item(&self, id: &str) -> Option<&ManifestItem> {
        self.manifest.iter().find(|m| m.id == id)
    }

    /// Manifest items in spine reading order (skips spine entries whose idref
    /// has no matching manifest item).
    pub fn spine_items(&self) -> Vec<&ManifestItem> {
        self.spine
            .iter()
            .filter_map(|s| self.manifest_item(&s.idref))
            .collect()
    }

    /// The EPUB 3 navigation document, if one is declared via `properties="nav"`.
    pub fn nav_item(&self) -> Option<&ManifestItem> {
        self.manifest.iter().find(|m| m.is_nav())
    }

    /// The EPUB 2 NCX document, resolved from the spine `toc` attribute, falling
    /// back to any manifest item with the NCX media type.
    pub fn ncx_item(&self) -> Option<&ManifestItem> {
        if let Some(toc) = &self.spine_toc {
            if let Some(item) = self.manifest_item(toc) {
                return Some(item);
            }
        }
        self.manifest
            .iter()
            .find(|m| m.media_type == "application/x-dtbncx+xml")
    }

    /// The cover image manifest item, if the book declares one.
    ///
    /// Prefers the EPUB 3 convention (`properties="cover-image"`), then falls
    /// back to the EPUB 2 convention (`<meta name="cover" content="ID"/>`
    /// pointing at a manifest item).
    pub fn cover_item(&self) -> Option<&ManifestItem> {
        if let Some(item) = self.manifest.iter().find(|m| m.is_cover_image()) {
            return Some(item);
        }
        if let Some(id) = &self.cover_id {
            return self.manifest_item(id);
        }
        None
    }
}

/// A single entry in a table of contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocEntry {
    pub label: String,
    pub href: String,
    /// Nesting depth (0 = top level).
    pub depth: usize,
}
