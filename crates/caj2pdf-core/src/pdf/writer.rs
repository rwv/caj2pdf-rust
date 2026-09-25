// SPDX-License-Identifier: MIT

//! A small, forward-only PDF 1.7 serializer.
//!
//! This module deliberately does not model arbitrary PDF objects. Callers
//! reserve indirect object numbers, emit each object once, then finish with a
//! classic cross-reference table. Only object offsets are retained in memory.

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

const HEADER: &[u8] = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n";
const FREE_XREF_ENTRY: &[u8; 20] = b"0000000000 65535 f \n";

/// A generation-zero indirect object number reserved by [`PdfWriter`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObjectId(u32);

impl ObjectId {
    /// The number to use in a PDF indirect reference (`number 0 R`).
    pub const fn number(self) -> u32 {
        self.0
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
    sink: &'a mut W,
    limits: &'a Limits,
    cancellation: &'a C,
    position: u64,
    /// Zero means reserved but not emitted; the PDF header makes zero an
    /// impossible offset for a real object.
    offsets: Vec<u64>,
    state: State,
    poisoned: bool,
}

impl<'a, W: SequentialSink, C: Cancellation> PdfWriter<'a, W, C> {
    /// Start a PDF 1.7 file, including its binary-content marker.
    pub async fn new(sink: &'a mut W, limits: &'a Limits, cancellation: &'a C) -> Result<Self> {
        limits.validate()?;
        let mut writer = Self {
            sink,
            limits,
            cancellation,
            position: 0,
            offsets: Vec::new(),
            state: State::Idle,
            poisoned: false,
        };
        writer.write_raw(HEADER).await?;
        Ok(writer)
    }

    /// Number of bytes accepted by the sink so far.
    pub const fn position(&self) -> u64 {
        self.position
    }

    /// Reserve one generation-zero object number before writing its body.
    ///
    /// The object index requests capacity within
    /// `Limits::max_allocation_bytes`; no object content is stored in it.
    pub fn reserve_object(&mut self) -> Result<ObjectId> {
        self.ensure_healthy()?;
        let next_count = self
            .offsets
            .len()
            .checked_add(1)
            .ok_or(Error::InvalidInput {
                reason: "PDF object count overflows address space",
            })?;
        let number = checked_object_number(next_count)?;
        let max_slots = (self.limits.max_allocation_bytes / size_of::<u64>() as u64)
            .min(u64::from(MAX_PDF_OBJECTS));
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
        self.limits.check_allocation(bytes)?;
        if next_count > self.offsets.capacity() {
            let refused = self
                .limits
                .allocation_refused("PDF object index allocation", bytes);
            let additional = next_capacity - self.offsets.len();
            reserve_exact(&mut self.offsets, additional, refused)?;
        }
        self.offsets.push(0);
        Ok(ObjectId(number))
    }

    /// Begin one previously reserved object. Follow with `write_bytes` and
    /// `end_object`; each object must be emitted exactly once.
    pub async fn begin_object(&mut self, id: ObjectId) -> Result<()> {
        self.ensure_idle()?;
        let index = self.unwritten_index(id)?;
        let offset = self.position;
        let header = format!("{} 0 obj\n", id.number());
        self.write_raw(header.as_bytes()).await?;
        self.offsets[index] = offset;
        self.state = State::Object;
        Ok(())
    }

    /// Emit bytes inside an ordinary indirect object. Large slices are split
    /// into configured chunks before reaching the sink.
    pub async fn write_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.ensure_healthy()?;
        if self.state != State::Object {
            return Err(Error::InvalidInput {
                reason: "PDF bytes require an open ordinary object",
            });
        }
        self.write_raw(bytes).await
    }

    /// Close an ordinary indirect object.
    pub async fn end_object(&mut self) -> Result<()> {
        self.ensure_healthy()?;
        if self.state != State::Object {
            return Err(Error::InvalidInput {
                reason: "no ordinary PDF object is open",
            });
        }
        self.write_raw(b"\nendobj\n").await?;
        self.state = State::Idle;
        Ok(())
    }

