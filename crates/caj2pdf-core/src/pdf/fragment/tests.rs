// SPDX-License-Identifier: MIT

use super::*;
use crate::test_support::{CancelAfter, NEVER, run};
use std::{cell::Cell, io};

/// Byte segments placed at offsets of a sparse test source.
type Segments = Vec<(u64, Vec<u8>)>;

/// The one source type of these tests, so every reconstruction shares a
/// single instantiation. Reads return at most seven bytes and are
/// counted; a sparse source is a large virtual span of zero bytes with a
/// few placed segments, read in full.
struct BytesSource {
    bytes: Vec<u8>,
    sparse: Option<(u64, Segments)>,
    over_report: bool,
    bytes_read: u64,
}

impl BytesSource {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            sparse: None,
            over_report: false,
            bytes_read: 0,
        }
    }

    fn sparse(size: u64, segments: Segments) -> Self {
        Self {
            sparse: Some((size, segments)),
            ..Self::new(Vec::new())
        }
    }
}

impl RangedSource for BytesSource {
    fn size(&self) -> u64 {
        self.sparse
            .as_ref()
            .map_or(self.bytes.len() as u64, |(size, _)| *size)
    }

    async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
        let copied = if let Some((size, segments)) = &self.sparse {
            let length = destination.len().min(size.saturating_sub(offset) as usize);
            let end = offset + length as u64;
            destination[..length].fill(0);
            for (start, bytes) in segments {
                let segment_end = start + bytes.len() as u64;
                let from = offset.max(*start);
                let to = end.min(segment_end);
                if from < to {
                    destination[(from - offset) as usize..(to - offset) as usize]
                        .copy_from_slice(&bytes[(from - start) as usize..(to - start) as usize]);
                }
            }
            length
        } else {
            let available = usize::try_from(offset)
                .ok()
                .and_then(|start| self.bytes.get(start..))
                .unwrap_or_default();
            let copied = available.len().min(destination.len()).min(7);
            destination[..copied].copy_from_slice(&available[..copied]);
            copied
        };
        self.bytes_read += copied as u64;
        if self.over_report {
            return Ok(destination.len() + 1);
        }
        Ok(copied)
    }
}

#[derive(Default)]
struct BytesSink {
    bytes: Vec<u8>,
    fail_after: Option<usize>,
}

impl SequentialSink for BytesSink {
    async fn write(&mut self, bytes: &[u8]) -> Result<usize> {
        if self
            .fail_after
            .is_some_and(|threshold| self.bytes.len() >= threshold)
        {
            return Err(Error::Io(io::Error::other("injected sink failure")));
        }
        let copied = bytes.len().min(11);
        self.bytes.extend_from_slice(&bytes[..copied]);
        Ok(copied)
    }

