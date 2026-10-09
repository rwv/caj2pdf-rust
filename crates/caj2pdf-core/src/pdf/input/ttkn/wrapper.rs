// SPDX-License-Identifier: MIT

//! Bounded rights metadata. Hash the exact original bytes, never reserialized XML.

use super::{TtknResponse, crypto};
use crate::pdf::PdfRange;
use crate::{Cancellation, Error, ErrorKind, Limits, RangedSource, Result, read_exact_at};
use base64::{Engine, engine::general_purpose::STANDARD};
use quick_xml::{Reader, events::Event};
use sha1::{Digest, Sha1};
use sha2::Sha256;
use std::ops::Range;
use zeroize::Zeroizing;

const MAX_XML_BYTES: u64 = 16 * 1024;
const MAX_XML_DEPTH: usize = 32;

pub(super) struct Wrapper {
    pub pdf: PdfRange,
    pub key: Zeroizing<[u8; 16]>,
}

fn malformed() -> Error {
    Error::from(ErrorKind::Malformed).because("invalid TTKN rights metadata")
}

fn unsupported() -> Error {
    Error::from(ErrorKind::UnsupportedFormat).because("unsupported TTKN rights profile")
}

pub(super) fn read<S: RangedSource, C: Cancellation>(
    source: &mut S,
    offset: u64,
    bytes: &mut [u8],
    limits: &Limits,
    cancellation: &C,
) -> Result<()> {
    for (i, chunk) in bytes.chunks_mut(limits.io_chunk_bytes).enumerate() {
        let at = offset
            .checked_add((i * limits.io_chunk_bytes) as u64)
            .ok_or_else(malformed)?;
        read_exact_at(source, at, chunk, limits, cancellation)?;
    }
    Ok(())
}

fn decimal(bytes: &[u8]) -> Result<u64> {
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_digit) {
        return Err(malformed());
    }
    bytes.iter().try_fold(0_u64, |n, b| {
        n.checked_mul(10)
            .and_then(|n| n.checked_add(u64::from(b - b'0')))
            .ok_or_else(malformed)
    })
}

pub(super) fn open<S: RangedSource, C: Cancellation>(
    source: &mut S,
    range: PdfRange,
    response: &TtknResponse,
    limits: &Limits,
    cancellation: &C,
) -> Result<Wrapper> {
    limits.validate()?;
    limits.check_input_size(range.length)?;
    let end = range
        .end()
        .filter(|&n| n <= source.size())
        .ok_or_else(malformed)?;
    let count = range.length.min(128) as usize;
    let mut tail = [0_u8; 128];
    read(
        source,
        end - count as u64,
        &mut tail[..count],
        limits,
        cancellation,
    )?;
    let tail = &tail[..count];
    let marker = tail
        .windows(12)
        .rposition(|b| b == b"startrights ")
        .ok_or_else(unsupported)?;
    let declaration = tail[marker + 12..].trim_ascii();
    let comma = declaration
        .iter()
        .position(|&b| b == b',')
        .ok_or_else(malformed)?;
    let start = decimal(&declaration[..comma])?;
    let length = decimal(&declaration[comma + 1..])?;
    if start < range.offset.saturating_add(12)
        || length == 0
        || start.checked_add(length) != Some(end - count as u64 + marker as u64)
    {
        return Err(malformed());
    }
    if length > MAX_XML_BYTES {
        return Err(Error::limit("TTKN XML bytes", MAX_XML_BYTES, length));
    }
    limits.check_allocation(length)?;
    let mut framing = [0_u8; 12];
    read(source, start - 12, &mut framing, limits, cancellation)?;
    if &framing != b"WebFastLoad\0" {
        return Err(malformed());
    }
    let mut xml = vec![0; length as usize];
    read(source, start, &mut xml, limits, cancellation)?;
    let fields = metadata(&xml)?;
    let password_span = fields.password.ok_or_else(malformed)?;
    let iv_span = fields.iv.ok_or_else(malformed)?;
    let rights_span = fields.rights.ok_or_else(malformed)?;
    let mut password = Zeroizing::new(decode(&xml[password_span])?);
    let iv = Zeroizing::new(decode(&xml[iv_span])?);
    let mut rights = Zeroizing::new(decode(&xml[rights_span.clone()])?);
    if password.len() != 48
        || iv.len() != 32
        || rights.is_empty()
        || !rights.len().is_multiple_of(16)
    {
        return Err(unsupported());
    }
    crypto::unwrap(&response.0, &crypto::WRAPPING_IV, &mut password)?;
    let mut hash = Sha256::new();
    hash.update(&password[..32]);
    hash.update(&xml[..rights_span.start]);
    hash.update(&xml[rights_span.end..]);
    let rights_key = Zeroizing::new(<[u8; 32]>::from(hash.finalize()));
    crypto::unwrap(
        &rights_key,
        iv[..16].try_into().expect("checked IV"),
        &mut rights,
    )?;
    let length = rights.iter().position(|&b| b == 0).unwrap_or(rights.len());
    if rights[length..].iter().any(|&b| b != 0) {
        return Err(crypto::rejected());
    }
    let encrypt = encrypt_field(&rights[..length]).map_err(|_| crypto::rejected())?;
    if encrypt.len() != 32 || !encrypt.iter().all(u8::is_ascii_hexdigit) {
        return Err(crypto::rejected());
    }
    let mut hash = Sha1::new();
    hash.update(encrypt);
    hash.update(b"AppendCA");
    let digest = Zeroizing::new(<[u8; 20]>::from(hash.finalize()));
    let mut key = Zeroizing::new([0_u8; 16]);
    key.copy_from_slice(&digest[..16]);
    Ok(Wrapper {
        pdf: PdfRange {
            offset: range.offset,
            length: start - 12 - range.offset,
        },
        key,
    })
}

