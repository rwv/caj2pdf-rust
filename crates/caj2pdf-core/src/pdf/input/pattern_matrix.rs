// SPDX-License-Identifier: MIT

//! Explicit identity for a measured malformed CAJ tiling-pattern Matrix.
//!
//! CAJViewer and Poppler use identity for this invalid array. MuPDF instead
//! substitutes zero for its invalid element; expanding the exponent also
//! changes the measured source rendering. Ordinary PDF syntax stays strict.

use super::fragment_scan::ScannedObject;
use super::parser::{Syntax, exact_name, exact_reference, exact_unsigned, parse_object_head};
use super::recovery::FragmentPatch;
use super::{ObjectTail, Reader, inspect_head};
use crate::pdf::{FragmentObject, PdfRange};
use crate::{Cancellation, ErrorKind, RangedSource, Result};

const MAX_HEAD_BYTES: usize = 512;
const INVALID_NUMBER: &[u8] = b"-5e-006";
const IDENTITY: &[u8] = b"[1 0 0 1 0 0]";

pub(super) fn candidate<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    start: u64,
) -> Result<Option<(ScannedObject, FragmentPatch)>> {
    let remaining = reader.range.length - start;
    let prefix = reader.bytes(start, remaining.min(32) as usize)?;
    let mut syntax = Syntax::new(&prefix);
    let (Ok(number), Ok(0)) = (syntax.unsigned(), syntax.unsigned()) else {
        return Ok(None);
    };
    syntax.skip_space();
    if number == 0 || number > u32::MAX as u64 || syntax.consume_keyword(b"obj") != Ok(true) {
        return Ok(None);
    }
    syntax.skip_space();
    if prefix.get(syntax.pos..syntax.pos + 2) != Some(b"<<") {
        return Ok(None);
    }
    let mut bytes = reader.bytes(start, remaining.min(MAX_HEAD_BYTES as u64) as usize)?;
    if !bytes.starts_with(&prefix) {
        return Err(reader.malformed(start, None, "pattern header changed while reading"));
    }
    let Some(token) = bytes
        .windows(INVALID_NUMBER.len())
        .position(|v| v == INVALID_NUMBER)
    else {
        return Ok(None);
    };
    let Some(begin) = bytes[..token].iter().rposition(|b| *b == b'[') else {
        return Ok(None);
    };
    let Some(close) = bytes[token..].iter().position(|b| *b == b']') else {
        return Ok(None);
    };
    let end = token + close + 1;
    if !array_is(
        &bytes[begin..end],
        &[b"0.72", b"0", b"0", b"-0.719999", INVALID_NUMBER, b"842"],
    ) {
        return Ok(None);
    }
    reader.limits.check_allocation((bytes.len() * 3) as u64)?;
    let original = bytes[begin..end].to_vec();
    bytes[begin..end].fill(b' ');
    bytes[begin..begin + IDENTITY.len()].copy_from_slice(IDENTITY);
    let head = match parse_object_head(bytes) {
        Ok(head) => head,
        Err(issue) if issue.limit.is_some() => return Err(reader.parse_issue(start, None, issue)),
        Err(_) => return Ok(None),
    };
    let ObjectTail::Stream { data_start } = head.tail else {
        return Ok(None);
    };
    let Some(dictionary) = head.dictionary.as_ref() else {
        return Ok(None);
    };
    let Some(matrix) = dictionary.entry(b"Matrix") else {
        return Ok(None);
    };
    // This exact dictionary entry must own the normalized byte span. A
    // lookalike in a string, nested dictionary or stream never qualifies.
    if head.reference.generation != 0
        || dictionary.entries.len() != 11
        || head.dictionary_start.map(|at| at + matrix.value.start) != Some(begin)
        || dictionary.value(b"Type").and_then(exact_name).as_deref() != Some(b"Pattern")
        || dictionary.value(b"Filter").and_then(exact_name).as_deref() != Some(b"FlateDecode")
        || dictionary
            .value(b"Resources")
            .and_then(exact_reference)
            .is_none_or(|reference| reference.number == 0 || reference.generation != 0)
        || !dictionary
            .value(b"BBox")
            .is_some_and(|value| array_is(value, &[b"0", b"0", b"64", b"64"]))
        || [
            (b"Length".as_slice(), 45),
            (b"PaintType", 1),
            (b"PatternType", 1),
            (b"TilingType", 1),
            (b"XStep", 64),
            (b"YStep", 64),
        ]
        .iter()
        .any(|(name, value)| dictionary.value(name).and_then(exact_unsigned) != Some(*value))
    {
        return Ok(None);
    }
    reader.reject_duplicate_names(dictionary, start, Some(head.reference))?;
    let inspection = inspect_head(&head, reader.range, start, reader.limits)?;
    // A Matrix repair cannot combine with an unproved stream boundary or a
    // second Length repair in this same object. Patch order stays monotonic.
    let payload_end = data_start as u64 + 45;
    if payload_end > remaining {
        return Ok(None);
    }
    let stream_end = match reader.check_stream_tail(start + payload_end, Some(head.reference)) {
        Ok(end) => end,
        Err(error) if matches!(error.kind, ErrorKind::Malformed) => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut checked = reader.bytes(start, data_start)?;
    checked[begin..end].copy_from_slice(&head.bytes[begin..end]);
    if checked != head.bytes[..data_start] {
        return Err(reader.malformed(
            start,
            Some(head.reference),
            "pattern header changed after validation",
        ));
    }
    // Recheck the original Matrix too, before the ranged output adapter
    // continues checking these exact source bytes on every later read.
    if reader.bytes(start + begin as u64, original.len())? != original {
        return Err(reader.malformed(
            start,
            Some(head.reference),
            "pattern Matrix changed after validation",
        ));
    }
    let patch = FragmentPatch {
        offset: reader.range.offset + start + begin as u64,
        original,
        replacement: head.bytes[begin..end].to_vec(),
    };
    Ok(Some((
        ScannedObject {
            object: FragmentObject {
                reference: head.reference,
                range: PdfRange {
                    offset: reader.range.offset + start,
                    length: stream_end - start,
                },
            },
            inspection: Ok(inspection),
        },
        patch,
    )))
}

fn array_is(value: &[u8], expected: &[&[u8]]) -> bool {
    let Some(inner) = value.strip_prefix(b"[").and_then(|v| v.strip_suffix(b"]")) else {
        return false;
    };
    inner
        .split(|b| matches!(b, 0 | b'\t' | b'\n' | b'\x0c' | b'\r' | b' '))
        .filter(|v| !v.is_empty())
        .eq(expected.iter().copied())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf::input::recovery::PatchedSource;
    use crate::test_support::CancelAfter;
    use crate::{Error, Limits, NeverCancel};

    fn fixture() -> Vec<u8> {
        let mut bytes = b"7 0 obj\n<< /Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /Length 45 /Filter /FlateDecode /Resources 8 0 R /XStep 64 /YStep 64 /BBox [0 0 64 64] /Matrix [0.72 0 0 -0.719999 -5e-006 842] >>\nstream\n".to_vec();
        bytes.extend_from_slice(&[b'x'; 45]);
        bytes.extend_from_slice(b"\nendstream\nendobj\n");
        bytes
    }

    fn run(
        bytes: &[u8],
        limits: &Limits,
        cancellation: &impl Cancellation,
    ) -> Result<Option<(ScannedObject, FragmentPatch)>> {
        let mut source = bytes;
        let mut reader = Reader::new(
            &mut source,
            PdfRange {
                offset: 0,
                length: bytes.len() as u64,
            },
            limits,
            cancellation,
        )?;
        candidate(&mut reader, 0)
    }

    #[test]
    fn only_the_measured_matrix_changes_and_reads_remain_bounded() {
        let bytes = fixture();
        let limits = Limits {
            io_chunk_bytes: 1,
            ..Limits::default()
        };
        let (object, patch) = run(&bytes, &limits, &NeverCancel).unwrap().unwrap();
        assert_eq!(object.object.reference.number, 7);
        assert_eq!(patch.original, b"[0.72 0 0 -0.719999 -5e-006 842]");
        assert_eq!(patch.original.len(), patch.replacement.len());
        assert_eq!(
            patch.replacement.strip_suffix(b" ").unwrap()[..IDENTITY.len()],
            *IDENTITY
        );
        let mut source = bytes.as_slice();
        let patches = [patch];
        let mut patched = PatchedSource::new(&mut source, &patches);
        let mut result = vec![0; bytes.len()];
        for (offset, byte) in result.iter_mut().enumerate() {
            assert_eq!(
                patched
                    .read_at(offset as u64, std::slice::from_mut(byte))
                    .unwrap(),
                1
            );
        }
        let start = patches[0].offset as usize;
        let end = start + patches[0].original.len();
        assert_eq!(result[..start], bytes[..start]);
        assert_eq!(result[end..], bytes[end..]);
        assert_eq!(result[start..end], patches[0].replacement);
        assert!(parse_object_head(result).is_ok());

        let checkpoints = CancelAfter::never();
        run(&bytes, &limits, &checkpoints).unwrap().unwrap();
        for allowed in 0..=checkpoints.queries() {
            let result = run(&bytes, &limits, &CancelAfter::new(allowed));
            assert!(matches!(
                result,
                Ok(Some(_))
                    | Err(Error {
                        kind: ErrorKind::Cancelled,
                        ..
                    })
            ));
        }
    }

    #[test]
    fn neighbors_and_unproved_streams_do_not_qualify() {
        let original = String::from_utf8(fixture()).unwrap();
        for (from, to) in [
            ("7 0 obj", "7 1 obj"),
            ("7 0 obj", "0 0 obj"),
            ("/Type /Pattern", "/Type /XObject"),
            ("/PatternType 1", "/PatternType 2"),
            ("/PaintType 1", "/PaintType 2"),
            ("/TilingType 1", "/TilingType 2"),
            ("/Length 45", "/Length 44"),
            ("/Length 45", "/Length 45 0 R"),
            ("/Filter /FlateDecode", "/Filter /DCTDecode"),
            ("/Resources 8 0 R", "/Resources << >>"),
            ("/Resources 8 0 R", "/Resources 8 1 R"),
            ("/Resources 8 0 R", "/Resources 0 0 R"),
            ("/XStep 64", "/XStep 32"),
            ("/YStep 64", "/YStep 32"),
            ("/BBox [0 0 64 64]", "/BBox [0 0 65 64]"),
            ("0.72 0 0 -0.719999", "0.73 0 0 -0.719999"),
            ("842]", "843]"),
            ("-5e-006", "-5e+001"),
            ("-5e-006", "-.000005"),
            ("-5e-006", "0"),
            ("/Matrix", "/Other"),
            ("/Matrix", "/Extra true /Matrix"),
            ("/Matrix", "/Nested << /Matrix"),
            ("842] >>", "842] >> >>"),
            ("endstream", "endstreaX"),
            ("endobj", "endobX"),
            ("/Matrix", "/Matr#69x [1 0 0 1 0 0] /Matrix"),
        ] {
            let bytes = original.replace(from, to);
            assert!(
                run(bytes.as_bytes(), &Limits::default(), &NeverCancel)
                    .unwrap()
                    .is_none(),
                "{from} -> {to}"
            );
        }
        let oversized = original.replace(
            "/Matrix",
            &format!("{} /Matrix", " ".repeat(MAX_HEAD_BYTES)),
        );
        assert!(
            run(oversized.as_bytes(), &Limits::default(), &NeverCancel)
                .unwrap()
                .is_none()
        );
        assert!(
            run(&[], &Limits::default(), &NeverCancel)
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn source_changes_and_tight_allocations_are_refused() {
        struct Changing {
            bytes: Vec<u8>,
            trigger_offset: usize,
            trigger_length: usize,
            change_at: usize,
        }
        impl RangedSource for Changing {
            fn size(&self) -> u64 {
                self.bytes.len() as u64
            }
            fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
                let offset = offset as usize;
                if offset == self.trigger_offset && destination.len() == self.trigger_length {
                    self.bytes[self.change_at] ^= 1;
                }
                let n = destination.len().min(self.bytes.len() - offset);
                destination[..n].copy_from_slice(&self.bytes[offset..offset + n]);
                Ok(n)
            }
        }
        let bytes = fixture();
        let matrix = bytes
            .windows(b"[0.72".len())
            .position(|v| v == b"[0.72")
            .unwrap();
        let matrix_len = bytes[matrix..].iter().position(|v| *v == b']').unwrap() + 1;
        let head_end = bytes
            .windows(b"stream\n".len())
            .position(|v| v == b"stream\n")
            .unwrap()
            + b"stream\n".len();
        let resources = bytes
            .windows(b"/Resources".len())
            .position(|v| v == b"/Resources")
            .unwrap();
        for (trigger_offset, trigger_length, change_at) in [
            (0, bytes.len(), 0),
            (0, head_end, resources + 11),
            (matrix, matrix_len, matrix + 2),
        ] {
            let mut source = Changing {
                bytes: bytes.clone(),
                trigger_offset,
                trigger_length,
                change_at,
            };
            let limits = Limits::default();
            let mut reader = Reader::new(
                &mut source,
                PdfRange {
                    offset: 0,
                    length: bytes.len() as u64,
                },
                &limits,
                &NeverCancel,
            )
            .unwrap();
            assert!(matches!(
                candidate(&mut reader, 0),
                Err(Error {
                    kind: ErrorKind::Malformed,
                    ..
                })
            ));
        }
        let tight = Limits {
            io_chunk_bytes: 1,
            max_allocation_bytes: (bytes.len() * 3 - 1) as u64,
            ..Limits::default()
        };
        assert!(matches!(
            run(&bytes, &tight, &NeverCancel),
            Err(Error {
                kind: ErrorKind::LimitExceeded { .. },
                ..
            })
        ));
    }
    #[test]
    fn a_stream_near_the_u64_source_limit_cannot_wrap() {
        struct Tail {
            bytes: Vec<u8>,
            start: u64,
        }
        impl RangedSource for Tail {
            fn size(&self) -> u64 {
                u64::MAX
            }
            fn read_at(&mut self, offset: u64, output: &mut [u8]) -> Result<usize> {
                let at = usize::try_from(offset.checked_sub(self.start).unwrap()).unwrap();
                let count = output.len().min(self.bytes.len().saturating_sub(at));
                output[..count].copy_from_slice(&self.bytes[at..at + count]);
                Ok(count)
            }
        }
        let bytes = fixture();
        let end = bytes
            .windows(b"stream\n".len())
            .position(|v| v == b"stream\n")
            .unwrap()
            + b"stream\n".len();
        let mut source = Tail {
            bytes: bytes[..end].to_vec(),
            start: u64::MAX - end as u64,
        };
        let start = source.start;
        let limits = Limits {
            max_input_bytes: u64::MAX,
            ..Limits::default()
        };
        let mut reader = Reader::new(
            &mut source,
            PdfRange {
                offset: 0,
                length: u64::MAX,
            },
            &limits,
            &NeverCancel,
        )
        .unwrap();
        assert!(candidate(&mut reader, start).unwrap().is_none());
    }
}