    async fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

fn reference(number: u32) -> PdfRef {
    PdfRef {
        number,
        generation: 0,
    }
}

fn add_object(source: &mut Vec<u8>, number: u32, body: &[u8]) -> FragmentObject {
    let offset = source.len() as u64;
    source.extend_from_slice(format!("{number} 0 obj\n").as_bytes());
    source.extend_from_slice(body);
    source.extend_from_slice(b"\nendobj\n");
    FragmentObject {
        reference: reference(number),
        range: PdfRange {
            offset,
            length: source.len() as u64 - offset,
        },
    }
}

fn two_page_fragment() -> (BytesSource, Vec<FragmentObject>, Vec<PdfRef>) {
    let mut bytes = b"CAJ\0unrelated metadata\n".to_vec();
    let second = add_object(
        &mut bytes,
        9,
        b"<< /Type /Page /Parent 5 0 R /MediaBox [0 0 400 250] /Resources << >> >>",
    );
    let first = add_object(
        &mut bytes,
        3,
        b"<< /Type /Page /Parent 5 0 R /MediaBox [0 0 200 300] /Resources << >> >>",
    );
    let fake_markers = b"endobj\nxref\nstartxref\n%%EOF\n";
    let scalar = add_object(&mut bytes, 4, fake_markers.len().to_string().as_bytes());
    let mut stream_body = b"<< /Length 4 0 R >>\nstream\n".to_vec();
    stream_body.extend_from_slice(fake_markers);
    stream_body.extend_from_slice(b"\nendstream");
    let stream = add_object(&mut bytes, 6, &stream_body);
    (
        BytesSource::new(bytes),
        vec![second, first, scalar, stream],
        vec![reference(9), reference(3)],
    )
}

fn existing_tree_fragment() -> (BytesSource, Vec<FragmentObject>, Vec<PdfRef>) {
    let mut bytes = b"CAJ\0object fragments\n".to_vec();
    let first = add_object(
        &mut bytes,
        9,
        b"<< /Type /Page /Parent 5 0 R /MediaBox [0 0 200 300] /Resources << >> >>",
    );
    let catalog = add_object(&mut bytes, 1, b"<< /Type /Catalog /Pages 5 0 R >>");
    let second = add_object(
        &mut bytes,
        3,
        b"<< /Type /Page /Parent 7 0 R /MediaBox [0 0 400 250] /Resources << >> >>",
    );
    let branch = add_object(
        &mut bytes,
        7,
        b"<< /Type /Pages /Parent 5 0 R /Count 1 /Kids [3 0 R] >>",
    );
    let root = add_object(
        &mut bytes,
        5,
        b"<< /Type /Pages /Count 2 /Kids [9 0 R 7 0 R] >>",
    );
    (
        BytesSource::new(bytes),
        vec![first, catalog, second, branch, root],
        vec![reference(9), reference(3)],
    )
}

fn replace_in_object(source: &mut BytesSource, object: FragmentObject, from: &[u8], to: &[u8]) {
    assert_eq!(from.len(), to.len());
    let start = object.range.offset as usize;
    let end = start + object.range.length as usize;
    let relative = source.bytes[start..end]
        .windows(from.len())
        .position(|window| window == from)
        .expect("test token is present");
    source.bytes[start + relative..start + relative + to.len()].copy_from_slice(to);
}

#[test]
fn sparse_out_of_order_objects_use_explicit_page_order() {
    run(async {
        let (mut source, objects, pages) = two_page_fragment();
        let mut sink = BytesSink::default();
        let plan = FragmentPlan {
            objects: &objects,
            pages: &pages,
            pages_root: reference(5),
            catalog: None,
        };
        let report =
            reconstruct_fragment(&mut source, &mut sink, &plan, &Limits::default(), &NEVER).await?;
        assert_eq!(report.pages_converted, 2);
        assert_eq!(report.output_bytes_written, sink.bytes.len() as u64);
        let pdf = String::from_utf8_lossy(&sink.bytes);
        assert!(pdf.starts_with("%PDF-1.7"));
        assert!(pdf.contains("/Kids [9 0 R 3 0 R ]"));
        assert!(pdf.contains("/Root 10 0 R"));
        assert!(pdf.contains("xref\n0 11\n"));
        Ok::<(), Error>(())
    })
    .unwrap();
}

#[test]
fn input_report_counts_validation_and_copy_reads() {
    run(async {
        let (mut source, objects, pages) = two_page_fragment();
        let mut sink = BytesSink::default();
        let plan = FragmentPlan {
            objects: &objects,
            pages: &pages,
            pages_root: reference(5),
            catalog: None,
        };
        let report =
            reconstruct_fragment(&mut source, &mut sink, &plan, &Limits::default(), &NEVER).await?;
        assert_eq!(report.input_bytes_read, source.bytes_read);
        assert!(
            report.input_bytes_read
                > objects
                    .iter()
                    .map(|object| object.range.length)
                    .sum::<u64>()
        );
        Ok::<(), Error>(())
    })
    .unwrap();
}

#[test]
fn existing_nested_pages_and_catalog_are_preserved() {
    run(async {
        let (mut source, objects, pages) = existing_tree_fragment();
        let mut sink = BytesSink::default();
        let plan = FragmentPlan {
            objects: &objects,
            pages: &pages,
            pages_root: reference(5),
            catalog: Some(reference(1)),
        };
        let report =
            reconstruct_fragment(&mut source, &mut sink, &plan, &Limits::default(), &NEVER).await?;
        assert_eq!(report.pages_converted, 2);
        let pdf = String::from_utf8_lossy(&sink.bytes);
        assert!(pdf.contains("/Root 1 0 R"));
        assert_eq!(pdf.matches("/Type /Catalog").count(), 1);
        assert_eq!(pdf.matches("/Type /Pages").count(), 2);
        Ok::<(), Error>(())
    })
    .unwrap();
}

#[test]
fn existing_pages_can_gain_catalog_and_missing_pages_can_keep_catalog() {
    run(async {
        let (mut source, objects, pages) = existing_tree_fragment();
        let without_catalog: Vec<_> = objects
            .iter()
            .copied()
            .filter(|object| object.reference != reference(1))
            .collect();
        let mut sink = BytesSink::default();
        let plan = FragmentPlan {
            objects: &without_catalog,
            pages: &pages,
            pages_root: reference(5),
            catalog: None,
        };
        reconstruct_fragment(&mut source, &mut sink, &plan, &Limits::default(), &NEVER).await?;
        assert!(String::from_utf8_lossy(&sink.bytes).contains("/Root 10 0 R"));

        let mut bytes = b"CAJ\0".to_vec();
        let page = add_object(
            &mut bytes,
            9,
            b"<< /Type /Page /Parent 5 0 R /MediaBox [0 0 200 300] /Resources << >> >>",
        );
        let catalog = add_object(&mut bytes, 1, b"<< /Type /Catalog /Pages 5 0 R >>");
        let mut source = BytesSource::new(bytes);
        let objects = [page, catalog];
        let pages = [reference(9)];
        let mut sink = BytesSink::default();
        let plan = FragmentPlan {
            objects: &objects,
            pages: &pages,
            pages_root: reference(5),
            catalog: Some(reference(1)),
        };
        reconstruct_fragment(&mut source, &mut sink, &plan, &Limits::default(), &NEVER).await?;
        let pdf = String::from_utf8_lossy(&sink.bytes);
        assert!(pdf.contains("/Root 1 0 R"));
        assert!(pdf.contains("/Kids [9 0 R ]"));
        Ok::<(), Error>(())
    })
    .unwrap();
}

#[test]
fn malformed_page_tree_links_counts_cycles_and_catalog_fail_before_output() {
    run(async {
        for case in 0..6 {
            let (mut source, objects, mut pages) = existing_tree_fragment();
            let mut catalog = Some(reference(1));
            let expected = match case {
                0 => {
                    replace_in_object(&mut source, objects[4], b"/Count 2", b"/Count 3");
                    PdfErrorKind::Malformed
                }
                1 => {
                    replace_in_object(&mut source, objects[2], b"/Parent 7", b"/Parent 5");
                    PdfErrorKind::Malformed
                }
                2 => {
                    replace_in_object(
                        &mut source,
                        objects[4],
                        b"/Kids [9 0 R 7 0 R]",
                        b"/Kids [9 0 R 5 0 R]",
                    );
                    PdfErrorKind::Malformed
                }
                3 => {
                    pages.reverse();
                    PdfErrorKind::AmbiguousRepair
                }
                4 => {
                    replace_in_object(&mut source, objects[1], b"/Pages 5", b"/Pages 7");
                    PdfErrorKind::Malformed
                }
                _ => {
                    catalog = None;
                    PdfErrorKind::AmbiguousRepair
                }
            };
            let mut sink = BytesSink::default();
            let plan = FragmentPlan {
                objects: &objects,
                pages: &pages,
                pages_root: reference(5),
                catalog,
            };
            let error =
                reconstruct_fragment(&mut source, &mut sink, &plan, &Limits::default(), &NEVER)
                    .await
                    .unwrap_err();
            assert!(
                matches!(error, Error::Pdf { kind, .. } if kind == expected),
                "case {case}: {error}"
            );
            assert!(sink.bytes.is_empty());
        }
    });
}

#[test]
fn ambiguous_parent_and_repeated_pages_fail_before_writing() {
    run(async {
        let (mut source, objects, pages) = two_page_fragment();
        let mut sink = BytesSink::default();
        let wrong_root = FragmentPlan {
            objects: &objects,
            pages: &pages,
            pages_root: reference(7),
            catalog: None,
        };
        assert!(matches!(
            reconstruct_fragment(
                &mut source,
                &mut sink,
                &wrong_root,
                &Limits::default(),
                &NEVER,
            )
            .await,
            Err(Error::Pdf { .. })
        ));
        assert!(sink.bytes.is_empty());

        let repeated = [pages[0], pages[0]];
        let plan = FragmentPlan {
            objects: &objects,
            pages: &repeated,
            pages_root: reference(5),
            catalog: None,
        };
        assert!(matches!(
            reconstruct_fragment(&mut source, &mut sink, &plan, &Limits::default(), &NEVER,).await,
            Err(Error::Pdf {
                kind: PdfErrorKind::AmbiguousRepair,
                ..
            })
        ));
        assert!(sink.bytes.is_empty());
    });
}

#[test]
fn a_sink_failure_never_returns_a_success_report() {
    run(async {
        let (mut source, objects, pages) = two_page_fragment();
        let mut sink = BytesSink {
            fail_after: Some(60),
            ..BytesSink::default()
        };
        let plan = FragmentPlan {
            objects: &objects,
            pages: &pages,
            pages_root: reference(5),
            catalog: None,
        };
        assert!(matches!(
            reconstruct_fragment(&mut source, &mut sink, &plan, &Limits::default(), &NEVER,).await,
            Err(Error::Io(_))
        ));
        assert!(!sink.bytes.is_empty());
    });
}

#[test]
fn malformed_indirect_stream_length_fails_before_output() {
    run(async {
        let (mut source, objects, pages) = two_page_fragment();
        let scalar_offset = objects[2].range.offset as usize + b"4 0 obj\n".len();
        source.bytes[scalar_offset] = b'1';
        let mut sink = BytesSink::default();
        let plan = FragmentPlan {
            objects: &objects,
            pages: &pages,
            pages_root: reference(5),
            catalog: None,
        };
        assert!(matches!(
            reconstruct_fragment(&mut source, &mut sink, &plan, &Limits::default(), &NEVER,).await,
            Err(Error::Pdf {
                kind: PdfErrorKind::Malformed,
                ..
            })
        ));
        assert!(sink.bytes.is_empty());
    });
}

#[test]
fn input_and_output_limits_are_preflighted_before_sink_write() {
    run(async {
        let (mut source, objects, pages) = two_page_fragment();
        let mut sink = BytesSink::default();
        let plan = FragmentPlan {
            objects: &objects,
            pages: &pages,
            pages_root: reference(5),
            catalog: None,
        };
        let limits = Limits {
            max_output_bytes: 100,
            ..Limits::default()
        };
        assert!(matches!(
            reconstruct_fragment(&mut source, &mut sink, &plan, &limits, &NEVER).await,
            Err(Error::PdfLimitExceeded {
                resource: "output bytes",
                ..
            })
        ));
        assert!(sink.bytes.is_empty());

        // The summed object spans are charged against the input limit.
        let first = objects[0].range.length;
        let limits = Limits {
            max_input_bytes: first,
            ..Limits::default()
        };
        let error = reconstruct_fragment(&mut source, &mut sink, &plan, &limits, &NEVER)
            .await
            .unwrap_err();
        assert!(
            matches!(
                error,
                Error::PdfLimitExceeded {
                    object: Some((number, 0)),
                    resource: "input bytes",
                    limit,
                    attempted,
                    ..
                } if number == objects[1].reference.number
                    && limit == first
                    && attempted == first + objects[1].range.length
            ),
            "{error:?}"
        );
        assert!(sink.bytes.is_empty());
    });
}

#[test]
fn outline_destinations_must_target_ordered_pages() {
    run(async {
        let (mut source, mut objects, pages) = two_page_fragment();
        let wrong_destination = add_object(
            &mut source.bytes,
            12,
            b"<< /Title (Wrong) /Dest [4 0 R /Fit] >>",
        );
        objects.push(wrong_destination);
        let mut sink = BytesSink::default();
        let plan = FragmentPlan {
            objects: &objects,
            pages: &pages,
            pages_root: reference(5),
            catalog: None,
        };
        assert!(matches!(
            reconstruct_fragment(&mut source, &mut sink, &plan, &Limits::default(), &NEVER,).await,
            Err(Error::Pdf {
                kind: PdfErrorKind::Malformed,
                reason: "outline destination does not target an ordered Page object",
                ..
            })
        ));
        assert!(sink.bytes.is_empty());

        let (mut source, mut objects, pages) = two_page_fragment();
        objects.push(add_object(
            &mut source.bytes,
            12,
            b"<< /Title (Named) /Dest (named-destination) >>",
        ));
        let plan = FragmentPlan {
            objects: &objects,
            pages: &pages,
            pages_root: reference(5),
            catalog: None,
        };
        assert!(matches!(
            reconstruct_fragment(&mut source, &mut sink, &plan, &Limits::default(), &NEVER,).await,
            Err(Error::Pdf {
                kind: PdfErrorKind::UnsupportedFeature,
                ..
            })
        ));
        assert!(sink.bytes.is_empty());
    });
}

#[test]
fn outline_count_limit_is_checked_before_writing() {
    run(async {
        let (mut source, mut objects, pages) = two_page_fragment();
        let outline = add_object(
            &mut source.bytes,
            12,
            b"<< /Title (Valid) /Dest [9 0 R /Fit] >>",
        );
        objects.push(outline);
        let plan = FragmentPlan {
            objects: &objects,
            pages: &pages,
            pages_root: reference(5),
            catalog: None,
        };
        let mut sink = BytesSink::default();
        let limits = Limits {
            max_bookmarks: 0,
            ..Limits::default()
        };
        assert!(matches!(
            reconstruct_fragment(&mut source, &mut sink, &plan, &limits, &NEVER).await,
            Err(Error::PdfLimitExceeded {
                resource: "bookmarks",
                offset,
                object: Some((12, 0)),
                ..
            }) if offset == outline.range.offset
        ));
        assert!(sink.bytes.is_empty());
    });
}

#[test]
fn one_byte_io_chunks_still_finish_with_bounded_output() {
    run(async {
        let (mut source, objects, pages) = two_page_fragment();
        let mut sink = BytesSink::default();
        let limits = Limits {
            io_chunk_bytes: 1,
            ..Limits::default()
        };
        let plan = FragmentPlan {
            objects: &objects,
            pages: &pages,
            pages_root: reference(5),
            catalog: None,
        };
        let report = reconstruct_fragment(&mut source, &mut sink, &plan, &limits, &NEVER).await?;
        assert_eq!(report.pages_converted, 2);
        assert_eq!(report.output_bytes_written, sink.bytes.len() as u64);
        Ok::<(), Error>(())
    })
    .unwrap();
}

#[test]
fn duplicate_overlapping_and_truncated_spans_fail_before_output() {
    run(async {
        for case in 0..3 {
            let (mut source, mut objects, pages) = two_page_fragment();
            match case {
                0 => objects[1].reference = objects[0].reference,
                1 => objects[1].range.offset = objects[0].range.offset + 1,
                _ => objects[1].range.length = source.size(),
            }
            let mut sink = BytesSink::default();
            let plan = FragmentPlan {
                objects: &objects,
                pages: &pages,
                pages_root: reference(5),
                catalog: None,
            };
            let error =
                reconstruct_fragment(&mut source, &mut sink, &plan, &Limits::default(), &NEVER)
                    .await
                    .unwrap_err();
            assert!(
                matches!(error, Error::Pdf { .. } | Error::TruncatedInput { .. }),
                "case {case}: {error}"
            );
            assert!(sink.bytes.is_empty());
        }
    });
}

#[test]
fn missing_reference_generation_and_memory_limit_are_typed() {
    run(async {
        let (mut source, mut objects, pages) = two_page_fragment();
        objects.push(add_object(&mut source.bytes, 12, b"<< /Contents 77 0 R >>"));
        let mut sink = BytesSink::default();
        let plan = FragmentPlan {
            objects: &objects,
            pages: &pages,
            pages_root: reference(5),
            catalog: None,
        };
        assert!(matches!(
            reconstruct_fragment(&mut source, &mut sink, &plan, &Limits::default(), &NEVER,).await,
            Err(Error::Pdf {
                kind: PdfErrorKind::Malformed,
                reason: "indirect reference targets a missing object",
                ..
            })
        ));
        assert!(sink.bytes.is_empty());

        let (mut source, mut objects, pages) = two_page_fragment();
        objects[0].reference.generation = 1;
        let plan = FragmentPlan {
            objects: &objects,
            pages: &pages,
            pages_root: reference(5),
            catalog: None,
        };
        assert!(matches!(
            reconstruct_fragment(&mut source, &mut sink, &plan, &Limits::default(), &NEVER,).await,
            Err(Error::Pdf {
                kind: PdfErrorKind::UnsupportedFeature,
                ..
            })
        ));
        assert!(sink.bytes.is_empty());

        let (mut source, objects, pages) = two_page_fragment();
        let limits = Limits {
            io_chunk_bytes: 16,
            max_allocation_bytes: 64,
            ..Limits::default()
        };
        let plan = FragmentPlan {
            objects: &objects,
            pages: &pages,
            pages_root: reference(5),
            catalog: None,
        };
        assert!(matches!(
            reconstruct_fragment(&mut source, &mut sink, &plan, &limits, &NEVER).await,
            Err(Error::PdfLimitExceeded {
                resource: "PDF allocation bytes",
                object: Some((9, 0)),
                ..
            })
        ));
        assert!(sink.bytes.is_empty());
    });
}

#[test]
fn empty_pages_missing_roots_and_invalid_ids_reject_without_output() {
    run(async {
        for case in 0..5 {
            let (mut source, objects, pages) = two_page_fragment();
            let mut requested = pages.clone();
            let mut root = reference(5);
            let mut catalog = None;
            match case {
                0 => requested.clear(),
                1 => requested[1] = reference(77),
                2 => catalog = Some(reference(11)),
                3 => root = reference(0),
                _ => root = reference(MAX_PDF_OBJECTS + 1),
            }
            let plan = FragmentPlan {
                objects: &objects,
                pages: &requested,
                pages_root: root,
                catalog,
            };
            let mut sink = BytesSink::default();
            assert!(
                reconstruct_fragment(&mut source, &mut sink, &plan, &Limits::default(), &NEVER,)
                    .await
                    .is_err(),
                "case {case} unexpectedly succeeded"
            );
            assert!(sink.bytes.is_empty());
        }
    });
}

#[test]
fn missing_root_rejects_nested_nodes_and_non_page_kids() {
    run(async {
        let (mut source, mut objects, pages) = two_page_fragment();
        objects.push(add_object(
            &mut source.bytes,
            7,
            b"<< /Type /Pages /Parent 5 0 R /Count 1 /Kids [9 0 R] >>",
        ));
        let plan = FragmentPlan {
            objects: &objects,
            pages: &pages,
            pages_root: reference(5),
            catalog: None,
        };
        let mut sink = BytesSink::default();
        assert!(matches!(
            reconstruct_fragment(&mut source, &mut sink, &plan, &Limits::default(), &NEVER,).await,
            Err(Error::Pdf {
                kind: PdfErrorKind::AmbiguousRepair,
                ..
            })
        ));
        assert!(sink.bytes.is_empty());

        let (mut source, objects, mut pages) = two_page_fragment();
        pages[0] = reference(4);
        let plan = FragmentPlan {
            objects: &objects,
            pages: &pages,
            pages_root: reference(5),
            catalog: None,
        };
        assert!(matches!(
            reconstruct_fragment(&mut source, &mut sink, &plan, &Limits::default(), &NEVER,).await,
            Err(Error::Pdf {
                kind: PdfErrorKind::Malformed,
                ..
            })
        ));
        assert!(sink.bytes.is_empty());
    });
}

#[test]
fn existing_tree_rejects_empty_count_over_limit_and_non_page_child() {
    run(async {
        for case in 0..3 {
            let (mut source, objects, pages) = existing_tree_fragment();
            let mut limits = Limits::default();
            match case {
                0 => replace_in_object(&mut source, objects[4], b"/Count 2", b"/Count 0"),
                1 => {
                    replace_in_object(&mut source, objects[4], b"/Count 2", b"/Count 3");
                    limits.max_pages = 2;
                }
                _ => replace_in_object(
                    &mut source,
                    objects[4],
                    b"/Kids [9 0 R 7 0 R]",
                    b"/Kids [9 0 R 1 0 R]",
                ),
            }
            let mut sink = BytesSink::default();
            let plan = FragmentPlan {
                objects: &objects,
                pages: &pages,
                pages_root: reference(5),
                catalog: Some(reference(1)),
            };
            assert!(
                reconstruct_fragment(&mut source, &mut sink, &plan, &limits, &NEVER)
                    .await
                    .is_err(),
                "case {case} unexpectedly succeeded"
            );
            assert!(sink.bytes.is_empty());
        }
    });
}

#[test]
fn multi_digit_page_refs_and_small_chunk_flushes_are_supported() {
    run(async {
        let mut bytes = Vec::new();
        let first = add_object(
            &mut bytes,
            12,
            b"<< /Type /Page /Parent 5 0 R /MediaBox [0 0 200 300] /Resources << >> >>",
        );
        let second = add_object(
            &mut bytes,
            3,
            b"<< /Type /Page /Parent 5 0 R /MediaBox [0 0 400 250] /Resources << >> >>",
        );
        let mut source = BytesSource::new(bytes);
        let objects = [first, second];
        let pages = [reference(12), reference(3)];
        let plan = FragmentPlan {
            objects: &objects,
            pages: &pages,
            pages_root: reference(5),
            catalog: None,
        };
        let limits = Limits {
            io_chunk_bytes: 20,
            ..Limits::default()
        };
        let mut sink = BytesSink::default();
        let report = reconstruct_fragment(&mut source, &mut sink, &plan, &limits, &NEVER).await?;
        assert_eq!(report.pages_converted, 2);
        assert!(String::from_utf8_lossy(&sink.bytes).contains("/Kids [12 0 R 3 0 R ]"));
        Ok::<(), Error>(())
    })
    .unwrap();
}

#[test]
fn xref_rows_are_fixed_width_and_link_sparse_free_slots() {
    assert_eq!(xref_entry(17, 0, b'n').unwrap(), *b"0000000017 00000 n \n");
    assert_eq!(
        xref_entry(9, 65_535, b'f').unwrap(),
        *b"0000000009 65535 f \n"
    );
    let records = [
        Record::synthetic(reference(2)),
        Record::synthetic(reference(5)),
    ];
    assert_eq!(next_free_number(0, 5, &records, 0), 1);
    assert_eq!(next_free_number(1, 5, &records, 0), 3);
    assert_eq!(next_free_number(4, 5, &records, 1), 0);
}

#[test]
fn repeated_page_tree_links_cannot_expand_the_walk_stack() {
    let root = reference(5);
    let leaf = reference(9);
    let records = [Record::synthetic(root), Record::synthetic(leaf)];
    let kinds = [
        Some(FragmentKind::Pages {
            parent: None,
            count: 1,
            kids: vec![leaf; 10],
            has_media_box: false,
        }),
        Some(FragmentKind::Page {
            parent: root,
            has_media_box: true,
        }),
    ];
    let pages = [leaf];
    let plan = FragmentPlan {
        objects: &[],
        pages: &pages,
        pages_root: root,
        catalog: None,
    };
    assert!(matches!(
        validate_existing_page_tree(&plan, &records, &kinds, 0, &Limits::default()),
        Err(Error::Pdf {
            kind: PdfErrorKind::AmbiguousRepair,
            ..
        })
    ));
}

#[test]
fn page_inventory_catalog_role_and_branch_parent_must_agree() {
    run(async {
        for case in 0..3 {
            let (mut source, mut objects, pages) = existing_tree_fragment();
            let expected = match case {
                0 => {
                    objects.push(add_object(
                        &mut source.bytes,
                        11,
                        b"<< /Type /Page /Parent 5 0 R /MediaBox [0 0 100 100] >>",
                    ));
                    PdfErrorKind::AmbiguousRepair
                }
                1 => {
                    replace_in_object(
                        &mut source,
                        objects[1],
                        b"/Type /Catalog",
                        b"/Type /Catolog",
                    );
                    PdfErrorKind::Malformed
                }
                _ => {
                    replace_in_object(&mut source, objects[3], b"/Parent 5", b"/Parent 9");
                    PdfErrorKind::Malformed
                }
            };
            let plan = FragmentPlan {
                objects: &objects,
                pages: &pages,
                pages_root: reference(5),
                catalog: Some(reference(1)),
            };
            let mut sink = BytesSink::default();
            let error =
                reconstruct_fragment(&mut source, &mut sink, &plan, &Limits::default(), &NEVER)
                    .await
                    .unwrap_err();
            assert!(
                matches!(error, Error::Pdf { kind, .. } if kind == expected),
                "case {case}: {error}"
            );
            assert!(sink.bytes.is_empty());
        }
    });
}

#[test]
fn page_media_box_must_be_direct_or_inherited_from_the_page_tree() {
    run(async {
        let (mut source, objects, pages) = two_page_fragment();
        replace_in_object(&mut source, objects[0], b"/MediaBox", b"/Mediabax");
        let plan = FragmentPlan {
            objects: &objects,
            pages: &pages,
            pages_root: reference(5),
            catalog: None,
        };
        let mut sink = BytesSink::default();
        assert!(matches!(
            reconstruct_fragment(&mut source, &mut sink, &plan, &Limits::default(), &NEVER,).await,
            Err(Error::Pdf {
                kind: PdfErrorKind::Malformed,
                ..
            })
        ));
        assert!(sink.bytes.is_empty());

        let (mut source, objects, pages) = existing_tree_fragment();
        replace_in_object(&mut source, objects[0], b"/MediaBox", b"/Mediabax");
        replace_in_object(&mut source, objects[2], b"/MediaBox", b"/Mediabax");
        let plan = FragmentPlan {
            objects: &objects,
            pages: &pages,
            pages_root: reference(5),
            catalog: Some(reference(1)),
        };
        assert!(matches!(
            reconstruct_fragment(&mut source, &mut sink, &plan, &Limits::default(), &NEVER,).await,
            Err(Error::Pdf {
                kind: PdfErrorKind::Malformed,
                ..
            })
        ));
        assert!(sink.bytes.is_empty());

        let mut bytes = Vec::new();
        let page = add_object(&mut bytes, 9, b"<< /Type /Page /Parent 7 0 R >>");
        let branch = add_object(
            &mut bytes,
            7,
            b"<< /Type /Pages /Parent 5 0 R /Count 1 /Kids [9 0 R] /MediaBox [0 0 200 300] >>",
        );
        let root = add_object(&mut bytes, 5, b"<< /Type /Pages /Count 1 /Kids [7 0 R] >>");
        let objects = [page, branch, root];
        let pages = [reference(9)];
        let plan = FragmentPlan {
            objects: &objects,
            pages: &pages,
            pages_root: reference(5),
            catalog: None,
        };
        let mut source = BytesSource::new(bytes);
        let report =
            reconstruct_fragment(&mut source, &mut sink, &plan, &Limits::default(), &NEVER).await?;
        assert_eq!(report.pages_converted, 1);
        assert_eq!(report.output_bytes_written, sink.bytes.len() as u64);
        Ok::<(), Error>(())
    })
    .unwrap();
}

#[test]
fn page_contents_references_only_streams_or_one_indirect_stream_array() {
    run(async {
        for case in 0..5 {
            let mut bytes = Vec::new();
            let contents = match case {
                0 => b"4 0 R".as_slice(),
                1 => b"[4 0 R]".as_slice(),
                4 => b"6 0 R".as_slice(),
                _ => b"4 0 R".as_slice(),
            };
            let mut page_body =
                b"<< /Type /Page /Parent 5 0 R /MediaBox [0 0 200 300] /Contents ".to_vec();
            page_body.extend_from_slice(contents);
            page_body.extend_from_slice(b" >>");
            let page = add_object(&mut bytes, 9, &page_body);
            let target = match case {
                0 | 1 => add_object(&mut bytes, 4, b"42"),
                2 => add_object(&mut bytes, 4, b"[6 0 R]"),
                _ => add_object(&mut bytes, 4, b"[7 0 R]"),
            };
            let stream = add_object(&mut bytes, 6, b"<< /Length 0 >>\nstream\nendstream");
            let scalar = add_object(&mut bytes, 7, b"0");
            let objects = [page, target, stream, scalar];
            let pages = [reference(9)];
            let plan = FragmentPlan {
                objects: &objects,
                pages: &pages,
                pages_root: reference(5),
                catalog: None,
            };
            let mut source = BytesSource::new(bytes);
            let mut sink = BytesSink::default();
            let result =
                reconstruct_fragment(&mut source, &mut sink, &plan, &Limits::default(), &NEVER)
                    .await;
            // Case 2 uses an indirect stream array; case 4 names the
            // stream directly.
            if case == 2 || case == 4 {
                let report = result?;
                assert_eq!(report.pages_converted, 1);
                assert_eq!(report.output_bytes_written, sink.bytes.len() as u64);
            } else {
                let rejected = matches!(
                    result,
                    Err(Error::Pdf {
                        kind: PdfErrorKind::Malformed,
                        ..
                    })
                );
                assert!(rejected, "case {case} was accepted");
                assert!(sink.bytes.is_empty());
            }
        }
        Ok::<(), Error>(())
    })
    .unwrap();
}

#[test]
fn many_pages_can_share_one_indirect_contents_array() {
    run(async {
        let mut bytes = Vec::new();
        let mut objects = Vec::new();
        let mut pages = Vec::new();
        for ordinal in 0..64 {
            let number = 10 + ordinal;
            pages.push(reference(number));
            objects.push(add_object(
                &mut bytes,
                number,
                b"<< /Type /Page /Parent 5 0 R /MediaBox [0 0 200 300] /Contents 4 0 R >>",
            ));
        }
        let mut array = b"[".to_vec();
        for ordinal in 0..64 {
            array.extend_from_slice(format!("{} 0 R ", 1000 + ordinal).as_bytes());
        }
        array.extend_from_slice(b"]");
        objects.push(add_object(&mut bytes, 4, &array));
        for ordinal in 0..64 {
            objects.push(add_object(
                &mut bytes,
                1000 + ordinal,
                b"<< /Length 0 >>\nstream\nendstream",
            ));
        }
        let mut source = BytesSource::new(bytes);
        let mut sink = BytesSink::default();
        let plan = FragmentPlan {
            objects: &objects,
            pages: &pages,
            pages_root: reference(5),
            catalog: None,
        };
        let report =
            reconstruct_fragment(&mut source, &mut sink, &plan, &Limits::default(), &NEVER).await?;
        assert_eq!(report.pages_converted, 64);
        assert_eq!(report.output_bytes_written, sink.bytes.len() as u64);
        Ok::<(), Error>(())
    })
    .unwrap();
}

#[test]
fn malformed_spans_and_orphaned_ordered_pages_fail_before_output() {
    run(async {
        for case in 0..2 {
            let (mut source, mut objects, pages) = two_page_fragment();
            if case == 0 {
                objects[0].range.length = 0;
            } else {
                objects[0].range.offset = u64::MAX;
                objects[0].range.length = 2;
            }
            let plan = FragmentPlan {
                objects: &objects,
                pages: &pages,
                pages_root: reference(5),
                catalog: None,
            };
            let mut sink = BytesSink::default();
            assert!(matches!(
                reconstruct_fragment(&mut source, &mut sink, &plan, &Limits::default(), &NEVER,)
                    .await,
                Err(Error::Pdf {
                    kind: PdfErrorKind::Malformed,
                    ..
                })
            ));
            assert!(sink.bytes.is_empty());
        }

        let (mut source, objects, pages) = two_page_fragment();
        replace_in_object(&mut source, objects[0], b"/Parent 5", b"/Parent 6");
        let plan = FragmentPlan {
            objects: &objects,
            pages: &pages,
            pages_root: reference(5),
            catalog: None,
        };
        let mut sink = BytesSink::default();
        assert!(matches!(
            reconstruct_fragment(&mut source, &mut sink, &plan, &Limits::default(), &NEVER,).await,
            Err(Error::Pdf {
                kind: PdfErrorKind::AmbiguousRepair,
                ..
            })
        ));
        assert!(sink.bytes.is_empty());

        let mut bytes = Vec::new();
        let first = add_object(
            &mut bytes,
            9,
            b"<< /Type /Page /Parent 5 0 R /MediaBox [0 0 200 300] >>",
        );
        let second = add_object(
            &mut bytes,
            3,
            b"<< /Type /Page /Parent 5 0 R /MediaBox [0 0 200 300] >>",
        );
        let root = add_object(&mut bytes, 5, b"<< /Type /Pages /Count 1 /Kids [9 0 R] >>");
        let objects = [first, second, root];
        let pages = [reference(9), reference(3)];
        let plan = FragmentPlan {
            objects: &objects,
            pages: &pages,
            pages_root: reference(5),
            catalog: None,
        };
        let mut source = BytesSource::new(bytes);
        assert!(matches!(
            reconstruct_fragment(&mut source, &mut sink, &plan, &Limits::default(), &NEVER,).await,
            Err(Error::Pdf {
                kind: PdfErrorKind::AmbiguousRepair,
                ..
            })
        ));
        assert!(sink.bytes.is_empty());
    });
}

struct TestCancel(Cell<bool>);

impl Cancellation for TestCancel {
    fn is_cancelled(&self) -> bool {
        self.0.get()
    }
}

struct CancelOnFlushSink<'a> {
    bytes: Vec<u8>,
    cancellation: &'a TestCancel,
}