    /// Convenience for one small plain object body.
    pub async fn write_object(&mut self, id: ObjectId, body: &[u8]) -> Result<()> {
        self.begin_object(id).await?;
        self.write_bytes(body).await?;
        self.end_object().await
    }

    /// Begin an unknown-length stream and put an indirect `/Length` reference
    /// in its dictionary. `dictionary_entries` contains only inner entries,
    /// excluding `/Length` and the surrounding `<<` and `>>` delimiters.
    /// The length object is emitted by `end_stream` after the payload.
    pub async fn begin_stream(
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
        self.begin_object(stream_id).await?;
        self.write_bytes(b"<<\n/Length ").await?;
        self.write_bytes(length_id.number().to_string().as_bytes())
            .await?;
        self.write_bytes(b" 0 R\n").await?;
        self.write_bytes(dictionary_entries).await?;
        if !dictionary_entries.is_empty() && !dictionary_entries.ends_with(b"\n") {
            self.write_bytes(b"\n").await?;
        }
        self.write_bytes(b">>\nstream\n").await?;
        self.state = State::Stream {
            length_id,
            data_start: self.position,
        };
        Ok(())
    }

    /// Write raw stream payload, unchanged. Delimiter-like binary bytes are
    /// safe because `/Length` records this exact payload byte count.
    pub async fn write_stream_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.ensure_healthy()?;
        let State::Stream { data_start, .. } = self.state else {
            return Err(Error::InvalidInput {
                reason: "no PDF stream is open",
            });
        };
        let current_length = self
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
        self.write_raw(bytes).await
    }

    /// Close a stream and write its measured length as a separate indirect
    /// object. The separator newline after the payload is not in `/Length`.
    pub async fn end_stream(&mut self) -> Result<()> {
        self.ensure_healthy()?;
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
            .position
            .checked_sub(data_start)
            .ok_or(Error::InvalidInput {
                reason: "PDF stream position moved backwards",
            })?;
        self.write_raw(b"\nendstream\nendobj\n").await?;
        self.state = State::Idle;
        self.write_object(length_id, length.to_string().as_bytes())
            .await
    }

    /// Emit the classic cross-reference table and trailer, flush the sink,
    /// and return the final byte count. Every reserved object must have been
    /// written exactly once, including any stream length object.
    pub async fn finish(mut self, root_id: ObjectId) -> Result<u64> {
        self.ensure_idle()?;
        let root_index = self.index(root_id)?;
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
        let size = self
            .offsets
            .len()
            .checked_add(1)
            .ok_or(Error::InvalidInput {
                reason: "PDF cross-reference size overflows address space",
            })?;
        let xref_offset = self.position;
        let xref_header = format!("xref\n0 {size}\n");
        let trailer = format!(
            "trailer\n<< /Size {size} /Root {} 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n",
            root_id.number()
        );
        let xref_bytes = u64::try_from(size)
            .ok()
            .and_then(|count| count.checked_mul(20))
            .ok_or(Error::InvalidInput {
                reason: "PDF cross-reference byte count overflows 64 bits",
            })?;
        let tail_bytes = u64::try_from(xref_header.len() + trailer.len())
            .ok()
            .and_then(|overhead| overhead.checked_add(xref_bytes))
            .ok_or(Error::InvalidInput {
                reason: "PDF trailer byte count overflows 64 bits",
            })?;
        let final_size = xref_offset
            .checked_add(tail_bytes)
            .ok_or(Error::InvalidInput {
                reason: "PDF output byte count overflows 64 bits",
            })?;
        if final_size > MAX_CLASSIC_PDF_BYTES {
            return Err(Error::LimitExceeded {
                resource: "classic PDF file bytes",
                limit: MAX_CLASSIC_PDF_BYTES,
                attempted: final_size,
            });
        }
        if final_size > self.limits.max_output_bytes {
            return Err(Error::LimitExceeded {
                resource: "output bytes",
                limit: self.limits.max_output_bytes,
                attempted: final_size,
            });
        }
        self.write_raw(xref_header.as_bytes()).await?;
        self.write_raw(FREE_XREF_ENTRY).await?;
        for index in 0..self.offsets.len() {
            let offset = self.offsets[index];
            let entry = xref_entry(offset)?;
            self.write_raw(&entry).await?;
        }
        self.write_raw(trailer.as_bytes()).await?;
        // Check cancellation on both sides of flush, as with the core copy
        // operation. A flush can itself await sink backpressure.
        self.write_raw(b"").await?;
        self.sink.flush().await?;
        self.write_raw(b"").await?;
        Ok(self.position)
    }

    fn ensure_healthy(&self) -> Result<()> {
        if self.poisoned {
            Err(Error::InvalidInput {
                reason: "PDF writer cannot continue after a sink failure",
            })
        } else {
            Ok(())
        }
    }

    fn ensure_idle(&self) -> Result<()> {
        self.ensure_healthy()?;
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

    async fn write_raw(&mut self, bytes: &[u8]) -> Result<()> {
        self.ensure_healthy()?;
        let length = len_u64(bytes.len());
        let attempted = self
            .position
            .checked_add(length)
            .ok_or(Error::InvalidInput {
                reason: "PDF output byte count overflows 64 bits",
            })?;
        if attempted > MAX_CLASSIC_PDF_BYTES {
            return Err(Error::LimitExceeded {
                resource: "classic PDF file bytes",
                limit: MAX_CLASSIC_PDF_BYTES,
                attempted,
            });
        }
        if let Err(error) = write_all(
            self.sink,
            bytes,
            &mut self.position,
            self.limits,
            self.cancellation,
        )
        .await
        {
            self.poisoned = true;
            return Err(error);
        }
        Ok(())
    }
}

