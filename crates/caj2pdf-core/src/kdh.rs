// SPDX-License-Identifier: MIT

//! Ranged KDH decoding over the PDF input and repair layer.
//!
//! The wrapper offset and XOR cycle are observations from the three pinned
//! CAJSamples KDH files recorded in `docs/provenance.md`. This module keeps
//! only a bounded scan buffer and exposes decrypted bytes on demand.

use crate::pdf::copy_pdf;
use crate::{
    Cancellation, ConversionReport, Error, Limits, RangedSource, Result, SequentialSink,
    read_exact_at,
};
use std::cmp::min;

const PDF_START: u64 = 254;
const HEADER_SIGNATURE: &[u8; 32] = b"KDH 2.00 Copyright(C) 2000 CAJCD";
const XOR_KEY: &[u8; 6] = b"FZHMEI";
const SCAN_CHUNK: usize = 64 * 1024;
const HISTORY_BYTES: usize = 160;

/// A KDH input presented as a PDF range, without a decrypted document buffer.
///
/// `open` checks the wrapper and PDF start, then scans the encoded bytes in
/// bounded chunks to locate the final syntactic `startxref`/`%%EOF` pair.
/// Reads through this source decrypt only the requested range. The source is
/// usable by native or WASM callers implementing the shared `RangedSource`.
pub struct KdhPdfSource<'a, S> {
    source: &'a mut S,
    pdf_len: u64,
    trailing_len: u64,
    scan_bytes_read: u64,
}

impl<'a, S: RangedSource> KdhPdfSource<'a, S> {
    pub async fn open<C: Cancellation>(
        source: &'a mut S,
        limits: &Limits,
        cancellation: &C,
    ) -> Result<Self> {
        limits.validate()?;
        let size = source.size();
        limits.check_input_size(size)?;
        if size < PDF_START {
            return Err(Error::TruncatedInput {
                offset: 0,
                expected: PDF_START,
                available: size,
            });
        }

        let mut header = [0_u8; PDF_START as usize];
        read_in_chunks(source, 0, &mut header, limits, cancellation).await?;
        if &header[..HEADER_SIGNATURE.len()] != HEADER_SIGNATURE {
            return Err(Error::Kdh {
                offset: 0,
                reason: "KDH signature is invalid",
            });
        }
        if header[0x28..0x2c] != [0, 0, 2, 0] {
            return Err(Error::Kdh {
                offset: 0x28,
                reason: "KDH version field is invalid",
            });
        }
        if size - PDF_START < 8 {
            return Err(Error::TruncatedInput {
                offset: PDF_START,
                expected: 8,
                available: size - PDF_START,
            });
        }
        let mut pdf_header = [0_u8; 8];
        read_in_chunks(source, PDF_START, &mut pdf_header, limits, cancellation).await?;
        xor_at(0, &mut pdf_header);
        if !pdf_header.starts_with(b"%PDF-") {
            return Err(Error::Kdh {
                offset: PDF_START,
                reason: "decoded payload does not start with a PDF header",
            });
        }

        let mut buffer = vec![0_u8; min(limits.io_chunk_bytes, SCAN_CHUNK)];
        let mut history = History::default();
        let mut eof: Option<(u64, u64)> = None;
        let mut candidate_bytes_read = 0_u64;
        let mut at = PDF_START;
        while at < size {
            let count = min(size - at, buffer.len() as u64) as usize;
            read_exact_at(source, at, &mut buffer[..count], limits, cancellation).await?;
            xor_at(at - PDF_START, &mut buffer[..count]);
            for (index, &byte) in buffer[..count].iter().enumerate() {
                history.push(byte);
                if history.ends_with(b"%%EOF") {
                    let marker = at + index as u64 - 4;
                    if let Some(xref) = history.startxref_before_eof() {
                        if xref < marker - PDF_START
                            && eof.is_none_or(|(previous, _)| xref > previous - PDF_START)
                        {
                            let (valid, bytes_read) =
                                xref_target_is_plausible(source, xref, size, limits, cancellation)
                                    .await?;
                            candidate_bytes_read = candidate_bytes_read
                                .checked_add(bytes_read)
                                .ok_or(Error::InvalidInput {
                                    reason: "KDH input byte count overflows",
                                })?;
                            if valid {
                                if eof.is_some() {
                                    return Err(Error::Kdh {
                                        offset: marker,
                                        reason: "ambiguous PDF end in KDH trailer",
                                    });
                                }
                                eof = Some((marker, xref));
                            }
                        }
                    }
                }
            }
            at += count as u64;
        }
        let (eof, _) = eof.ok_or(Error::Kdh {
            offset: size,
            reason: "decoded PDF startxref and EOF were not found",
        })?;
        let after_marker = eof + 5;
        let mut eol = [0_u8; 2];
        let eol_read = min(size - after_marker, 2) as usize;
        if eol_read != 0 {
            read_in_chunks(
                source,
                after_marker,
                &mut eol[..eol_read],
                limits,
                cancellation,
            )
            .await?;
            xor_at(after_marker - PDF_START, &mut eol[..eol_read]);
        }
        let eol_len = if eol_read >= 2 && eol == *b"\r\n" {
            2
        } else if eol_read >= 1 && matches!(eol[0], b'\r' | b'\n') {
            1
        } else {
            0
        };
        let pdf_end = after_marker + eol_len;
        let pdf_len = pdf_end - PDF_START;
        limits.check_input_size(pdf_len)?;
        let scan_bytes_read = size
            .checked_add(8)
            .and_then(|bytes| bytes.checked_add(eol_read as u64))
            .and_then(|bytes| bytes.checked_add(candidate_bytes_read))
            .ok_or(Error::InvalidInput {
                reason: "KDH input byte count overflows",
            })?;
        Ok(Self {
            source,
            pdf_len,
            trailing_len: size - pdf_end,
            scan_bytes_read,
        })
    }