impl SequentialSink for CancelOnFlushSink<'_> {
    async fn write(&mut self, bytes: &[u8]) -> Result<usize> {
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    async fn flush(&mut self) -> Result<()> {
        self.cancellation.0.set(true);
        Ok(())
    }
}

#[test]
fn cancellation_before_input_and_during_flush_never_reports_success() {
    run(async {
        let (mut source, objects, pages) = two_page_fragment();
        let plan = FragmentPlan {
            objects: &objects,
            pages: &pages,
            pages_root: reference(5),
            catalog: None,
        };
        let cancelled = TestCancel(Cell::new(true));
        let mut sink = BytesSink::default();
        assert!(matches!(
            reconstruct_fragment(
                &mut source,
                &mut sink,
                &plan,
                &Limits::default(),
                &cancelled,
            )
            .await,
            Err(Error::Cancelled)
        ));
        assert!(sink.bytes.is_empty());

        let cancellation = TestCancel(Cell::new(false));
        let mut sink = CancelOnFlushSink {
            bytes: Vec::new(),
            cancellation: &cancellation,
        };
        assert!(matches!(
            reconstruct_fragment(
                &mut source,
                &mut sink,
                &plan,
                &Limits::default(),
                &cancellation,
            )
            .await,
            Err(Error::Cancelled)
        ));
        assert!(!sink.bytes.is_empty());
    });
}

