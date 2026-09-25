// SPDX-License-Identifier: MIT

//! Bounded object scanning for headerless CAJ PDF fragments.

use super::{ObjectTail, Reader, exact_unsigned};
use crate::pdf::writer::MAX_PDF_OBJECTS;
use crate::pdf::{FragmentObject, PdfRange, PdfRef};
use crate::{Cancellation, Error, Limits, PdfErrorKind, RangedSource, Result};

/// An equal-width correction to an observed, understated direct `/Length`.
/// The PDF bytes remain at their original offsets when the patch is applied.
#[derive(Clone, Debug)]
pub(crate) struct LengthPatch {
    pub(super) offset: u64,
    pub(super) original: Vec<u8>,
    pub(super) replacement: Vec<u8>,
}

/// Validated object boundaries within a headerless PDF fragment. The CAJ
/// page table supplies the minimum body end and page order, never object spans.
pub(crate) struct FragmentScan {
    pub objects: Vec<FragmentObject>,
    pub patches: Vec<LengthPatch>,
}

/// Apply verified same-width stream length repairs while forwarding ranged
/// reads. This adapter never buffers an object or stream payload.
pub(crate) struct PatchedSource<'a, S> {
    source: &'a mut S,
    patches: &'a [LengthPatch],
}

impl<'a, S> PatchedSource<'a, S> {
    pub fn new(source: &'a mut S, patches: &'a [LengthPatch]) -> Self {
        Self { source, patches }
    }
}

impl<S: RangedSource> RangedSource for PatchedSource<'_, S> {
    fn size(&self) -> u64 {
        self.source.size()
    }

    async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
        let count = self.source.read_at(offset, destination).await?;
        if count > destination.len() {
            return Err(Error::InvalidInput {
                reason: "source reported more bytes than requested",
            });
        }
        let end = offset.saturating_add(count as u64);
        // The fragment scan records patches in ascending source order. Most
        // reads overlap no patch, so seek directly to the first possible one.
        let first_patch = self.patches.partition_point(|patch| {
            patch.offset.saturating_add(patch.replacement.len() as u64) <= offset
        });
        for patch in &self.patches[first_patch..] {
            if patch.offset >= end {
                break;
            }
            let patch_end = patch.offset.saturating_add(patch.replacement.len() as u64);
            let first = offset.max(patch.offset);
            let last = end.min(patch_end);
            if first < last {
                let source_start = (first - patch.offset) as usize;
                let target_start = (first - offset) as usize;
                let length = (last - first) as usize;
                if destination[target_start..target_start + length]
                    != patch.original[source_start..source_start + length]
                {
                    return Err(Error::Pdf {
                        offset: first,
                        object: None,
                        kind: PdfErrorKind::Malformed,
                        reason: "source changed after stream Length validation",
                    });
                }
                destination[target_start..target_start + length]
                    .copy_from_slice(&patch.replacement[source_start..source_start + length]);
            }
        }
        Ok(count)
    }
}

const MAX_FRAGMENT_TAIL_EXTENSION: u64 = 64 * 1024;
const MAX_STREAM_LENGTH_REPAIR: u64 = 64;

