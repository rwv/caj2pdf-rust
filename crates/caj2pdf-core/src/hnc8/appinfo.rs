// SPDX-License-Identifier: MIT

//! The independently observed C8 application-info package at the end of the
//! source; see docs/research/c8-native-records.md. Only the DOI, DURL and the number of
//! `NoteItems` entries are extracted, by a small scanner rather than a general
//! XML parser. A defect here is located but must not fail page conversion.

use super::inflate::{ExactInflate, InflateFault, InflateFaultKind};
use super::{ErrorKind, Hnc8Error, Hnc8Reader, Location, Result, Variant, read_fixed};
use crate::fallible::{len_u64, reserve_exact};
use crate::hnc8::TEXT_DECODER_RESERVATION_BYTES;
use crate::{Cancellation, RangedSource};
use std::fmt;

/// Ceiling for both the compressed and the inflated package.
pub const MAX_APPLICATION_INFO_BYTES: u32 = 1024 * 1024;
/// Ceiling for one decoded DOI or URL value.
pub const MAX_APPLICATION_INFO_FIELD_BYTES: usize = 4096;

const MARKER: &[u8] = b"APPINFOSIGN ";
const MARKER_NAME: &[u8] = b"APPINFOSIGN";
/// The marker plus at most 20 decimal digits of a 64-bit offset.
const TRAILER_WINDOW: u64 = 32;
const HEADER_BYTES: u64 = 8;
const CHUNK_BYTES: usize = 64 * 1024;
const MAX_DEPTH: usize = 16;
const XML: &str = "application-info XML";
const BOM: &str = "\u{feff}";

/// Values read from a C8 application-info package. Text is trimmed of XML
/// white space and entity-decoded; an empty value is `None`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ApplicationInfo {
    /// `Package/FileProperty-Package/DOI`. The observed values are CNKI
    /// identifiers; they are not verified as registered DOIs.
    pub doi: Option<String>,
    /// `Package/FileProperty-Package/DURL`. Never followed.
    pub url: Option<String>,
    /// `Item` children of every `Package/Note-Package/NoteItems`.
    pub note_count: u32,
}

/// Why a present application-info package was ignored.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApplicationInfoDefect {
    /// Absolute byte offset in the source.
    pub offset: u64,
    pub field: &'static str,
    pub reason: &'static str,
}

impl From<&Hnc8Error> for ApplicationInfoDefect {
    fn from(error: &Hnc8Error) -> Self {
        Self {
            offset: error.offset,
            field: error.kind.field(),
            reason: match error.kind {
                ErrorKind::Malformed { reason, .. } => reason,
                ref other => other.as_str(),
            },
        }
    }
}

impl fmt::Display for ApplicationInfoDefect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ignored C8 application-info package at byte {}: {}: {}",
            self.offset, self.field, self.reason
        )
    }
}

/// What conversion did with a C8 application-info package.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ApplicationInfoStatus {
    /// No package, or not a C8 source.
    #[default]
    Absent,
    /// The package was read; its DOI and URL went to the PDF `/Info`.
    Read,
    /// The package was defective and ignored; pages are unaffected.
    Ignored(ApplicationInfoDefect),
}

/// A lenient read: the package when it was read, and what happened to it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ApplicationInfoReport {
    pub info: Option<ApplicationInfo>,
    pub status: ApplicationInfoStatus,
}

