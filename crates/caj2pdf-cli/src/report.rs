// SPDX-License-Identifier: MIT

//! Human-readable and JSON forms of an inspection. `docs/cli.md` documents
//! both; the JSON form is versioned by `schema_version`.

use crate::document::unsupported_reason;
use caj2pdf_core::{
    Bookmark, DocumentInfo, InputFormat, Structure,
    hnc8::{ApplicationInfoStatus, ImageRecord, OutlineReport, PageRecord, TextStructure},
};
use serde::Serialize;
use serde_json::ser::{CharEscape, CompactFormatter, Formatter};
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
    info: &DocumentInfo,
    list: bool,
    pages: bool,
) -> io::Result<()> {
    writeln!(out, "Format: {}", info.format.name())?;
    if let Some(variant) = info.variant {
        writeln!(out, "Variant: {}", variant.as_str())?;
    }
    let supported = info.format.is_convertible();
    writeln!(
        out,
        "Conversion: {}",
        if matches!(info.format, InputFormat::Hn | InputFormat::C8) {
            "experimental"
        } else if supported {
            "supported"
        } else if info.format == InputFormat::Teb {
            "not supported (DRM-encrypted container)"
        } else if info.format == InputFormat::Caa {
            "not supported (target descriptor; obtain the referenced document)"
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
            info.format.name()
        )?,
        None => {}
    }
    if let Some(package) = &info.application_info.info {
        if let Some(doi) = &package.doi {
            writeln!(out, "DOI: {}", printable(doi))?;
        }
        if let Some(url) = &package.url {
            writeln!(out, "URL: {}", printable(url))?;
        }
        writeln!(out, "Notes: {}", package.note_count)?;
    }
    match &info.structure {
        Some(structure) if pages => write_structure_text(out, structure),
        _ => Ok(()),
    }
}

/// serde_json's compact layout, except that backspace and form feed are
/// written as `\u0008` and `\u000c` like the other control characters, as
/// schema version 1 always has. `"`, `\`, `\n`, `\r` and `\t` use their
/// short escapes; all other characters are written as UTF-8.
struct ReportFormatter;

impl Formatter for ReportFormatter {
    fn write_char_escape<W: ?Sized + Write>(
        &mut self,
        writer: &mut W,
        escape: CharEscape,
    ) -> io::Result<()> {
        let escape = match escape {
            CharEscape::Backspace => CharEscape::AsciiControl(0x08),
            CharEscape::FormFeed => CharEscape::AsciiControl(0x0c),
            escape => escape,
        };
        CompactFormatter.write_char_escape(writer, escape)
    }
}

/// A JSON document written member by member. serde_json serializes every
/// key and value and the formatter writes every separator; this only
/// tracks whether the innermost object or array is still empty. The report
/// is streamed rather than built as one value so that pages and image
/// descriptors are written as they are read, and so that the outline is
/// nested from its flat, depth-ordered list without recursion.
struct Json<'w, W: Write> {
    out: &'w mut W,
    first: bool,
}

impl<'w, W: Write> Json<'w, W> {
    /// `open` continues an object that already has members.
    fn new(out: &'w mut W, open: bool) -> Self {
        Self { out, first: !open }
    }

    fn value<T: Serialize + ?Sized>(&mut self, value: &T) -> io::Result<()> {
        let mut serializer =
            serde_json::Serializer::with_formatter(&mut *self.out, ReportFormatter);
        value.serialize(&mut serializer).map_err(io::Error::from)
    }

    fn key(&mut self, key: &str) -> io::Result<()> {
        ReportFormatter.begin_object_key(self.out, self.first)?;
        self.first = false;
        self.value(key)?;
        ReportFormatter.begin_object_value(self.out)
    }

    fn field<T: Serialize + ?Sized>(&mut self, key: &str, value: &T) -> io::Result<()> {
        self.key(key)?;
        self.value(value)
    }

    /// Start the next element of the innermost array.
    fn element(&mut self) -> io::Result<()> {
        ReportFormatter.begin_array_value(self.out, self.first)?;
        self.first = false;
        Ok(())
    }

    fn begin_object(&mut self) -> io::Result<()> {
        self.first = true;
        ReportFormatter.begin_object(self.out)
    }

    /// Closing a container completes a member of its parent, which is
    /// therefore no longer empty.
    fn end_object(&mut self) -> io::Result<()> {
        self.first = false;
        ReportFormatter.end_object(self.out)
    }

    fn begin_array(&mut self) -> io::Result<()> {
        self.first = true;
        ReportFormatter.begin_array(self.out)
    }

