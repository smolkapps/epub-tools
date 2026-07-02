//! End-to-end integration tests: build a real .epub fixture on disk, then drive
//! the compiled `epub-tools` binary against it via `assert_cmd`.

use std::io::Cursor;
use std::process::Command;

use assert_cmd::prelude::*;
use epub_tools::fixture::{
    build_default_epub_bytes, build_epub_bytes, FixtureSpec, SAMPLE_COVER_PNG,
};
use epub_tools::package::{write_epub_to_vec, EPUB_MIMETYPE};
use predicates::prelude::*;
use tempfile::TempDir;
use zip::{CompressionMethod, ZipArchive};

/// Write the default fixture EPUB into a temp dir and return (dir, path).
fn fixture_on_disk() -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("book.epub");
    let bytes = build_default_epub_bytes().expect("build fixture");
    std::fs::write(&path, bytes).expect("write fixture");
    (dir, path)
}

#[test]
fn info_reports_title_author_language() {
    let (_dir, path) = fixture_on_disk();
    Command::cargo_bin("epub-tools")
        .unwrap()
        .arg("info")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("The Sample Book"))
        .stdout(predicate::str::contains("Ada Lovelace"))
        .stdout(predicate::str::contains("Charles Babbage"))
        .stdout(predicate::str::contains("Language:"))
        .stdout(predicate::str::contains("en"))
        .stdout(predicate::str::contains("Analytical Press"))
        .stdout(predicate::str::contains("2026-06-21"));
}

#[test]
fn list_shows_spine_in_order() {
    let (_dir, path) = fixture_on_disk();
    let out = Command::cargo_bin("epub-tools")
        .unwrap()
        .arg("list")
        .arg(&path)
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);

    // chapter1 must appear before chapter2 in the spine section.
    let p1 = stdout.find("chapter1.xhtml").expect("chapter1 listed");
    let p2 = stdout.find("chapter2.xhtml").expect("chapter2 listed");
    assert!(p1 < p2, "spine order should list chapter1 before chapter2");
    assert!(stdout.contains("application/xhtml+xml"));
    // The nav doc should be in the manifest section.
    assert!(stdout.contains("nav.xhtml"));
}

#[test]
fn text_extracts_chapter_bodies_in_order() {
    let (_dir, path) = fixture_on_disk();
    let out = Command::cargo_bin("epub-tools")
        .unwrap()
        .arg("text")
        .arg(&path)
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);

    assert!(stdout.contains("It was the best of times."));
    assert!(stdout.contains("Call me Ishmael."));
    // Reading order: chapter one body precedes chapter two body.
    let a = stdout.find("best of times").unwrap();
    let b = stdout.find("Ishmael").unwrap();
    assert!(a < b, "text must follow spine reading order");
    // Tags are stripped — no angle brackets from markup remain.
    assert!(!stdout.contains("<p>"));
    assert!(!stdout.contains("<h1>"));
}