impl<S: RangedSource, C: Cancellation> Hnc8Reader<'_, S, C> {
    /// Like [`Self::application_info`], but a defect becomes
    /// [`ApplicationInfoStatus::Ignored`]. Only cancellation is an error.
    pub async fn application_info_report(&mut self) -> Result<ApplicationInfoReport> {
        Ok(match self.application_info().await {
            Ok(Some(info)) => ApplicationInfoReport {
                info: Some(info),
                status: ApplicationInfoStatus::Read,
            },
            Ok(None) => ApplicationInfoReport::default(),
            Err(error) if matches!(error.kind, ErrorKind::Cancelled) => return Err(error),
            Err(error) => ApplicationInfoReport {
                info: None,
                status: ApplicationInfoStatus::Ignored(ApplicationInfoDefect::from(&error)),
            },
        })
    }

    /// Read the trailing application-info package of a C8 source.
    ///
    /// Returns `None` for other variants and when the file does not end with
    /// the `APPINFOSIGN` trailer. Declared lengths must exactly frame one zlib
    /// stream ending at the trailer. At most [`MAX_APPLICATION_INFO_BYTES`]
    /// are inflated, within `Limits::max_allocation_bytes`. Callers should
    /// treat every error except cancellation as a warning. Independent of the
    /// page cursor.
    pub async fn application_info(&mut self) -> Result<Option<ApplicationInfo>> {
        if self.header.variant != Variant::C8 {
            return Ok(None);
        }
        let loc = Location {
            variant: Some(Variant::C8),
            offset: 0,
            page: None,
            image: None,
        };
        if self.cancellation.is_cancelled() {
            return Err(loc.error(ErrorKind::Cancelled));
        }
        let Some(package) = self.find_package(loc).await? else {
            return Ok(None);
        };
        let xml = self.inflate(package, loc.at(package.offset)).await?;
        scan(&xml, loc.at(package.offset)).map(Some)
    }

    /// Locate a final `APPINFOSIGN <decimal start>` trailer in the last 32
    /// bytes of the source, for any variant. `None` when that window holds no
    /// marker name; a marker that is not followed by a final decimal offset
    /// fitting 64 bits is malformed. Shared by the package reader and the
    /// structure report. Independent of the page cursor.
    pub(super) async fn application_info_trailer(&mut self) -> Result<Option<Trailer>> {
        let loc = Location {
            variant: Some(self.header.variant),
            offset: 0,
            page: None,
            image: None,
        };
        let size = self.source.size();
        let window_start = size.saturating_sub(TRAILER_WINDOW);
        let mut window = [0; TRAILER_WINDOW as usize];
        let window = &mut window[..(size - window_start) as usize];
        let field = "application-info trailer";
        read_fixed(
            self.source,
            self.limits,
            self.cancellation,
            window_start,
            window,
            loc.at(window_start),
            field,
        )
        .await?;
        let digits = window
            .iter()
            .rev()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        let marker_at = window.len().checked_sub(digits + MARKER.len());
        if digits == 0 || marker_at.is_none_or(|at| &window[at..at + MARKER.len()] != MARKER) {
            return match window
                .windows(MARKER_NAME.len())
                .position(|part| part == MARKER_NAME)
            {
                Some(at) => Err(loc
                    .at(window_start + len_u64(at))
                    .malformed(field, "marker is not followed by a final decimal offset")),
                None => Ok(None),
            };
        }
        let marker = window_start + len_u64(marker_at.expect("checked marker position"));
        let start = std::str::from_utf8(&window[window.len() - digits..])
            .expect("ASCII digits")
            .parse::<u64>()
            .map_err(|_| loc.at(marker).malformed(field, "offset overflows 64 bits"))?;
        Ok(Some(Trailer { marker, start }))
    }

    /// Locate the trailer and return the zlib stream it frames.
    async fn find_package(&mut self, loc: Location) -> Result<Option<Package>> {
        let Some(Trailer { marker, start }) = self.application_info_trailer().await? else {
            return Ok(None);
        };
        let lengths = "application-info lengths";
        if start < self.header.page_index.offset + self.header.page_index.length {
            return Err(loc
                .at(marker)
                .malformed(lengths, "package starts inside the header or page index"));
        }
        if start > marker || marker - start < HEADER_BYTES {
            return Err(loc.at(marker).error(ErrorKind::Truncated {
                field: lengths,
                expected: HEADER_BYTES,
                available: marker.saturating_sub(start),
            }));
        }
        let mut header = [0; HEADER_BYTES as usize];
        read_fixed(
            self.source,
            self.limits,
            self.cancellation,
            start,
            &mut header,
            loc.at(start),
            lengths,
        )
        .await?;
        let decoded = u32::from_le_bytes(header[..4].try_into().expect("fixed field width"));
        let compressed = u32::from_le_bytes(header[4..].try_into().expect("fixed field width"));
        for (resource, value) in [
            ("application-info decoded bytes", decoded),
            ("application-info compressed bytes", compressed),
        ] {
            if value > MAX_APPLICATION_INFO_BYTES {
                return Err(loc.at(start).limit(
                    resource,
                    u64::from(MAX_APPLICATION_INFO_BYTES),
                    u64::from(value),
                ));
            }
        }
        if decoded == 0 {
            return Err(loc.at(start).malformed(lengths, "declared empty package"));
        }
        let offset = start + HEADER_BYTES;
        if offset + u64::from(compressed) != marker {
            return Err(loc
                .at(start)
                .malformed(lengths, "compressed length does not end at the marker"));
        }
        Ok(Some(Package {
            offset,
            compressed: u64::from(compressed),
            decoded: u64::from(decoded),
        }))
    }

    /// Inflate exactly the declared stream into exactly the declared length.
    async fn inflate(&mut self, package: Package, loc: Location) -> Result<Vec<u8>> {
        let Package {
            compressed,
            decoded,
            ..
        } = package;
        let field = "application-info zlib stream";
        let chunk = len_u64(self.limits.io_chunk_bytes.min(CHUNK_BYTES)).min(compressed);
        // One sentinel byte detects output beyond the declared length.
        let planned = (decoded + 1) + chunk + TEXT_DECODER_RESERVATION_BYTES;
        let resource = "application-info allocation bytes";
        if planned > self.limits.max_allocation_bytes {
            return Err(loc.limit(resource, self.limits.max_allocation_bytes, planned));
        }
        let refused = || loc.limit(resource, self.limits.max_allocation_bytes, planned);
        let mut input = Vec::new();
        reserve_exact(&mut input, chunk as usize, refused())?;
        input.resize(chunk as usize, 0);
        let mut output = Vec::new();
        reserve_exact(&mut output, decoded as usize + 1, refused())?;
        output.resize(decoded as usize + 1, 0);
        let mut inflate = ExactInflate::new(package.offset, compressed, decoded);
        let fault = |fault: InflateFault| {
            let reason = match fault.kind {
                InflateFaultKind::Invalid => "invalid stream or checksum",
                InflateFaultKind::Excess => "output exceeds the declared length",
                InflateFaultKind::EndMismatch => "stream ends before the declared lengths",
                InflateFaultKind::Stalled => "truncated stream",
            };
            loc.at(fault.offset).malformed(field, reason)
        };
        loop {
            if self.cancellation.is_cancelled() {
                return Err(loc.at(inflate.position()).error(ErrorKind::Cancelled));
            }
            if let Some((at, length)) = inflate.next_read(input.len()) {
                read_fixed(
                    self.source,
                    self.limits,
                    self.cancellation,
                    at,
                    &mut input[..length],
                    loc.at(at),
                    field,
                )
                .await?;
            }
            let window = &mut output[inflate.total_out() as usize..];
            let step = inflate.step(&input, window).map_err(fault)?;
            inflate.check_length(&step).map_err(fault)?;
            if inflate.finished(&step).map_err(fault)? {
                output.truncate(decoded as usize);
                return Ok(output);
            }
        }
    }
}

