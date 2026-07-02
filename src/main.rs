//! `epub-tools` command-line interface.
//!
//! A thin layer over the `epub_tools` library: parse args, call the library, and
//! print results. All real logic lives in the library so it can be unit-tested
//! without spawning a process.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};

use epub_tools::{Epub, MetadataEdit};

/// Inspect, extract, and edit EPUB files.
#[derive(Parser)]
#[command(name = "epub-tools", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show Dublin Core metadata (title, authors, language, identifier, …).
    Info(FileArg),
    /// List the spine reading order and the manifest (href, media-type).
    List(FileArg),
    /// Extract plain text of the spine documents in reading order.
    Text(TextArgs),
    /// Rewrite OPF metadata and repackage a new EPUB.
    SetMetadata(SetMetadataArgs),
    /// List chapter titles from the nav (EPUB 3) or NCX (EPUB 2).
    Toc(FileArg),
    /// Extract the cover image to a file.
    Cover(CoverArgs),
    /// Write a generated sample EPUB to a path (handy for trying the tool).
    MakeSample(MakeSampleArgs),
}

#[derive(Args)]
struct FileArg {
    /// Path to the .epub file.
    book: PathBuf,
}

#[derive(Args)]
struct TextArgs {
    /// Path to the .epub file.
    book: PathBuf,
    /// Extract only this 1-based chapter (spine document) instead of the whole book.
    #[arg(long, value_name = "N")]
    chapter: Option<usize>,
}

#[derive(Args)]
struct SetMetadataArgs {
    /// Path to the input .epub file.
    book: PathBuf,
    /// New title (dc:title).
    #[arg(long)]
    title: Option<String>,
    /// New author (dc:creator).
    #[arg(long)]
    author: Option<String>,
    /// New language code (dc:language).
    #[arg(long)]
    language: Option<String>,
    /// New publisher (dc:publisher).
    #[arg(long)]
    publisher: Option<String>,
    /// Output path for the repackaged EPUB.
    #[arg(short = 'o', long = "output", value_name = "OUT.epub")]
    output: PathBuf,
}

#[derive(Args)]
struct CoverArgs {
    /// Path to the .epub file.
    book: PathBuf,
    /// Output path for the extracted cover image. Defaults to the cover's own
    /// filename inside the EPUB (e.g. `cover.png`) in the current directory.
    #[arg(short = 'o', long = "output", value_name = "OUT")]
    output: Option<PathBuf>,
}

#[derive(Args)]
struct MakeSampleArgs {
    /// Output path for the generated sample .epub.
    #[arg(short = 'o', long = "output", default_value = "sample.epub")]
    output: PathBuf,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Info(a) => cmd_info(&a.book),
        Command::List(a) => cmd_list(&a.book),
        Command::Text(a) => cmd_text(&a.book, a.chapter),
        Command::SetMetadata(a) => cmd_set_metadata(a),
        Command::Toc(a) => cmd_toc(&a.book),
        Command::Cover(a) => cmd_cover(a),
        Command::MakeSample(a) => cmd_make_sample(&a.output),
    }
}

fn cmd_info(book: &std::path::Path) -> Result<()> {
    let epub = Epub::open(book)?;
    let md = epub.metadata();
    println!("Title:      {}", md.title.as_deref().unwrap_or("(none)"));
    if md.creators.is_empty() {
        println!("Author(s):  (none)");
    } else {
        println!("Author(s):  {}", md.creators.join(", "));
    }
    println!(
        "Language:   {}",
        if md.languages.is_empty() {
            "(none)".to_string()
        } else {
            md.languages.join(", ")
        }
    );
    println!(
        "Identifier: {}",
        md.identifier.as_deref().unwrap_or("(none)")
    );
    println!(
        "Publisher:  {}",
        md.publisher.as_deref().unwrap_or("(none)")
    );
    println!("Date:       {}", md.date.as_deref().unwrap_or("(none)"));
    if let Some(desc) = &md.description {
        println!("Description: {desc}");
    }
    Ok(())
}

fn cmd_list(book: &std::path::Path) -> Result<()> {
    let epub = Epub::open(book)?;
    let pkg = epub.package();

    println!("Spine (reading order):");
    for (i, item) in epub.spine_items().iter().enumerate() {
        println!("  {:>3}. {} [{}]", i + 1, item.href, item.media_type);
    }

    println!("\nManifest ({} items):", pkg.manifest.len());
    for item in &pkg.manifest {
        let props = item
            .properties
            .as_deref()
            .map(|p| format!(" ({p})"))
            .unwrap_or_default();
        println!("  {:<28} {}{}", item.href, item.media_type, props);
    }
    Ok(())
}

fn cmd_text(book: &std::path::Path, chapter: Option<usize>) -> Result<()> {
    let epub = Epub::open(book)?;
    let text = match chapter {
        Some(n) => epub.chapter_text(n)?,
        None => epub.full_text(),
    };
    println!("{text}");
    Ok(())
}

fn cmd_set_metadata(a: SetMetadataArgs) -> Result<()> {
    let edit = MetadataEdit {
        title: a.title,
        author: a.author,
        language: a.language,
        publisher: a.publisher,
    };
    if edit.is_empty() {
        anyhow::bail!(
            "nothing to change: pass at least one of --title/--author/--language/--publisher"
        );
    }
    let epub = Epub::open(&a.book)?;
    epub.write_with_metadata(&edit, &a.output)
        .with_context(|| format!("writing {}", a.output.display()))?;
    println!("Wrote {}", a.output.display());
    Ok(())
}

fn cmd_toc(book: &std::path::Path) -> Result<()> {
    let epub = Epub::open(book)?;
    let toc = epub.toc();
    if toc.is_empty() {
        println!("(no table of contents found)");
        return Ok(());
    }
    for entry in toc {
        let indent = "  ".repeat(entry.depth);
        if entry.href.is_empty() {
            println!("{indent}{}", entry.label);
        } else {
            println!("{indent}{}  -> {}", entry.label, entry.href);
        }
    }
    Ok(())
}

fn cmd_cover(a: CoverArgs) -> Result<()> {
    let epub = Epub::open(&a.book)?;
    let (item, bytes) = epub
        .cover_image()
        .context("this EPUB does not declare a cover image")?;
    let out = a.output.unwrap_or_else(|| {
        let name = item.resolved_path.rsplit('/').next().unwrap_or("cover");
        PathBuf::from(name)
    });
    std::fs::write(&out, bytes).with_context(|| format!("writing cover image {}", out.display()))?;
    println!(
        "Wrote {} ({}, {} bytes)",
        out.display(),
        item.media_type,
        bytes.len()
    );
    Ok(())
}

fn cmd_make_sample(out: &std::path::Path) -> Result<()> {
    let bytes = epub_tools::fixture::build_default_epub_bytes()?;
    std::fs::write(out, bytes).with_context(|| format!("writing {}", out.display()))?;
    println!("Wrote sample EPUB to {}", out.display());
    Ok(())
}