#[test]
fn fragment_bookmarks_form_a_readable_unicode_outline_tree() {
    run(async {
        let (mut source, objects, pages) = two_page_fragment();
        let plan = FragmentPlan {
            objects: &objects,
            pages: &pages,
            pages_root: reference(5),
            catalog: None,
        };
        let bookmarks = [
            Bookmark {
                depth: 0,
                title: "第一章".into(),
                page_index: 0,
            },
            Bookmark {
                depth: 1,
                title: "Section".into(),
                page_index: 1,
            },
            Bookmark {
                depth: 2,
                title: "𝄞".into(),
                page_index: 0,
            },
            Bookmark {
                depth: 0,
                title: "末章".into(),
                page_index: 1,
            },
        ];
        let mut sink = BytesSink::default();
        let report = reconstruct_fragment_with_bookmarks(
            &mut source,
            &mut sink,
            &plan,
            &bookmarks,
            &Limits::default(),
            &NEVER,
        )
        .await?;
        assert_eq!(report.pages_converted, 2);
        assert_eq!(report.bookmarks_written, 4);
        assert_eq!(report.output_bytes_written, sink.bytes.len() as u64);
        let text = String::from_utf8_lossy(&sink.bytes);
        assert!(text.contains("/Outlines 11 0 R"));
        assert!(text.contains("/First 12 0 R /Last 15 0 R /Count 4"));
        assert!(text.contains("/First 13 0 R /Last 13 0 R /Count 2"));
        assert!(text.contains("/First 14 0 R /Last 14 0 R /Count 1"));
        assert!(text.contains("/Next 15 0 R"));
        assert!(text.contains("/Prev 12 0 R"));
        assert!(text.contains("/Title <FEFFD834DD1E>"));
        let mut output = BytesSource::new(sink.bytes);
        let output_size = output.size();
        let inspected = super::super::input::PdfIndex::open(
            &mut output,
            PdfRange {
                offset: 0,
                length: output_size,
            },
            &Limits::default(),
            &NEVER,
        )
        .await?;
        assert_eq!(inspected.pages(), pages);
        assert!(inspected.has_outlines());
        Ok::<(), Error>(())
    })
    .unwrap();
}

