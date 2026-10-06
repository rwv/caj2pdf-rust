// SPDX-License-Identifier: MIT

//! The classic cross-reference section and trailer of every written PDF.

use super::types::PdfRef;
use super::writer::{MAX_CLASSIC_PDF_BYTES, Output};
use crate::{Cancellation, Error, Result};
use std::fmt::Write as _;
use std::io::Write;
use std::iter::Peekable;

/// Rows gathered before each write, so a large table costs few sink calls.
const ROWS_PER_WRITE: usize = 200;

/// The trailer dictionary entries an emitter writes.
pub(super) struct Trailer<'a> {
    /// `/Size`; in a dense table also the number of rows.
    pub(super) size: u64,
    pub(super) root: PdfRef,
    /// The previous cross-reference section of an incremental update.
    pub(super) prev: Option<u64>,
    pub(super) info: Option<PdfRef>,
    /// The kept first `/ID` string, as written, and the new second one.
    pub(super) id: Option<(&'a [u8], u128)>,
}

/// Write an `xref` section for in-use `entries`, given as `(reference,
/// offset)` in ascending object-number order, then `trailer` and the
/// `startxref` offset of the section.
///
/// A `dense` table runs from object 0 to `trailer.size - 1`; each number
/// without an entry, and object 0, is a free entry on the free list.
/// Otherwise only the given entries are written, in subsections of
/// consecutive numbers, as an incremental update does.
pub(super) fn write_xref<W, C, I>(
    out: &mut Output<'_, W, C>,
    entries: I,
    dense: bool,
    trailer: &Trailer<'_>,
) -> Result<()>
where
    W: Write,
    C: Cancellation,
    I: Iterator<Item = (PdfRef, u64)> + Clone,
{
    let start = out.position;
    let mut rows = Rows::default();
    if dense {
        out.write(format!("xref\n0 {}\n", trailer.size).as_bytes())?;
        let mut used = entries.peekable();
        for number in 0..trailer.size {
            let row = match used.peek() {
                Some((reference, _)) if number != 0 && u64::from(reference.number) == number => {
                    let (reference, offset) = used.next().ok_or(Error::InvalidInput {
                        reason: "PDF xref entry disappeared",
                    })?;
                    in_use_row(offset, reference.generation)?
                }
                _ => {
                    let generation = if number == 0 { 65_535 } else { 0 };
                    let next = next_free(number, trailer.size, used.clone());
                    row(next, generation, b'f')
                }
            };
            rows.push(out, &row)?;
        }
    } else {
        out.write(b"xref\n")?;
        let mut entries = entries.peekable();
        while let Some(&(first, _)) = entries.peek() {
            let count = run_length(entries.clone());
            out.write(format!("{} {count}\n", first.number).as_bytes())?;
            for _ in 0..count {
                let (reference, offset) = entries.next().ok_or(Error::InvalidInput {
                    reason: "PDF xref entry disappeared",
                })?;
                rows.push(out, &in_use_row(offset, reference.generation)?)?;
            }
            rows.flush(out)?;
        }
    }
    rows.flush(out)?;
    out.write(trailer_text(trailer).as_bytes())?;
    if let Some((first, _)) = trailer.id {
        // The kept ID string is copied as the input wrote it.
        out.write(first)?;
    }
    out.write(trailer_end(trailer, start).as_bytes())
}

/// The byte length of a dense [`write_xref`] section that starts at `start`.
pub(super) fn dense_xref_len(trailer: &Trailer<'_>, start: u64) -> Option<u64> {
    let header = format!("xref\n0 {}\n", trailer.size).len();
    let id = trailer.id.map_or(0, |(first, _)| first.len());
    let text = header + trailer_text(trailer).len() + id + trailer_end(trailer, start).len();
    trailer.size.checked_mul(20)?.checked_add(text as u64)
}

