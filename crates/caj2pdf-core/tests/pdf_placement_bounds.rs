// SPDX-License-Identifier: MIT

//! Original synthetic checks for reusable PDF image and placement contracts.
//! Generated sources and counting sinks retain no image or PDF payload.

mod common;

use caj2pdf_core::{
    Error, ErrorKind, Limits, RangedSource, Result,
    pdf::{
        ImageEncoding, ImageObject, ImagePlacement, ImageSpec, MAX_PAGE_IMAGE_PLACEMENTS, PageSpec,
        PdfDocument,
    },
};
use common::CancelAfter;
use std::io::Write;
use std::{cell::Cell, io, rc::Rc};

fn assert_document_refusal<T>(result: Result<T>) {
    assert!(matches!(
        result,
        Err(Error {
            kind: ErrorKind::Malformed,
            ..
        })
    ));
}

fn page() -> PageSpec {
    PageSpec {
        width_points: 12.5,
        height_points: 7.25,
    }
}

fn gray(width: u32) -> ImageSpec {
    ImageSpec {
        pixel_width: width,
        pixel_height: 1,
        encoding: ImageEncoding::Gray8,
    }
}

fn placed(image: ImageObject) -> ImagePlacement {
    ImagePlacement {
        image,
        transform: [2.5, 0.0, 0.0, -1.25, -3.125, 9.75],
    }
}

#[derive(Clone, Copy, Debug)]
enum Fault {
    Zero,
    Overreport,
    Io,
}

struct GeneratedSource {
    size: u64,
    max_return: usize,
    calls: usize,
    bytes: u64,
    max_request: usize,
    next_offset: Option<u64>,
    fault_after_calls: Option<(usize, Fault)>,
    cancel_after_bytes: Option<(u64, Rc<Cell<bool>>)>,
}

impl GeneratedSource {
    fn new(size: u64) -> Self {
        Self {
            size,
            max_return: usize::MAX,
            calls: 0,
            bytes: 0,
            max_request: 0,
            next_offset: None,
            fault_after_calls: None,
            cancel_after_bytes: None,
        }
    }
}

impl RangedSource for GeneratedSource {
    fn size(&self) -> u64 {
        self.size
    }

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
        self.calls += 1;
        self.max_request = self.max_request.max(destination.len());
        if let Some((allowed, fault)) = self.fault_after_calls
            && self.calls > allowed
        {
            return match fault {
                Fault::Zero => Ok(0),
                Fault::Overreport => Ok(destination.len() + 1),
                Fault::Io => Err(Error::from(ErrorKind::Io(io::Error::other(
                    "synthetic source failure",
                )))),
            };
        }
        assert!(offset <= self.size, "reader requested a nonexistent range");
        if let Some(expected) = self.next_offset {
            assert_eq!(offset, expected, "image reads must advance without gaps");
        }
        let count = destination
            .len()
            .min(self.max_return)
            .min(usize::try_from(self.size - offset).unwrap_or(usize::MAX));
        for (index, byte) in destination[..count].iter_mut().enumerate() {
            *byte = offset.wrapping_add(index as u64).wrapping_mul(37) as u8;
        }
        self.bytes += count as u64;
        self.next_offset = Some(offset + count as u64);
        if let Some((threshold, flag)) = &self.cancel_after_bytes
            && self.bytes >= *threshold
        {
            flag.set(true);
        }
        Ok(count)
    }
}

#[derive(Default)]
struct SinkState {
    bytes: Cell<u64>,
    calls: Cell<usize>,
    max_request: Cell<usize>,
    flushes: Cell<usize>,
    fault_at: Cell<Option<(u64, Fault)>>,
    pending_at: Cell<Option<u64>>,
    fail_flush: Cell<bool>,
    cancel_after_bytes: Cell<Option<u64>>,
    cancelled: Cell<bool>,
}

struct CountingSink {
    state: Rc<SinkState>,
    max_return: usize,
}