#[test]
fn invalid_fragment_bookmarks_fail_before_any_output() {
    run(async {
        let (mut source, objects, pages) = two_page_fragment();
        let plan = FragmentPlan {
            objects: &objects,
            pages: &pages,
            pages_root: reference(5),
            catalog: None,
        };
        let cases = [
            Bookmark {
                depth: 1,
                title: "skips root".into(),
                page_index: 0,
            },
            Bookmark {
                depth: 0,
                title: "invalid page".into(),
                page_index: 2,
            },
            Bookmark {
                depth: 0,
                title: String::new(),
                page_index: 0,
            },
        ];
        for bookmark in cases {
            let mut sink = BytesSink::default();
            let result = reconstruct_fragment_with_bookmarks(
                &mut source,
                &mut sink,
                &plan,
                &[bookmark],
                &Limits::default(),
                &NEVER,
            )
            .await;
            assert!(matches!(result, Err(Error::Pdf { .. })));
            assert!(sink.bytes.is_empty());
        }
        let limits = Limits {
            max_bookmarks: 0,
            ..Limits::default()
        };
        let mut sink = BytesSink::default();
        let result = reconstruct_fragment_with_bookmarks(
            &mut source,
            &mut sink,
            &plan,
            &[Bookmark {
                depth: 0,
                title: "too many".into(),
                page_index: 0,
            }],
            &limits,
            &NEVER,
        )
        .await;
        assert!(matches!(result, Err(Error::PdfLimitExceeded { .. })));
        assert!(sink.bytes.is_empty());
    });
}