/// Scan indirect objects with the existing PDF syntax parser, advancing over
/// stream payloads by `/Length` rather than searching them for object markers.
/// A narrowly bounded repair accepts a unique nearby `endstream`/`endobj`
/// delimiter when a direct length is understated. An ambiguous marker is an
/// error. Bytes after the complete final object are excluded from the plan.
pub(crate) async fn scan_fragment_objects<S: RangedSource, C: Cancellation>(
    source: &mut S,
    body_start: u64,
    minimum_end: u64,
    limits: &Limits,
    cancellation: &C,
) -> Result<FragmentScan> {
    limits.validate()?;
    if body_start >= minimum_end || minimum_end > source.size() {
        return Err(Error::Caj {
            offset: body_start,
            record: None,
            reason: "CAJ PDF fragment body range is invalid",
        });
    }
    let minimum_length = minimum_end - body_start;
    limits
        .check_input_size(minimum_length)
        .map_err(|error| match error {
            Error::LimitExceeded {
                resource,
                limit,
                attempted,
            } => Error::CajLimitExceeded {
                offset: body_start,
                record: None,
                resource,
                limit,
                attempted,
            },
            other => other,
        })?;
    let scan_end = minimum_end
        .saturating_add(MAX_FRAGMENT_TAIL_EXTENSION)
        .min(source.size());
    let range = PdfRange {
        offset: body_start,
        length: scan_end - body_start,
    };
    let mut reader = Reader::new(source, range, limits, cancellation)?;
    let mut objects = Vec::new();
    let mut patches = Vec::new();
    let mut cursor = 0_u64;
    let mut final_object_repaired = false;
    let minimum_relative = minimum_end - body_start;
    let logical_end = loop {
        reader.skip_space(&mut cursor).await?;
        if cursor >= minimum_relative {
            break cursor;
        }
        if cursor >= range.length {
            return Err(reader.problem(
                cursor,
                None,
                PdfErrorKind::Malformed,
                "CAJ PDF fragment ends before page table body end",
            ));
        }
        let start = cursor;
        let head = reader.load_head(start, None).await?;
        let mut object_repaired = false;
        let reference = head.reference;
        if reference.generation != 0 {
            return Err(reader.problem(
                start,
                Some(reference),
                PdfErrorKind::UnsupportedFeature,
                "CAJ PDF fragment has a nonzero object generation",
            ));
        }
        let end = match head.tail {
            ObjectTail::EndObject { end } => start.checked_add(end as u64),
            ObjectTail::Stream { data_start } => {
                let dictionary = head.dictionary.as_ref().ok_or_else(|| {
                    reader.problem(
                        start,
                        Some(reference),
                        PdfErrorKind::Malformed,
                        "stream has no dictionary",
                    )
                })?;
                let entry = dictionary.entry(b"Length").ok_or_else(|| {
                    reader.problem(
                        start,
                        Some(reference),
                        PdfErrorKind::Malformed,
                        "stream lacks Length",
                    )
                })?;
                let value = entry.value(&dictionary.bytes);
                let length = exact_unsigned(value).ok_or_else(|| {
                    reader.problem(
                        start,
                        Some(reference),
                        PdfErrorKind::UnsupportedFeature,
                        "CAJ fragment stream requires a direct Length",
                    )
                })?;
                let data_at = start.checked_add(data_start as u64).ok_or_else(|| {
                    reader.problem(
                        start,
                        Some(reference),
                        PdfErrorKind::Malformed,
                        "stream offset overflows",
                    )
                })?;
                let after_data = data_at.checked_add(length).ok_or_else(|| {
                    reader.problem(
                        data_at,
                        Some(reference),
                        PdfErrorKind::Malformed,
                        "stream extent overflows",
                    )
                })?;
                let end = match reader.check_stream_tail(after_data, Some(reference)).await {
                    Ok(end) => end,
                    Err(Error::Pdf {
                        kind: PdfErrorKind::Malformed,
                        ..
                    }) => {
                        let (corrected_length, corrected_end) =
                            repair_stream_length(&mut reader, after_data, data_at, reference)
                                .await?;
                        let original = value.to_vec();
                        let replacement = corrected_length.to_string().into_bytes();
                        if original.len() != replacement.len() {
                            return Err(reader.problem(
                                after_data,
                                Some(reference),
                                PdfErrorKind::UnsupportedFeature,
                                "stream Length repair changes PDF object width",
                            ));
                        }
                        let dictionary_start = head.dictionary_start.ok_or_else(|| {
                            reader.problem(
                                start,
                                Some(reference),
                                PdfErrorKind::Malformed,
                                "stream dictionary offset is missing",
                            )
                        })?;
                        let patch_offset = body_start
                            .checked_add(start)
                            .and_then(|n| n.checked_add(dictionary_start as u64))
                            .and_then(|n| n.checked_add(entry.value.start as u64))
                            .ok_or(Error::InvalidInput {
                                reason: "stream Length offset overflows",
                            })?;
                        patches.push(LengthPatch {
                            offset: patch_offset,
                            original,
                            replacement,
                        });
                        object_repaired = true;
                        corrected_end
                    }
                    Err(other) => return Err(other),
                };
                Some(end)
            }
        }
        .ok_or_else(|| {
            reader.problem(
                start,
                Some(reference),
                PdfErrorKind::Malformed,
                "fragment object end overflows",
            )
        })?;
        if end <= start || end > range.length {
            return Err(reader.problem(
                start,
                Some(reference),
                PdfErrorKind::Malformed,
                "fragment object extends beyond the bounded body",
            ));
        }
        let count = objects.len().saturating_add(1);
        if count > MAX_PDF_OBJECTS as usize {
            return Err(reader.locate_limit(
                start,
                Some(reference),
                Error::LimitExceeded {
                    resource: "PDF fragment objects",
                    limit: MAX_PDF_OBJECTS as u64,
                    attempted: count as u64,
                },
            ));
        }
        let allocation = (count as u64)
            .saturating_mul(std::mem::size_of::<FragmentObject>() as u64)
            .saturating_add(
                (patches.len() as u64)
                    .saturating_mul(std::mem::size_of::<LengthPatch>() as u64 + 40),
            );
        limits
            .check_allocation(allocation)
            .map_err(|error| reader.locate_limit(start, Some(reference), error))?;
        objects.try_reserve(1).map_err(|_| {
            reader.locate_limit(
                start,
                Some(reference),
                Error::LimitExceeded {
                    resource: "PDF fragment object index allocation",
                    limit: limits.max_allocation_bytes,
                    attempted: allocation,
                },
            )
        })?;
        objects.push(FragmentObject {
            reference,
            range: PdfRange {
                offset: body_start + start,
                length: end - start,
            },
        });
        cursor = end;
        final_object_repaired = object_repaired;
        if end >= minimum_relative {
            break end;
        }
    };
    if objects.is_empty() {
        return Err(reader.problem(
            0,
            None,
            PdfErrorKind::Malformed,
            "CAJ PDF fragment has no indirect objects",
        ));
    }
    if final_object_repaired {
        // A repaired final stream can otherwise stop at a fake terminator in
        // its own data once that candidate passes the page-table end hint.
        // The only observed such repair ends immediately before an XML CAJ
        // trailer. Unknown trailing bytes remain an ambiguous repair.
        let mut tail = logical_end;
        reader.skip_space(&mut tail).await?;
        if body_start.saturating_add(tail) < reader.source.size()
            && (tail.saturating_add(5) > range.length
                || reader.bytes(tail, 5).await?.as_slice() != b"<?xml")
        {
            return Err(reader.problem(
                tail,
                None,
                PdfErrorKind::AmbiguousRepair,
                "repaired final stream has an unrecognized CAJ trailer",
            ));
        }
    }
    let actual_length = logical_end.saturating_sub(0);
    limits
        .check_input_size(actual_length)
        .map_err(|error| reader.locate_limit(logical_end, None, error))?;
    Ok(FragmentScan { objects, patches })
}