impl CountingSink {
    fn new() -> (Self, Rc<SinkState>) {
        let state = Rc::new(SinkState::default());
        (
            Self {
                state: state.clone(),
                max_return: usize::MAX,
            },
            state,
        )
    }
}

impl Write for CountingSink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let state = &self.state;
        state.calls.set(state.calls.get() + 1);
        state
            .max_request
            .set(state.max_request.get().max(bytes.len()));
        let mut count = bytes.len().min(self.max_return);
        if let Some(threshold) = state.pending_at.get() {
            count = count.min((threshold - state.bytes.get()) as usize);
        }
        if let Some((threshold, fault)) = state.fault_at.get() {
            if state.bytes.get() >= threshold {
                return match fault {
                    Fault::Zero => Ok(0),
                    Fault::Io => Err(io::Error::other("synthetic sink failure")),
                    Fault::Overreport => unreachable!("an io::Write sink cannot over-report"),
                };
            }
            count = count.min((threshold - state.bytes.get()) as usize);
        }
        state.bytes.set(state.bytes.get() + count as u64);
        if state
            .cancel_after_bytes
            .get()
            .is_some_and(|threshold| state.bytes.get() >= threshold)
        {
            state.cancelled.set(true);
        }
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.state.flushes.set(self.state.flushes.get() + 1);
        if self.state.fail_flush.get() {
            Err(io::Error::other("synthetic flush failure"))
        } else {
            Ok(())
        }
    }
}

impl caj2pdf_core::Cancellation for SinkState {
    fn is_cancelled(&self) -> bool {
        self.cancelled.get()
    }
}

#[test]
fn complete_page_preflight_refuses_invalid_later_items_without_output() -> Result<()> {
    let limits = Limits::default();
    let (mut sink, state) = CountingSink::new();
    let mut source = GeneratedSource::new(1);
    let report = (|| {
        let mut document = PdfDocument::new(&mut sink, &limits, &CancelAfter::Never)?;
        let image = document.add_image(&mut source, 0, 1, gray(1))?;
        let good = placed(image);
        for component in 0..6 {
            for value in [
                f64::NAN,
                f64::INFINITY,
                f64::NEG_INFINITY,
                2_147_483_648.0,
                -2_147_483_648.0,
            ] {
                let mut bad = good;
                bad.transform[component] = value;
                let before = state.bytes.get();
                let error = document.add_placed_page(page(), &[good, bad]).unwrap_err();
                assert!(
                    matches!(
                        error,
                        Error {
                            kind: ErrorKind::Malformed,
                            ..
                        }
                    ),
                    "{error:?}"
                );
                assert_eq!(state.bytes.get(), before, "later component {component}");
            }
        }
        let before = state.bytes.get();
        let error = document.add_placed_page(page(), &[]).unwrap_err();
        assert!(matches!(
            error,
            Error {
                kind: ErrorKind::Malformed,
                ..
            }
        ));
        assert_eq!(state.bytes.get(), before);
        let too_many = vec![good; MAX_PAGE_IMAGE_PLACEMENTS + 1];
        let error = document.add_placed_page(page(), &too_many).unwrap_err();
        assert!(matches!(
            error,
            Error { kind: ErrorKind::LimitExceeded { resource: "PDF image placements per page", limit, attempted, .. }, .. } if limit == MAX_PAGE_IMAGE_PLACEMENTS as u64
                && attempted == MAX_PAGE_IMAGE_PLACEMENTS as u64 + 1
        ));
        assert_eq!(state.bytes.get(), before);
        for dimension in [0.0, -1.0, f64::NAN, f64::INFINITY, 14_400.01, 0.000_000_1] {
            for bad_page in [
                PageSpec {
                    width_points: dimension,
                    ..page()
                },
                PageSpec {
                    height_points: dimension,
                    ..page()
                },
            ] {
                let error = document.add_placed_page(bad_page, &[good]).unwrap_err();
                assert!(matches!(
                    error,
                    Error {
                        kind: ErrorKind::Malformed,
                        ..
                    }
                ));
                assert_eq!(state.bytes.get(), before);
            }
        }
        assert_eq!(document.add_placed_page(page(), &[good])?, 0);
        document.finish()
    })()?;
    assert_eq!(report.pages_converted, 1);
    assert_eq!(report.input_bytes_read, 1);
    assert_eq!(report.output_bytes_written, state.bytes.get());
    Ok(())
}

