// SPDX-License-Identifier: MIT

//! Human-readable and JSON forms of an inspection. `docs/cli.md` documents
//! both; the JSON form is versioned by `schema_version`.

use crate::document::{
    Inspection, Structure, conversion_supported, format_name, unsupported_reason,
};
use crate::json::{write_literal, write_string};
use caj2pdf_core::{
    Bookmark, InputFormat,
    hnc8::{ImageRecord, OutlineReport, PageRecord, TextStructure},
};
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

/// Show wrapper bytes as ASCII. Quotes, backslashes and bytes outside
/// printable ASCII are written as `\xNN`.
fn signature_text(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len());
    for &byte in bytes {
        if matches!(byte, b' '..=b'~') && byte != b'"' && byte != b'\\' {
            text.push(char::from(byte));
        } else {
            text.push_str(&format!("\\x{byte:02x}"));
        }
    }
    text
}

fn pair_text(pair: Option<[u16; 2]>) -> String {
    pair.map_or_else(|| "unknown".to_owned(), |[a, b]| format!("{a} {b}"))
}

fn write_structure_text<W: Write>(out: &mut W, structure: &Structure) -> io::Result<()> {
    match structure {
        Structure::Kdh {
            signature,
            supported,
        } => writeln!(
            out,
            "KDH signature: \"{}\" ({})",
            signature_text(signature),
            if *supported {
                "supported"
            } else {
                "unsupported"
            }
        ),
        Structure::Hnc8 {
            header,
            page_row_bytes,
            application_info,
        } => {
            writeln!(
                out,
                "Page index: {}+{} ({page_row_bytes}-byte rows)",
                header.page_index.offset, header.page_index.length
            )?;
            let mode = header
                .native_mode
                .map_or_else(|| "unknown".to_owned(), |mode| mode.to_string());
            writeln!(out, "Native mode: {mode}")?;
            writeln!(out, "Native origin: {}", pair_text(header.native_origin))?;
            writeln!(out, "Page size: {}", pair_text(header.page_size))?;
            match application_info {
                None => writeln!(out, "Application info: none"),
                Some(tail) => match tail.length {
                    Some(length) => {
                        writeln!(out, "Application info: {length} bytes at {}", tail.offset)
                    }
                    None => writeln!(
                        out,
                        "Application info: declared at {}, outside the input",
                        tail.offset
                    ),
                },
            }
        }
    }
}