    fn end_array(&mut self) -> io::Result<()> {
        self.first = false;
        ReportFormatter.end_array(self.out)
    }

    /// Close the top-level object and end the line.
    fn finish(&mut self) -> io::Result<()> {
        self.end_object()?;
        self.out.write_all(b"\n")
    }
}

/// The C8 application-info package, as decoded.
#[derive(Serialize)]
struct ApplicationInfoJson<'a> {
    doi: Option<&'a str>,
    url: Option<&'a str>,
    note_count: u32,
}

/// Where the `APPINFOSIGN` trailer declares the package.
#[derive(Serialize)]
struct TailJson {
    offset: u64,
    length: Option<u64>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum StructureJson {
    Kdh {
        kdh_signature: String,
        kdh_signature_supported: bool,
    },
    Hnc8 {
        page_index_offset: u64,
        page_index_length: u64,
        page_row_bytes: u64,
        native_mode: Option<u32>,
        native_origin: Option<[u16; 2]>,
        page_size: Option<[u16; 2]>,
        application_info: Option<TailJson>,
    },
}

impl From<&Structure> for StructureJson {
    fn from(structure: &Structure) -> Self {
        match structure {
            Structure::Kdh {
                signature,
                supported,
            } => Self::Kdh {
                kdh_signature: signature_text(signature),
                kdh_signature_supported: *supported,
            },
            Structure::Hnc8 {
                header,
                page_row_bytes,
                application_info,
            } => Self::Hnc8 {
                page_index_offset: header.page_index.offset,
                page_index_length: header.page_index.length,
                page_row_bytes: *page_row_bytes,
                native_mode: header.native_mode,
                native_origin: header.native_origin,
                page_size: header.page_size,
                application_info: application_info.map(|tail| TailJson {
                    offset: tail.offset,
                    length: tail.length,
                }),
            },
        }
    }
}

/// An image descriptor: its type and payload span, never its bytes.
#[derive(Serialize)]
struct ImageJson {
    #[serde(rename = "type")]
    record_type: u32,
    offset: u64,
    length: u64,
}

/// Write a depth-ordered outline as nested `children` arrays. A bookmark
/// deeper than its predecessor allows becomes that predecessor's child.
fn write_tree<W: Write>(json: &mut Json<'_, W>, bookmarks: &[Bookmark]) -> io::Result<()> {
    json.begin_array()?;
    let mut open = 0u32;
    for bookmark in bookmarks {
        let depth = bookmark.depth.min(open);
        for _ in depth..open {
            json.end_array()?;
            json.end_object()?;
        }
        json.element()?;
        json.begin_object()?;
        json.field("title", &bookmark.title)?;
        json.field("page", &(u64::from(bookmark.page_index) + 1))?;
        json.key("children")?;
        json.begin_array()?;
        open = depth + 1;
    }
    for _ in 0..open {
        json.end_array()?;
        json.end_object()?;
    }
    json.end_array()
}

/// With `pages`, the `structure` field follows and the object stays open
/// for [`Pages`] to add `pages` and close it.
pub fn write_json<W: Write>(
    out: &mut W,
    info: &DocumentInfo,
    list: bool,
    pages: bool,
) -> io::Result<()> {
    let mut json = Json::new(out, false);
    let bookmarks = info.bookmarks.as_deref();
    json.begin_object()?;
    json.field("schema_version", &SCHEMA_VERSION)?;
    json.field("format", info.format.name())?;
    json.field(
        "variant",
        &info.variant.as_ref().map(|variant| variant.as_str()),
    )?;
    json.field("conversion_supported", &info.format.is_convertible())?;
    json.field("page_count", &info.page_count)?;
    json.field("has_outline", &info.has_outline)?;
    json.field("bookmark_count", &bookmarks.map(<[Bookmark]>::len))?;
    if list {
        json.key("bookmarks")?;
        match bookmarks {
            Some(bookmarks) => write_tree(&mut json, bookmarks)?,
            None => json.value(&None::<()>)?,
        }
    }
    json.field("outline_warnings", &bookmarks.map(|_| info.outline.defects))?;
    if let Some(reason) = unsupported_reason(info.format) {
        json.field("unsupported_reason", reason)?;
    }
    if let Some(package) = &info.application_info.info {
        let package = ApplicationInfoJson {
            doi: package.doi.as_deref(),
            url: package.url.as_deref(),
            note_count: package.note_count,
        };
        json.field("application_info", &package)?;
    }
    if pages {
        json.field(
            "structure",
            &info.structure.as_ref().map(StructureJson::from),
        )
    } else {
        json.finish()
    }
}

/// Streams one line (text) or one object (JSON) per page, so retained state
/// does not grow with the page or image count.
pub struct Pages<'w, W: Write> {
    out: Json<'w, W>,
    json: bool,
    /// Whether the current page has a row, and how many of its images were written.
    row: bool,
    images: u32,
}