async fn repair_stream_length<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    declared_after: u64,
    data_at: u64,
    reference: PdfRef,
) -> Result<(u64, u64)> {
    let last = declared_after
        .saturating_add(MAX_STREAM_LENGTH_REPAIR)
        .min(reader.range.length.saturating_sub(9));
    let mut found = None;
    for marker in declared_after.saturating_add(1)..=last {
        if reader.bytes(marker, 9).await?.as_slice() != b"endstream" {
            continue;
        }
        let after = if marker >= 2
            && reader.byte(marker - 2).await? == Some(b'\r')
            && reader.byte(marker - 1).await? == Some(b'\n')
        {
            marker - 2
        } else if matches!(reader.byte(marker - 1).await?, Some(b'\r' | b'\n')) {
            marker - 1
        } else {
            marker
        };
        if after <= declared_after || after < data_at {
            continue;
        }
        let end = match reader.check_stream_tail(after, Some(reference)).await {
            Ok(end) => end,
            Err(Error::Pdf {
                kind: PdfErrorKind::Malformed,
                ..
            }) => continue,
            Err(other) => return Err(other),
        };
        if found.is_some() {
            return Err(reader.problem(
                marker,
                Some(reference),
                PdfErrorKind::AmbiguousRepair,
                "multiple nearby stream terminators match an understated Length",
            ));
        }
        found = Some((after - data_at, end));
    }
    found.ok_or_else(|| {
        reader.problem(
            declared_after,
            Some(reference),
            PdfErrorKind::Malformed,
            "stream Length has no unique bounded repair",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::SeekableSource;
    use crate::{NeverCancel, read_exact_at};
    use std::future::Future;
    use std::io::{self, Cursor};
    use std::pin::pin;
    use std::task::{Context, Poll, Waker};

    fn run<F: Future>(future: F) -> F::Output {
        let mut context = Context::from_waker(Waker::noop());
        let mut future = pin!(future);
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("in-memory fragment source yielded unexpectedly"),
        }
    }

    /// A source whose bytes from `unreadable_from` onward fail with an I/O
    /// error, as a truncated network range or failing disk sector would.
    struct UnreadableTail {
        bytes: Vec<u8>,
        unreadable_from: u64,
    }

    impl RangedSource for UnreadableTail {
        fn size(&self) -> u64 {
            self.bytes.len() as u64
        }

        async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
            if offset >= self.unreadable_from {
                return Err(Error::Io(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "injected unreadable fragment tail",
                )));
            }
            let start = offset as usize;
            let end = (self.unreadable_from.min(self.bytes.len() as u64) as usize)
                .min(start + destination.len());
            destination[..end - start].copy_from_slice(&self.bytes[start..end]);
            Ok(end - start)
        }
    }

    fn one_byte_reads() -> Limits {
        Limits {
            io_chunk_bytes: 1,
            ..Limits::default()
        }
    }

    fn expect_injected_io(result: Result<FragmentScan>) {
        match result {
            Err(Error::Io(error)) => {
                assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
                assert_eq!(error.to_string(), "injected unreadable fragment tail");
            }
            Err(other) => panic!("unexpected error: {other:?}"),
            Ok(_) => panic!("unreadable fragment bytes were accepted"),
        }
    }

    #[test]
    fn repair_skips_terminators_at_the_declared_end_or_without_endobj() {
        // The first candidate sits exactly at the declared end, the second
        // lacks `endobj`, and only the LF-separated third one is complete.
        let payload = b"0123456789\nendstream junk\nendstream junk2";
        let mut bytes = b"1 0 obj\n<< /Length 10 >>\nstream\n".to_vec();
        let data_at = bytes.len() as u64;
        bytes.extend_from_slice(payload);
        bytes.extend_from_slice(b"\nendstream\nendobj");
        let end = bytes.len() as u64;
        let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
        let scan = run(scan_fragment_objects(
            &mut source,
            0,
            end,
            &Limits::default(),
            &NeverCancel,
        ))
        .unwrap();
        assert_eq!(scan.objects.len(), 1);
        assert_eq!(scan.objects[0].range.length, end);
        assert_eq!(scan.patches.len(), 1);
        let patch = &scan.patches[0];
        assert_eq!(patch.original, b"10");
        assert_eq!(patch.replacement, payload.len().to_string().into_bytes());
        assert!(patch.offset < data_at);

        let mut patched = PatchedSource::new(&mut source, &scan.patches);
        let mut length = [0_u8; 2];
        run(read_exact_at(
            &mut patched,
            patch.offset,
            &mut length,
            &Limits::default(),
            &NeverCancel,
        ))
        .unwrap();
        assert_eq!(&length, b"41");
    }

    #[test]
    fn repair_accepts_a_terminator_without_a_preceding_end_of_line() {
        let mut bytes = b"1 0 obj\n<< /Length 10 >>\nstream\n0123456789ab".to_vec();
        bytes.extend_from_slice(b"endstream\nendobj");
        let end = bytes.len() as u64;
        let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
        let scan = run(scan_fragment_objects(
            &mut source,
            0,
            end,
            &Limits::default(),
            &NeverCancel,
        ))
        .unwrap();
        assert_eq!(scan.objects[0].range.length, end);
        assert_eq!(scan.patches.len(), 1);
        assert_eq!(scan.patches[0].replacement, b"12");
    }

    #[test]
    fn source_failure_after_the_declared_stream_extent_is_not_repaired() {
        let mut bytes = b"1 0 obj\n<< /Length 2000 >>\nstream\n".to_vec();
        let data_at = bytes.len() as u64;
        bytes.extend(std::iter::repeat_n(b'x', 2000));
        bytes.extend_from_slice(b"\nendstream\nendobj");
        let size = bytes.len() as u64;
        let mut source = UnreadableTail {
            bytes,
            unreadable_from: data_at + 2000,
        };
        expect_injected_io(run(scan_fragment_objects(
            &mut source,
            0,
            size,
            &one_byte_reads(),
            &NeverCancel,
        )));
    }

    #[test]
    fn source_failure_while_checking_a_repair_candidate_is_propagated() {
        let mut bytes = b"1 0 obj\n<< /Length 1000 >>\nstream\n".to_vec();
        bytes.extend(std::iter::repeat_n(b'x', 1002));
        let marker = bytes.len() as u64 + 1;
        bytes.extend_from_slice(b"\nendstream\nendobj");
        let size = bytes.len() as u64;
        let mut source = UnreadableTail {
            bytes,
            unreadable_from: marker + 9,
        };
        expect_injected_io(run(scan_fragment_objects(
            &mut source,
            0,
            size,
            &one_byte_reads(),
            &NeverCancel,
        )));

        // The same bytes repair cleanly once the tail is readable.
        source.unreadable_from = u64::MAX;
        let scan = run(scan_fragment_objects(
            &mut source,
            0,
            size,
            &one_byte_reads(),
            &NeverCancel,
        ))
        .unwrap();
        assert_eq!(scan.patches.len(), 1);
        assert_eq!(scan.patches[0].original, b"1000");
        assert_eq!(scan.patches[0].replacement, b"1002");
    }
}
