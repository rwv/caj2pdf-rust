// SPDX-License-Identifier: MIT

//! A measured malformed QITE source-file path, never a rendering string.

use super::fragment_scan::ScannedObject;
use super::parser::{Syntax, exact_reference, parse_object_head};
use super::{FragmentInspection, ObjectTail, Reader, inspect_head};
use crate::fallible::reserve_exact;
use crate::pdf::{FragmentObject, PdfRange};
use crate::{Cancellation, Error, Limits, RangedSource, Result};

/// Includes the object header, a single-line path and its complete endobj.
const MAX_PATH_OBJECT_BYTES: usize = 640;

pub(crate) struct SourcePathRepair {
    pub object: FragmentObject,
    original: Vec<u8>,
    pub replacement: Vec<u8>,
}

impl SourcePathRepair {
    pub(super) fn retained_bytes(&self) -> usize {
        self.original.len() + self.replacement.len() + size_of::<Self>()
    }
}

/// Identify only the observed unescaped, drive-prefixed single-line path.
/// Every incoming edge must be proved separately before this may be emitted.
pub(super) fn candidate<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    start: u64,
) -> Result<Option<(SourcePathRepair, FragmentInspection)>> {
    // Most malformed objects are unrelated. A small initial probe prevents
    // metadata recovery from adding a large read to their original failure.
    let remaining = reader.range.length - start;
    let prefix = reader.bytes(start, remaining.min(32) as usize)?;
    let Some((reference, _)) = path_start(&prefix) else {
        return Ok(None);
    };
    let count = remaining.min(MAX_PATH_OBJECT_BYTES as u64) as usize;
    let bytes = reader.bytes(start, count)?;
    if !bytes.starts_with(&prefix) {
        return Err(reader.malformed(start, Some(reference), "source path changed while reading"));
    }
    let Some((reference, value_start, close, end)) = path_span(&bytes) else {
        return Ok(None);
    };
    let payload = &bytes[value_start + 1..close];
    let length = end + payload.len(); // one-byte delimiters become < and >
    reader.limits.check_allocation((count + length) as u64)?;
    let mut replacement = Vec::new();
    reserve_exact(
        &mut replacement,
        length,
        reader
            .limits
            .allocation_refused("CAJ source path replacement", length as u64),
    )?;
    replacement.extend_from_slice(&bytes[..value_start]);
    replacement.push(b'<');
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for byte in payload {
        replacement.push(HEX[(byte >> 4) as usize]);
        replacement.push(HEX[(byte & 15) as usize]);
    }
    replacement.push(b'>');
    replacement.extend_from_slice(&bytes[close + 1..end]);
    let generated = parse_object_head(replacement)
        .map_err(|issue| reader.parse_issue(start, Some(reference), issue))?;
    let inspection = inspect_head(&generated, reader.range, start, reader.limits)?;
    let original = reader.bytes(start, end)?;
    if original != bytes[..end] {
        return Err(reader.malformed(start, Some(reference), "source path changed while reading"));
    }
    Ok(Some((
        SourcePathRepair {
            object: FragmentObject {
                reference,
                range: PdfRange {
                    offset: reader.range.offset + start,
                    length: end as u64,
                },
            },
            original,
            replacement: generated.bytes,
        },
        inspection,
    )))
}

fn path_start(bytes: &[u8]) -> Option<(crate::pdf::PdfRef, usize)> {
    let mut syntax = Syntax::new(bytes);
    let number = u32::try_from(syntax.unsigned().ok()?).ok()?;
    let generation = syntax.unsigned().ok()?;
    if number == 0 || generation != 0 {
        return None;
    }
    syntax.skip_space();
    if !syntax.consume_keyword(b"obj").ok()? {
        return None;
    }
    syntax.skip_space();
    let value_start = syntax.pos;
    let rest = bytes.get(value_start..)?;
    if rest.get(..2)? != b"(/" || !rest.get(2)?.is_ascii_uppercase() || rest.get(3)? != &b'/' {
        return None;
    }
    Some((
        crate::pdf::PdfRef {
            number,
            generation: 0,
        },
        value_start,
    ))
}