/// A final `APPINFOSIGN <start>` trailer: the marker's offset and the
/// declared package start, which is not checked against the source.
#[derive(Clone, Copy)]
pub(super) struct Trailer {
    pub(super) marker: u64,
    pub(super) start: u64,
}

/// A checked zlib stream that ends exactly at the trailer marker.
#[derive(Clone, Copy)]
struct Package {
    offset: u64,
    compressed: u64,
    decoded: u64,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Field {
    Doi,
    Url,
}

fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
}

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':') || byte >= 0x80
}

/// Extract the selected fields from a complete UTF-8 package. Element nesting
/// is checked; attributes are skipped after a quoting check; declarations,
/// unknown entities and text outside the root are rejected.
fn scan(bytes: &[u8], loc: Location) -> Result<ApplicationInfo> {
    let bad = |reason| loc.malformed(XML, reason);
    let xml = std::str::from_utf8(bytes).map_err(|_| bad("package is not UTF-8"))?;
    let body = xml.strip_prefix(BOM).unwrap_or(xml);
    let mut info = ApplicationInfo::default();
    let mut stack = [""; MAX_DEPTH];
    let mut depth = 0;
    let mut root_closed = false;
    let mut seen = [false; 2];
    let mut capture: Option<(Field, String)> = None;
    let mut rest = body;
    while !rest.is_empty() {
        if !rest.starts_with('<') {
            let end = rest.find('<').unwrap_or(rest.len());
            let text = &rest[..end];
            if let Some((_, value)) = &mut capture {
                decode_text(text, value, loc)?;
            } else if depth == 0 && !text.bytes().all(is_space) {
                return Err(bad("text outside the root element"));
            }
            rest = &rest[end..];
        } else if let Some(tail) = rest.strip_prefix("<?") {
            let end = tail
                .find("?>")
                .ok_or(bad("unterminated processing instruction"))?;
            if rest.len() == body.len() && tail.starts_with("xml") {
                check_encoding(&tail[3..end]).ok_or(bad("declared encoding is not UTF-8"))?;
            }
            rest = &tail[end + 2..];
        } else if let Some(tail) = rest.strip_prefix("<!--") {
            let end = tail.find("-->").ok_or(bad("unterminated comment"))?;
            rest = &tail[end + 3..];
        } else if let Some(tail) = rest.strip_prefix("<![CDATA[") {
            let end = tail.find("]]>").ok_or(bad("unterminated character data"))?;
            if depth == 0 {
                return Err(bad("character data outside the root element"));
            }
            if let Some((_, value)) = &mut capture {
                push(value, &tail[..end], loc)?;
            }
            rest = &tail[end + 3..];
        } else if rest.starts_with("<!") {
            return Err(bad("declarations are not accepted"));
        } else if let Some(tail) = rest.strip_prefix("</") {
            let end = tail.find('>').ok_or(bad("unterminated end tag"))?;
            let name = tail[..end].trim_end_matches([' ', '\t', '\r', '\n']);
            if depth == 0 || stack[depth - 1] != name {
                return Err(bad("end tag does not match the open element"));
            }
            depth -= 1;
            root_closed = depth == 0;
            if let Some((field, value)) = capture.take() {
                store(&mut info, field, value);
            }
            rest = &tail[end + 1..];
        } else {
            let (name, empty, length) = start_tag(rest).ok_or(bad("malformed start tag"))?;
            if capture.is_some() {
                return Err(bad("unexpected element inside DOI or DURL"));
            }
            if depth == 0 && (root_closed || name != "Package") {
                return Err(bad("root element is not one Package"));
            }
            let path = &stack[..depth];
            let field = match name {
                "DOI" => Some(Field::Doi),
                "DURL" => Some(Field::Url),
                _ => None,
            };
            if let Some(field) = field.filter(|_| path == ["Package", "FileProperty-Package"]) {
                if std::mem::replace(&mut seen[field as usize], true) {
                    return Err(bad("duplicate DOI or DURL"));
                }
                if !empty {
                    capture = Some((field, String::new()));
                }
            } else if name == "Item" && path == ["Package", "Note-Package", "NoteItems"] {
                // A 1 MiB package holds fewer than 2^32 elements.
                info.note_count += 1;
            }
            if !empty {
                if depth == MAX_DEPTH {
                    return Err(bad("elements nest too deeply"));
                }
                stack[depth] = name;
                depth += 1;
            } else if depth == 0 {
                root_closed = true;
            }
            rest = &rest[length..];
        }
    }
    if !root_closed || depth != 0 {
        return Err(bad("root element is missing or unclosed"));
    }
    Ok(info)
}

