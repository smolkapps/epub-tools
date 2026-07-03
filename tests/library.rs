//! Library-level integration tests exercising the public `Epub` API directly
//! (no subprocess), including the set-metadata round-trip and OCF packaging rule.

use epub_tools::fixture::{
    build_default_epub_bytes, build_epub_bytes, FixtureSpec, SAMPLE_COVER_PNG,
};
use epub_tools::package::first_entry_info;
use epub_tools::{Epub, MetadataEdit};

#[test]
fn info_fields_parse() {
    let epub = Epub::from_bytes(build_default_epub_bytes().unwrap()).unwrap();
    let md = epub.metadata();
    assert_eq!(md.title.as_deref(), Some("The Sample Book"));
    assert_eq!(md.creators, vec!["Ada Lovelace", "Charles Babbage"]);
    assert_eq!(md.first_language(), Some("en"));
    assert_eq!(md.publisher.as_deref(), Some("Analytical Press"));
    assert_eq!(md.date.as_deref(), Some("2026-06-21"));
    assert!(md.identifier.as_deref().unwrap().contains("urn:uuid"));
}

#[test]
fn spine_in_reading_order_with_resolved_paths() {
    let epub = Epub::from_bytes(build_default_epub_bytes().unwrap()).unwrap();
    let spine = epub.spine_items();
    assert_eq!(spine.len(), 2);
    assert_eq!(spine[0].href, "chapter1.xhtml");
    assert_eq!(spine[0].resolved_path, "OEBPS/chapter1.xhtml");
    assert_eq!(spine[1].href, "chapter2.xhtml");
    assert_eq!(spine[1].resolved_path, "OEBPS/chapter2.xhtml");
}

#[test]
fn full_text_strips_tags_and_keeps_order() {
    let epub = Epub::from_bytes(build_default_epub_bytes().unwrap()).unwrap();
    let text = epub.full_text();
    assert!(text.contains("It was the best of times."));
    assert!(text.contains("It was the worst of times."));
    assert!(text.contains("Call me Ishmael."));
    assert!(!text.contains('<'));
    let a = text.find("best of times").unwrap();
    let b = text.find("Ishmael").unwrap();
    assert!(a < b);
}

#[test]
fn chapter_text_by_index() {
    let epub = Epub::from_bytes(build_default_epub_bytes().unwrap()).unwrap();
    let c1 = epub.chapter_text(1).unwrap();
    assert!(c1.contains("best of times"));
    assert!(!c1.contains("Ishmael"));
    let c2 = epub.chapter_text(2).unwrap();
    assert!(c2.contains("Ishmael"));
    // Out of range errors.
    assert!(epub.chapter_text(0).is_err());
    assert!(epub.chapter_text(99).is_err());
}

#[test]
fn toc_from_nav() {
    let epub = Epub::from_bytes(build_default_epub_bytes().unwrap()).unwrap();
    let toc = epub.toc();
    assert_eq!(toc.len(), 2);
    assert_eq!(toc[0].label, "Chapter One");
    assert_eq!(toc[0].href, "chapter1.xhtml");
    assert_eq!(toc[1].label, "Chapter Two");
}

#[test]
fn cover_extracts_declared_image() {
    let epub = Epub::from_bytes(build_default_epub_bytes().unwrap()).unwrap();
    let cover = epub.cover().expect("fixture declares a cover");
    assert_eq!(cover.media_type, "image/png");
    assert_eq!(cover.resolved_path, "OEBPS/cover.png");
    assert_eq!(cover.extension(), "png");
    // The extracted bytes are exactly the embedded image.
    assert_eq!(cover.bytes, SAMPLE_COVER_PNG);
    // ...and they are a real PNG (starts with the PNG signature).
    assert_eq!(&cover.bytes[..8], b"\x89PNG\r\n\x1a\n");
}

