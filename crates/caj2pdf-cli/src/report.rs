// SPDX-License-Identifier: MIT

//! Human-readable and JSON forms of an inspection. `docs/cli.md` documents
//! both; the JSON form is versioned by `schema_version`.

use crate::document::{Inspection, conversion_supported, format_name, unsupported_reason};
use crate::json::{write_literal, write_string};
use caj2pdf_core::{Bookmark, hnc8::OutlineReport};
use std::io::{self, Write};

pub const SCHEMA_VERSION: u32 = 1;

fn yes_no(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "yes",
        Some(false) => "no",
        None => "unknown",
    }
}

/// Replace control characters so that a title cannot drive the terminal.
fn printable(title: &str) -> String {
    let mut text = String::with_capacity(title.len());
    for character in title.chars() {
        if character.is_control() {
            text.push_str(&format!("\\u{{{:x}}}", u32::from(character)));
        } else {
            text.push(character);
        }
    }
    text
}

pub fn write_text<W: Write>(out: &mut W, info: &Inspection, list: bool) -> io::Result<()> {
    writeln!(out, "Format: {}", format_name(info.format))?;
    if let Some(variant) = info.variant {
        writeln!(out, "Variant: {variant}")?;
    }
    let supported = conversion_supported(info.format);
    writeln!(
        out,
        "Conversion: {}",
        if matches!(
            info.format,
            caj2pdf_core::InputFormat::Hn | caj2pdf_core::InputFormat::C8
        ) {
            "experimental (caller codec states may be required)"
        } else if supported {
            "supported"
        } else if unsupported_reason(info.format).is_some() {
            "not supported (DRM-encrypted container)"
        } else {
            "not supported"
        }
    )?;
    match info.page_count {
        Some(pages) => writeln!(out, "Pages: {pages}")?,
        None => writeln!(out, "Pages: unknown")?,
    }
    writeln!(out, "Outline: {}", yes_no(info.has_outline))?;
    match &info.bookmarks {
        Some(bookmarks) => {
            writeln!(out, "Bookmarks: {}", bookmarks.len())?;
            if info.outline.defects != 0 {
                writeln!(out, "Outline warnings: {}", info.outline.defects)?;
            }
            if list {
                for bookmark in bookmarks {
                    writeln!(
                        out,
                        "{:indent$}- {} (page {})",
                        "",
                        printable(&bookmark.title),
                        u64::from(bookmark.page_index) + 1,
                        indent = 2 + 2 * bookmark.depth as usize
                    )?;
                }
            }
        }
        None if list => writeln!(
            out,
            "Bookmarks: listing is not available for {} input",
            format_name(info.format)
        )?,
        None => {}
    }
    Ok(())
}

/// Write a depth-ordered outline as nested `children` arrays.
fn write_tree<W: Write>(out: &mut W, bookmarks: &[Bookmark]) -> io::Result<()> {
    out.write_all(b"[")?;
    let mut open = 0u32;
    let mut comma = false;
    for bookmark in bookmarks {
        let depth = bookmark.depth.min(open);
        while open > depth {
            out.write_all(b"]}")?;
            open -= 1;
            comma = true;
        }
        if comma {
            out.write_all(b",")?;
        }
        out.write_all(b"{\"title\":")?;
        write_string(out, &bookmark.title)?;
        write!(
            out,
            ",\"page\":{},\"children\":[",
            u64::from(bookmark.page_index) + 1
        )?;
        open += 1;
        comma = false;
    }
    for _ in 0..open {
        out.write_all(b"]}")?;
    }
    out.write_all(b"]")
}

pub fn write_json<W: Write>(out: &mut W, info: &Inspection, list: bool) -> io::Result<()> {
    write!(out, "{{\"schema_version\":{SCHEMA_VERSION},\"format\":")?;
    write_string(out, format_name(info.format))?;
    out.write_all(b",\"variant\":")?;
    match info.variant {
        Some(variant) => write_string(out, variant)?,
        None => out.write_all(b"null")?,
    }
    write!(
        out,
        ",\"conversion_supported\":{},\"page_count\":",
        conversion_supported(info.format)
    )?;
    write_literal(out, info.page_count)?;
    out.write_all(b",\"has_outline\":")?;
    write_literal(out, info.has_outline)?;
    out.write_all(b",\"bookmark_count\":")?;
    let bookmarks = info.bookmarks.as_deref();
    write_literal(out, bookmarks.map(<[Bookmark]>::len))?;
    if list {
        out.write_all(b",\"bookmarks\":")?;
        match bookmarks {
            Some(bookmarks) => write_tree(out, bookmarks)?,
            None => out.write_all(b"null")?,
        }
    }
    out.write_all(b",\"outline_warnings\":")?;
    write_literal(out, bookmarks.map(|_| info.outline.defects))?;
    if let Some(reason) = unsupported_reason(info.format) {
        out.write_all(b",\"unsupported_reason\":")?;
        write_string(out, reason)?;
    }
    out.write_all(b"}\n")
}

/// Write one diagnostic line per recorded HN-A outline defect, then a count
/// of defects whose locations were not retained.
pub fn write_warnings<W: Write>(out: &mut W, outline: &OutlineReport) -> io::Result<()> {
    if outline.unverified {
        writeln!(
            out,
            "caj2pdf: warning: C8/HN-B bookmarks are not verified; wrote no outline \
             (--no-bookmarks silences this)"
        )?;
    }
    let recorded = outline.recorded_defects();
    for defect in recorded {
        writeln!(out, "caj2pdf: warning: {defect}")?;
    }
    let omitted = outline.defects as usize - recorded.len();
    if omitted != 0 {
        writeln!(
            out,
            "caj2pdf: warning: {omitted} more HN-A bookmark defects were not listed"
        )?;
    }
    Ok(())
}