fn store(info: &mut ApplicationInfo, field: Field, value: String) {
    let trimmed = value.trim_matches([' ', '\t', '\r', '\n']);
    let value = (!trimmed.is_empty()).then(|| trimmed.to_owned());
    match field {
        Field::Doi => info.doi = value,
        Field::Url => info.url = value,
    }
}

/// Parse one start tag, returning its name, whether it is empty-element, and
/// its byte length. Attribute names and quoted values are checked, not kept.
fn start_tag(tag: &str) -> Option<(&str, bool, usize)> {
    let bytes = tag.as_bytes();
    let mut at = 1;
    while bytes.get(at).copied().is_some_and(is_name_byte) {
        at += 1;
    }
    let name = &tag[1..at];
    if name.is_empty() {
        return None;
    }
    loop {
        let spaced = bytes.get(at).copied().is_some_and(is_space);
        while bytes.get(at).copied().is_some_and(is_space) {
            at += 1;
        }
        match *bytes.get(at)? {
            b'>' => return Some((name, false, at + 1)),
            b'/' => return (bytes.get(at + 1) == Some(&b'>')).then_some((name, true, at + 2)),
            _ if !spaced => return None,
            _ => {}
        }
        let attribute = at;
        while bytes.get(at).copied().is_some_and(is_name_byte) {
            at += 1;
        }
        while bytes.get(at).copied().is_some_and(is_space) {
            at += 1;
        }
        if at == attribute || bytes.get(at) != Some(&b'=') {
            return None;
        }
        at += 1;
        while bytes.get(at).copied().is_some_and(is_space) {
            at += 1;
        }
        let quote = *bytes.get(at).filter(|byte| matches!(byte, b'"' | b'\''))?;
        let value = tag[at + 1..].find(char::from(quote))?;
        if tag[at + 1..at + 1 + value].contains('<') {
            return None;
        }
        at += value + 2;
    }
}

