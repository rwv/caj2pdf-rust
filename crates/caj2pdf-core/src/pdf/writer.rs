// SPDX-License-Identifier: MIT

//! A small, forward-only PDF 1.7 serializer.
//!
//! This module deliberately does not model arbitrary PDF objects. Callers
//! reserve indirect object numbers, emit each object once, then finish with a
//! classic cross-reference table. Only object offsets are retained in memory.

use super::outline::{ObjectAllocator, ObjectSink};
use super::types::PdfRef;
use super::xref::{Trailer, dense_xref_len, write_xref};
use crate::fallible::{len_u64, reserve_exact};
use crate::{Cancellation, Error, Limits, Result, SequentialSink, write_all};
use std::mem::size_of;

/// Classic cross-reference entries have ten decimal digits for byte offsets.
/// The whole file is kept below 10^10 bytes, including its trailer.
pub const MAX_CLASSIC_PDF_BYTES: u64 = 9_999_999_999;
/// PDF 1.7 Annex C's recommended maximum integer for broad reader support.
pub const MAX_PDF_INTEGER: u64 = i32::MAX as u64;
/// PDF 1.7 Annex C's recommended maximum number of indirect objects.
pub const MAX_PDF_OBJECTS: u32 = 8_388_607;

/// The header and binary-content marker of every PDF written from scratch.
pub(super) const HEADER: &[u8] = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n";

/// A generation-zero indirect object number reserved by [`PdfWriter`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObjectId(u32);

impl ObjectId {
    /// The number to use in a PDF indirect reference (`number 0 R`).
    pub const fn number(self) -> u32 {
        self.0
    }
}

impl From<ObjectId> for PdfRef {
    fn from(id: ObjectId) -> Self {
        Self {
            number: id.0,
            generation: 0,
        }
    }
}

/// The sink side shared by every PDF emitter.
///
/// It counts accepted bytes, refuses a write that would end past the classic
/// xref's ten-digit offsets before the sink sees it, and poisons itself after
/// a sink failure, which may leave a partial PDF.
pub(super) struct Output<'a, W: SequentialSink, C: Cancellation> {
    sink: &'a mut W,
    pub(super) limits: &'a Limits,
    pub(super) cancellation: &'a C,
    pub(super) position: u64,
    poisoned: bool,
}

impl<'a, W: SequentialSink, C: Cancellation> Output<'a, W, C> {
    pub(super) fn new(sink: &'a mut W, limits: &'a Limits, cancellation: &'a C) -> Self {
        Self {
            sink,
            limits,
            cancellation,
            position: 0,
            poisoned: false,
        }
    }

    pub(super) fn ensure_healthy(&self) -> Result<()> {
        if self.poisoned {
            Err(Error::InvalidInput {
                reason: "PDF writer cannot continue after a sink failure",
            })
        } else {
            Ok(())
        }
    }

    /// Write `bytes`, which must end within [`MAX_CLASSIC_PDF_BYTES`].
    pub(super) fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.ensure_healthy()?;
        let attempted =
            self.position
                .checked_add(len_u64(bytes.len()))
                .ok_or(Error::InvalidInput {
                    reason: "PDF output byte count overflows 64 bits",
                })?;
        check_classic_pdf_bytes(attempted)?;
        self.write_unbounded(bytes)
    }

    /// Write `bytes` without the classic-xref ceiling, to copy an inspected
    /// input PDF unchanged.
    pub(super) fn write_unbounded(&mut self, bytes: &[u8]) -> Result<()> {
        self.ensure_healthy()?;
        let result = write_all(
            self.sink,
            bytes,
            &mut self.position,
            self.limits,
            self.cancellation,
        );
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }

    /// Flush the sink, checking cancellation on both sides because a flush
    /// can itself await sink backpressure.
    pub(super) fn flush(&mut self) -> Result<()> {
        self.ensure_healthy()?;
        if self.cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        self.sink.flush().inspect_err(|_| {
            self.poisoned = true;
        })?;
        if self.cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum State {
    Idle,
    Object,
    Stream {
        length_id: ObjectId,
        data_start: u64,
    },
}

/// Writes one PDF to a caller-owned, forward-only sink.
///
/// The writer retains one checked `u64` offset per reserved object. Payload
/// bytes are passed directly to the bounded core sink helper, which awaits
/// backpressure and checks cancellation between writes. An I/O failure poisons
/// this writer because the sink may then contain a partial PDF.
pub struct PdfWriter<'a, W: SequentialSink, C: Cancellation> {
    out: Output<'a, W, C>,
    /// Zero means reserved but not emitted; the PDF header makes zero an
    /// impossible offset for a real object.
    offsets: Vec<u64>,
    state: State,
}