#[test]
fn old_and_placed_page_methods_share_one_image_handle() -> Result<()> {
    let limits = Limits::default();
    let (mut sink, _) = CountingSink::new();
    let mut source = GeneratedSource::new(1);
    let report = (|| {
        let mut document = PdfDocument::new(&mut sink, &limits, &CancelAfter::Never)?;
        let image = document.add_image(&mut source, 0, 1, gray(1))?;
        assert_eq!(document.add_page(page(), &[image])?, 0);
        assert_eq!(document.add_placed_page(page(), &[placed(image)])?, 1);
        document.finish()
    })()?;
    assert_eq!(report.pages_converted, 2);
    assert_eq!(source.calls, 1);
    Ok(())
}

#[test]
fn maximum_placement_count_uses_bounded_allocations_and_reuses_image() -> Result<()> {
    let limits = Limits {
        io_chunk_bytes: 3,
        max_allocation_bytes: 128,
        ..Limits::default()
    };
    let (mut sink, state) = CountingSink::new();
    let mut source = GeneratedSource::new(1);
    let report = (|| {
        let mut document = PdfDocument::new(&mut sink, &limits, &CancelAfter::Never)?;
        let image = document.add_image(&mut source, 0, 1, gray(1))?;
        let placements = vec![placed(image); MAX_PAGE_IMAGE_PLACEMENTS];
        assert_eq!(document.add_placed_page(page(), &placements)?, 0);
        assert_eq!(document.add_placed_page(page(), &[placed(image)])?, 1);
        document.finish()
    })()?;
    assert_eq!(report.pages_converted, 2);
    assert_eq!(report.input_bytes_read, 1);
    assert_eq!(source.calls, 1);
    assert!(state.max_request.get() <= limits.io_chunk_bytes);
    assert_eq!(report.output_bytes_written, state.bytes.get());
    Ok(())
}

#[test]
fn page_cap_refusal_leaves_existing_page_finishable() -> Result<()> {
    let limits = Limits {
        max_pages: 1,
        ..Limits::default()
    };
    let (mut sink, state) = CountingSink::new();
    let mut source = GeneratedSource::new(1);
    let report = (|| {
        let mut document = PdfDocument::new(&mut sink, &limits, &CancelAfter::Never)?;
        let image = document.add_image(&mut source, 0, 1, gray(1))?;
        document.add_placed_page(page(), &[placed(image)])?;
        let before = state.bytes.get();
        let error = document
            .add_placed_page(page(), &[placed(image)])
            .unwrap_err();
        assert!(matches!(
            error,
            Error {
                kind: ErrorKind::LimitExceeded {
                    resource: "pages",
                    limit: 1,
                    attempted: 2,
                    ..
                },
                ..
            }
        ));
        assert_eq!(state.bytes.get(), before);
        document.finish()
    })()?;
    assert_eq!(report.pages_converted, 1);
    Ok(())
}

#[test]
fn object_index_budget_refusal_reserves_no_unwritten_page_objects() -> Result<()> {
    // Catalog/Pages + image/length + two tree nodes + page/content/length:
    // nine u64 offsets fit, whereas a second page's three offsets do not.
    let limits = Limits {
        io_chunk_bytes: 1,
        max_allocation_bytes: 72,
        ..Limits::default()
    };
    let (mut sink, state) = CountingSink::new();
    let mut source = GeneratedSource::new(1);
    let report = (|| {
        let mut document = PdfDocument::new(&mut sink, &limits, &CancelAfter::Never)?;
        let image = document.add_image(&mut source, 0, 1, gray(1))?;
        document.add_placed_page(page(), &[placed(image)])?;
        let before = state.bytes.get();
        let error = document
            .add_placed_page(page(), &[placed(image)])
            .unwrap_err();
        assert!(
            matches!(
                error,
                Error {
                    kind: ErrorKind::LimitExceeded { .. },
                    ..
                }
            ),
            "{error:?}"
        );
        assert_eq!(state.bytes.get(), before);
        document.finish()
    })()?;
    assert_eq!(report.pages_converted, 1);
    Ok(())
}