#[test]
fn cover_resolves_when_entry_name_literally_contains_percent() {
    // Regression: some archives store an entry whose NAME literally contains a
    // percent sequence — here the file is really named `cover%20art.png` (the
    // `%` is a genuine character in the zip entry name, NOT a URL-encoding of a
    // space), and the manifest href is written the same literal way. Percent-
    // decoding the href yields `cover art.png`, which is ABSENT from the archive.
    // Resolution must therefore fall back to the raw, undecoded href and still
    // find the real entry `OEBPS/cover%20art.png`.
    let spec = FixtureSpec {
        cover: Some(epub_tools::fixture::CoverImage {
            filename: "cover%20art.png".to_string(),
            media_type: "image/png".to_string(),
            bytes: SAMPLE_COVER_PNG.to_vec(),
        }),
        ..Default::default()
    };
    let epub = Epub::from_bytes(build_epub_bytes(&spec).unwrap()).unwrap();
    let cover = epub
        .cover()
        .expect("cover whose entry name literally contains '%20' must resolve via raw fallback");
    assert_eq!(cover.resolved_path, "OEBPS/cover%20art.png");
    assert_eq!(cover.media_type, "image/png");
    assert_eq!(cover.bytes, SAMPLE_COVER_PNG);
}

#[test]
fn cover_absent_returns_none() {
    let spec = FixtureSpec {
        cover: None,
        ..Default::default()
    };
    let epub = Epub::from_bytes(build_epub_bytes(&spec).unwrap()).unwrap();
    assert!(epub.cover().is_none());
}

#[test]
fn cover_extension_falls_back_to_media_type() {
    // A cover advertised as JPEG but stored as `cover.bin` still saves as `.jpg`.
    let spec = FixtureSpec {
        cover: Some(epub_tools::fixture::CoverImage {
            filename: "cover.bin".to_string(),
            media_type: "image/jpeg".to_string(),
            bytes: vec![0xFF, 0xD8, 0xFF, 0xE0],
        }),
        ..Default::default()
    };
    let epub = Epub::from_bytes(build_epub_bytes(&spec).unwrap()).unwrap();
    let cover = epub.cover().unwrap();
    assert_eq!(cover.extension(), "jpg");
    assert_eq!(cover.resolved_path, "OEBPS/cover.bin");
}

#[test]
fn cover_extension_fallback_lowercases() {
    // An unknown media type forces the filename-extension fallback, which the
    // docs promise is lowercase — even when the filename shouts.
    let spec = FixtureSpec {
        cover: Some(epub_tools::fixture::CoverImage {
            filename: "COVER.JPG".to_string(),
            media_type: "application/octet-stream".to_string(),
            bytes: vec![0x00, 0x01, 0x02],
        }),
        ..Default::default()
    };
    let epub = Epub::from_bytes(build_epub_bytes(&spec).unwrap()).unwrap();
    let cover = epub.cover().unwrap();
    assert_eq!(cover.extension(), "jpg");
}

#[test]
fn set_metadata_roundtrip_updates_and_keeps_mimetype_first_stored() {
    let epub = Epub::from_bytes(build_default_epub_bytes().unwrap()).unwrap();
    let edit = MetadataEdit {
        title: Some("Edited Title".to_string()),
        author: Some("Edited Author".to_string()),
        ..Default::default()
    };
    let new_bytes = epub.serialize_with_metadata(&edit).unwrap();

    // mimetype first & stored.
    let (first_name, stored) = first_entry_info(&new_bytes).unwrap();
    assert_eq!(first_name, "mimetype");
    assert!(stored, "mimetype must be stored");

    // Re-read: the new metadata is in effect; other fields are preserved.
    let reopened = Epub::from_bytes(new_bytes).unwrap();
    let md = reopened.metadata();
    assert_eq!(md.title.as_deref(), Some("Edited Title"));
    assert_eq!(md.first_creator(), Some("Edited Author"));
    // Language/publisher untouched.
    assert_eq!(md.first_language(), Some("en"));
    assert_eq!(md.publisher.as_deref(), Some("Analytical Press"));
    // The body text still extracts fine, proving the repackage is intact.
    assert!(reopened.full_text().contains("Call me Ishmael."));
}

#[test]
fn set_metadata_only_replaces_first_creator() {
    // The fixture has two creators; editing author replaces the FIRST dc:creator
    // and leaves the rest, so re-read sees [Edited Author, Charles Babbage].
    let epub = Epub::from_bytes(build_default_epub_bytes().unwrap()).unwrap();
    let edit = MetadataEdit {
        author: Some("Edited Author".to_string()),
        ..Default::default()
    };
    let new_bytes = epub.serialize_with_metadata(&edit).unwrap();
    let reopened = Epub::from_bytes(new_bytes).unwrap();
    assert_eq!(
        reopened.metadata().creators,
        vec!["Edited Author", "Charles Babbage"]
    );
}