/// The trailer through `/Info`, and `/ID [` when an ID follows.
fn trailer_text(trailer: &Trailer<'_>) -> String {
    let mut text = format!(
        "trailer\n<< /Size {} /Root {}",
        trailer.size,
        pdf_ref(trailer.root)
    );
    if let Some(prev) = trailer.prev {
        let _ = write!(text, " /Prev {prev}");
    }
    if let Some(info) = trailer.info {
        let _ = write!(text, " /Info {}", pdf_ref(info));
    }
    if trailer.id.is_some() {
        text.push_str(" /ID [");
    }
    text
}

/// The trailer after the kept first ID string, through `%%EOF`.
fn trailer_end(trailer: &Trailer<'_>, start: u64) -> String {
    let mut text = String::new();
    if let Some((_, second)) = trailer.id {
        let _ = write!(text, " <{second:032X}>]");
    }
    let _ = write!(text, " >>\nstartxref\n{start}\n%%EOF\n");
    text
}

fn pdf_ref(reference: PdfRef) -> String {
    format!("{} {} R", reference.number, reference.generation)
}

/// The number of entries from the start of `entries` with consecutive
/// object numbers.
fn run_length(mut entries: impl Iterator<Item = (PdfRef, u64)>) -> usize {
    let Some((first, _)) = entries.next() else {
        return 0;
    };
    let mut previous = first.number;
    let mut count = 1;
    for (reference, _) in entries {
        if previous.checked_add(1) != Some(reference.number) {
            break;
        }
        previous = reference.number;
        count += 1;
    }
    count
}

/// The smallest unused number after `number` and below `size`, or zero when
/// there is none. `used` holds the in-use entries after `number`.
fn next_free<I: Iterator<Item = (PdfRef, u64)>>(
    number: u64,
    size: u64,
    mut used: Peekable<I>,
) -> u64 {
    let mut candidate = number + 1;
    while candidate < size {
        while used
            .peek()
            .is_some_and(|(reference, _)| u64::from(reference.number) < candidate)
        {
            used.next();
        }
        if used
            .peek()
            .is_some_and(|(reference, _)| u64::from(reference.number) == candidate)
        {
            candidate += 1;
        } else {
            return candidate;
        }
    }
    0
}

/// An in-use row, whose offset must fit the ten-digit field.
fn in_use_row(offset: u64, generation: u16) -> Result<[u8; 20]> {
    if offset > MAX_CLASSIC_PDF_BYTES {
        return Err(Error::LimitExceeded {
            resource: "classic PDF object offset",
            limit: MAX_CLASSIC_PDF_BYTES,
            attempted: offset,
        });
    }
    Ok(row(offset, generation, b'n'))
}

/// A fixed-width row; `value` has at most ten decimal digits.
fn row(value: u64, generation: u16, kind: u8) -> [u8; 20] {
    let mut row = *b"0000000000 00000 n \n";
    let mut remainder = value;
    for digit in row[..10].iter_mut().rev() {
        *digit = b'0' + (remainder % 10) as u8;
        remainder /= 10;
    }
    let mut remainder = generation;
    for digit in row[11..16].iter_mut().rev() {
        *digit = b'0' + (remainder % 10) as u8;
        remainder /= 10;
    }
    row[17] = kind;
    row
}

/// A fixed buffer of rows awaiting one write.
struct Rows {
    bytes: [u8; 20 * ROWS_PER_WRITE],
    used: usize,
}

impl Default for Rows {
    fn default() -> Self {
        Self {
            bytes: [0; 20 * ROWS_PER_WRITE],
            used: 0,
        }
    }
}

impl Rows {
    fn push<W: Write, C: Cancellation>(
        &mut self,
        out: &mut Output<'_, W, C>,
        row: &[u8; 20],
    ) -> Result<()> {
        if self.used == self.bytes.len() {
            self.flush(out)?;
        }
        self.bytes[self.used..self.used + 20].copy_from_slice(row);
        self.used += 20;
        Ok(())
    }

