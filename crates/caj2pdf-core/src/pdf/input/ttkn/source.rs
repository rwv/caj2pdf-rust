// SPDX-License-Identifier: MIT

//! A plaintext PDF view backed by bounded metadata and original ciphertext ranges.
//! Streams are decrypted on demand; the complete document is never buffered.

use super::super::parser::{
    Dictionary, ObjectTail, Syntax, exact_name, exact_reference, exact_unsigned, first_id_string,
};
use super::super::{Reader, XrefKind, XrefSlot, named_destinations::name_bytes};
use super::{TtknResponse, crypto, wrapper};
use crate::fallible::reserve_exact;
use crate::pdf::{
    PdfRange, PdfRef, copy_pdf_range,
    writer::{HEADER, Output},
    xref,
};
use crate::{
    Cancellation, ConversionReport, CountingSource, Error, ErrorKind, Limits, RangedSource, Result,
};
use aes::{Aes128, cipher::KeyInit};
use std::io::Write;
use zeroize::Zeroizing;

fn unsupported() -> Error {
    Error::from(ErrorKind::UnsupportedFormat).because("unsupported TTKN PDF encryption profile")
}

fn malformed() -> Error {
    Error::from(ErrorKind::Malformed).because("invalid TTKN PDF structure")
}

fn dictionary(bytes: &[u8]) -> Result<Dictionary> {
    let mut syntax = Syntax::new(bytes);
    let entries = syntax.dictionary(0).map_err(|_| malformed())?;
    syntax.skip_space();
    if syntax.pos != bytes.len() {
        return Err(malformed());
    }
    for (index, entry) in entries.iter().enumerate() {
        if entries[..index]
            .iter()
            .any(|previous| previous.name == entry.name)
        {
            return Err(malformed());
        }
    }
    Ok(Dictionary {
        bytes: bytes.to_vec(),
        entries,
    })
}

fn required<'a>(dictionary: &'a Dictionary, name: &[u8]) -> Result<&'a [u8]> {
    dictionary.value(name).ok_or_else(unsupported)
}

fn handler(dictionary: &Dictionary) -> Result<()> {
    let allowed: &[&[u8]] = &[
        b"Filter",
        b"SubFilter",
        b"V",
        b"R",
        b"Length",
        b"EncryptMetadata",
        b"CF",
        b"StrF",
        b"StmF",
    ];
    if dictionary
        .entries
        .iter()
        .any(|entry| !allowed.contains(&entry.name.as_slice()))
    {
        return Err(unsupported());
    }

    for (field, name) in [
        (b"Filter".as_slice(), b"TTKN.PubSec".as_slice()),
        (b"SubFilter", b"TTKN.PubSec.s1"),
        (b"StrF", b"DefaultCryptFilter"),
        (b"StmF", b"DefaultCryptFilter"),
    ] {
        if exact_name(required(dictionary, field)?).as_deref() != Some(name) {
            return Err(unsupported());
        }
    }
    for (field, value) in [(b"V".as_slice(), 2), (b"R", 2), (b"Length", 40)] {
        if exact_unsigned(required(dictionary, field)?) != Some(value) {
            return Err(unsupported());
        }
    }
    if required(dictionary, b"EncryptMetadata")? != b"true" {
        return Err(unsupported());
    }
    let filters = self::dictionary(required(dictionary, b"CF")?)?;
    if filters.entries.len() != 1 {
        return Err(unsupported());
    }
    let filter = self::dictionary(required(&filters, b"DefaultCryptFilter")?)?;
    if exact_name(required(&filter, b"CFM")?).as_deref() != Some(b"AESV2") {
        return Err(unsupported());
    }
    if filter.entries.len() != 2 {
        return Err(unsupported());
    }
    let recipients = required(&filter, b"Recipients")?.trim_ascii();
    let value = recipients
        .strip_prefix(b"[")
        .and_then(|s| s.strip_suffix(b"]"))
        .ok_or_else(unsupported)?;
    if name_bytes(value, 32)?.as_deref() != Some(b"AppendCA") {
        return Err(unsupported());
    }
    Ok(())
}

fn check_filters(raw: &[u8]) -> Result<()> {
    let mut parser = Syntax::new(raw);
    parser.skip_space();
    let array = raw.get(parser.pos) == Some(&b'[');
    if array {
        parser.pos += 1;
    }
    loop {
        parser.skip_space();
        if array && raw.get(parser.pos) == Some(&b']') {
            parser.pos += 1;
            break;
        }
        let start = parser.pos;
        parser.skip_value(0).map_err(|_| malformed())?;
        let name = exact_name(&raw[start..parser.pos]).ok_or_else(unsupported)?;
        if name == b"Crypt" {
            return Err(unsupported());
        }
        if !array {
            break;
        }
    }
    parser.skip_space();
    if parser.pos != raw.len() {
        return Err(malformed());
    }
    Ok(())
}