impl<'a, W: SequentialSink, C: Cancellation> PdfWriter<'a, W, C> {
    /// Start a PDF 1.7 file, including its binary-content marker.
    pub fn new(sink: &'a mut W, limits: &'a Limits, cancellation: &'a C) -> Result<Self> {
        limits.validate()?;
        let mut out = Output::new(sink, limits, cancellation);
        out.write(HEADER)?;
        Ok(Self {
            out,
            offsets: Vec::new(),
            state: State::Idle,
        })
    }

    /// Number of bytes accepted by the sink so far.
    pub const fn position(&self) -> u64 {
        self.out.position
    }

    /// Pretend the sink has accepted `position` bytes, to reach output-size
    /// limits without writing them.
    #[cfg(test)]
    pub(crate) fn set_position_for_test(&mut self, position: u64) {
        self.out.position = position;
    }

    /// Reserve one generation-zero object number before writing its body.
    ///
    /// The object index requests capacity within
    /// `Limits::max_allocation_bytes`; no object content is stored in it.
    pub fn reserve_object(&mut self) -> Result<ObjectId> {
        self.prepare_objects(1)?;
        let number = checked_object_number(self.offsets.len() + 1)?;
        self.offsets.push(0);
        Ok(ObjectId(number))
    }

    /// Preflight/reserve index capacity without minting object numbers or
    /// writing bytes. A document uses this before a page-tree rollover so a
    /// known object/count budget refusal cannot follow partially written nodes.
    pub(crate) fn prepare_objects(&mut self, additional_objects: usize) -> Result<()> {
        self.out.ensure_healthy()?;
        let limits = self.out.limits;
        let next_count =
            self.offsets
                .len()
                .checked_add(additional_objects)
                .ok_or(Error::InvalidInput {
                    reason: "PDF object count overflows address space",
                })?;
        checked_object_number(next_count)?;
        let max_slots =
            (limits.max_allocation_bytes / size_of::<u64>() as u64).min(u64::from(MAX_PDF_OBJECTS));
        let next_capacity = if next_count > self.offsets.capacity() {
            let doubled = if self.offsets.capacity() == 0 {
                4
            } else {
                self.offsets.capacity().saturating_mul(2)
            };
            doubled.min(max_slots as usize).max(next_count)
        } else {
            self.offsets.capacity()
        };
        let bytes = next_capacity
            .checked_mul(size_of::<u64>())
            .ok_or(Error::InvalidInput {
                reason: "PDF object index size overflows address space",
            })?;
        let bytes = len_u64(bytes);
        limits.check_allocation(bytes)?;
        if next_count > self.offsets.capacity() {
            let refused = limits.allocation_refused("PDF object index allocation", bytes);
            let additional = next_capacity - self.offsets.len();
            reserve_exact(&mut self.offsets, additional, refused)?;
        }
        Ok(())
    }

    /// Begin one previously reserved object. Follow with `write_bytes` and
    /// `end_object`; each object must be emitted exactly once.
    pub fn begin_object(&mut self, id: ObjectId) -> Result<()> {
        self.ensure_idle()?;
        let index = self.unwritten_index(id)?;
        let offset = self.out.position;
        let header = format!("{} 0 obj\n", id.number());
        self.out.write(header.as_bytes())?;
        self.offsets[index] = offset;
        self.state = State::Object;
        Ok(())
    }