#[test]
fn invalid_image_specs_and_unavailable_ranges_are_refused_before_emission() -> Result<()> {
    let limits = Limits::default();
    let (mut sink, state) = CountingSink::new();
    let mut source = GeneratedSource::new(10);
    let report = (|| {
        let mut document = PdfDocument::new(&mut sink, &limits, &CancelAfter::Never)?;
        let before = state.bytes.get();
        for (offset, length, spec) in [
            (0, 1, gray(0)),
            (
                0,
                1,
                ImageSpec {
                    pixel_height: 0,
                    ..gray(1)
                },
            ),
            (0, 2, gray(3)),
            (
                0,
                2,
                ImageSpec {
                    encoding: ImageEncoding::Rgb8,
                    ..gray(1)
                },
            ),
            (
                0,
                0,
                ImageSpec {
                    encoding: ImageEncoding::JpegGray8,
                    ..gray(1)
                },
            ),
            (11, 1, gray(1)),
        ] {
            let error = document
                .add_image(&mut source, offset, length, spec)
                .unwrap_err();
            assert!(
                matches!(
                    error,
                    Error {
                        kind: ErrorKind::Malformed,
                        ..
                    }
                ),
                "{error:?}"
            );
            assert_eq!(state.bytes.get(), before);
        }
        let error = document.add_image(&mut source, 9, 2, gray(2)).unwrap_err();
        assert!(matches!(
            error,
            Error {
                kind: ErrorKind::Truncated {
                    expected: 2,
                    available: 1,
                    ..
                },
                offset: Some(9),
                ..
            }
        ));
        assert_eq!(state.bytes.get(), before);
        for spec in [
            ImageSpec {
                pixel_width: u32::MAX,
                encoding: ImageEncoding::JpegGray8,
                ..gray(1)
            },
            ImageSpec {
                pixel_height: u32::MAX,
                encoding: ImageEncoding::JpegRgb8,
                ..gray(1)
            },
        ] {
            assert!(matches!(
                document.add_image(&mut source, 0, 1, spec),
                Err(Error {
                    kind: ErrorKind::LimitExceeded { .. },
                    ..
                })
            ));
            assert_eq!(state.bytes.get(), before);
        }
        assert_eq!(source.calls, 0);
        let image = document.add_image(&mut source, 4, 3, gray(3))?;
        document.add_placed_page(page(), &[placed(image)])?;
        document.finish()
    })()?;
    assert_eq!(report.input_bytes_read, 3);
    Ok(())
}

#[test]
fn stream_length_and_u64_range_edges_are_refused_without_reading() -> Result<()> {
    let limits = Limits {
        max_input_bytes: u64::MAX,
        ..Limits::default()
    };
    let (mut sink, state) = CountingSink::new();
    let mut source = GeneratedSource::new(u64::MAX);
    (|| {
        let mut document = PdfDocument::new(&mut sink, &limits, &CancelAfter::Never)?;
        let before = state.bytes.get();
        let jpeg = ImageSpec {
            encoding: ImageEncoding::JpegGray8,
            ..gray(1)
        };
        let error = document
            .add_image(&mut source, 0, 2_147_483_648, jpeg)
            .unwrap_err();
        assert!(matches!(
            error,
            Error {
                kind: ErrorKind::LimitExceeded {
                    resource: "PDF image stream bytes",
                    limit: 2_147_483_647,
                    attempted: 2_147_483_648,
                    ..
                },
                ..
            }
        ));
        assert_eq!(state.bytes.get(), before);
        let error = document
            .add_image(&mut source, u64::MAX, 1, gray(1))
            .unwrap_err();
        assert!(matches!(
            error,
            Error {
                kind: ErrorKind::Truncated {
                    expected: 1,
                    available: 0,
                    ..
                },
                offset: Some(u64::MAX),
                ..
            }
        ));
        assert_eq!(state.bytes.get(), before);
        assert_eq!(source.calls, 0);
        let image = document.add_image(&mut source, u64::MAX - 1, 1, gray(1))?;
        document.add_placed_page(page(), &[placed(image)])?;
        document.finish()
    })()?;
    assert_eq!(source.bytes, 1);
    Ok(())
}