    fn flush<W: Write, C: Cancellation>(&mut self, out: &mut Output<'_, W, C>) -> Result<()> {
        if self.used != 0 {
            out.write(&self.bytes[..self.used])?;
            self.used = 0;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Limits;
    use crate::test_support::NEVER;

    fn reference(number: u32, generation: u16) -> PdfRef {
        PdfRef { number, generation }
    }

    fn written(entries: &[(PdfRef, u64)], dense: bool, trailer: &Trailer<'_>) -> Result<Vec<u8>> {
        let mut sink = Vec::new();
        let limits = Limits::default();
        {
            let mut out = Output::new(&mut sink, &limits, &NEVER);
            out.position = 7;
            write_xref(&mut out, entries.iter().copied(), dense, trailer)
        }?;
        Ok(sink)
    }

    #[test]
    fn rows_are_fixed_width_and_offsets_have_ten_digits() {
        assert_eq!(row(17, 0, b'n'), *b"0000000017 00000 n \n");
        assert_eq!(row(9, 65_535, b'f'), *b"0000000009 65535 f \n");
        assert_eq!(
            in_use_row(MAX_CLASSIC_PDF_BYTES, 0).unwrap(),
            *b"9999999999 00000 n \n"
        );
        assert!(matches!(
            in_use_row(MAX_CLASSIC_PDF_BYTES + 1, 0),
            Err(Error::LimitExceeded {
                resource: "classic PDF object offset",
                ..
            })
        ));
    }

    #[test]
    fn a_dense_table_links_unused_numbers_on_the_free_list() -> Result<()> {
        let trailer = Trailer {
            size: 6,
            root: reference(5, 0),
            prev: None,
            info: Some(reference(2, 0)),
            id: None,
        };
        let entries = [
            (reference(2, 0), 30),
            (reference(4, 0), 40),
            (reference(5, 0), 50),
        ];
        assert_eq!(
            written(&entries, true, &trailer)?,
            b"xref\n0 6\n\
              0000000001 65535 f \n\
              0000000003 00000 f \n\
              0000000030 00000 n \n\
              0000000000 00000 f \n\
              0000000040 00000 n \n\
              0000000050 00000 n \n\
              trailer\n<< /Size 6 /Root 5 0 R /Info 2 0 R >>\nstartxref\n7\n%%EOF\n"
        );
        Ok(())
    }

    #[test]
    fn a_sparse_update_writes_consecutive_subsections_and_keeps_the_id() -> Result<()> {
        let trailer = Trailer {
            size: 12,
            root: reference(1, 0),
            prev: Some(99),
            info: Some(reference(3, 2)),
            id: Some((b"<AB>", 0x1f)),
        };
        let entries = [
            (reference(1, 0), 30),
            (reference(7, 2), 40),
            (reference(8, 0), 50),
        ];
        assert_eq!(
            written(&entries, false, &trailer)?,
            b"xref\n1 1\n0000000030 00000 n \n7 2\n\
              0000000040 00002 n \n0000000050 00000 n \n\
              trailer\n<< /Size 12 /Root 1 0 R /Prev 99 /Info 3 2 R \
              /ID [<AB> <0000000000000000000000000000001F>] >>\nstartxref\n7\n%%EOF\n"
        );
        Ok(())
    }

    #[test]
    fn large_tables_are_written_in_bounded_row_batches() -> Result<()> {
        let entries: Vec<_> = (1..=450).map(|number| (reference(number, 0), 15)).collect();
        let trailer = Trailer {
            size: 451,
            root: reference(1, 0),
            prev: None,
            info: None,
            id: None,
        };
        let output = written(&entries, true, &trailer)?;
        let rows = &output[b"xref\n0 451\n".len()..];
        assert_eq!(&rows[..20], b"0000000000 65535 f \n");
        assert!(
            rows[20..20 * 451]
                .chunks(20)
                .all(|row| row == b"0000000015 00000 n \n")
        );
        Ok(())
    }
}