    /// Emit bytes inside an ordinary indirect object. Large slices are split
    /// into configured chunks before reaching the sink.
    pub fn write_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.out.ensure_healthy()?;
        if self.state != State::Object {
            return Err(Error::InvalidInput {
                reason: "PDF bytes require an open ordinary object",
            });
        }
        self.out.write(bytes)
    }

    /// Close an ordinary indirect object.
    pub fn end_object(&mut self) -> Result<()> {
        self.out.ensure_healthy()?;
        if self.state != State::Object {
            return Err(Error::InvalidInput {
                reason: "no ordinary PDF object is open",
            });
        }
        self.out.write(b"\nendobj\n")?;
        self.state = State::Idle;
        Ok(())
    }

    /// Convenience for one small plain object body.
    pub fn write_object(&mut self, id: ObjectId, body: &[u8]) -> Result<()> {
        self.begin_object(id)?;
        self.write_bytes(body)?;
        self.end_object()
    }

    /// Begin an unknown-length stream and put an indirect `/Length` reference
    /// in its dictionary. `dictionary_entries` contains only inner entries,
    /// excluding `/Length` and the surrounding `<<` and `>>` delimiters.
    /// The length object is emitted by `end_stream` after the payload.
    pub fn begin_stream(
        &mut self,
        stream_id: ObjectId,
        length_id: ObjectId,
        dictionary_entries: &[u8],
    ) -> Result<()> {
        self.ensure_idle()?;
        if stream_id == length_id {
            return Err(Error::InvalidInput {
                reason: "PDF stream and length objects must be distinct",
            });
        }
        self.unwritten_index(length_id)?;
        self.begin_object(stream_id)?;
        self.write_bytes(b"<<\n/Length ")?;
        self.write_bytes(length_id.number().to_string().as_bytes())?;
        self.write_bytes(b" 0 R\n")?;
        self.write_bytes(dictionary_entries)?;
        if !dictionary_entries.is_empty() && !dictionary_entries.ends_with(b"\n") {
            self.write_bytes(b"\n")?;
        }
        self.write_bytes(b">>\nstream\n")?;
        self.state = State::Stream {
            length_id,
            data_start: self.out.position,
        };
        Ok(())
    }

    /// Write raw stream payload, unchanged. Delimiter-like binary bytes are
    /// safe because `/Length` records this exact payload byte count.
    pub fn write_stream_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.out.ensure_healthy()?;
        let State::Stream { data_start, .. } = self.state else {
            return Err(Error::InvalidInput {
                reason: "no PDF stream is open",
            });
        };
        let current_length =
            self.out
                .position
                .checked_sub(data_start)
                .ok_or(Error::InvalidInput {
                    reason: "PDF stream position moved backwards",
                })?;
        let chunk_length = len_u64(bytes.len());
        let attempted = current_length
            .checked_add(chunk_length)
            .ok_or(Error::InvalidInput {
                reason: "PDF stream length overflows 64 bits",
            })?;
        if attempted > MAX_PDF_INTEGER {
            return Err(Error::LimitExceeded {
                resource: "PDF stream length",
                limit: MAX_PDF_INTEGER,
                attempted,
            });
        }
        self.out.write(bytes)
    }

    /// Close a stream and write its measured length as a separate indirect
    /// object. The separator newline after the payload is not in `/Length`.
    pub fn end_stream(&mut self) -> Result<()> {
        self.out.ensure_healthy()?;
        let State::Stream {
            length_id,
            data_start,
        } = self.state
        else {
            return Err(Error::InvalidInput {
                reason: "no PDF stream is open",
            });
        };
        let length = self
            .out
            .position
            .checked_sub(data_start)
            .ok_or(Error::InvalidInput {
                reason: "PDF stream position moved backwards",
            })?;
        self.out.write(b"\nendstream\nendobj\n")?;
        self.state = State::Idle;
        self.write_object(length_id, length.to_string().as_bytes())
    }

    /// Emit the classic cross-reference table and trailer, flush the sink,
    /// and return the final byte count. Every reserved object must have been
    /// written exactly once, including any stream length object.
    pub fn finish(self, root_id: ObjectId) -> Result<u64> {
        self.finish_with_info(root_id, None)
    }

    /// Like [`Self::finish`], also naming a written document information
    /// dictionary in the trailer when `info_id` is present.
    pub fn finish_with_info(mut self, root_id: ObjectId, info_id: Option<ObjectId>) -> Result<u64> {
        self.ensure_idle()?;
        let root_index = self.index(root_id)?;
        if let Some(id) = info_id {
            self.index(id)?;
        }
        if self.offsets[root_index] == 0 {
            return Err(Error::InvalidInput {
                reason: "PDF catalog object has not been written",
            });
        }
        if self.offsets.contains(&0) {
            return Err(Error::InvalidInput {
                reason: "a reserved PDF object has not been written",
            });
        }
        // Object numbers are checked when reserved, so each index fits `u32`.
        let entries = self
            .offsets
            .iter()
            .enumerate()
            .map(|(index, &offset)| (PdfRef::from(ObjectId(index as u32 + 1)), offset));
        let trailer = Trailer {
            size: len_u64(self.offsets.len()) + 1,
            root: root_id.into(),
            prev: None,
            info: info_id.map(PdfRef::from),
            id: None,
        };
        // Refuse a table that cannot fit before writing any of it.
        let start = self.out.position;
        let final_size = dense_xref_len(&trailer, start)
            .and_then(|tail| start.checked_add(tail))
            .ok_or(Error::InvalidInput {
                reason: "PDF output byte count overflows 64 bits",
            })?;
        check_classic_pdf_bytes(final_size)?;
        let limits = self.out.limits;
        if final_size > limits.max_output_bytes {
            return Err(Error::LimitExceeded {
                resource: "output bytes",
                limit: limits.max_output_bytes,
                attempted: final_size,
            });
        }
        write_xref(&mut self.out, entries, true, &trailer)?;
        self.out.flush()?;
        Ok(self.out.position)
    }

    pub(crate) fn ensure_idle(&self) -> Result<()> {
        self.out.ensure_healthy()?;
        if self.state != State::Idle {
            return Err(Error::InvalidInput {
                reason: "another PDF object is already open",
            });
        }
        Ok(())
    }

    fn index(&self, id: ObjectId) -> Result<usize> {
        let Some(index) = id.number().checked_sub(1) else {
            return Err(Error::InvalidInput {
                reason: "PDF object number zero is reserved",
            });
        };
        let index = index as usize;
        if index >= self.offsets.len() {
            return Err(Error::InvalidInput {
                reason: "PDF object number was not reserved",
            });
        }
        Ok(index)
    }

    fn unwritten_index(&self, id: ObjectId) -> Result<usize> {
        let index = self.index(id)?;
        if self.offsets[index] != 0 {
            return Err(Error::InvalidInput {
                reason: "PDF object has already been written",
            });
        }
        Ok(index)
    }
}