/// The usual plan: `pages` under a synthetic-or-existing root 5 and no
/// catalog.
fn plan<'a>(objects: &'a [FragmentObject], pages: &'a [PdfRef]) -> FragmentPlan<'a> {
    FragmentPlan {
        objects,
        pages,
        pages_root: reference(5),
        catalog: None,
    }
}

/// Nested page tree, an indirect Contents array shared through a scalar
/// array object, a direct Contents array, and an indirect stream Length.
fn content_rich_fragment() -> (BytesSource, Vec<FragmentObject>, Vec<PdfRef>) {
    let mut bytes = b"CAJ\0content fragments\n".to_vec();
    let objects = vec![
        add_object(
            &mut bytes,
            9,
            b"<< /Type /Page /Parent 5 0 R /MediaBox [0 0 200 300] /Contents 4 0 R >>",
        ),
        add_object(
            &mut bytes,
            3,
            b"<< /Type /Page /Parent 7 0 R /Contents [6 0 R 8 0 R] >>",
        ),
        add_object(
            &mut bytes,
            7,
            b"<< /Type /Pages /Parent 5 0 R /Count 1 /Kids [3 0 R] /MediaBox [0 0 400 250] >>",
        ),
        add_object(
            &mut bytes,
            5,
            b"<< /Type /Pages /Count 2 /Kids [9 0 R 7 0 R] >>",
        ),
        add_object(&mut bytes, 4, b"[6 0 R 8 0 R]"),
        add_object(
            &mut bytes,
            6,
            b"<< /Length 2 0 R >>\nstream\nq Q\nendstream",
        ),
        add_object(&mut bytes, 8, b"<< /Length 0 >>\nstream\nendstream"),
        add_object(&mut bytes, 2, b"3"),
    ];
    (
        BytesSource::new(bytes),
        objects,
        vec![reference(9), reference(3)],
    )
}

fn alternating_bookmarks(count: u32) -> Vec<Bookmark> {
    (0..count)
        .map(|index| Bookmark {
            depth: index % 2,
            title: format!("Item {index}"),
            page_index: index % 2,
        })
        .collect()
}

#[test]
fn source_that_over_reports_reads_is_rejected() {
    run(async {
        let (mut source, objects, pages) = two_page_fragment();
        source.over_report = true;
        let plan = plan(&objects, &pages);
        let mut sink = BytesSink::default();
        assert!(matches!(
            reconstruct_fragment(&mut source, &mut sink, &plan, &Limits::default(), &NEVER,).await,
            Err(Error::InvalidInput {
                reason: "PDF source reported more bytes than requested"
            })
        ));
        assert!(sink.bytes.is_empty());
    });
}

#[test]
fn fragment_span_total_overflow_is_an_input_limit_before_reads() {
    run(async {
        let objects = [
            FragmentObject {
                reference: reference(1),
                range: PdfRange {
                    offset: 0,
                    length: u64::MAX,
                },
            },
            FragmentObject {
                reference: reference(3),
                range: PdfRange {
                    offset: 0,
                    length: 2,
                },
            },
        ];
        let pages = [reference(3)];
        let plan = FragmentPlan {
            objects: &objects,
            pages: &pages,
            pages_root: reference(2),
            catalog: None,
        };
        let limits = Limits {
            max_input_bytes: u64::MAX,
            ..Limits::default()
        };
        let mut source = BytesSource::sparse(u64::MAX, Vec::new());
        let mut sink = BytesSink::default();
        let error = reconstruct_fragment(&mut source, &mut sink, &plan, &limits, &NEVER)
            .await
            .unwrap_err();
        assert!(
            matches!(
                error,
                Error::PdfLimitExceeded {
                    object: Some((3, 0)),
                    resource: "input bytes",
                    attempted: u64::MAX,
                    ..
                }
            ),
            "{error}"
        );
        assert_eq!(source.bytes_read, 0);
        assert!(sink.bytes.is_empty());
    });
}