/// Accept an XML declaration whose `encoding`, if present, names UTF-8.
fn check_encoding(declaration: &str) -> Option<()> {
    let Some(at) = declaration.find("encoding") else {
        return Some(());
    };
    let value = declaration[at + 8..]
        .trim_start()
        .strip_prefix('=')?
        .trim_start();
    let quote = value.chars().next().filter(|c| matches!(c, '"' | '\''))?;
    let value = &value[1..];
    let end = value.find(quote)?;
    value[..end].eq_ignore_ascii_case("UTF-8").then_some(())
}

fn push(value: &mut String, text: &str, loc: Location) -> Result<()> {
    let total = value.len() + text.len();
    if total > MAX_APPLICATION_INFO_FIELD_BYTES {
        return Err(loc.limit(
            "application-info field bytes",
            len_u64(MAX_APPLICATION_INFO_FIELD_BYTES),
            len_u64(total),
        ));
    }
    value.push_str(text);
    Ok(())
}

/// Append text with the five predefined entities and numeric character
/// references decoded. Any other reference is rejected.
fn decode_text(mut text: &str, value: &mut String, loc: Location) -> Result<()> {
    while let Some(at) = text.find('&') {
        push(value, &text[..at], loc)?;
        let tail = &text[at + 1..];
        let end = tail
            .find(';')
            .ok_or(loc.malformed(XML, "unterminated entity reference"))?;
        let decoded = match &tail[..end] {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            reference => character_reference(reference),
        }
        .ok_or(loc.malformed(XML, "unknown entity or character reference"))?;
        push(value, decoded.encode_utf8(&mut [0; 4]), loc)?;
        text = &tail[end + 1..];
    }
    push(value, text, loc)
}

/// Decode `#NNN` or `#xHH`, rejecting signs, empty digits and NUL.
fn character_reference(reference: &str) -> Option<char> {
    let (digits, radix) = match reference.strip_prefix("#x") {
        Some(hex) => (hex, 16),
        None => (reference.strip_prefix('#')?, 10),
    };
    if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
        return None;
    }
    u32::from_str_radix(digits, radix)
        .ok()
        .and_then(char::from_u32)
        .filter(|&c| c != '\0')
}

#[cfg(test)]
mod tests;