/// Fixed-width entry for a generation-zero, in-use classic xref object.
fn xref_entry(offset: u64) -> Result<[u8; 20]> {
    if offset > MAX_CLASSIC_PDF_BYTES {
        return Err(Error::LimitExceeded {
            resource: "classic PDF object offset",
            limit: MAX_CLASSIC_PDF_BYTES,
            attempted: offset,
        });
    }
    let mut entry = *b"0000000000 00000 n \n";
    let mut remainder = offset;
    for digit in entry[..10].iter_mut().rev() {
        *digit = b'0' + (remainder % 10) as u8;
        remainder /= 10;
    }
    Ok(entry)
}

fn checked_object_number(count: usize) -> Result<u32> {
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
    use crate::test_support::run;

    struct CountSink;

    impl SequentialSink for CountSink {
        async fn write(&mut self, bytes: &[u8]) -> Result<usize> {
            Ok(bytes.len())
        }

        async fn flush(&mut self) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn xref_entry_width_and_offset_ceiling() {
        assert_eq!(xref_entry(0).unwrap(), *b"0000000000 00000 n \n");
        assert_eq!(
            xref_entry(MAX_CLASSIC_PDF_BYTES).unwrap(),
            *b"9999999999 00000 n \n"
        );
        assert!(matches!(
            xref_entry(MAX_CLASSIC_PDF_BYTES + 1),
            Err(Error::LimitExceeded { .. })
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
        run(async {
            let mut sink = CountSink;
            let limits = Limits::default();
            let mut pdf = PdfWriter::new(&mut sink, &limits, &NeverCancel).await?;
            let object = pdf.reserve_object()?;
            pdf.begin_object(object).await?;
            pdf.position = MAX_CLASSIC_PDF_BYTES;
            assert!(matches!(
                pdf.write_bytes(b"x").await,
                Err(Error::LimitExceeded {
                    resource: "classic PDF file bytes",
                    ..
                })
            ));
            Ok::<(), Error>(())
        })
        .unwrap();
    }

    #[test]
    fn stream_integer_limit_is_checked_before_writing() {
        run(async {
            let mut sink = CountSink;
            let limits = Limits::default();
            let mut pdf = PdfWriter::new(&mut sink, &limits, &NeverCancel).await?;
            let stream = pdf.reserve_object()?;
            let length = pdf.reserve_object()?;
            pdf.begin_stream(stream, length, b"").await?;
            let State::Stream { data_start, .. } = pdf.state else {
                panic!("stream should be open")
            };
            pdf.position = data_start + MAX_PDF_INTEGER;
            assert!(matches!(
                pdf.write_stream_bytes(b"x").await,
                Err(Error::LimitExceeded {
                    resource: "PDF stream length",
                    ..
                })
            ));
            Ok::<(), Error>(())
        })
        .unwrap();
    }

    #[test]
    fn minimal_pdf_byte_count_includes_xref_and_trailer() {
        let mut sink = crate::native::WriteSink::new(Vec::new());
        let written = run(async {
            let limits = Limits::default();
            let mut pdf = PdfWriter::new(&mut sink, &limits, &NeverCancel).await?;
            let catalog = pdf.reserve_object()?;
            pdf.write_object(catalog, b"<< >>").await?;
            assert_eq!(pdf.position(), 15 + 21);
            pdf.finish(catalog).await
        })
        .unwrap();
        let output = sink.into_inner();
        assert_eq!(written, output.len() as u64);
        let xref = output
            .windows(b"\nxref\n".len())
            .position(|window| window == b"\nxref\n")
            .unwrap()
            + 1;
        assert!(output.ends_with(format!("\nstartxref\n{xref}\n%%EOF\n").as_bytes()));
    }

    #[test]
    fn finish_requires_a_written_catalog() {
        run(async {
            let mut sink = CountSink;
            let limits = Limits::default();
            let mut pdf = PdfWriter::new(&mut sink, &limits, &NeverCancel).await?;
            let catalog = pdf.reserve_object()?;
            assert!(matches!(
                pdf.finish(catalog).await,
                Err(Error::InvalidInput {
                    reason: "PDF catalog object has not been written"
                })
            ));
            Ok::<(), Error>(())
        })
        .unwrap();
    }

    #[test]
    fn unreserved_object_numbers_are_rejected() {
        run(async {
            let mut sink = CountSink;
            let limits = Limits::default();
            let mut pdf = PdfWriter::new(&mut sink, &limits, &NeverCancel).await?;
            let reserved = pdf.reserve_object()?;
            let before = pdf.position();
            assert!(matches!(
                pdf.begin_object(ObjectId(reserved.number() + 1)).await,
                Err(Error::InvalidInput {
                    reason: "PDF object number was not reserved"
                })
            ));
            assert_eq!(pdf.position(), before);
            Ok::<(), Error>(())
        })
        .unwrap();
    }

    #[test]
    fn xref_and_trailer_are_checked_against_the_classic_ceiling() {
        run(async {
            let mut sink = CountSink;
            let limits = Limits {
                max_output_bytes: u64::MAX,
                ..Limits::default()
            };
            let mut pdf = PdfWriter::new(&mut sink, &limits, &NeverCancel).await?;
            let catalog = pdf.reserve_object()?;
            pdf.write_object(catalog, b"<< >>").await?;
            pdf.position = MAX_CLASSIC_PDF_BYTES - 20;
            assert!(matches!(
                pdf.finish(catalog).await,
                Err(Error::LimitExceeded {
                    resource: "classic PDF file bytes",
                    limit: MAX_CLASSIC_PDF_BYTES,
                    attempted,
                }) if attempted > MAX_CLASSIC_PDF_BYTES
            ));
            Ok::<(), Error>(())
        })
        .unwrap();
    }

    #[test]
    fn object_zero_cannot_be_referenced() {
        run(async {
            let mut sink = CountSink;
            let limits = Limits::default();
            let pdf = PdfWriter::new(&mut sink, &limits, &NeverCancel).await?;
            assert!(matches!(
                pdf.index(ObjectId(0)),
                Err(Error::InvalidInput { .. })
            ));
            Ok::<(), Error>(())
        })
        .unwrap();
    }
}