    pub fn pdf_len(&self) -> u64 {
        self.pdf_len
    }

    pub fn trailing_len(&self) -> u64 {
        self.trailing_len
    }

    pub fn scan_bytes_read(&self) -> u64 {
        self.scan_bytes_read
    }
}

async fn xref_target_is_plausible<S: RangedSource, C: Cancellation>(
    source: &mut S,
    relative_offset: u64,
    size: u64,
    limits: &Limits,
    cancellation: &C,
) -> Result<(bool, u64)> {
    let absolute = PDF_START + relative_offset;
    let count = min(size - absolute, 32) as usize;
    if count < 5 {
        return Ok((false, 0));
    }
    let mut prefix = [0_u8; 32];
    read_in_chunks(source, absolute, &mut prefix[..count], limits, cancellation).await?;
    xor_at(relative_offset, &mut prefix[..count]);
    let prefix = &prefix[..count];
    if prefix.starts_with(b"xref") && prefix.get(4).is_some_and(u8::is_ascii_whitespace) {
        return Ok((true, count as u64));
    }
    let mut at = 0;
    for _ in 0..2 {
        let first = at;
        while prefix.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
        }
        if at == first || !prefix.get(at).is_some_and(u8::is_ascii_whitespace) {
            return Ok((false, count as u64));
        }
        while prefix.get(at).is_some_and(u8::is_ascii_whitespace) {
            at += 1;
        }
    }
    let valid = prefix.get(at..at + 3) == Some(b"obj".as_slice())
        && prefix.get(at + 3).is_some_and(u8::is_ascii_whitespace);
    Ok((valid, count as u64))
}

impl<S: RangedSource> RangedSource for KdhPdfSource<'_, S> {
    fn size(&self) -> u64 {
        self.pdf_len
    }

    async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
        if offset > self.pdf_len {
            return Err(Error::InvalidInput {
                reason: "KDH PDF read starts beyond payload end",
            });
        }
        let count = min(self.pdf_len - offset, destination.len() as u64) as usize;
        if count == 0 {
            return Ok(0);
        }
        let read = self
            .source
            .read_at(PDF_START + offset, &mut destination[..count])
            .await?;
        if read > count {
            return Err(Error::InvalidInput {
                reason: "KDH source reported more bytes than requested",
            });
        }
        if read == 0 {
            return Err(Error::TruncatedInput {
                offset: PDF_START + offset,
                expected: count as u64,
                available: 0,
            });
        }
        xor_at(offset, &mut destination[..read]);
        Ok(read)
    }
}

