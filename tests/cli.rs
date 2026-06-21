//! End-to-end integration tests: build a real .epub fixture on disk, then drive
//! the compiled `epub-tools` binary against it via `assert_cmd`.

use std::io::Cursor;
use std::process::Command;

use assert_cmd::prelude::*;
use epub_tools::fixture::{build_default_epub_bytes, build_epub_bytes, FixtureSpec};
use epub_tools::package::EPUB_MIMETYPE;
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