/// With `pages`, the document-level structure lines follow; [`Pages`] then
/// streams one line per page.
pub fn write_text<W: Write>(
    out: &mut W,
    info: &Inspection,
    list: bool,
    pages: bool,
) -> io::Result<()> {
    writeln!(out, "Format: {}", format_name(info.format))?;
    if let Some(variant) = info.variant {
        writeln!(out, "Variant: {variant}")?;
    }
    let supported = conversion_supported(info.format);
    writeln!(
        out,
        "Conversion: {}",
        if matches!(info.format, InputFormat::Hn | InputFormat::C8) {
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
    match &info.structure {
        Some(structure) if pages => write_structure_text(out, structure),
        _ => Ok(()),
    }
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

fn write_pair<W: Write>(out: &mut W, pair: Option<[u16; 2]>) -> io::Result<()> {
    match pair {
        Some([a, b]) => write!(out, "[{a},{b}]"),
        None => out.write_all(b"null"),
    }
}

fn write_structure_json<W: Write>(out: &mut W, structure: Option<&Structure>) -> io::Result<()> {
    match structure {
        None => out.write_all(b"null"),
        Some(Structure::Kdh {
            signature,
            supported,
        }) => {
            out.write_all(b"{\"kdh_signature\":")?;
            write_string(out, &signature_text(signature))?;
            write!(out, ",\"kdh_signature_supported\":{supported}}}")
        }
        Some(Structure::Hnc8 {
            header,
            page_row_bytes,
            application_info,
        }) => {
            write!(
                out,
                "{{\"page_index_offset\":{},\"page_index_length\":{},\
                 \"page_row_bytes\":{page_row_bytes},\"native_mode\":",
                header.page_index.offset, header.page_index.length
            )?;
            write_literal(out, header.native_mode)?;
            out.write_all(b",\"native_origin\":")?;
            write_pair(out, header.native_origin)?;
            out.write_all(b",\"page_size\":")?;
            write_pair(out, header.page_size)?;
            out.write_all(b",\"application_info\":")?;
            match application_info {
                Some(tail) => {
                    write!(out, "{{\"offset\":{},\"length\":", tail.offset)?;
                    write_literal(out, tail.length)?;
                    out.write_all(b"}}")
                }
                None => out.write_all(b"null}"),
            }
        }
    }
}

/// With `pages`, the `structure` field follows and the object stays open
/// for [`Pages`] to add `pages` and close it.
pub fn write_json<W: Write>(
    out: &mut W,
    info: &Inspection,
    list: bool,
    pages: bool,
) -> io::Result<()> {
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
    if pages {
        out.write_all(b",\"structure\":")?;
        write_structure_json(out, info.structure.as_ref())
    } else {
        out.write_all(b"}\n")
    }
}

fn write_optional<W: Write>(out: &mut W, value: Option<&str>) -> io::Result<()> {
    match value {
        Some(value) => write_string(out, value),
        None => out.write_all(b"null"),
    }
}

/// Streams one line (text) or one object (JSON) per page, so retained state
/// does not grow with the page or image count.
pub struct Pages<'w, W: Write> {
    out: &'w mut W,
    json: bool,
    /// Whether the current page has a row, and how many of its images were written.
    row: bool,
    images: u32,
    pages: u32,
}

impl<'w, W: Write> Pages<'w, W> {
    pub fn new(out: &'w mut W, json: bool) -> Self {
        Self {
            out,
            json,
            row: false,
            images: 0,
            pages: 0,
        }
    }

    /// Report that `format` has no per-page structure and end the report.
    pub fn unavailable(&mut self, format: InputFormat) -> io::Result<()> {
        if self.json {
            self.out.write_all(b",\"pages\":null}\n")
        } else {
            writeln!(
                self.out,
                "Page structure: not available for {} input",
                format_name(format)
            )
        }
    }

    pub fn begin(&mut self) -> io::Result<()> {
        if self.json {
            self.out.write_all(b",\"pages\":[")?;
        }
        Ok(())
    }

    /// Start a page; `row` is absent when its page-index row was rejected.
    pub fn page(&mut self, number: u32, row: Option<&PageRecord>) -> io::Result<()> {
        self.row = row.is_some();
        self.images = 0;
        if self.json {
            if self.pages != 0 {
                self.out.write_all(b",")?;
            }
            write!(self.out, "{{\"page\":{number},\"text_offset\":")?;
            write_literal(self.out, row.map(|row| row.text.offset))?;
            self.out.write_all(b",\"text_length\":")?;
            write_literal(self.out, row.map(|row| row.text.length))?;
            self.out.write_all(b",\"image_count\":")?;
            write_literal(self.out, row.map(|row| row.image_count))?;
            self.out.write_all(b",\"images\":[")?;
        } else {
            write!(self.out, "Page {number}:")?;
            if let Some(row) = row {
                write!(
                    self.out,
                    " text {}+{}, images [",
                    row.text.offset, row.text.length
                )?;
            }
        }
        self.pages += 1;
        Ok(())
    }

    /// One image descriptor: its type and payload span, never its bytes.
    pub fn image(&mut self, image: &ImageRecord) -> io::Result<()> {
        let first = self.images == 0;
        self.images += 1;
        let (kind, offset, length) = (
            image.record_type,
            image.payload.offset,
            image.payload.length,
        );
        if self.json {
            let separator = if first { "" } else { "," };
            write!(
                self.out,
                "{separator}{{\"type\":{kind},\"offset\":{offset},\"length\":{length}}}"
            )
        } else {
            let separator = if first { "" } else { ", " };
            write!(self.out, "{separator}type {kind} at {offset}+{length}")
        }
    }

    /// Finish a page with its text framing, or the error that stopped it.
    pub fn end_page(
        &mut self,
        text: Option<&TextStructure>,
        text_error: Option<&str>,
        error: Option<&str>,
    ) -> io::Result<()> {
        if self.json {
            self.out.write_all(b"],\"text_framing\":")?;
            write_optional(self.out, text.map(|text| text.framing.as_str()))?;
            self.out.write_all(b",\"text_records\":")?;
            write_literal(self.out, text.map(|text| text.records))?;
            self.out.write_all(b",\"text_decoded_length\":")?;
            write_literal(self.out, text.and_then(|text| text.decoded_length))?;
            self.out.write_all(b",\"text_error\":")?;
            write_optional(self.out, text_error)?;
            self.out.write_all(b",\"error\":")?;
            write_optional(self.out, error)?;
            return self.out.write_all(b"}");
        }
        if self.row {
            self.out.write_all(b"]")?;
        }
        if let Some(text) = text {
            write!(
                self.out,
                ", framing {} ({} records",
                text.framing.as_str(),
                text.records
            )?;
            if let Some(length) = text.decoded_length {
                write!(self.out, ", {length} decoded bytes")?;
            }
            self.out.write_all(b")")?;
        }
        if let Some(message) = text_error {
            write!(self.out, ", text error: {message}")?;
        }
        if let Some(message) = error {
            let separator = if self.row { "," } else { "" };
            write!(self.out, "{separator} error: {message}")?;
        }
        writeln!(self.out)
    }

    pub fn finish(&mut self) -> io::Result<()> {
        if self.json {
            self.out.write_all(b"]}\n")?;
        }
        Ok(())
    }
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