fn path_span(bytes: &[u8]) -> Option<(crate::pdf::PdfRef, usize, usize, usize)> {
    let (reference, value_start) = path_start(bytes)?;
    let rest = &bytes[value_start..];
    let mut syntax = Syntax::new(bytes);
    let eol = rest.iter().position(|b| matches!(b, b'\r' | b'\n'))?;
    let close = value_start + eol.checked_sub(1)?;
    if bytes[close] != b')' {
        return None;
    }
    let payload = &bytes[value_start + 1..close];
    // No escape sequence or earlier close is reinterpreted. Preserve all
    // bytes of the measured unmatched ASCII '(' / GBK full-width ')' path.
    let open = payload.iter().position(|b| *b == b'(')?;
    let full_close = payload.windows(2).position(|b| b == [0xa3, 0xa9])?;
    if !payload.ends_with(b".pdf")
        || payload.iter().any(|b| *b < 32 || matches!(b, b')' | b'\\'))
        || payload.iter().filter(|b| **b == b'(').count() != 1
        || payload.windows(2).filter(|b| *b == [0xa3, 0xa9]).count() != 1
        || full_close <= open
    {
        return None;
    }
    syntax.pos = close + 1;
    syntax.skip_space();
    if !syntax.consume_keyword(b"endobj").ok()? {
        return None;
    }
    Some((reference, value_start, close, syntax.pos))
}