enum Data {
    Metadata(Vec<u8>),
    Stream {
        source: u64,
        key: Zeroizing<[u8; 16]>,
    },
}

struct Piece {
    start: u64,
    length: u64,
    data: Data,
}

struct View<'a, S, C> {
    source: &'a mut S,
    pieces: Vec<Piece>,
    size: u64,
    limits: &'a Limits,
    cancellation: &'a C,
}

// All retained metadata, including indexes, shares one allocation budget.
struct Pieces {
    pieces: Vec<Piece>,
    size: u64,
    retained: u64,
}

impl Pieces {
    fn add(&mut self, data: Data, length: u64, limits: &Limits) -> Result<()> {
        let bytes = match &data {
            Data::Metadata(bytes) => bytes.capacity() as u64,
            Data::Stream { .. } => 0,
        };
        self.retained = self
            .retained
            .checked_add(bytes + std::mem::size_of::<Piece>() as u64)
            .ok_or_else(malformed)?;
        limits.check_allocation(self.retained)?;
        let next = self.size.checked_add(length).ok_or_else(malformed)?;
        if next > limits.max_output_bytes {
            return Err(Error::limit("output bytes", limits.max_output_bytes, next));
        }
        reserve_exact(&mut self.pieces, 1, malformed())?;
        self.pieces.push(Piece {
            start: self.size,
            length,
            data,
        });
        self.size = next;
        Ok(())
    }

    fn metadata(&mut self, bytes: Vec<u8>, limits: &Limits) -> Result<()> {
        let length = bytes.len() as u64;
        self.add(Data::Metadata(bytes), length, limits)
    }
}

