// SPDX-License-Identifier: MIT

//! Narrow repairs for link destinations pointing at omitted fragment pages.
//!
//! This module only identifies candidates. The caller must prove that the
//! target is absent from the retained page set before replacing an object.

use super::parser::{Syntax, destination_page, exact_name, exact_reference, parse_object_head};
use super::{FragmentObject, ObjectTail, Reader};
use crate::fallible::reserve_exact;
use crate::pdf::PdfRef;
use crate::{Cancellation, Error, Limits, PdfErrorKind, RangedSource, Result};

/// The parsed object form. A scalar destination must be linked from an actual
/// link annotation before its null replacement can be used.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LinkRepairKind {
    Link,
    ScalarDestination,
}

/// The reference that makes a link destination repair eligible.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LinkDestinationTarget {
    /// A direct destination array begins with this page reference.
    DirectPage(PdfRef),
    /// A link's `/Dest` value refers to a separate destination object.
    IndirectArray(PdfRef),
}

/// A complete, bounded replacement object plus the evidence needed to decide
/// whether it may be used. `replacement` retains the original object number.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LinkRepairCandidate {
    pub object: PdfRef,
    pub kind: LinkRepairKind,
    pub target: LinkDestinationTarget,
    /// A link's non-destination fields still refer to its indirect `/Dest`.
    /// Nulling that destination would also change those unrelated fields.
    pub retains_destination_reference: bool,
    pub replacement: Vec<u8>,
}

fn is_link_with_destination(head: &super::parser::ObjectHead) -> bool {
    let Some(dictionary) = head.dictionary.as_ref() else {
        return false;
    };
    dictionary
        .value(b"Subtype")
        .and_then(exact_name)
        .is_some_and(|name| name == b"Link")
        && dictionary.value(b"Dest").is_some()
        && dictionary.value(b"A").is_none()
        && dictionary
            .value(b"Type")
            .is_none_or(|value| exact_name(value).is_some_and(|name| name == b"Annot"))
}