#[test]
fn total_input_budget_is_checked_before_second_image_and_remains_recoverable() -> Result<()> {
    let limits = Limits {
        io_chunk_bytes: 2,
        max_input_bytes: 5,
        ..Limits::default()
    };
    let (mut sink, state) = CountingSink::new();
    let mut first = GeneratedSource::new(3);
    let mut refused = GeneratedSource::new(3);
    let mut last = GeneratedSource::new(2);
    let report = (|| {
        let mut document = PdfDocument::new(&mut sink, &limits, &CancelAfter::Never)?;
        let image = document.add_image(&mut first, 0, 3, gray(3))?;
        let before = state.bytes.get();
        let error = document.add_image(&mut refused, 0, 3, gray(3)).unwrap_err();
        assert!(matches!(
            error,
            Error {
                kind: ErrorKind::LimitExceeded {
                    resource: "input bytes",
                    limit: 5,
                    attempted: 6,
                    ..
                },
                ..
            }
        ));
        assert_eq!(state.bytes.get(), before);
        assert_eq!(refused.calls, 0);
        let last_image = document.add_image(&mut last, 0, 2, gray(2))?;
        document.add_placed_page(page(), &[placed(image), placed(last_image)])?;
        document.finish()
    })()?;
    assert_eq!(report.input_bytes_read, 5);
    Ok(())
}

#[test]
fn large_ranged_image_short_reads_and_writes_reuse_one_stream_on_multiple_pages() -> Result<()> {
    const LENGTH: u64 = 2 * 1024 * 1024 + 17;
    let limits = Limits {
        io_chunk_bytes: 4096,
        max_allocation_bytes: 4096,
        ..Limits::default()
    };
    let mut source = GeneratedSource::new(LENGTH + 29);
    source.max_return = 43;
    let (mut sink, state) = CountingSink::new();
    sink.max_return = 127;
    let report = (|| {
        let mut document = PdfDocument::new(&mut sink, &limits, &CancelAfter::Never)?;
        let image = document.add_image(&mut source, 13, LENGTH, gray(LENGTH as u32))?;
        let read_calls = source.calls;
        for expected in 0..3 {
            assert_eq!(
                document.add_placed_page(page(), &[placed(image), placed(image)])?,
                expected
            );
            assert_eq!(
                source.calls, read_calls,
                "placing an object must not reread it"
            );
        }
        document.finish()
    })()?;
    assert_eq!(report.input_bytes_read, LENGTH);
    assert_eq!(source.bytes, LENGTH);
    assert_eq!(source.next_offset, Some(13 + LENGTH));
    assert_eq!(report.pages_converted, 3);
    assert_eq!(report.output_bytes_written, state.bytes.get());
    assert!(state.bytes.get() > LENGTH);
    assert!(
        state.bytes.get() < LENGTH + 4096,
        "image payload must be emitted only once"
    );
    assert!(source.calls > (LENGTH / limits.io_chunk_bytes as u64) as usize);
    assert!(source.max_request <= limits.io_chunk_bytes);
    assert!(state.max_request.get() <= limits.io_chunk_bytes);
    assert_eq!(state.flushes.get(), 1);
    Ok(())
}