impl<'a, S: RangedSource, C: Cancellation> View<'a, S, C> {
    fn open(
        source: &'a mut S,
        range: PdfRange,
        response: &TtknResponse,
        limits: &'a Limits,
        cancellation: &'a C,
    ) -> Result<Self> {
        let wrapper = wrapper::open(source, range, response, limits, cancellation)?;
        let mut reader = Reader::new(source, wrapper.pdf, limits, cancellation)?;
        reader.allow_encrypted = true;
        reader.check_header()?;
        let (latest, _) = reader.find_tail()?;
        // The demonstrated profile has one classic table. Other encryption
        // revisions and object streams need separate, measured controls.
        if reader.bytes(latest, 4)? != b"xref" {
            return Err(unsupported());
        }
        let (slots, trailer) = reader.read_xref_chain(latest)?;
        if trailer.prev.is_some() {
            return Err(unsupported());
        }
        let encryption = trailer.encrypt.ok_or_else(unsupported)?;
        let root = trailer.root.ok_or_else(malformed)?;
        let encryption_at = match slots.get(encryption.number as usize).and_then(|s| *s) {
            Some(XrefSlot {
                generation,
                kind: XrefKind::InUse(at),
            }) if generation == encryption.generation => at,
            _ => return Err(malformed()),
        };
        let encryption_head = reader.load_head(encryption_at, Some(encryption))?;
        if !matches!(encryption_head.tail, ObjectTail::EndObject { .. }) {
            return Err(unsupported());
        }
        let encryption_dictionary = encryption_head
            .dictionary
            .as_ref()
            .ok_or_else(unsupported)?;
        reader.reject_duplicate_names(encryption_dictionary, encryption_at, Some(encryption))?;
        handler(encryption_dictionary)?;
        let index_bytes = slots.len() as u64
            * (std::mem::size_of::<Option<XrefSlot>>() + std::mem::size_of::<(PdfRef, u64)>())
                as u64;
        limits.check_allocation(index_bytes)?;
        let mut pieces = Pieces {
            pieces: Vec::new(),
            size: 0,
            retained: index_bytes,
        };
        pieces.metadata(HEADER.to_vec(), limits)?;
        let mut entries = Vec::new();
        reserve_exact(&mut entries, slots.len(), malformed())?;
        for (number, slot) in slots.iter().enumerate() {
            let Some(slot) = slot else {
                continue;
            };
            let at = match slot.kind {
                XrefKind::Free => continue,
                XrefKind::InUse(at) => at,
                XrefKind::Compressed { .. } => return Err(unsupported()),
            };
            let reference = PdfRef {
                number: number as u32,
                generation: slot.generation,
            };
            if reference.number == 0 {
                return Err(malformed());
            }
            if reference == encryption {
                continue;
            }
            let (head, _) = reader.load_object(at, reference, &slots)?;
            let key = crypto::object_key(&wrapper.key, reference);
            entries.push((reference, pieces.size));
            match head.tail {
                ObjectTail::EndObject { end } => {
                    pieces.metadata(strings(&head.bytes[..end], &key, limits)?, limits)?;
                    pieces.metadata(b"\n".to_vec(), limits)?;
                }
                ObjectTail::Stream { data_start } => {
                    let dictionary = head.dictionary.as_ref().ok_or_else(malformed)?;
                    // Crypt filters can change the encryption of an individual
                    // stream; do not interpret them as ordinary payload filters.
                    if let Some(filter) = dictionary.value(b"Filter") {
                        check_filters(filter)?;
                    }
                    let length = dictionary.entry(b"Length").ok_or_else(malformed)?;
                    let raw = length.value(&dictionary.bytes);
                    let encrypted_length = match exact_unsigned(raw) {
                        Some(n) => n,
                        None => reader
                            .resolve_length(exact_reference(raw).ok_or_else(malformed)?, &slots)?,
                    };
                    if encrypted_length < 32 || !encrypted_length.is_multiple_of(16) {
                        return Err(crypto::rejected());
                    }
                    let source_at = wrapper.pdf.offset + at + data_start as u64;
                    let mut tail = [0_u8; 32];
                    wrapper::read(
                        reader.source,
                        source_at + encrypted_length - 32,
                        &mut tail,
                        limits,
                        cancellation,
                    )?;
                    let cipher = Aes128::new((&*key).into());
                    let last = crypto::block(
                        &cipher,
                        tail[16..].try_into().expect("block"),
                        tail[..16].try_into().expect("block"),
                    );
                    let plain_length = encrypted_length - 16 - crypto::padding(&last)? as u64;
                    let start = head.dictionary_start.ok_or_else(malformed)?;
                    let before = start + length.value.start;
                    let after = start + length.value.end;
                    let mut rewritten = strings(&head.bytes[..before], &key, limits)?;
                    append(&mut rewritten, plain_length.to_string().as_bytes(), limits)?;
                    append(
                        &mut rewritten,
                        &strings(&head.bytes[after..data_start], &key, limits)?,
                        limits,
                    )?;
                    pieces.metadata(rewritten, limits)?;
                    if plain_length != 0 {
                        pieces.add(
                            Data::Stream {
                                source: source_at,
                                key,
                            },
                            plain_length,
                            limits,
                        )?;
                    }
                    pieces.metadata(b"\nendstream\nendobj\n".to_vec(), limits)?;
                }
            }
        }
        let xref = xref::Trailer {
            size: u64::from(trailer.size),
            root,
            prev: None,
            info: trailer.info,
            id: trailer
                .id
                .as_deref()
                .and_then(first_id_string)
                .map(|first| (first, 0)),
        };
        let length = xref::dense_xref_len(&xref, pieces.size).ok_or_else(malformed)?;
        limits.check_allocation(pieces.retained.saturating_add(length))?;
        let mut table = Vec::new();
        reserve_exact(
            &mut table,
            usize::try_from(length).map_err(|_| malformed())?,
            malformed(),
        )?;
        let mut output = Output::new(&mut table, limits, cancellation);
        output.position = pieces.size;
        xref::write_xref(&mut output, entries.iter().copied(), true, &xref)?;
        pieces.metadata(table, limits)?;
        Ok(Self {
            source,
            pieces: pieces.pieces,
            size: pieces.size,
            limits,
            cancellation,
        })
    }
}

fn append(out: &mut Vec<u8>, bytes: &[u8], limits: &Limits) -> Result<()> {
    limits.check_allocation(out.len() as u64 + bytes.len() as u64)?;
    reserve_exact(out, bytes.len(), malformed())?;
    out.extend_from_slice(bytes);
    Ok(())
}

/// Rewrite strings only, leaving names, comments and other syntax intact.
fn strings(bytes: &[u8], key: &[u8; 16], limits: &Limits) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut pos = 0;
    let mut copied = 0;
    while pos < bytes.len() {
        match bytes[pos] {
            b'%' => {
                while pos < bytes.len() && !matches!(bytes[pos], b'\r' | b'\n') {
                    pos += 1;
                }
            }
            b'/' => {
                pos += 1;
                while pos < bytes.len()
                    && !super::super::parser::is_delimiter(bytes[pos])
                    && !super::super::parser::is_space(bytes[pos])
                {
                    pos += 1;
                }
            }
            b'<' if bytes.get(pos + 1) == Some(&b'<') => pos += 2,
            b'(' | b'<' => {
                append(&mut out, &bytes[copied..pos], limits)?;
                let mut syntax = Syntax::new(&bytes[pos..]);
                syntax.skip_value(0).map_err(|_| malformed())?;
                let mut value =
                    name_bytes(&bytes[pos..pos + syntax.pos], limits.max_allocation_bytes)?
                        .ok_or_else(malformed)?;
                crypto::decrypt_string(key, &mut value)?;
                append(&mut out, b"<", limits)?;
                limits.check_allocation(out.len() as u64 + value.len() as u64 * 2 + 1)?;
                reserve_exact(&mut out, value.len() * 2 + 1, malformed())?;
                for byte in value {
                    out.push(b"0123456789ABCDEF"[usize::from(byte >> 4)]);
                    out.push(b"0123456789ABCDEF"[usize::from(byte & 15)]);
                }
                out.push(b'>');
                pos += syntax.pos;
                copied = pos;
            }
            _ => pos += 1,
        }
    }
    append(&mut out, &bytes[copied..], limits)?;
    Ok(out)
}