impl<'w, W: Write> Pages<'w, W> {
    /// For JSON, `out` continues the open object of [`write_json`].
    pub fn new(out: &'w mut W, json: bool) -> Self {
        Self {
            out: Json::new(out, true),
            json,
            row: false,
            images: 0,
        }
    }

    /// Report that `format` has no per-page structure and end the report.
    pub fn unavailable(&mut self, format: InputFormat) -> io::Result<()> {
        if self.json {
            self.out.field("pages", &None::<()>)?;
            self.out.finish()
        } else {
            writeln!(
                self.out.out,
                "Page structure: not available for {} input",
                format.name()
            )
        }
    }

    pub fn begin(&mut self) -> io::Result<()> {
        if self.json {
            self.out.key("pages")?;
            self.out.begin_array()?;
        }
        Ok(())
    }

    /// Start a page; `row` is absent when its page-index row was rejected.
    pub fn page(&mut self, number: u32, row: Option<&PageRecord>) -> io::Result<()> {
        self.row = row.is_some();
        self.images = 0;
        if self.json {
            let json = &mut self.out;
            json.element()?;
            json.begin_object()?;
            json.field("page", &number)?;
            json.field("text_offset", &row.map(|row| row.text.offset))?;
            json.field("text_length", &row.map(|row| row.text.length))?;
            json.field("image_count", &row.map(|row| row.image_count))?;
            json.key("images")?;
            json.begin_array()
        } else {
            let out = &mut self.out.out;
            write!(out, "Page {number}:")?;
            if let Some(row) = row {
                write!(
                    out,
                    " text {}+{}, images [",
                    row.text.offset, row.text.length
                )?;
            }
            Ok(())
        }
    }

    /// One image descriptor: its type and payload span, never its bytes.
    pub fn image(&mut self, image: &ImageRecord) -> io::Result<()> {
        let first = self.images == 0;
        self.images += 1;
        let image = ImageJson {
            record_type: image.record_type,
            offset: image.payload.offset,
            length: image.payload.length,
        };
        if self.json {
            self.out.element()?;
            self.out.value(&image)
        } else {
            let separator = if first { "" } else { ", " };
            write!(
                self.out.out,
                "{separator}type {} at {}+{}",
                image.record_type, image.offset, image.length
            )
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
            let json = &mut self.out;
            json.end_array()?;
            json.field("text_framing", &text.map(|text| text.framing.as_str()))?;
            json.field("text_records", &text.map(|text| text.records))?;
            json.field(
                "text_decoded_length",
                &text.and_then(|text| text.decoded_length),
            )?;
            json.field("text_error", &text_error)?;
            json.field("error", &error)?;
            return json.end_object();
        }
        let out = &mut self.out.out;
        if self.row {
            out.write_all(b"]")?;
        }
        if let Some(text) = text {
            write!(
                out,
                ", framing {} ({} records",
                text.framing.as_str(),
                text.records
            )?;
            if let Some(length) = text.decoded_length {
                write!(out, ", {length} decoded bytes")?;
            }
            out.write_all(b")")?;
        }
        if let Some(message) = text_error {
            write!(out, ", text error: {message}")?;
        }
        if let Some(message) = error {
            let separator = if self.row { "," } else { "" };
            write!(out, "{separator} error: {message}")?;
        }
        writeln!(out)
    }

    pub fn finish(&mut self) -> io::Result<()> {
        if self.json {
            self.out.end_array()?;
            self.out.finish()?;
        }
        Ok(())
    }
}

/// Write one diagnostic line when a C8 application-info package was ignored.
pub fn write_application_info_warning<W: Write>(
    out: &mut W,
    status: ApplicationInfoStatus,
) -> io::Result<()> {
    if let ApplicationInfoStatus::Ignored(defect) = status {
        writeln!(out, "caj2pdf: warning: {defect}")?;
    }
    Ok(())
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

/// Report visible substitutes without assigning semantics to private-use text.
pub fn write_glyph_warning<W: Write>(out: &mut W, count: u64) -> io::Result<()> {
    if count != 0 {
        writeln!(
            out,
            "caj2pdf: warning: {count} private-use glyph(s) rendered with a visual substitute; original private-use codes retained in PDF ActualText"
        )?;
    }
    Ok(())
}