#[test]
fn source_failure_after_partial_image_prevents_any_later_success() -> Result<()> {
    let limits = Limits {
        io_chunk_bytes: 4,
        ..Limits::default()
    };
    for fault in [Fault::Zero, Fault::Overreport, Fault::Io] {
        let (mut sink, state) = CountingSink::new();
        let mut valid = GeneratedSource::new(1);
        let mut source = GeneratedSource::new(12);
        source.fault_after_calls = Some((1, fault));
        (|| {
            let mut document = PdfDocument::new(&mut sink, &limits, &CancelAfter::Never)?;
            let image = document.add_image(&mut valid, 0, 1, gray(1))?;
            document.add_placed_page(page(), &[placed(image)])?;
            let before = state.bytes.get();
            let error = document
                .add_image(&mut source, 0, 12, gray(12))
                .unwrap_err();
            match fault {
                Fault::Zero => assert!(matches!(
                    error,
                    Error {
                        kind: ErrorKind::Truncated { .. },
                        ..
                    }
                )),
                Fault::Overreport => assert!(matches!(
                    error,
                    Error {
                        kind: ErrorKind::Malformed,
                        ..
                    }
                )),
                Fault::Io => assert!(matches!(
                    error,
                    Error {
                        kind: ErrorKind::Io(_),
                        ..
                    }
                )),
            }
            assert!(
                state.bytes.get() > before,
                "a partial image has been emitted"
            );
            assert_eq!(source.calls, 2);
            assert_eq!(source.bytes, 4);
            let stopped = state.bytes.get();
            assert_document_refusal(document.add_placed_page(page(), &[placed(image)]));
            assert_document_refusal(document.add_image(&mut valid, 0, 1, gray(1)));
            assert_document_refusal(document.finish());
            assert_eq!(state.bytes.get(), stopped);
            Ok::<_, Error>(())
        })()?;
    }
    Ok(())
}

#[test]
fn cancellation_after_source_read_prevents_completion_after_signal_is_reset() -> Result<()> {
    let flag = Rc::new(Cell::new(false));
    let cancellation = CancelAfter::While(flag.clone());
    let limits = Limits {
        io_chunk_bytes: 4,
        ..Limits::default()
    };
    let (mut sink, state) = CountingSink::new();
    let mut valid = GeneratedSource::new(1);
    let mut source = GeneratedSource::new(12);
    source.cancel_after_bytes = Some((4, flag.clone()));
    (|| {
        let mut document = PdfDocument::new(&mut sink, &limits, &cancellation)?;
        let image = document.add_image(&mut valid, 0, 1, gray(1))?;
        document.add_placed_page(page(), &[placed(image)])?;
        let before = state.bytes.get();
        assert!(matches!(
            document.add_image(&mut source, 0, 12, gray(12)),
            Err(Error {
                kind: ErrorKind::Cancelled,
                ..
            })
        ));
        assert!(state.bytes.get() > before);
        assert_eq!(source.calls, 1);
        assert_eq!(source.bytes, 4);
        flag.set(false);
        let stopped = state.bytes.get();
        assert_document_refusal(document.add_placed_page(page(), &[placed(image)]));
        assert_document_refusal(document.finish());
        assert_eq!(state.bytes.get(), stopped);
        Ok::<_, Error>(())
    })()
}

#[test]
fn sink_failures_during_page_emission_poison_document_even_after_sink_recovers() -> Result<()> {
    let limits = Limits {
        io_chunk_bytes: 4,
        ..Limits::default()
    };
    for fault in [Fault::Zero, Fault::Io] {
        let (mut sink, state) = CountingSink::new();
        let mut source = GeneratedSource::new(1);
        (|| {
            let mut document = PdfDocument::new(&mut sink, &limits, &CancelAfter::Never)?;
            let image = document.add_image(&mut source, 0, 1, gray(1))?;
            document.add_placed_page(page(), &[placed(image)])?;
            let before = state.bytes.get();
            state.fault_at.set(Some((before + 5, fault)));
            let error = document
                .add_placed_page(page(), &[placed(image)])
                .unwrap_err();
            match fault {
                Fault::Zero => assert!(
                    matches!(error, Error { kind: ErrorKind::Io(ref e), .. } if e.kind() == io::ErrorKind::WriteZero)
                ),
                Fault::Io => assert!(matches!(
                    error,
                    Error {
                        kind: ErrorKind::Io(_),
                        ..
                    }
                )),
                Fault::Overreport => unreachable!(),
            }
            assert_eq!(state.bytes.get(), before + 5);
            state.fault_at.set(None);
            let stopped = state.bytes.get();
            assert_document_refusal(document.add_placed_page(page(), &[placed(image)]));
            assert_document_refusal(document.finish());
            assert_eq!(state.bytes.get(), stopped);
            Ok::<_, Error>(())
        })()?;
    }
    Ok(())
}