/// Require every use to be the sole matching reference in a retained Page's
/// direct QITE_pageid/F entry. Ordinary PDF strings and rendering uses do not
/// qualify, even when their bytes resemble a source-file path.
pub(crate) fn validate<S: RangedSource, C: Cancellation>(
    source: &mut S,
    repair: &SourcePathRepair,
    objects: &[ScannedObject],
    sorted_page_ids: &[u32],
    limits: &Limits,
    cancellation: &C,
) -> Result<bool> {
    let target = repair.object.reference;
    if !objects.iter().any(|object| object.object == repair.object) {
        return Ok(false);
    }
    let mut uses = 0;
    for object in objects {
        if cancellation.is_cancelled() {
            return Err(Error::cancelled());
        }
        let Ok(inspection) = &object.inspection else {
            continue;
        };
        let count = inspection
            .references
            .iter()
            .filter(|r| **r == target)
            .count();
        if count == 0 {
            continue;
        }
        if count != 1
            || sorted_page_ids
                .binary_search(&object.object.reference.number)
                .is_err()
        {
            return Ok(false);
        }
        let mut reader = Reader::new(source, object.object.range, limits, cancellation)?;
        let head = reader.load_head(0, Some(object.object.reference))?;
        let ObjectTail::EndObject { end } = head.tail else {
            return Ok(false);
        };
        let mut rest = end as u64;
        reader.skip_space(&mut rest)?;
        if rest != object.object.range.length {
            return Ok(false);
        }
        let Some(dictionary) = head.dictionary.as_ref() else {
            return Ok(false);
        };
        reader.reject_duplicate_names(dictionary, 0, Some(object.object.reference))?;
        if dictionary
            .value(b"Type")
            .and_then(super::parser::exact_name)
            .as_deref()
            != Some(b"Page")
            || head.references.iter().filter(|r| **r == target).count() != 1
        {
            return Ok(false);
        }
        let Some(qite) = dictionary.value(b"QITE_pageid") else {
            return Ok(false);
        };
        let mut syntax = Syntax::new(qite);
        let Ok(entries) = syntax.dictionary(0) else {
            return Ok(false);
        };
        let Some(entry) = entries.iter().find(|entry| entry.name() == b"F") else {
            return Ok(false);
        };
        if !syntax.at_end() || exact_reference(entry.value(qite)) != Some(target) {
            return Ok(false);
        }
        uses += 1;
    }
    if uses == 0 {
        return Ok(false);
    }
    let mut reader = Reader::new(source, repair.object.range, limits, cancellation)?;
    if reader.bytes(0, repair.original.len())? != repair.original {
        return Err(reader.malformed(0, Some(target), "source path changed after validation"));
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::CancelAfter;
    use crate::{ErrorKind, NeverCancel};

    const RAW: &[u8] = b"13 0 obj\r(/C/report(draft\xa3\xa9.pdf)\rendobj\r";

    struct Source {
        bytes: Vec<u8>,
        starts: usize,
        change_at: usize,
        largest: usize,
    }

    impl Source {
        fn new(bytes: Vec<u8>) -> Self {
            Self {
                bytes,
                starts: 0,
                change_at: usize::MAX,
                largest: 0,
            }
        }
    }

    impl RangedSource for Source {
        fn size(&self) -> u64 {
            self.bytes.len() as u64
        }
        fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
            self.largest = self.largest.max(destination.len());
            if offset == 0 {
                self.starts += 1;
            }
            if self.starts >= self.change_at {
                self.bytes[11] = b'D';
            }
            let start = offset as usize;
            let count = destination
                .len()
                .min(self.bytes.len().saturating_sub(start));
            destination[..count].copy_from_slice(&self.bytes[start..start + count]);
            Ok(count)
        }
    }

    fn inspect(
        source: &mut Source,
        limits: &Limits,
        cancellation: &CancelAfter,
    ) -> Result<Option<(SourcePathRepair, FragmentInspection)>> {
        let range = PdfRange {
            offset: 0,
            length: source.size(),
        };
        let mut reader = Reader::new(source, range, limits, cancellation)?;
        candidate(&mut reader, 0)
    }

    #[test]
    fn source_path_reads_are_bounded_cancellable_and_rechecked() {
        let limits = Limits {
            io_chunk_bytes: 1,
            ..Limits::default()
        };
        let mut source = Source::new(RAW.to_vec());
        let signal = CancelAfter::never();
        let (repair, _) = inspect(&mut source, &limits, &signal).unwrap().unwrap();
        assert_eq!(repair.original, &RAW[..RAW.len() - 1]);
        assert!(source.largest <= 1);
        for allowed in 0..signal.queries() {
            let result = inspect(
                &mut Source::new(RAW.to_vec()),
                &limits,
                &CancelAfter::new(allowed),
            );
            assert!(matches!(
                result,
                Err(Error {
                    kind: ErrorKind::Cancelled,
                    ..
                })
            ));
        }
        let mut source = Source::new(RAW.to_vec());
        source.change_at = 2;
        assert!(matches!(
            inspect(&mut source, &limits, &CancelAfter::never()),
            Err(Error {
                kind: ErrorKind::Malformed,
                ..
            })
        ));
        let mut bytes = RAW.to_vec();
        bytes.extend_from_slice(&[b' '; 640]);
        let limits = Limits {
            max_allocation_bytes: 512,
            ..limits
        };
        assert!(matches!(
            inspect(&mut Source::new(bytes), &limits, &CancelAfter::never()),
            Err(Error {
                kind: ErrorKind::LimitExceeded { .. },
                ..
            })
        ));
    }

    #[test]
    fn neighboring_literal_profiles_are_not_source_path_repairs() {
        for bytes in [
            b"".as_slice(),
            b"x 0 obj\n(/C/a(b\xa3\xa9.pdf)\nendobj",
            b"0 0 obj\n(/C/a(b\xa3\xa9.pdf)\nendobj",
            b"13 1 obj\n(/C/a(b\xa3\xa9.pdf)\nendobj",
            b"13 0 nope\n(/C/a(b\xa3\xa9.pdf)\nendobj",
            b"13 0 obj\n(/c/a(b\xa3\xa9.pdf)\nendobj",
            b"13 0 obj\n(/C:a(b\xa3\xa9.pdf)\nendobj",
            b"13 0 obj\n(/C/a(b\xa3\xa9.pdf)",
            b"13 0 obj\n(/C/a(b\xa3\xa9.pdf\nendobj",
            b"13 0 obj\n(/C/a(b).pdf)\nendobj",
            b"13 0 obj\n(/C/a(b\xa3\xa9.txt)\nendobj",
            b"13 0 obj\n(/C/a(b\xa3\xa9\0.pdf)\nendobj",
            b"13 0 obj\n(/C/a\\(b\xa3\xa9.pdf)\nendobj",
            b"13 0 obj\n(/C/a((b\xa3\xa9.pdf)\nendobj",
            b"13 0 obj\n(/C/a(b\xa3\xa9\xa3\xa9.pdf)\nendobj",
            b"13 0 obj\n(/C/a\xa3\xa9(b.pdf)\nendobj",
            b"13 0 obj\n(/C/a(b\xa3\xa9.pdf)\nendobx",
        ] {
            assert!(path_span(bytes).is_none(), "{bytes:?}");
        }
        let mut long = b"13 0 obj\n(/C/".to_vec();
        long.extend_from_slice(&[b'a'; 640]);
        long.extend_from_slice(b"(draft\xa3\xa9.pdf)\nendobj");
        assert!(
            inspect(
                &mut Source::new(long),
                &Limits::default(),
                &CancelAfter::never()
            )
            .unwrap()
            .is_none()
        );
        let mut source = Source::new(b"13 0 obj\n(null)\nendobj".to_vec());
        assert!(
            inspect(&mut source, &Limits::default(), &CancelAfter::never())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn validation_rechecks_the_original_after_metadata_graph_proof() {
        let limits = Limits::default();
        let mut source = Source::new(RAW.to_vec());
        let (repair, inspection) = inspect(&mut source, &limits, &CancelAfter::never())
            .unwrap()
            .unwrap();
        let mut objects = vec![ScannedObject {
            object: repair.object,
            inspection: Ok(inspection),
        }];
        let at = source.size();
        let owner = b"9 0 obj\n<</Type/Page/Parent 5 0 R/QITE_pageid<</F 13 0 R>>>>\nendobj\n";
        source.bytes.truncate(at as usize);
        source.bytes.extend_from_slice(owner);
        let object = FragmentObject {
            reference: crate::pdf::PdfRef {
                number: 9,
                generation: 0,
            },
            range: PdfRange {
                offset: at,
                length: owner.len() as u64,
            },
        };
        let head = parse_object_head(owner.to_vec()).unwrap();
        let inspected = inspect_head(&head, object.range, 0, &limits).unwrap();
        objects.push(ScannedObject {
            object,
            inspection: Ok(inspected),
        });
        assert!(validate(&mut source, &repair, &objects, &[9], &limits, &NeverCancel).unwrap());
        let signal = CancelAfter::never();
        assert!(validate(&mut source, &repair, &objects, &[9], &limits, &signal).unwrap());
        for allowed in 0..signal.queries() {
            assert!(matches!(
                validate(
                    &mut source,
                    &repair,
                    &objects,
                    &[9],
                    &limits,
                    &CancelAfter::new(allowed)
                ),
                Err(Error {
                    kind: ErrorKind::Cancelled,
                    ..
                })
            ));
        }
        assert!(
            !validate(
                &mut source,
                &repair,
                &objects[..1],
                &[9],
                &limits,
                &NeverCancel
            )
            .unwrap()
        );
        assert!(
            !validate(
                &mut source,
                &repair,
                &objects[1..],
                &[9],
                &limits,
                &NeverCancel
            )
            .unwrap()
        );
        assert!(!validate(&mut source, &repair, &objects, &[], &limits, &NeverCancel).unwrap());
        source.bytes[11] = b'D';
        assert!(matches!(
            validate(&mut source, &repair, &objects, &[9], &limits, &NeverCancel),
            Err(Error {
                kind: ErrorKind::Malformed,
                ..
            })
        ));
    }
}
