# epub-tools

A small, fast Rust CLI to **inspect, extract, and edit EPUB files**. No network,
no services, no telemetry — it just reads the zip, parses the OPF package
document, and does the obvious thing.

## What an EPUB is (in 30 seconds)

An EPUB is a ZIP archive containing:

- `mimetype` — the literal bytes `application/epub+zip`, which **must be the
  first entry and stored uncompressed** (the classic gotcha this tool honors).
- `META-INF/container.xml` — points at the OPF package document.
- a `.opf` package document — Dublin Core metadata + a **manifest** (every
  resource) + a **spine** (the reading order).
- XHTML content documents — the actual chapters.
- optionally a navigation document (EPUB 3 `nav.xhtml`) or an NCX (EPUB 2) for
  the table of contents.

## Install / build

```sh
cargo build --release
# binary at target/release/epub-tools
```

## Usage

```sh
# Dublin Core metadata: title, author(s), language, identifier, publisher, date
epub-tools info BOOK.epub

# Spine reading order + the full manifest (href, media-type)
epub-tools list BOOK.epub

# Plain text of every spine document, in reading order (XHTML tags stripped)
epub-tools text BOOK.epub
epub-tools text BOOK.epub --chapter 2      # just the 2nd spine document

# Table of contents (EPUB 3 nav, falling back to EPUB 2 NCX)
epub-tools toc BOOK.epub

# Rewrite OPF metadata and repackage a valid EPUB (mimetype first & stored)
epub-tools set-metadata BOOK.epub --title "New Title" --author "New Author" -o OUT.epub
#   --language and --publisher are also supported

# Generate a known-good sample EPUB to experiment with
epub-tools make-sample -o sample.epub
```

### Example

```sh
$ epub-tools make-sample -o sample.epub
$ epub-tools info sample.epub
Title:      The Sample Book
Author(s):  Ada Lovelace, Charles Babbage
Language:   en
Identifier: urn:uuid:11111111-2222-3333-4444-555555555555
Publisher:  Analytical Press
Date:       2026-06-21
```

## How it works

- **`container`** parses `META-INF/container.xml` to find the OPF path.
- **`opf`** parses the OPF: Dublin Core metadata, manifest items (with hrefs
  resolved to full archive paths), and the spine.
- **`text`** strips XHTML to readable plain text (drops `<script>`/`<style>`,
  decodes entities, turns block elements into line breaks).
- **`nav`** reads the table of contents from the EPUB 3 nav document or the
  EPUB 2 NCX.
- **`edit`** rewrites `dc:*` fields in place (replacing existing element text or
  inserting a new element before `</metadata>`), preserving the rest of the OPF.
- **`package`** reads raw zip entries and writes a fresh EPUB with the
  **mimetype entry first and stored**, everything else deflated.

## Tests

```sh
cargo test
```

Unit tests cover each parser; integration tests (`tests/cli.rs`) build a real
`.epub` fixture on disk and drive the compiled binary, including verifying that
`set-metadata` output keeps `mimetype` first and uncompressed.

## License

MIT — see [LICENSE](LICENSE).