#[test]
fn cancellation_between_short_sink_writes_poison_document_after_partial_page() -> Result<()> {
    let limits = Limits {
        io_chunk_bytes: 4,
        ..Limits::default()
    };
    let (mut sink, state) = CountingSink::new();
    sink.max_return = 2;
    let mut source = GeneratedSource::new(1);
    (|| {
        let mut document = PdfDocument::new(&mut sink, &limits, state.as_ref())?;
        let image = document.add_image(&mut source, 0, 1, gray(1))?;
        document.add_placed_page(page(), &[placed(image)])?;
        let before = state.bytes.get();
        state.cancel_after_bytes.set(Some(before + 5));
        assert!(matches!(
            document.add_placed_page(page(), &[placed(image)]),
            Err(Error {
                kind: ErrorKind::Cancelled,
                ..
            })
        ));
        assert!(state.bytes.get() >= before + 5);
        state.cancel_after_bytes.set(None);
        state.cancelled.set(false);
        let stopped = state.bytes.get();
        assert_document_refusal(document.add_placed_page(page(), &[placed(image)]));
        assert_document_refusal(document.finish());
        assert_eq!(state.bytes.get(), stopped);
        Ok::<_, Error>(())
    })()
}

#[test]
fn output_budget_failure_during_placed_page_prevents_any_later_success() -> Result<()> {
    let limits = Limits {
        io_chunk_bytes: 11,
        max_output_bytes: 512,
        ..Limits::default()
    };
    let (mut sink, state) = CountingSink::new();
    let mut source = GeneratedSource::new(256);
    (|| {
        let mut document = PdfDocument::new(&mut sink, &limits, &CancelAfter::Never)?;
        let image = document.add_image(&mut source, 0, 256, gray(256))?;
        let before = state.bytes.get();
        let error = document
            .add_placed_page(page(), &[placed(image)])
            .unwrap_err();
        assert!(matches!(
            error,
            Error { kind: ErrorKind::LimitExceeded { resource: "output bytes", limit: 512, attempted, .. }, .. } if attempted > 512
        ));
        assert!(
            state.bytes.get() > before,
            "page output started before refusal"
        );
        assert!(state.bytes.get() <= limits.max_output_bytes);
        let stopped = state.bytes.get();
        assert_document_refusal(document.add_placed_page(page(), &[placed(image)]));
        assert_document_refusal(document.finish());
        assert_eq!(state.bytes.get(), stopped);
        Ok::<_, Error>(())
    })()
}

#[test]
fn flush_failure_cannot_return_a_successful_conversion_report() -> Result<()> {
    let limits = Limits::default();
    let (mut sink, state) = CountingSink::new();
    let mut source = GeneratedSource::new(1);
    let error = (|| {
        let mut document = PdfDocument::new(&mut sink, &limits, &CancelAfter::Never)?;
        let image = document.add_image(&mut source, 0, 1, gray(1))?;
        document.add_placed_page(page(), &[placed(image)])?;
        state.fail_flush.set(true);
        document.finish()
    })()
    .unwrap_err();
    assert!(matches!(
        error,
        Error {
            kind: ErrorKind::Io(_),
            ..
        }
    ));
    assert_eq!(state.flushes.get(), 1);
    assert!(state.bytes.get() > 1);
    Ok(())
}