impl<S: RangedSource, C: Cancellation> RangedSource for View<'_, S, C> {
    fn size(&self) -> u64 {
        self.size
    }

    fn read_at(&mut self, offset: u64, bytes: &mut [u8]) -> Result<usize> {
        if self.cancellation.is_cancelled() {
            return Err(Error::from(ErrorKind::Cancelled));
        }
        if offset >= self.size || bytes.is_empty() {
            return Ok(0);
        }
        let index = self.pieces.partition_point(|piece| piece.start <= offset) - 1;
        let piece = &self.pieces[index];
        let relative = offset - piece.start;
        let count = (bytes.len() as u64).min(piece.length - relative) as usize;
        match &piece.data {
            Data::Metadata(data) => {
                bytes[..count].copy_from_slice(&data[relative as usize..relative as usize + count])
            }
            Data::Stream { source, key } => {
                let cipher = Aes128::new((&**key).into());
                let mut done = 0;
                let mut scratch = [0_u8; 8192 + 16];
                while done < count {
                    let position = relative + done as u64;
                    let skip = (position % 16) as usize;
                    let take = (count - done).min(8192 - skip);
                    let encrypted = (skip + take).div_ceil(16) * 16;
                    wrapper::read(
                        self.source,
                        source + position / 16 * 16,
                        &mut scratch[..encrypted + 16],
                        self.limits,
                        self.cancellation,
                    )?;
                    let mut previous: [u8; 16] = scratch[..16].try_into().expect("block");
                    for chunk in scratch[16..encrypted + 16].as_chunks_mut::<16>().0 {
                        let current = *chunk;
                        chunk.copy_from_slice(&crypto::block(&cipher, &current, &previous));
                        previous = current;
                    }
                    bytes[done..done + take].copy_from_slice(&scratch[16 + skip..16 + skip + take]);
                    done += take;
                }
            }
        }
        Ok(count)
    }
}

pub(crate) fn convert<S: RangedSource, W: Write, C: Cancellation>(
    source: &mut S,
    sink: &mut W,
    range: PdfRange,
    response: &TtknResponse,
    limits: &Limits,
    cancellation: &C,
) -> Result<ConversionReport> {
    let mut input_bytes_read = 0;
    let mut counted = CountingSource::new(source, &mut input_bytes_read);
    let result = (|| {
        let mut view = View::open(&mut counted, range, response, limits, cancellation)?;
        let plain = PdfRange {
            offset: 0,
            length: view.size(),
        };
        copy_pdf_range(&mut view, sink, plain, limits, cancellation)
    })();
    result
        .map(|report| ConversionReport {
            input_bytes_read,
            ..report
        })
        .map_err(|error| error.in_pdf(None))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crypt_filter_names_are_decoded_before_profile_admission() {
        for accepted in [
            b"/FlateDecode".as_slice(),
            b"[/FlateDecode /DCTDecode]",
            b"[]",
        ] {
            check_filters(accepted).unwrap();
        }
        for refused in [
            b"/Crypt".as_slice(),
            b"/Cr#79pt",
            b"[/FlateDecode /Cr#79pt]",
            b"9 0 R",
            b"[/FlateDecode 9 0 R]",
            b"[/FlateDecode",
            b"/FlateDecode /DCTDecode",
        ] {
            assert!(check_filters(refused).is_err());
        }
    }

    #[test]
    fn empty_strings_and_syntax_are_preserved_without_interpreting_comments() {
        let bytes = b"<</Empty() /Name/Cr#79pt % (comment) <00>\n/Array[() < >]>>";
        assert_eq!(
            strings(bytes, &[0; 16], &Limits::default()).unwrap(),
            b"<</Empty<> /Name/Cr#79pt % (comment) <00>\n/Array[<> <>]>>"
        );
        assert!(strings(b"(short)", &[0; 16], &Limits::default()).is_err());
    }
}