fn decode(bytes: &[u8]) -> Result<Vec<u8>> {
    STANDARD.decode(bytes).map_err(|_| malformed())
}

#[derive(Default)]
struct Fields {
    password: Option<Range<usize>>,
    iv: Option<Range<usize>>,
    rights: Option<Range<usize>>,
    version: bool,
    auth: bool,
    permit: bool,
}

struct Element {
    name: Vec<u8>,
    content: usize,
}

fn path(stack: &[Element], names: &[&[u8]]) -> bool {
    stack.len() == names.len() && stack.iter().zip(names).all(|(e, name)| e.name == *name)
}

fn field(place: &mut Option<Range<usize>>, span: Range<usize>) -> Result<()> {
    if place.replace(span).is_some() {
        Err(malformed())
    } else {
        Ok(())
    }
}

// This small visitor also validates the irrelevant rights/metadata branches;
// it never resolves a DTD, external entity, URL or document reference.
fn visit(
    xml: &[u8],
    root: &[u8],
    mut end: impl FnMut(&[Element], Range<usize>, Option<&[u8]>) -> Result<()>,
) -> Result<()> {
    let mut reader = Reader::from_reader(xml);
    let mut stack: Vec<Element> = Vec::new();
    let mut seen_root = false;
    let mut seen_declaration = false;
    loop {
        let before = reader.buffer_position() as usize;
        match reader.read_event().map_err(|_| malformed())? {
            Event::Start(e) | Event::Empty(e) => {
                // Empty events are handled using their original closing token.
                let empty = xml[before..reader.buffer_position() as usize].ends_with(b"/>");
                if stack.is_empty() {
                    if seen_root || e.name().as_ref().as_bytes() != root {
                        return Err(malformed());
                    }
                    seen_root = true;
                }
                if stack.len() == MAX_XML_DEPTH {
                    return Err(malformed());
                }
                let mut kind = None;
                for attr in e.attributes() {
                    let attr = attr.map_err(|_| malformed())?;
                    if attr.key.as_ref() == "type" {
                        kind = Some(attr.value.into_owned().into_bytes());
                    }
                }
                stack.push(Element {
                    name: e.name().as_ref().as_bytes().to_vec(),
                    content: reader.buffer_position() as usize,
                });
                end(&stack, 0..0, kind.as_deref())?;
                if empty {
                    let content = reader.buffer_position() as usize;
                    end(&stack, content..content, None)?;
                    stack.pop();
                }
            }
            Event::End(_) => {
                let element = stack.last().ok_or_else(malformed)?;
                end(&stack, element.content..before, None)?;
                stack.pop();
            }
            Event::Text(text) => {
                if stack.is_empty() && !text.as_ref().as_bytes().iter().all(u8::is_ascii_whitespace)
                {
                    return Err(malformed());
                }
            }
            Event::Decl(declaration) if !seen_root && !seen_declaration => {
                if declaration.version().map_err(|_| malformed())?.as_ref() != "1.0" {
                    return Err(unsupported());
                }
                if let Some(encoding) = declaration.encoding()
                    && !encoding
                        .map_err(|_| malformed())?
                        .eq_ignore_ascii_case("utf-8")
                {
                    return Err(unsupported());
                }
                seen_declaration = true;
            }
            Event::Comment(_) => {}
            Event::Eof => break,
            // The measured profile uses plain UTF-8 text and base64 fields.
            _ => return Err(unsupported()),
        }
    }
    if !seen_root || !stack.is_empty() {
        return Err(malformed());
    }
    Ok(())
}