/// Inspect one already indexed, complete PDF object without scanning adjacent
/// fragment bytes. Streams and unrelated object types return `None`.
///
/// A link dictionary candidate has only its parsed `/Dest` key/value pair
/// removed; every other original byte is preserved. A standalone destination
/// array is replaced with a generation-zero `null` object. Both rewrites are
/// bounded by the existing PDF syntax and allocation limits.
pub(crate) async fn inspect_link_destination_candidate<S: RangedSource, C: Cancellation>(
    source: &mut S,
    fragment: FragmentObject,
    limits: &Limits,
    cancellation: &C,
) -> Result<Option<LinkRepairCandidate>> {
    limits.validate()?;
    let range = fragment.range;
    let reference = fragment.reference;
    if reference.generation != 0 {
        return Ok(None);
    }
    let end = range.end().ok_or(Error::Pdf {
        offset: range.offset,
        object: Some((reference.number, reference.generation)),
        kind: PdfErrorKind::Malformed,
        reason: "link object source range overflows",
    })?;
    if end > source.size() {
        return Err(Error::TruncatedInput {
            offset: range.offset,
            expected: range.length,
            available: source.size().saturating_sub(range.offset),
        });
    }
    let mut reader = Reader::new(source, range, limits, cancellation)?;
    limits
        .check_input_size(range.length)
        .map_err(|error| reader.locate_limit(0, Some(reference), error))?;
    let head = reader.load_head(0, Some(reference)).await?;
    let ObjectTail::EndObject { end } = head.tail else {
        return Ok(None);
    };
    let mut rest = end as u64;
    reader.skip_space(&mut rest).await?;
    if rest != range.length {
        return Err(reader.malformed(
            rest,
            Some(reference),
            "link repair object has trailing non-whitespace bytes",
        ));
    }

    let likely_link = is_link_with_destination(&head);
    let array_page = head
        .scalar
        .as_ref()
        .and_then(|span| destination_page(&head.bytes[span.clone()]));
    if !likely_link && array_page.is_none() {
        return Ok(None);
    }

    let maximum = reader.syntax_limit();
    if range.length > maximum {
        return Err(Error::PdfLimitExceeded {
            offset: range.offset,
            object: Some((reference.number, reference.generation)),
            resource: "PDF link repair object bytes",
            limit: maximum,
            attempted: range.length,
        });
    }
    // `maximum` is at most the 4 MiB object syntax limit.
    let length = usize::try_from(range.length).expect("object syntax limit fits usize");
    let complete = reader.bytes(0, length).await?;
    if !complete.starts_with(&head.bytes) {
        return Err(reader.malformed(
            0,
            Some(reference),
            "link repair source changed while reading",
        ));
    }
    let complete = parse_object_head(complete)
        .map_err(|issue| reader.parse_issue(0, Some(reference), issue))?;
    // `complete` extends `head.bytes`, which parsed to this reference with its
    // `endobj` keyword inside those bytes. Every earlier token is delimited
    // within the shared prefix, so a successful parse of the longer buffer
    // reads the same header, value, and tail; only a byte glued to `endobj`
    // can differ, and that fails the parse above.
    debug_assert_eq!(complete.reference, reference);
    debug_assert!(matches!(complete.tail, ObjectTail::EndObject { end: tail } if tail == end));
    if !Syntax::new(&complete.bytes[end..]).at_end() {
        return Err(reader.malformed(
            end as u64,
            Some(reference),
            "link repair object has trailing non-whitespace bytes",
        ));
    }

    debug_assert_eq!(is_link_with_destination(&complete), likely_link);
    if likely_link {
        let dictionary = complete.dictionary.as_ref().expect("classified dictionary");
        reader.reject_duplicate_names(dictionary, 0, Some(reference))?;
        let destination = dictionary.entry(b"Dest").expect("classified destination");
        let target = if let Some(page) = destination_page(destination.value(&dictionary.bytes)) {
            LinkDestinationTarget::DirectPage(page)
        } else if let Some(array) = exact_reference(destination.value(&dictionary.bytes)) {
            LinkDestinationTarget::IndirectArray(array)
        } else {
            return Ok(None);
        };
        let retains_destination_reference = match target {
            LinkDestinationTarget::IndirectArray(array) => {
                complete
                    .references
                    .iter()
                    .filter(|reference| **reference == array)
                    .count()
                    != 1
            }
            LinkDestinationTarget::DirectPage(_) => false,
        };
        // The parser records the dictionary at `dictionary_start` in
        // `complete.bytes` and each entry's nonempty pair relative to it, so
        // the absolute pair lies inside the object.
        let dictionary_start = complete
            .dictionary_start
            .expect("classified dictionary offset");
        let pair_start = dictionary_start + destination.pair.start;
        let pair_end = dictionary_start + destination.pair.end;
        debug_assert!(pair_start < pair_end && pair_end <= complete.bytes.len());
        let mut replacement = Vec::new();
        let refused = Error::PdfLimitExceeded {
            offset: range.offset,
            object: Some((reference.number, reference.generation)),
            resource: "PDF link repair allocation",
            limit: limits.max_allocation_bytes,
            attempted: complete.bytes.len() as u64,
        };
        let additional = complete.bytes.len() - (pair_end - pair_start);
        reserve_exact(&mut replacement, additional, refused)?;
        replacement.extend_from_slice(&complete.bytes[..pair_start]);
        replacement.extend_from_slice(&complete.bytes[pair_end..]);
        return Ok(Some(LinkRepairCandidate {
            object: reference,
            kind: LinkRepairKind::Link,
            target,
            retains_destination_reference,
            replacement,
        }));
    }

    let page = array_page.expect("classified destination array");
    // At most 29 bytes for a 32-bit object number. The parsed destination
    // object needs more than one byte, so `syntax_limit` came from
    // `max_allocation_bytes / 32` and the allocation limit is at least 64.
    let replacement = format!("{} 0 obj\nnull\nendobj\n", reference.number).into_bytes();
    debug_assert!(replacement.len() as u64 <= limits.max_allocation_bytes);
    Ok(Some(LinkRepairCandidate {
        object: reference,
        kind: LinkRepairKind::ScalarDestination,
        target: LinkDestinationTarget::DirectPage(page),
        retains_destination_reference: false,
        replacement,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NeverCancel;
    use crate::pdf::PdfRange;
    use crate::test_support::ready;

    /// Serves `first` until `switch(reads, starts)` holds for a read, then
    /// `second`. `reads` counts earlier reads; `starts` counts reads at the
    /// object start, including this one. One source type keeps every test on
    /// the same generic instantiation.
    struct Source {
        first: Vec<u8>,
        second: Vec<u8>,
        reads: usize,
        starts: usize,
        switch: fn(usize, usize) -> bool,
        largest_request: usize,
    }

    impl Source {
        fn new(bytes: Vec<u8>) -> Self {
            Self::switching(bytes, Vec::new(), |_, _| false)
        }

        fn switching(first: Vec<u8>, second: Vec<u8>, switch: fn(usize, usize) -> bool) -> Self {
            Self {
                first,
                second,
                reads: 0,
                starts: 0,
                switch,
                largest_request: 0,
            }
        }
    }

    impl RangedSource for Source {
        fn size(&self) -> u64 {
            self.first.len() as u64
        }

        async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
            self.largest_request = self.largest_request.max(destination.len());
            if offset == 0 {
                self.starts += 1;
            }
            let bytes = if (self.switch)(self.reads, self.starts) {
                &self.second
            } else {
                &self.first
            };
            let start = offset as usize;
            let count = destination.len().min(bytes.len().saturating_sub(start));
            destination[..count].copy_from_slice(&bytes[start..start + count]);
            self.reads += 1;
            Ok(count)
        }
    }

    fn inspect(bytes: &[u8]) -> Result<Option<LinkRepairCandidate>> {
        let mut source = Source::new(bytes.to_vec());
        let fragment = FragmentObject {
            reference: PdfRef {
                number: 9,
                generation: 0,
            },
            range: PdfRange {
                offset: 0,
                length: bytes.len() as u64,
            },
        };
        ready(inspect_link_destination_candidate(
            &mut source,
            fragment,
            &Limits::default(),
            &NeverCancel,
        ))
    }

    #[test]
    fn removes_only_a_direct_link_destination_pair() {
        let bytes = b"9 0 obj\n<< /Type /Annot /Subtype /Link /Rect [0 0 10 10] /AP << /Dest (appearance) >> /Dest [6 0 R /Fit] /Border [0 0 0] >>\nendobj\n";
        let candidate = inspect(bytes).unwrap().unwrap();
        assert_eq!(candidate.kind, LinkRepairKind::Link);
        assert_eq!(
            candidate.target,
            LinkDestinationTarget::DirectPage(PdfRef {
                number: 6,
                generation: 0
            })
        );
        assert_eq!(candidate.object.number, 9);
        let replacement = candidate.replacement;
        assert!(replacement.windows(5).any(|window| window == b"/Rect"));
        assert!(replacement.windows(7).any(|window| window == b"/Border"));
        let parsed = parse_object_head(replacement).unwrap();
        let dictionary = parsed.dictionary.unwrap();
        assert!(dictionary.value(b"Dest").is_none());
        assert!(
            dictionary
                .value(b"AP")
                .unwrap()
                .windows(5)
                .any(|window| window == b"/Dest")
        );
    }

    #[test]
    fn recognizes_indirect_links_and_nulls_scalar_arrays() {
        let link = inspect(b"9 0 obj\n<</Subtype/Link /Dest 42 0 R /Rect [0 0 2 2]>>\nendobj\n")
            .unwrap()
            .unwrap();
        assert_eq!(
            link.target,
            LinkDestinationTarget::IndirectArray(PdfRef {
                number: 42,
                generation: 0
            })
        );
        assert!(link.replacement.windows(5).any(|window| window == b"/Rect"));

        let array = inspect(b"9 0 obj\n[6 0 R /XYZ null null 1]\nendobj\n")
            .unwrap()
            .unwrap();
        assert_eq!(array.kind, LinkRepairKind::ScalarDestination);
        assert_eq!(
            array.target,
            LinkDestinationTarget::DirectPage(PdfRef {
                number: 6,
                generation: 0
            })
        );
        assert_eq!(array.replacement, b"9 0 obj\nnull\nendobj\n");
    }

    #[test]
    fn ignores_streams_strings_and_unrelated_dictionaries() {
        assert!(inspect(b"9 0 obj\n<</Subtype/Link /Dest [6 0 R /Fit] /Length 0>>\nstream\n\nendstream\nendobj\n").unwrap().is_none());
        assert!(
            inspect(b"9 0 obj\n(Note /Subtype /Link /Dest [6 0 R /Fit])\nendobj\n")
                .unwrap()
                .is_none()
        );
        assert!(
            inspect(b"9 0 obj\n<</Subtype/Widget /Dest [6 0 R /Fit]>>\nendobj\n")
                .unwrap()
                .is_none()
        );
        assert!(
            inspect(b"9 0 obj\n<</Subtype/Link /Dest (named)>>\nendobj\n")
                .unwrap()
                .is_none()
        );
        assert!(inspect(b"9 0 obj\n<</Subtype/Link /Dest [6 0 R /Fit] /A <</S/URI /URI (example)>> >>\nendobj\n").unwrap().is_none());
    }

    #[test]
    fn rejects_duplicate_destination_and_wrong_object_header() {
        let duplicate =
            b"9 0 obj\n<</Subtype/Link /Dest [6 0 R /Fit] /Dest [7 0 R /Fit]>>\nendobj\n";
        assert!(matches!(
            inspect(duplicate),
            Err(Error::Pdf {
                kind: PdfErrorKind::AmbiguousRepair,
                ..
            })
        ));
        let wrong = b"10 0 obj\n[6 0 R /Fit]\nendobj\n";
        assert!(matches!(inspect(wrong), Err(Error::Pdf { offset: 0, .. })));
    }

    #[test]
    fn small_io_chunks_bound_reads() {
        let bytes = b"9 0 obj\n[6 0 R /Fit]\nendobj\n";
        let mut source = Source::new(bytes.to_vec());
        let fragment = FragmentObject {
            reference: PdfRef {
                number: 9,
                generation: 0,
            },
            range: PdfRange {
                offset: 0,
                length: bytes.len() as u64,
            },
        };
        let limits = Limits {
            io_chunk_bytes: 3,
            ..Limits::default()
        };
        let candidate = ready(inspect_link_destination_candidate(
            &mut source,
            fragment,
            &limits,
            &NeverCancel,
        ))
        .unwrap()
        .unwrap();
        assert_eq!(candidate.kind, LinkRepairKind::ScalarDestination);
        assert!(source.largest_request <= 3);
    }

    #[test]
    fn rejects_a_large_candidate_before_buffering_its_full_span() {
        let mut bytes = b"9 0 obj\n<</Subtype/Link /Dest [6 0 R /Fit]>>\nendobj\n".to_vec();
        bytes.extend(std::iter::repeat_n(b' ', 256));
        let mut source = Source::new(bytes);
        let fragment = FragmentObject {
            reference: PdfRef {
                number: 9,
                generation: 0,
            },
            range: PdfRange {
                offset: 0,
                length: source.size(),
            },
        };
        let limits = Limits {
            io_chunk_bytes: 8,
            max_allocation_bytes: 4096,
            ..Limits::default()
        };
        assert!(matches!(
            ready(inspect_link_destination_candidate(
                &mut source,
                fragment,
                &limits,
                &NeverCancel
            )),
            Err(Error::PdfLimitExceeded {
                resource: "PDF link repair object bytes",
                ..
            })
        ));
        assert!(source.largest_request <= 8);
    }

    #[test]
    fn candidate_span_is_checked_against_the_input_limit() {
        let bytes = b"9 0 obj\n[6 0 R /Fit]\nendobj\n";
        let mut source = Source::new(bytes.to_vec());
        let fragment = FragmentObject {
            reference: PdfRef {
                number: 9,
                generation: 0,
            },
            range: PdfRange {
                offset: 0,
                length: bytes.len() as u64,
            },
        };
        let limits = Limits {
            max_input_bytes: 8,
            ..Limits::default()
        };
        let error = ready(inspect_link_destination_candidate(
            &mut source,
            fragment,
            &limits,
            &NeverCancel,
        ))
        .expect_err("an oversized candidate span was read");
        assert!(matches!(
            error,
            Error::PdfLimitExceeded {
                offset: 0,
                object: Some((9, 0)),
                resource: "input bytes",
                limit: 8,
                attempted,
            } if attempted == bytes.len() as u64
        ));
        assert_eq!(source.largest_request, 0);
    }

    #[test]
    fn rejects_a_source_that_changes_between_candidate_reads() {
        let bytes = b"9 0 obj\n<</Subtype/Link /Dest [6 0 R /Fit]>>\nendobj\n";
        let mut changed = bytes.to_vec();
        let digit = changed
            .windows(5)
            .position(|window| window == b"6 0 R")
            .unwrap();
        changed[digit] = b'7';
        let mut source = Source::switching(bytes.to_vec(), changed, |reads, _| reads > 0);
        let fragment = FragmentObject {
            reference: PdfRef {
                number: 9,
                generation: 0,
            },
            range: PdfRange {
                offset: 0,
                length: bytes.len() as u64,
            },
        };
        assert!(matches!(
            ready(inspect_link_destination_candidate(
                &mut source,
                fragment,
                &Limits::default(),
                &NeverCancel
            )),
            Err(Error::Pdf {
                reason: "link repair source changed while reading",
                ..
            })
        ));
    }

    #[test]
    fn truncated_located_range_is_rejected_before_reading() {
        let prefix = b"CAJ!!";
        let object = b"9 0 obj\n<</Subtype/Link /Dest [6 0 R /Fit]>>\nendobj\n";
        let mut bytes = prefix.to_vec();
        bytes.extend_from_slice(object);
        let mut source = Source::new(bytes);
        let fragment = FragmentObject {
            reference: PdfRef {
                number: 9,
                generation: 0,
            },
            range: PdfRange {
                offset: prefix.len() as u64,
                length: object.len() as u64 + 3,
            },
        };
        let error = ready(inspect_link_destination_candidate(
            &mut source,
            fragment,
            &Limits::default(),
            &NeverCancel,
        ))
        .expect_err("truncated link object was accepted");
        assert!(matches!(
            error,
            Error::TruncatedInput {
                offset,
                expected,
                available,
            } if offset == prefix.len() as u64
                && expected == object.len() as u64 + 3
                && available == object.len() as u64
        ));
        assert_eq!(source.largest_request, 0);
    }

    #[test]
    fn nonzero_generation_is_excluded_without_reading() {
        let object = b"9 0 obj\n<</Subtype/Link /Dest [6 0 R /Fit]>>\nendobj\n";
        let mut source = Source::new(object.to_vec());
        let fragment = FragmentObject {
            reference: PdfRef {
                number: 9,
                generation: 1,
            },
            range: PdfRange {
                offset: 0,
                length: object.len() as u64,
            },
        };
        assert!(
            ready(inspect_link_destination_candidate(
                &mut source,
                fragment,
                &Limits::default(),
                &NeverCancel,
            ))
            .unwrap()
            .is_none()
        );
        assert_eq!(source.largest_request, 0);
    }

    #[test]
    fn rejects_non_whitespace_after_a_link_object_with_absolute_offset() {
        let prefix = b"CAJ!!";
        let object = b"9 0 obj\n<</Subtype/Link /Dest [6 0 R /Fit]>>\nendobj";
        let tail = b"\r\n<container-tail>";
        let mut bytes = prefix.to_vec();
        bytes.extend_from_slice(object);
        bytes.extend_from_slice(tail);
        let mut source = Source::new(bytes);
        let fragment = FragmentObject {
            reference: PdfRef {
                number: 9,
                generation: 0,
            },
            range: PdfRange {
                offset: prefix.len() as u64,
                length: (object.len() + tail.len()) as u64,
            },
        };
        let error = ready(inspect_link_destination_candidate(
            &mut source,
            fragment,
            &Limits::default(),
            &NeverCancel,
        ))
        .expect_err("object span swallowed the container tail");
        assert!(matches!(
            error,
            Error::Pdf {
                offset,
                object: Some((9, 0)),
                kind: PdfErrorKind::Malformed,
                reason: "link repair object has trailing non-whitespace bytes",
            } if offset == (prefix.len() + object.len() + 2) as u64
        ));
    }

    #[test]
    fn rejects_changed_link_form_between_head_and_full_read() {
        let first = b"9 0 obj\n<</Subtype/Link /Dest [6 0 R /Fit]>>\nendobj\n".to_vec();
        let mut second = first.clone();
        let subtype = second
            .windows(5)
            .position(|window| window == b"/Link")
            .unwrap();
        second[subtype..subtype + 5].copy_from_slice(b"/Null");
        // Only the first read sees the original bytes.
        let mut source = Source::switching(first, second, |reads, _| reads > 0);
        let fragment = FragmentObject {
            reference: PdfRef {
                number: 9,
                generation: 0,
            },
            range: PdfRange {
                offset: 0,
                length: source.size(),
            },
        };
        assert!(matches!(
            ready(inspect_link_destination_candidate(
                &mut source,
                fragment,
                &Limits::default(),
                &NeverCancel,
            )),
            Err(Error::Pdf {
                kind: PdfErrorKind::Malformed,
                reason: "link repair source changed while reading",
                ..
            })
        ));
        assert!(source.reads >= 2);
    }

    #[test]
    fn rejects_trailing_bytes_that_appear_after_the_bounded_head_read() {
        let object = b"9 0 obj\n[6 0 R /Fit]\nendobj";
        let mut first = object.to_vec();
        first.extend(std::iter::repeat_n(b' ', 1024));
        let mut second = first.clone();
        // Beyond the 512-byte head window, so the head prefix still matches.
        second[800] = b'x';
        // The tail changes once a second read restarts at the object start,
        // between the whitespace scan and the complete candidate read.
        let mut source = Source::switching(first, second, |_, starts| starts >= 2);
        let fragment = FragmentObject {
            reference: PdfRef {
                number: 9,
                generation: 0,
            },
            range: PdfRange {
                offset: 0,
                length: source.size(),
            },
        };
        let error = ready(inspect_link_destination_candidate(
            &mut source,
            fragment,
            &Limits::default(),
            &NeverCancel,
        ))
        .expect_err("changed trailing bytes were accepted");
        assert!(matches!(
            error,
            Error::Pdf {
                offset,
                object: Some((9, 0)),
                kind: PdfErrorKind::Malformed,
                reason: "link repair object has trailing non-whitespace bytes",
            } if offset == object.len() as u64
        ));
    }

    #[test]
    fn identifies_an_indirect_destination_used_by_another_link_field() {
        let ordinary = inspect(b"9 0 obj\n<</Subtype/Link /Dest 42 0 R /AP 43 0 R>>\nendobj\n")
            .unwrap()
            .unwrap();
        assert!(!ordinary.retains_destination_reference);

        let shared = inspect(b"9 0 obj\n<</Subtype/Link /Dest 42 0 R /AP 42 0 R>>\nendobj\n")
            .unwrap()
            .unwrap();
        assert_eq!(
            shared.target,
            LinkDestinationTarget::IndirectArray(PdfRef {
                number: 42,
                generation: 0
            })
        );
        assert!(shared.retains_destination_reference);
        assert!(
            shared
                .replacement
                .windows(10)
                .any(|window| window == b"/AP 42 0 R")
        );
    }

    #[test]
    fn rejects_a_keyword_glued_to_endobj_after_the_bounded_head_read() {
        // The 512-byte head window ends exactly after `endobj`, so the head
        // parses; the complete read then sees a byte glued to that keyword.
        let mut object = b"9 0 obj\n[6 0 R /Fit".to_vec();
        object.resize(512 - b"]\nendobj".len(), b' ');
        object.extend_from_slice(b"]\nendobj");
        assert_eq!(object.len(), 512);
        let mut first = object.clone();
        first.extend_from_slice(&[b' '; 16]);
        let mut second = first.clone();
        second[512] = b'x';
        let mut source = Source::switching(first, second, |_, starts| starts >= 2);
        let fragment = FragmentObject {
            reference: PdfRef {
                number: 9,
                generation: 0,
            },
            range: PdfRange {
                offset: 0,
                length: source.size(),
            },
        };
        let error = ready(inspect_link_destination_candidate(
            &mut source,
            fragment,
            &Limits::default(),
            &NeverCancel,
        ))
        .expect_err("a glued endobj keyword was accepted");
        assert!(matches!(
            error,
            Error::Pdf {
                offset: 512,
                object: Some((9, 0)),
                kind: PdfErrorKind::Malformed,
                reason: "PDF keyword lacks a delimiter",
            }
        ));
    }
}