#[test]
fn text_single_chapter() {
    let (_dir, path) = fixture_on_disk();
    let out = Command::cargo_bin("epub-tools")
        .unwrap()
        .args(["text", path.to_str().unwrap(), "--chapter", "2"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Call me Ishmael."));
    // Chapter one content must NOT be present when only chapter 2 is requested.
    assert!(!stdout.contains("best of times"));
}

#[test]
fn toc_lists_chapter_titles() {
    let (_dir, path) = fixture_on_disk();
    Command::cargo_bin("epub-tools")
        .unwrap()
        .arg("toc")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("Chapter One"))
        .stdout(predicate::str::contains("Chapter Two"));
}

#[test]
fn cover_writes_image_to_requested_path() {
    let (dir, path) = fixture_on_disk();
    let out_path = dir.path().join("extracted-cover.png");

    Command::cargo_bin("epub-tools")
        .unwrap()
        .args([
            "cover",
            path.to_str().unwrap(),
            "-o",
            out_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("image/png"));

    let written = std::fs::read(&out_path).expect("cover file written");
    // Exact bytes of the embedded fixture cover, and a valid PNG signature.
    assert_eq!(written, SAMPLE_COVER_PNG);
    assert_eq!(&written[..8], b"\x89PNG\r\n\x1a\n");
}

#[test]
fn cover_errors_when_no_cover_declared() {
    // Build a coverless book on disk and confirm the command fails cleanly.
    let spec = FixtureSpec {
        cover: None,
        ..Default::default()
    };
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("nocover.epub");
    std::fs::write(&path, build_epub_bytes(&spec).unwrap()).unwrap();

    Command::cargo_bin("epub-tools")
        .unwrap()
        .arg("cover")
        .arg(&path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("does not declare a cover"));
}

#[test]
fn cover_default_output_names_by_extension() {
    // With no -o, the cover is written to `cover.<ext>` in the current directory.
    let (dir, path) = fixture_on_disk();

    Command::cargo_bin("epub-tools")
        .unwrap()
        .arg("cover")
        .arg(&path)
        .current_dir(dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("cover.png"));

    let written = std::fs::read(dir.path().join("cover.png")).expect("default cover.png written");
    assert_eq!(written, SAMPLE_COVER_PNG);
}

#[test]
fn cover_refuses_to_overwrite_existing_output() {
    let (dir, path) = fixture_on_disk();
    let out_path = dir.path().join("exists.png");
    std::fs::write(&out_path, b"do-not-clobber").unwrap();

    Command::cargo_bin("epub-tools")
        .unwrap()
        .args([
            "cover",
            path.to_str().unwrap(),
            "-o",
            out_path.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("refusing to overwrite"));

    // The pre-existing file is left untouched.
    assert_eq!(std::fs::read(&out_path).unwrap(), b"do-not-clobber");
}

#[test]
fn cover_reports_missing_resource_distinctly() {
    // A book that DECLARES a cover whose resource is absent must not be reported
    // as "does not declare a cover"; it names the missing archive path instead.
    let container = r#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#;
    let opf = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>T</dc:title></metadata>
  <manifest>
    <item id="cover" href="missing.png" media-type="image/png" properties="cover-image"/>
  </manifest>
  <spine/>
</package>"#;
    let entries = vec![
        (
            "META-INF/container.xml".to_string(),
            container.as_bytes().to_vec(),
        ),
        ("OEBPS/content.opf".to_string(), opf.as_bytes().to_vec()),
    ];
    let bytes = write_epub_to_vec(&entries).unwrap();
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("broken.epub");
    std::fs::write(&path, bytes).unwrap();

    Command::cargo_bin("epub-tools")
        .unwrap()
        .arg("cover")
        .arg(&path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("missing from archive"))
        .stderr(predicate::str::contains("OEBPS/missing.png"))
        .stderr(predicate::str::contains("does not declare").not());
}

#[test]
fn cover_guide_pointing_at_xhtml_is_not_extracted() {
    // A guide-only EPUB2 book whose cover reference points at an XHTML wrapper
    // page must NOT have the HTML page written out and reported as a cover image.
    // It should fail with a distinct "unresolvable cover declaration" message and
    // never touch the xhtml bytes.
    let container = r#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#;
    let opf = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>T</dc:title></metadata>
  <manifest>
    <item id="cover-page" href="cover.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine><itemref idref="cover-page"/></spine>
  <guide>
    <reference type="cover" title="Cover" href="cover.xhtml"/>
  </guide>
</package>"#;
    let cover_html = b"<html><body>NOT-AN-IMAGE</body></html>";
    let entries = vec![
        (
            "META-INF/container.xml".to_string(),
            container.as_bytes().to_vec(),
        ),
        ("OEBPS/content.opf".to_string(), opf.as_bytes().to_vec()),
        ("OEBPS/cover.xhtml".to_string(), cover_html.to_vec()),
    ];
    let bytes = write_epub_to_vec(&entries).unwrap();
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("guide-xhtml.epub");
    std::fs::write(&path, bytes).unwrap();
    let out = dir.path().join("cover.xhtml");

    Command::cargo_bin("epub-tools")
        .unwrap()
        .arg("cover")
        .arg(&path)
        .arg("-o")
        .arg(&out)
        .assert()
        .failure()
        .stderr(predicate::str::contains("unresolvable cover declaration"))
        .stderr(predicate::str::contains("does not declare").not());

    // The HTML wrapper bytes were never written to the output path.
    assert!(!out.exists(), "no cover file should have been written");
}

#[test]
fn cover_dangling_meta_declaration_is_unresolvable_not_undeclared() {
    // A `<meta name="cover" content="no-such-id"/>` names a manifest id that does
    // not exist. This is a broken/unresolvable declaration, NOT the absence of a
    // cover, so the error must say so rather than "does not declare a cover".
    let container = r#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#;
    let opf = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>T</dc:title>
    <meta name="cover" content="no-such-id"/>
  </metadata>
  <manifest>
    <item id="c1" href="c1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine><itemref idref="c1"/></spine>
</package>"#;
    let entries = vec![
        (
            "META-INF/container.xml".to_string(),
            container.as_bytes().to_vec(),
        ),
        ("OEBPS/content.opf".to_string(), opf.as_bytes().to_vec()),
        (
            "OEBPS/c1.xhtml".to_string(),
            b"<html><body>hi</body></html>".to_vec(),
        ),
    ];
    let bytes = write_epub_to_vec(&entries).unwrap();
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("dangling.epub");
    std::fs::write(&path, bytes).unwrap();

    Command::cargo_bin("epub-tools")
        .unwrap()
        .arg("cover")
        .arg(&path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("unresolvable cover declaration"))
        .stderr(predicate::str::contains("no-such-id"))
        .stderr(predicate::str::contains("does not declare").not());
}

#[test]
fn set_metadata_updates_title_and_author_and_keeps_mimetype_first_stored() {
    let (dir, path) = fixture_on_disk();
    let out_path = dir.path().join("edited.epub");

    Command::cargo_bin("epub-tools")
        .unwrap()
        .args([
            "set-metadata",
            path.to_str().unwrap(),
            "--title",
            "A Whole New Title",
            "--author",
            "Grace Hopper",
            "-o",
            out_path.to_str().unwrap(),
        ])
        .assert()
        .success();

    assert!(out_path.exists(), "output EPUB was written");

    // Re-read via the binary's `info` — title and author must be updated.
    Command::cargo_bin("epub-tools")
        .unwrap()
        .arg("info")
        .arg(&out_path)
        .assert()
        .success()
        .stdout(predicate::str::contains("A Whole New Title"))
        .stdout(predicate::str::contains("Grace Hopper"))
        .stdout(
            predicate::str::contains("A Whole New Title")
                .and(predicate::str::contains("The Sample Book").not()),
        );

    // Inspect the zip directly: first entry must be `mimetype`, stored, exact bytes.
    let bytes = std::fs::read(&out_path).unwrap();
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    {
        let first = archive.by_index(0).unwrap();
        assert_eq!(first.name(), "mimetype", "first entry must be mimetype");
        assert_eq!(
            first.compression(),
            CompressionMethod::Stored,
            "mimetype must be stored (uncompressed)"
        );
    }
    // And the mimetype contents are exactly right.
    {
        let mut mt = archive.by_name("mimetype").unwrap();
        use std::io::Read;
        let mut s = String::new();
        mt.read_to_string(&mut s).unwrap();
        assert_eq!(s, EPUB_MIMETYPE);
    }
}

#[test]
fn set_metadata_inserts_missing_field() {
    // Build a fixture, then set a brand-new publisher that we override to a known
    // value first so we can assert the inserted-when-present path through the CLI.
    let spec = FixtureSpec {
        title: "Edits Test".to_string(),
        ..Default::default()
    };
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("in.epub");
    std::fs::write(&path, build_epub_bytes(&spec).unwrap()).unwrap();
    let out = dir.path().join("out.epub");

    Command::cargo_bin("epub-tools")
        .unwrap()
        .args([
            "set-metadata",
            path.to_str().unwrap(),
            "--language",
            "fr",
            "-o",
            out.to_str().unwrap(),
        ])
        .assert()
        .success();

    Command::cargo_bin("epub-tools")
        .unwrap()
        .arg("info")
        .arg(&out)
        .assert()
        .success()
        .stdout(predicate::str::contains("fr"));
}

#[test]
fn make_sample_then_info_roundtrip() {
    let dir = TempDir::new().unwrap();
    let sample = dir.path().join("generated.epub");

    Command::cargo_bin("epub-tools")
        .unwrap()
        .args(["make-sample", "-o", sample.to_str().unwrap()])
        .assert()
        .success();

    assert!(sample.exists());

    Command::cargo_bin("epub-tools")
        .unwrap()
        .arg("info")
        .arg(&sample)
        .assert()
        .success()
        .stdout(predicate::str::contains("The Sample Book"));
}

#[test]
fn missing_file_errors_cleanly() {
    Command::cargo_bin("epub-tools")
        .unwrap()
        .arg("info")
        .arg("/nonexistent/path/to/book.epub")
        .assert()
        .failure();
}