#[test]
fn page_limit_names_the_first_ordered_page_span() {
    run(async {
        let (mut source, objects, pages) = two_page_fragment();
        let plan = plan(&objects, &pages);
        let limits = Limits {
            max_pages: 1,
            ..Limits::default()
        };
        let mut sink = BytesSink::default();
        let error = reconstruct_fragment(&mut source, &mut sink, &plan, &limits, &NEVER)
            .await
            .unwrap_err();
        assert!(
            matches!(
                error,
                Error::PdfLimitExceeded {
                    resource: "pages",
                    object: Some((9, 0)),
                    offset,
                    limit: 1,
                    attempted: 2,
                } if offset == objects[0].range.offset
            ),
            "{error}"
        );
        assert!(sink.bytes.is_empty());
    });
}

#[test]
fn outline_depth_is_bounded_before_output() {
    run(async {
        let (mut source, objects, pages) = two_page_fragment();
        let plan = plan(&objects, &pages);
        let deep: Vec<Bookmark> = (0..=MAX_OUTLINE_DEPTH as u32)
            .map(|depth| Bookmark {
                depth,
                title: format!("Level {depth}"),
                page_index: 0,
            })
            .collect();
        let mut sink = BytesSink::default();
        let error = reconstruct_fragment_with_bookmarks(
            &mut source,
            &mut sink,
            &plan,
            &deep,
            &Limits::default(),
            &NEVER,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(
                error,
                Error::PdfLimitExceeded {
                    resource: "PDF outline depth",
                    limit: 256,
                    attempted: 257,
                    ..
                }
            ),
            "{error}"
        );
        assert!(sink.bytes.is_empty());

        let report = reconstruct_fragment_with_bookmarks(
            &mut source,
            &mut sink,
            &plan,
            &deep[..MAX_OUTLINE_DEPTH],
            &Limits::default(),
            &NEVER,
        )
        .await?;
        assert_eq!(report.bookmarks_written, 256);
        assert_eq!(report.output_bytes_written, sink.bytes.len() as u64);
        Ok::<(), Error>(())
    })
    .unwrap();
}

#[test]
fn long_outline_titles_are_emitted_in_bounded_chunks() {
    run(async {
        let (mut source, objects, pages) = two_page_fragment();
        let plan = plan(&objects, &pages);
        let bookmarks = [Bookmark {
            depth: 0,
            title: "A".repeat(3000),
            page_index: 1,
        }];
        let mut sink = BytesSink::default();
        let report = reconstruct_fragment_with_bookmarks(
            &mut source,
            &mut sink,
            &plan,
            &bookmarks,
            &Limits::default(),
            &NEVER,
        )
        .await?;
        assert_eq!(report.output_bytes_written, sink.bytes.len() as u64);
        let text = String::from_utf8_lossy(&sink.bytes);
        assert!(text.contains(&format!(
            "<< /Title <FEFF{}> /Parent 11 0 R /Dest [3 0 R /XYZ null null null] >>",
            "0041".repeat(3000)
        )));
        Ok::<(), Error>(())
    })
    .unwrap();
}

#[test]
fn nested_contents_and_outline_round_trip_through_the_reader() {
    run(async {
        let (mut source, objects, pages) = content_rich_fragment();
        let plan = plan(&objects, &pages);
        let bookmarks = alternating_bookmarks(4);
        let mut sink = BytesSink::default();
        let report = reconstruct_fragment_with_bookmarks(
            &mut source,
            &mut sink,
            &plan,
            &bookmarks,
            &Limits::default(),
            &NEVER,
        )
        .await?;
        assert_eq!(report.pages_converted, 2);
        assert_eq!(report.bookmarks_written, 4);
        let mut output = BytesSource::new(sink.bytes);
        let output_size = output.size();
        let inspected = super::super::input::PdfIndex::open(
            &mut output,
            PdfRange {
                offset: 0,
                length: output_size,
            },
            &Limits::default(),
            &NEVER,
        )
        .await?;
        assert_eq!(inspected.pages(), pages);
        assert!(inspected.has_outlines());
        Ok::<(), Error>(())
    })
    .unwrap();
}

/// The content-rich fragment plus many tiny unreferenced objects, so the
/// reconstruction indexes outweigh the per-object parser budget.
fn indexed_heavy_fragment() -> (BytesSource, Vec<FragmentObject>, Vec<PdfRef>) {
    let (mut source, mut objects, pages) = content_rich_fragment();
    for number in 100..220 {
        objects.push(add_object(&mut source.bytes, number, b"0"));
    }
    (source, objects, pages)
}

#[test]
fn allocation_ceiling_fails_closed_until_the_reported_need_fits() {
    run(async {
        let bookmarks = alternating_bookmarks(200);
        let mut expected = BytesSink::default();
        {
            let (mut source, objects, pages) = indexed_heavy_fragment();
            let plan = plan(&objects, &pages);
            reconstruct_fragment_with_bookmarks(
                &mut source,
                &mut expected,
                &plan,
                &bookmarks,
                &Limits::default(),
                &NEVER,
            )
            .await?;
        }
        let mut ceiling = 1_u64;
        let mut resources = Vec::new();
        loop {
            let (mut source, objects, pages) = indexed_heavy_fragment();
            let plan = plan(&objects, &pages);
            let limits = Limits {
                io_chunk_bytes: 1,
                max_allocation_bytes: ceiling,
                ..Limits::default()
            };
            let mut sink = BytesSink::default();
            match reconstruct_fragment_with_bookmarks(
                &mut source,
                &mut sink,
                &plan,
                &bookmarks,
                &limits,
                &NEVER,
            )
            .await
            {
                Ok(report) => {
                    assert_eq!(sink.bytes, expected.bytes);
                    assert_eq!(report.bookmarks_written, 200);
                    break;
                }
                Err(error) => {
                    assert!(
                        matches!(error, Error::PdfLimitExceeded { .. }),
                        "ceiling {ceiling}: {error}"
                    );
                    if let Error::PdfLimitExceeded {
                        resource,
                        limit,
                        attempted,
                        ..
                    } = error
                    {
                        // Parser budgets are derived from, and never
                        // exceed, the allocation ceiling; scale the
                        // ceiling so the failed budget would just admit
                        // the attempt.
                        assert!(limit <= ceiling && limit > 0, "{error}");
                        assert!(attempted > limit, "{error}");
                        assert!(sink.bytes.is_empty(), "{error}");
                        resources.push(resource);
                        ceiling = (ceiling * attempted).div_ceil(limit);
                    }
                }
            }
        }
        // The object index, page index, scalar and structure indexes,
        // page-tree kids, content evidence, traversal stack, and outline
        // nodes each tighten the requirement once; with many small
        // objects the per-object parser budget never binds first.
        assert!(resources.len() >= 10, "{resources:?}");
        assert!(
            resources
                .iter()
                .all(|resource| *resource == "PDF allocation bytes"),
            "{resources:?}"
        );
        Ok::<(), Error>(())
    })
    .unwrap();
}