fn metadata(xml: &[u8]) -> Result<Fields> {
    let mut fields = Fields::default();
    visit(xml, b"right-meta", |stack, span, kind| {
        if span == (0..0) {
            if path(stack, &[b"right-meta", b"protect", b"auth"]) {
                if fields.auth || kind != Some(b"1") {
                    return Err(unsupported());
                }
                fields.auth = true;
            }
            if path(stack, &[b"right-meta", b"protect", b"auth", b"permit"]) {
                if fields.permit || kind != Some(b"3") {
                    return Err(unsupported());
                }
                fields.permit = true;
            }
            return Ok(());
        }
        if path(stack, &[b"right-meta", b"version"]) {
            if fields.version || xml[span.clone()].trim_ascii() != b"2.0" {
                return Err(unsupported());
            }
            fields.version = true;
        } else if path(
            stack,
            &[b"right-meta", b"protect", b"auth", b"permit", b"password"],
        ) {
            field(&mut fields.password, span)?;
        } else if path(stack, &[b"right-meta", b"protect", b"auth", b"iv"]) {
            field(&mut fields.iv, span)?;
        } else if path(stack, &[b"right-meta", b"rights"]) {
            field(&mut fields.rights, span)?;
        }
        Ok(())
    })?;
    if !fields.version || !fields.auth || !fields.permit {
        return Err(unsupported());
    }
    Ok(fields)
}

fn encrypt_field(xml: &[u8]) -> Result<&[u8]> {
    let mut found = None;
    visit(xml, b"rights", |stack, span, _| {
        if span != (0..0) && path(stack, &[b"rights", b"encrypt"]) {
            field(&mut found, span)?;
        }
        Ok(())
    })?;
    let span = found.ok_or_else(malformed)?;
    Ok(&xml[span])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_fields_are_unique_bounded_and_never_resolve_entities() {
        let xml = b"<right-meta><version>2.0</version><protect><auth type=\"1\"><permit type=\"3\"><password>AAAA</password></permit><iv>BBBB</iv></auth></protect><rights>CCCC</rights></right-meta>";
        let fields = metadata(xml).unwrap();
        assert_eq!(&xml[fields.password.unwrap()], b"AAAA");
        assert_eq!(&xml[fields.iv.unwrap()], b"BBBB");
        assert_eq!(&xml[fields.rights.unwrap()], b"CCCC");
        for (needle, replacement) in [
            (
                "<rights>CCCC</rights>",
                "<rights>CCCC</rights><rights>DDDD</rights>",
            ),
            (
                "<password>AAAA</password>",
                "<password>AAAA</password><password>BBBB</password>",
            ),
            ("<iv>BBBB</iv>", "<iv>BBBB</iv><iv>CCCC</iv>"),
            (
                "<version>2.0</version>",
                "<version>2.0</version><version>2.0</version>",
            ),
            ("<version>2.0</version>", "<version>3.0</version>"),
            ("type=\"1\"", "type=\"2\""),
            ("<rights>CCCC</rights>", "<rights>&external;</rights>"),
        ] {
            let altered = std::str::from_utf8(xml)
                .unwrap()
                .replace(needle, replacement);
            assert!(metadata(altered.as_bytes()).is_err());
        }
        for prefix in [
            "<!DOCTYPE right-meta SYSTEM 'https://example.invalid/'>",
            "<?xml version='1.0' encoding='UTF-16'?>",
            "<?xml version='1.0'?><?xml version='1.0'?>",
        ] {
            let altered = [prefix.as_bytes(), xml].concat();
            assert!(metadata(&altered).is_err());
        }
        let declaration = [b"<?xml version='1.0' encoding='utf-8'?>".as_slice(), xml].concat();
        metadata(&declaration).unwrap();
        let nested = format!(
            "<rights>{}<encrypt>00</encrypt>{}</rights>",
            "<x>".repeat(32),
            "</x>".repeat(32)
        );
        assert!(encrypt_field(nested.as_bytes()).is_err());
        assert!(
            encrypt_field(b"<rights><encrypt>a</encrypt><encrypt>b</encrypt></rights>").is_err()
        );
        assert!(decode(b"@AAA").is_err());
        assert!(decimal(b"18446744073709551616").is_err());
    }
}