/// Decode KDH and normalize its PDF through the shared bounded PDF path.
pub async fn convert_kdh<S: RangedSource, W: SequentialSink, C: Cancellation>(
    source: &mut S,
    sink: &mut W,
    limits: &Limits,
    cancellation: &C,
) -> Result<ConversionReport> {
    let mut decoded = KdhPdfSource::open(source, limits, cancellation).await?;
    let scan_bytes_read = decoded.scan_bytes_read();
    let mut report = copy_pdf(&mut decoded, sink, limits, cancellation)
        .await
        .map_err(map_pdf_offset)?;
    report.input_bytes_read =
        report
            .input_bytes_read
            .checked_add(scan_bytes_read)
            .ok_or(Error::InvalidInput {
                reason: "KDH input byte count overflows",
            })?;
    Ok(report)
}

fn map_pdf_offset(error: Error) -> Error {
    match error {
        Error::Pdf {
            offset,
            object,
            kind,
            reason,
        } => Error::Pdf {
            offset: offset.saturating_add(PDF_START),
            object,
            kind,
            reason,
        },
        Error::PdfLimitExceeded {
            offset,
            object,
            resource,
            limit,
            attempted,
        } => Error::PdfLimitExceeded {
            offset: offset.saturating_add(PDF_START),
            object,
            resource,
            limit,
            attempted,
        },
        other => other,
    }
}

async fn read_in_chunks<S: RangedSource, C: Cancellation>(
    source: &mut S,
    offset: u64,
    destination: &mut [u8],
    limits: &Limits,
    cancellation: &C,
) -> Result<()> {
    let mut done = 0;
    while done < destination.len() {
        let count = min(destination.len() - done, limits.io_chunk_bytes);
        read_exact_at(
            source,
            offset + done as u64,
            &mut destination[done..done + count],
            limits,
            cancellation,
        )
        .await?;
        done += count;
    }
    Ok(())
}

fn xor_at(offset: u64, bytes: &mut [u8]) {
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte ^= XOR_KEY[((offset + index as u64) % XOR_KEY.len() as u64) as usize];
    }
}

struct History {
    bytes: [u8; HISTORY_BYTES],
    next: usize,
    len: usize,
}

impl Default for History {
    fn default() -> Self {
        Self {
            bytes: [0; HISTORY_BYTES],
            next: 0,
            len: 0,
        }
    }
}

impl History {
    fn push(&mut self, byte: u8) {
        self.bytes[self.next] = byte;
        self.next = (self.next + 1) % HISTORY_BYTES;
        self.len = min(self.len + 1, HISTORY_BYTES);
    }

    fn ends_with(&self, marker: &[u8; 5]) -> bool {
        self.len >= marker.len()
            && marker.iter().enumerate().all(|(index, byte)| {
                self.bytes[(self.next + HISTORY_BYTES - marker.len() + index) % HISTORY_BYTES]
                    == *byte
            })
    }

    fn startxref_before_eof(&self) -> Option<u64> {
        let mut tail = [0_u8; HISTORY_BYTES];
        let first = (self.next + HISTORY_BYTES - self.len) % HISTORY_BYTES;
        for (index, byte) in tail[..self.len].iter_mut().enumerate() {
            *byte = self.bytes[(first + index) % HISTORY_BYTES];
        }
        let tail = &tail[..self.len];
        let eof_at = tail.len().checked_sub(5)?;
        if eof_at == 0 || !matches!(tail[eof_at - 1], b'\r' | b'\n') {
            return None;
        }
        let marker_at = tail[..eof_at]
            .windows(9)
            .rposition(|window| window == b"startxref")?;
        if marker_at > 0 && !matches!(tail[marker_at - 1], b'\r' | b'\n') {
            return None;
        }
        let mut at = marker_at + 9;
        while at < eof_at && matches!(tail[at], b' ' | b'\t' | b'\r' | b'\n') {
            at += 1;
        }
        let first_digit = at;
        let mut xref = 0_u64;
        while at < eof_at && tail[at].is_ascii_digit() {
            xref = xref
                .checked_mul(10)?
                .checked_add(u64::from(tail[at] - b'0'))?;
            at += 1;
        }
        if at == first_digit
            || !tail[at..eof_at]
                .iter()
                .all(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
        {
            return None;
        }
        Some(xref)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_recognizes_a_split_tail() {
        let mut history = History::default();
        for &byte in b"\nstartxref\r\n123\r\n%%EOF" {
            history.push(byte);
        }
        assert!(history.ends_with(b"%%EOF"));
        assert_eq!(history.startxref_before_eof(), Some(123));
    }
}