impl<W: SequentialSink, C: Cancellation> ObjectSink for PdfWriter<'_, W, C> {
    type Ref = ObjectId;

    fn begin_object(&mut self, reference: ObjectId) -> Result<()> {
        PdfWriter::begin_object(self, reference)
    }

    fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.write_bytes(bytes)
    }

    fn end_object(&mut self) -> Result<()> {
        PdfWriter::end_object(self)
    }
}

impl<W: SequentialSink, C: Cancellation> ObjectAllocator for PdfWriter<'_, W, C> {
    fn reserve(&mut self) -> Result<ObjectId> {
        self.reserve_object()
    }
}

/// Reject output that would end past the classic xref's ten-digit offsets.
pub(crate) fn check_classic_pdf_bytes(attempted: u64) -> Result<()> {
    if attempted > MAX_CLASSIC_PDF_BYTES {
        return Err(Error::LimitExceeded {
            resource: "classic PDF file bytes",
            limit: MAX_CLASSIC_PDF_BYTES,
            attempted,
        });
    }
    Ok(())
}

pub(crate) fn checked_object_number(count: usize) -> Result<u32> {
    let attempted = len_u64(count);
    if attempted > u64::from(MAX_PDF_OBJECTS) {
        return Err(Error::LimitExceeded {
            resource: "PDF object count",
            limit: u64::from(MAX_PDF_OBJECTS),
            attempted,
        });
    }
    Ok(count as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NeverCancel;
    use crate::native::WriteSink;

    /// Every test uses this one sink type, so their paths share one
    /// instantiation of the generic writer.
    fn vec_sink() -> WriteSink<Vec<u8>> {
        WriteSink::new(Vec::new())
    }

    #[test]
    fn preflighted_objects_do_not_mint_ids_and_overflow_is_recoverable() {
        let mut sink = vec_sink();
        let limits = Limits::default();
        let mut writer = PdfWriter::new(&mut sink, &limits, &NeverCancel).unwrap();
        let before = writer.position();
        writer.prepare_objects(5).unwrap();
        assert!(writer.offsets.is_empty());
        assert_eq!(writer.position(), before);
        assert_eq!(writer.reserve_object().unwrap().number(), 1);
        assert!(matches!(
            writer.prepare_objects(usize::MAX),
            Err(Error::InvalidInput { .. })
        ));
        assert_eq!(writer.offsets.len(), 1);
        assert_eq!(writer.position(), before);
        assert_eq!(writer.reserve_object().unwrap().number(), 2);
    }

    #[test]
    fn classic_output_ends_within_ten_digit_offsets() {
        assert!(check_classic_pdf_bytes(MAX_CLASSIC_PDF_BYTES).is_ok());
        assert!(matches!(
            check_classic_pdf_bytes(MAX_CLASSIC_PDF_BYTES + 1),
            Err(Error::LimitExceeded {
                resource: "classic PDF file bytes",
                limit: MAX_CLASSIC_PDF_BYTES,
                attempted,
            }) if attempted == MAX_CLASSIC_PDF_BYTES + 1
        ));
    }

    #[test]
    fn object_count_stays_within_annex_c_reader_profile() {
        assert_eq!(
            checked_object_number(MAX_PDF_OBJECTS as usize).unwrap(),
            MAX_PDF_OBJECTS
        );
        assert!(matches!(
            checked_object_number(MAX_PDF_OBJECTS as usize + 1),
            Err(Error::LimitExceeded {
                resource: "PDF object count",
                limit: 8_388_607,
                attempted: 8_388_608
            })
        ));
    }

    #[test]
    fn output_ceiling_is_checked_before_writing() {
        (|| {
            let mut sink = vec_sink();
            let limits = Limits::default();
            let mut pdf = PdfWriter::new(&mut sink, &limits, &NeverCancel)?;
            let object = pdf.reserve_object()?;
            pdf.begin_object(object)?;
            pdf.set_position_for_test(MAX_CLASSIC_PDF_BYTES);
            assert!(matches!(
                pdf.write_bytes(b"x"),
                Err(Error::LimitExceeded {
                    resource: "classic PDF file bytes",
                    ..
                })
            ));
            Ok::<(), Error>(())
        })()
        .unwrap();
    }

    #[test]
    fn stream_integer_limit_is_checked_before_writing() {
        (|| {
            let mut sink = vec_sink();
            let limits = Limits::default();
            let mut pdf = PdfWriter::new(&mut sink, &limits, &NeverCancel)?;
            let stream = pdf.reserve_object()?;
            let length = pdf.reserve_object()?;
            pdf.begin_stream(stream, length, b"")?;
            assert!(
                matches!(pdf.state, State::Stream { .. }),
                "stream should be open"
            );
            if let State::Stream { data_start, .. } = pdf.state {
                pdf.set_position_for_test(data_start + MAX_PDF_INTEGER);
            }
            assert!(matches!(
                pdf.write_stream_bytes(b"x"),
                Err(Error::LimitExceeded {
                    resource: "PDF stream length",
                    ..
                })
            ));
            Ok::<(), Error>(())
        })()
        .unwrap();
    }

    #[test]
    fn stream_bytes_need_an_open_stream_and_are_written_verbatim() {
        let mut sink = vec_sink();
        (|| {
            let limits = Limits::default();
            let mut pdf = PdfWriter::new(&mut sink, &limits, &NeverCancel)?;
            assert!(matches!(
                pdf.write_stream_bytes(b"x"),
                Err(Error::InvalidInput {
                    reason: "no PDF stream is open"
                })
            ));
            let stream = pdf.reserve_object()?;
            let length = pdf.reserve_object()?;
            pdf.begin_stream(stream, length, b"")?;
            let start = pdf.position();
            pdf.write_stream_bytes(b"endstream\0")?;
            assert_eq!(pdf.position(), start + 10);
            Ok::<(), Error>(())
        })()
        .unwrap();
        assert!(sink.into_inner().ends_with(b"stream\nendstream\0"));
    }

    #[test]
    fn minimal_pdf_byte_count_includes_xref_and_trailer() {
        let mut sink = vec_sink();
        let written = (|| {
            let limits = Limits::default();
            let mut pdf = PdfWriter::new(&mut sink, &limits, &NeverCancel)?;
            let catalog = pdf.reserve_object()?;
            pdf.write_object(catalog, b"<< >>")?;
            assert_eq!(pdf.position(), 15 + 21);
            pdf.finish(catalog)
        })()
        .unwrap();
        let output = sink.into_inner();
        assert_eq!(written, output.len() as u64);
        let xref = output
            .windows(b"\nxref\n".len())
            .position(|window| window == b"\nxref\n")
            .unwrap()
            + 1;
        assert!(output.ends_with(
            format!(
                "\nxref\n0 2\n0000000000 65535 f \n0000000015 00000 n \ntrailer\n<< /Size 2 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n"
            )
            .as_bytes()
        ));
    }

    #[test]
    fn finish_requires_a_written_catalog() {
        (|| {
            let mut sink = vec_sink();
            let limits = Limits::default();
            let mut pdf = PdfWriter::new(&mut sink, &limits, &NeverCancel)?;
            let catalog = pdf.reserve_object()?;
            assert!(matches!(
                pdf.finish(catalog),
                Err(Error::InvalidInput {
                    reason: "PDF catalog object has not been written"
                })
            ));
            Ok::<(), Error>(())
        })()
        .unwrap();
    }

    #[test]
    fn finish_requires_every_reserved_object_and_fits_the_output_limit() {
        let mut sink = vec_sink();
        let limits = Limits::default();
        (|| {
            let mut pdf = PdfWriter::new(&mut sink, &limits, &NeverCancel)?;
            let catalog = pdf.reserve_object()?;
            pdf.reserve_object()?;
            pdf.write_object(catalog, b"<< >>")?;
            assert!(matches!(
                pdf.finish(catalog),
                Err(Error::InvalidInput {
                    reason: "a reserved PDF object has not been written"
                })
            ));
            Ok::<(), Error>(())
        })()
        .unwrap();

        let mut sink = vec_sink();
        let limits = Limits {
            max_output_bytes: 100,
            ..Limits::default()
        };
        let written = (|| {
            let mut pdf = PdfWriter::new(&mut sink, &limits, &NeverCancel)?;
            let catalog = pdf.reserve_object()?;
            pdf.write_object(catalog, b"<< >>")?;
            let written = pdf.position();
            assert!(matches!(
                pdf.finish(catalog),
                Err(Error::LimitExceeded {
                    resource: "output bytes",
                    limit: 100,
                    attempted,
                }) if attempted > 100
            ));
            Ok::<u64, Error>(written)
        })()
        .unwrap();
        assert_eq!(sink.into_inner().len() as u64, written);
    }

    #[test]
    fn the_xref_preflight_length_matches_the_written_table() {
        let mut sink = vec_sink();
        let limits = Limits::default();
        let (start, written) = (|| {
            let mut pdf = PdfWriter::new(&mut sink, &limits, &NeverCancel)?;
            let catalog = pdf.reserve_object()?;
            let info = pdf.reserve_object()?;
            pdf.write_object(catalog, b"<< >>")?;
            pdf.write_object(info, b"<< >>")?;
            let start = pdf.position();
            Ok::<_, Error>((start, pdf.finish_with_info(catalog, Some(info))?))
        })()
        .unwrap();
        let trailer = Trailer {
            size: 3,
            root: ObjectId(1).into(),
            prev: None,
            info: Some(ObjectId(2).into()),
            id: None,
        };
        assert_eq!(dense_xref_len(&trailer, start), Some(written - start));
    }

    #[test]
    fn info_reference_must_be_reserved() {
        (|| {
            let mut sink = vec_sink();
            let limits = Limits::default();
            let mut pdf = PdfWriter::new(&mut sink, &limits, &NeverCancel)?;
            let catalog = pdf.reserve_object()?;
            pdf.write_object(catalog, b"<< /Type /Catalog >>")?;
            assert!(matches!(
                pdf.finish_with_info(catalog, Some(ObjectId(catalog.number() + 1))),
                Err(Error::InvalidInput {
                    reason: "PDF object number was not reserved"
                })
            ));
            Ok::<(), Error>(())
        })()
        .unwrap();
    }

    #[test]
    fn unreserved_object_numbers_are_rejected() {
        (|| {
            let mut sink = vec_sink();
            let limits = Limits::default();
            let mut pdf = PdfWriter::new(&mut sink, &limits, &NeverCancel)?;
            let reserved = pdf.reserve_object()?;
            let before = pdf.position();
            assert!(matches!(
                pdf.begin_object(ObjectId(reserved.number() + 1)),
                Err(Error::InvalidInput {
                    reason: "PDF object number was not reserved"
                })
            ));
            assert_eq!(pdf.position(), before);
            Ok::<(), Error>(())
        })()
        .unwrap();
    }

    #[test]
    fn xref_and_trailer_are_checked_against_the_classic_ceiling() {
        (|| {
            let mut sink = vec_sink();
            let limits = Limits {
                max_output_bytes: u64::MAX,
                ..Limits::default()
            };
            let mut pdf = PdfWriter::new(&mut sink, &limits, &NeverCancel)?;
            let catalog = pdf.reserve_object()?;
            pdf.write_object(catalog, b"<< >>")?;
            pdf.set_position_for_test(MAX_CLASSIC_PDF_BYTES - 20);
            assert!(matches!(
                pdf.finish(catalog),
                Err(Error::LimitExceeded {
                    resource: "classic PDF file bytes",
                    limit: MAX_CLASSIC_PDF_BYTES,
                    attempted,
                }) if attempted > MAX_CLASSIC_PDF_BYTES
            ));
            Ok::<(), Error>(())
        })()
        .unwrap();
    }

    #[test]
    fn object_zero_cannot_be_referenced() {
        (|| {
            let mut sink = vec_sink();
            let limits = Limits::default();
            let pdf = PdfWriter::new(&mut sink, &limits, &NeverCancel)?;
            assert!(matches!(
                pdf.index(ObjectId(0)),
                Err(Error::InvalidInput { .. })
            ));
            Ok::<(), Error>(())
        })()
        .unwrap();
    }
}