#[test]
fn output_buffer_is_charged_together_with_the_object_index() {
    run(async {
        let (mut source, objects, pages) = two_page_fragment();
        let plan = plan(&objects, &pages);
        let limits = Limits {
            io_chunk_bytes: 4096,
            max_allocation_bytes: 4096,
            ..Limits::default()
        };
        let mut sink = BytesSink::default();
        let error = reconstruct_fragment(&mut source, &mut sink, &plan, &limits, &NEVER)
            .await
            .unwrap_err();
        assert!(
            matches!(
                error,
                Error::PdfLimitExceeded {
                    resource: "PDF allocation bytes",
                    object: Some((5, 0)),
                    offset: 0,
                    limit: 4096,
                    attempted,
                } if attempted == 4096 + 6 * size_of::<Record>() as u64
            ),
            "{error}"
        );
        assert!(sink.bytes.is_empty());
    });
}

#[test]
fn page_reference_buffer_flushes_before_it_overflows_a_chunk() {
    run(async {
        let (mut source, objects, pages) = two_page_fragment();
        let plan = plan(&objects, &pages);
        let limits = Limits {
            io_chunk_bytes: 10,
            ..Limits::default()
        };
        let mut sink = BytesSink::default();
        let report = reconstruct_fragment(&mut source, &mut sink, &plan, &limits, &NEVER).await?;
        assert_eq!(report.output_bytes_written, sink.bytes.len() as u64);
        assert!(String::from_utf8_lossy(&sink.bytes).contains("/Kids [9 0 R 3 0 R ]"));
        Ok::<(), Error>(())
    })
    .unwrap();
}

/// The smallest fragment that still reads a scalar, an indirect Contents
/// array, and a stream, and copies an existing page tree.
fn lean_content_fragment() -> (BytesSource, Vec<FragmentObject>, Vec<PdfRef>) {
    let mut bytes = b"CAJ\0".to_vec();
    let objects = vec![
        add_object(
            &mut bytes,
            9,
            b"<< /Type /Page /Parent 5 0 R /MediaBox [0 0 1 1] /Contents 4 0 R >>",
        ),
        add_object(&mut bytes, 5, b"<< /Type /Pages /Count 1 /Kids [9 0 R] >>"),
        add_object(&mut bytes, 4, b"[6 0 R]"),
        add_object(
            &mut bytes,
            6,
            b"<< /Length 2 0 R >>\nstream\nq Q\nendstream",
        ),
        add_object(&mut bytes, 2, b"3"),
    ];
    (BytesSource::new(bytes), objects, vec![reference(9)])
}

#[test]
fn cancellation_at_every_checkpoint_never_reports_success() {
    run(async {
        // Every query is tripped once, so the run is quadratic in the
        // checkpoint count: keep the fixture to one of each read, copy,
        // outline, xref, and trailer checkpoint.
        let bookmarks = alternating_bookmarks(1);
        let mut allowed = 0;
        loop {
            let (mut source, objects, pages) = lean_content_fragment();
            let plan = plan(&objects, &pages);
            let cancellation = CancelAfter::new(allowed);
            let mut sink = BytesSink::default();
            match reconstruct_fragment_with_bookmarks(
                &mut source,
                &mut sink,
                &plan,
                &bookmarks,
                &Limits::default(),
                &cancellation,
            )
            .await
            {
                Err(Error::Cancelled) => allowed += 1,
                result => {
                    let report = result.expect("only cancellation may stop the run");
                    assert!(allowed >= 100, "only {allowed} cancellation checks");
                    assert_eq!(report.output_bytes_written, sink.bytes.len() as u64);
                    assert_eq!(report.bookmarks_written, 1);
                    break;
                }
            }
        }
    });
}

#[test]
fn spans_beyond_the_classic_xref_ceiling_fail_before_output() {
    run(async {
        const STREAM_BYTES: u64 = 2_100_000_000;
        let mut segments = Vec::new();
        let mut objects = Vec::new();
        let page = b"9 0 obj\n<< /Type /Page /Parent 5 0 R /MediaBox [0 0 1 1] >>\nendobj\n";
        objects.push(FragmentObject {
            reference: reference(9),
            range: PdfRange {
                offset: 0,
                length: page.len() as u64,
            },
        });
        segments.push((0, page.to_vec()));
        let mut cursor = page.len() as u64;
        for number in 20..25 {
            let head = format!("{number} 0 obj\n<< /Length {STREAM_BYTES} >>\nstream\n");
            let tail = b"\nendstream\nendobj\n";
            let tail_offset = cursor + head.len() as u64 + STREAM_BYTES;
            let end = tail_offset + tail.len() as u64;
            objects.push(FragmentObject {
                reference: reference(number),
                range: PdfRange {
                    offset: cursor,
                    length: end - cursor,
                },
            });
            segments.push((cursor, head.into_bytes()));
            segments.push((tail_offset, tail.to_vec()));
            cursor = end;
        }
        let mut source = BytesSource::sparse(cursor, segments);
        let pages = [reference(9)];
        let plan = plan(&objects, &pages);
        let limits = Limits {
            max_input_bytes: u64::MAX,
            max_output_bytes: u64::MAX,
            ..Limits::default()
        };
        let mut sink = BytesSink::default();
        let error = reconstruct_fragment(&mut source, &mut sink, &plan, &limits, &NEVER)
            .await
            .unwrap_err();
        assert!(
            matches!(
                error,
                Error::PdfLimitExceeded {
                    resource: "classic PDF file bytes",
                    object: Some((25, 0)),
                    limit: MAX_CLASSIC_PDF_BYTES,
                    attempted,
                    ..
                } if attempted > cursor
            ),
            "{error}"
        );
        assert!(sink.bytes.is_empty());
    });
}

#[test]
fn xref_offsets_are_limited_to_ten_digits() {
    assert_eq!(
        xref_entry(MAX_CLASSIC_PDF_BYTES, 0, b'n').unwrap(),
        *b"9999999999 00000 n \n"
    );
    assert!(matches!(
        xref_entry(MAX_CLASSIC_PDF_BYTES + 1, 0, b'n'),
        Err(Error::LimitExceeded {
            resource: "classic PDF xref offset",
            limit: MAX_CLASSIC_PDF_BYTES,
            attempted,
        }) if attempted == MAX_CLASSIC_PDF_BYTES + 1
    ));
}

#[test]
fn references_beyond_the_object_profile_are_unsupported() {
    run(async {
        let (mut source, mut objects, pages) = two_page_fragment();
        let extra = add_object(&mut source.bytes, 12, b"<< /Next 9000000 0 R >>");
        objects.push(extra);
        let plan = plan(&objects, &pages);
        let mut sink = BytesSink::default();
        let error = reconstruct_fragment(&mut source, &mut sink, &plan, &Limits::default(), &NEVER)
            .await
            .unwrap_err();
        assert!(
            matches!(
                error,
                Error::Pdf {
                    kind: PdfErrorKind::UnsupportedFeature,
                    object: Some((12, 0)),
                    offset,
                    reason: "indirect reference exceeds the supported PDF profile",
                } if offset == extra.range.offset
            ),
            "{error}"
        );
        assert!(sink.bytes.is_empty());
    });
}

#[test]
fn fragment_outline_requires_synthetic_catalog() {
    run(async {
        let (mut source, objects, pages) = existing_tree_fragment();
        let plan = FragmentPlan {
            catalog: Some(reference(1)),
            ..plan(&objects, &pages)
        };
        let mut sink = BytesSink::default();
        let result = reconstruct_fragment_with_bookmarks(
            &mut source,
            &mut sink,
            &plan,
            &[Bookmark {
                depth: 0,
                title: "outline".into(),
                page_index: 0,
            }],
            &Limits::default(),
            &NEVER,
        )
        .await;
        assert!(matches!(
            result,
            Err(Error::Pdf {
                kind: PdfErrorKind::UnsupportedFeature,
                ..
            })
        ));
        assert!(sink.bytes.is_empty());
    });
}
