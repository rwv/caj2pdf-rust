// SPDX-License-Identifier: MIT

use caj2pdf_core::{
    Cancellation, Error, Limits, NeverCancel, Payload, RangedSource,
    jbig2::{
        SegmentHeader, SegmentSpan,
        generic::{
            GenericError, GenericErrorKind, GenericProgress, GenericRegionDecoder,
            GenericRegionHeader, GenericRegionInfo, GenericReport,
        },
        mq::{ArithmeticSnapshot, CodedSpan, ContextBank, MqTable},
        page_compose::{PageComposeError, PageComposeErrorKind, PageOrSink},
        page_info::read_page_info,
        page_profile::{PageProfile, validate_observed_page_profile},
        read_embedded_directory,
        text::{
            ReferenceCorner, RegionCombination, RegionInfo, SymbolCombination, TextHeaderAnomaly,
            TextRegionFlags, TextRegionHeader,
        },
        text_composer::{TextComposeProgress, TextComposeReport, TextComposeStage},
    },
};
use std::io::Write;
use std::{cell::Cell, rc::Rc, sync::LazyLock};

static DEFAULT_LIMITS: LazyLock<Limits> = LazyLock::new(Limits::default);

struct BytesSource(Vec<u8>);

impl RangedSource for BytesSource {
    fn size(&self) -> u64 {
        self.0.len() as u64
    }

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> caj2pdf_core::Result<usize> {
        let start = offset as usize;
        let count = destination.len().min(self.0.len().saturating_sub(start));
        destination[..count].copy_from_slice(&self.0[start..start + count]);
        Ok(count)
    }
}

fn segment(number: u8, kind: u8, refs: &[u8], data: &[u8]) -> Vec<u8> {
    let retain = 1 | (((1 << refs.len()) - 1) << 1);
    let mut bytes = vec![0, 0, 0, number, kind, ((refs.len() as u8) << 5) | retain];
    bytes.extend_from_slice(refs);
    bytes.push(1); // page association
    bytes.extend_from_slice(&(data.len() as u32).to_be_bytes());
    bytes.extend_from_slice(data);
    bytes
}

const SYNTHETIC_MQ: &[u8] = &[0xfc, 0xff, 0xac];

fn generic_body(width: u32, height: u32) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(&width.to_be_bytes());
    data.extend_from_slice(&height.to_be_bytes());
    data.extend_from_slice(&0_u32.to_be_bytes());
    data.extend_from_slice(&0_u32.to_be_bytes());
    data.extend_from_slice(&[0, 4, 2, 0xff]); // OR, template 2, adaptive (2,-1)
    data.extend_from_slice(SYNTHETIC_MQ);
    data
}

fn page_body(width: u32, height: u32) -> [u8; 19] {
    let mut bytes = [0_u8; 19];
    bytes[0..4].copy_from_slice(&width.to_be_bytes());
    bytes[4..8].copy_from_slice(&height.to_be_bytes());
    bytes[8..12].copy_from_slice(&3000_u32.to_be_bytes());
    bytes[12..16].copy_from_slice(&4000_u32.to_be_bytes());
    bytes[16] = 1; // zero default pixel, OR, no override or auxiliary buffer
    bytes
}

fn observed_bytes_with_text_length(width: u32, height: u32, text_length: usize) -> Vec<u8> {
    [
        segment(0, 48, &[], &page_body(width, height)),
        segment(1, 0, &[], &[]),
        segment(2, 0, &[1], &[]),
        segment(3, 6, &[2], &vec![0; text_length]),
        segment(4, 38, &[], &generic_body(width, height)),
    ]
    .concat()
}

fn observed_bytes(width: u32, height: u32) -> Vec<u8> {
    observed_bytes_with_text_length(width, height, 23)
}

fn directory(source: &mut BytesSource) -> caj2pdf_core::jbig2::SegmentDirectory {
    let span = SegmentSpan {
        offset: 0,
        length: source.size(),
    };
    read_embedded_directory(source, span, &DEFAULT_LIMITS, &NeverCancel).unwrap()
}

fn synthetic_table() -> MqTable {
    MqTable::standard()
}

fn try_arm_from_source_with_header<C: Cancellation>(
    sink: &mut PageOrSink<'_, Output, C>,
    bytes: Vec<u8>,
    expected: GenericRegionHeader,
    change_header: impl FnOnce(&mut SegmentHeader),
) -> Result<(), GenericError> {
    let mut source = BytesSource(bytes);
    let mut directory = directory(&mut source);
    change_header(&mut directory.segments[4]);
    let table = synthetic_table();
    let mut contexts = ContextBank::new(1024, &DEFAULT_LIMITS).unwrap();
    let never = NeverCancel;
    let mut decoder = GenericRegionDecoder::new(
        Payload::from(&source.0[..]),
        &directory.segments[4],
        &table,
        &mut contexts,
        sink,
        &DEFAULT_LIMITS,
        &never,
    )?;
    decoder.arm_page_output(expected)
}

fn try_arm_from_source<C: Cancellation>(
    sink: &mut PageOrSink<'_, Output, C>,
    bytes: Vec<u8>,
    expected: GenericRegionHeader,
) -> Result<(), GenericError> {
    try_arm_from_source_with_header(sink, bytes, expected, |_| {})
}

fn arm_from_real_decoder<C: Cancellation>(
    sink: &mut PageOrSink<'_, Output, C>,
    profile: PageProfile,
) {
    try_arm_from_source(
        sink,
        observed_bytes(profile.page().width, profile.page().height),
        profile.generic_header(),
    )
    .unwrap();
}

fn text_header(width: u32, height: u32) -> TextRegionHeader {
    TextRegionHeader {
        segment: 3,
        page_association: 1,
        dictionary_segment: 2,
        region: RegionInfo {
            width,
            height,
            x: 0,
            y: 0,
            combination: RegionCombination::Or,
        },
        flags: TextRegionFlags {
            raw: 0,
            huffman: false,
            refine: false,
            log_strips: 0,
            reference_corner: ReferenceCorner::TopLeft,
            transposed: false,
            combination: SymbolCombination::Or,
            default_pixel: false,
            ds_offset: 0,
            refinement_template: 0,
        },
        anomaly: None,
        huffman_flags: None,
        refinement_at: None,
        instances: 0,
        header_bytes: 23,
        body: SegmentSpan {
            offset: 0,
            length: 0,
        },
    }
}

fn generic_info(width: u32, height: u32) -> GenericRegionInfo {
    GenericRegionInfo {
        width,
        height,
        x: 0,
        y: 0,
        combination_operator: 0,
        row_stride: width.div_ceil(8) as usize,
    }
}

fn profile_with_header(width: u32, height: u32, mut text: TextRegionHeader) -> PageProfile {
    let mut source = BytesSource(observed_bytes(width, height));
    let directory = directory(&mut source);
    let page = read_page_info(
        &mut source,
        &directory.segments[0],
        &DEFAULT_LIMITS,
        &NeverCancel,
    )
    .unwrap();
    text.body.offset = directory.segments[3].data.offset + text.header_bytes;
    text.body.length = directory.segments[3].data.length - text.header_bytes;
    let generic_data = directory.segments[4].data;
    let generic = GenericRegionHeader {
        segment: directory.segments[4].number,
        page_association: directory.segments[4].page_association,
        reference_count: directory.segments[4].referred_to.len(),
        data: generic_data,
        info: generic_info(width, height),
        mq_span: CodedSpan {
            offset: generic_data.offset + 20,
            length: generic_data.length - 20,
        },
        pixels: u64::from(width) * u64::from(height),
    };
    validate_observed_page_profile(&directory, page, &text, generic).unwrap()
}

fn profile(width: u32, height: u32) -> PageProfile {
    profile_with_header(width, height, text_header(width, height))
}

fn text_report(profile: PageProfile) -> TextComposeReport {
    let page = profile.page();
    TextComposeReport {
        header: profile.text_header(),
        width: page.width,
        height: page.height,
        row_stride: page.row_stride as u32,
        packed_bytes: page.packed_bytes,
        text_flags_raw: 0,
        header_anomaly: None,
        progress: TextComposeProgress {
            stage: TextComposeStage::Complete,
            ..TextComposeProgress::default()
        },
    }
}

fn generic_report(profile: PageProfile) -> GenericReport {
    let page = profile.page();
    let pixels = u64::from(page.width) * u64::from(page.height);
    GenericReport {
        data: profile.generic_header().data,
        mq_span: profile.generic_header().mq_span,
        progress: GenericProgress {
            info: generic_info(page.width, page.height),
            rows_written: page.height,
            pixels_decoded: pixels,
            output_bytes_written: page.packed_bytes,
            mq: ArithmeticSnapshot {
                interval: 0,
                code: 0,
                bit_counter: 0,
                input_offset: 0,
                synthesized_inputs: 0,
                symbols_decoded: pixels,
            },
        },
    }
}

#[derive(Default)]
struct Output {
    bytes: Vec<u8>,
    max_write: usize,
    zero_write: bool,
    fail_write: bool,
    fail_flush: bool,
    cancel_write: bool,
    cancel_flush: bool,
    cancel_on_write: Option<Rc<Cell<bool>>>,
    cancel_on_flush: Option<Rc<Cell<bool>>>,
    write_calls: usize,
    flush_calls: usize,
}

impl Output {
    fn new() -> Self {
        Self {
            max_write: usize::MAX,
            ..Self::default()
        }
    }
}

impl Write for Output {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.write_calls += 1;
        if self.cancel_write {
            return Err(Error::Cancelled.into());
        }
        if self.fail_write {
            return Err(Error::InvalidInput {
                reason: "injected output error",
            }
            .into());
        }
        if self.zero_write {
            return Ok(0);
        }
        let count = bytes.len().min(self.max_write);
        self.bytes.extend_from_slice(&bytes[..count]);
        if let Some(flag) = &self.cancel_on_write {
            flag.set(true);
        }
        Ok(count)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.flush_calls += 1;
        if self.cancel_flush {
            return Err(Error::Cancelled.into());
        }
        if let Some(flag) = &self.cancel_on_flush {
            flag.set(true);
        }
        if self.fail_flush {
            return Err(Error::InvalidInput {
                reason: "injected output flush error",
            }
            .into());
        }
        Ok(())
    }
}

struct FlagCancel(Rc<Cell<bool>>);

impl Cancellation for FlagCancel {
    fn is_cancelled(&self) -> bool {
        self.0.get()
    }
}

fn feed_all(sink: &mut impl Write, mut bytes: &[u8]) -> caj2pdf_core::Result<()> {
    while !bytes.is_empty() {
        let accepted = sink.write(bytes)?;
        assert!(accepted > 0 && accepted <= bytes.len());
        bytes = &bytes[accepted..];
    }
    Ok(())
}

fn constructor_error(limits: Limits) -> PageComposeError {
    let profile = profile(9, 2);
    let scratch = vec![0; 4];
    let mut output = Output::new();
    PageOrSink::new(
        profile,
        text_report(profile),
        &scratch,
        &mut output,
        &limits,
        &NeverCancel,
    )
    .err()
    .unwrap()
}

#[test]
fn constructor_rejects_the_output_limit_before_output() {
    let error = constructor_error(Limits {
        max_output_bytes: 3,
        ..Limits::default()
    });
    assert!(matches!(
        error.kind,
        PageComposeErrorKind::LimitExceeded {
            resource: "page output bytes",
            ..
        }
    ));
}

#[test]
fn cancellation_rejects_before_output() {
    let profile = profile(8, 1);
    let scratch = vec![0];
    let mut output = Output::new();
    let cancelled = FlagCancel(Rc::new(Cell::new(true)));
    let error = PageOrSink::new(
        profile,
        text_report(profile),
        &scratch,
        &mut output,
        &DEFAULT_LIMITS,
        &cancelled,
    )
    .err()
    .unwrap();
    assert!(matches!(error.kind, PageComposeErrorKind::Cancelled));
    assert!(output.bytes.is_empty());
}

#[test]
fn public_error_diagnostics_and_sources_are_stable() {
    let variants = [
        (
            PageComposeErrorKind::Malformed("bad flags"),
            "malformed bad flags",
            false,
        ),
        (
            PageComposeErrorKind::InvalidSpan("bitmap size"),
            "invalid span: bitmap size",
            false,
        ),
        (
            PageComposeErrorKind::LimitExceeded {
                resource: "pixels",
                limit: 2,
                attempted: 3,
            },
            "pixels limit 2 exceeded by 3",
            false,
        ),
        (
            PageComposeErrorKind::AllocationFailed,
            "chunk allocation failed",
            false,
        ),
        (PageComposeErrorKind::Cancelled, "cancelled", false),
        (
            PageComposeErrorKind::Output(Error::InvalidInput {
                reason: "bad write",
            }),
            "output: ",
            true,
        ),
        (
            PageComposeErrorKind::Incomplete,
            "page rows are incomplete",
            false,
        ),
    ];
    for (kind, expected, has_source) in variants {
        let error = PageComposeError {
            offset: 7,
            progress: Box::default(),
            kind,
        };
        let message = error.to_string();
        assert!(message.starts_with("JBIG2 page OR at byte 7: "));
        assert!(message.contains(expected));
        assert_eq!(std::error::Error::source(&error).is_some(), has_source);
    }
}

#[test]
fn bytewise_or_respects_rows_partial_io_and_single_final_flush() {
    let profile = profile(9, 2);
    let scratch = vec![0x80, 0x80, 0x20, 0x00];
    let mut output = Output::new();
    output.max_write = 1;
    let mut sink = PageOrSink::new(
        profile,
        text_report(profile),
        &scratch,
        &mut output,
        &DEFAULT_LIMITS,
        &NeverCancel,
    )
    .unwrap();
    arm_from_real_decoder(&mut sink, profile);
    assert_eq!(sink.write(&[]).unwrap(), 0);
    feed_all(&mut sink, &[0x01, 0x00, 0x80, 0x80]).unwrap();
    assert_eq!(sink.progress().rows_written, 2);
    sink.flush().unwrap();
    let report = sink.finish(&generic_report(profile)).unwrap();
    assert_eq!(report.progress.generic_bytes_accepted, 4);
    assert_eq!(report.progress.max_request_bytes, 2);
    assert_eq!(output.bytes, [0x81, 0x80, 0xa0, 0x80]);
    assert_eq!(output.flush_calls, 1);
}

#[test]
fn real_generic_decoder_arms_and_streams_rows_through_page_sink() {
    let profile = profile(3, 2);
    let mut source = BytesSource(observed_bytes(3, 2));
    let directory = directory(&mut source);
    let table = synthetic_table();
    let mut contexts = ContextBank::new(1024, &DEFAULT_LIMITS).unwrap();
    let scratch = vec![0, 0];
    let mut output = Output::new();
    output.max_write = 1;
    let mut sink = PageOrSink::new(
        profile,
        text_report(profile),
        &scratch,
        &mut output,
        &DEFAULT_LIMITS,
        &NeverCancel,
    )
    .unwrap();
    let mut decoder = GenericRegionDecoder::new(
        Payload::from(&source.0[..]),
        &directory.segments[4],
        &table,
        &mut contexts,
        &mut sink,
        &DEFAULT_LIMITS,
        &NeverCancel,
    )
    .unwrap();
    decoder.arm_page_output(profile.generic_header()).unwrap();
    assert!(decoder.decode_next_row().unwrap());
    assert!(decoder.decode_next_row().unwrap());
    assert!(!decoder.decode_next_row().unwrap());
    let generic = decoder.finish().unwrap();
    let report = sink.finish(&generic).unwrap();
    assert_eq!(report.progress.rows_written, 2);
    assert_eq!(report.progress.output_bytes_written, 2);
    assert_eq!(output.bytes, [0xe0, 0xe0]);
    assert_eq!(output.flush_calls, 1);
}

#[test]
fn a_late_or_mismatched_arming_is_refused() {
    // 0: a second arming after a row; 1: a preflight header other than the
    // parsed one.
    for case in 0..2 {
        let profile = profile(3, 2);
        let mut source = BytesSource(observed_bytes(3, 2));
        let directory = directory(&mut source);
        let table = synthetic_table();
        let mut contexts = ContextBank::new(1024, &DEFAULT_LIMITS).unwrap();
        let scratch = vec![0, 0];
        let mut output = Output::new();
        let mut sink = PageOrSink::new(
            profile,
            text_report(profile),
            &scratch,
            &mut output,
            &DEFAULT_LIMITS,
            &NeverCancel,
        )
        .unwrap();
        let mut decoder = GenericRegionDecoder::new(
            Payload::from(&source.0[..]),
            &directory.segments[4],
            &table,
            &mut contexts,
            &mut sink,
            &DEFAULT_LIMITS,
            &NeverCancel,
        )
        .unwrap();
        let mut expected = profile.generic_header();
        let error = if case == 0 {
            decoder.arm_page_output(expected).unwrap();
            assert!(decoder.decode_next_row().unwrap());
            decoder.arm_page_output(expected).unwrap_err()
        } else {
            expected.info.width = 2;
            decoder.arm_page_output(expected).unwrap_err()
        };
        if case == 0 {
            assert!(matches!(
                error.kind,
                GenericErrorKind::Malformed("page output armed after the first row")
            ));
        } else {
            assert!(matches!(
                error.kind,
                GenericErrorKind::Malformed("generic header differs from page preflight")
            ));
        }
        drop(decoder);
        drop(sink);
        // Only the row decoded before the late arming reached the output.
        assert_eq!(output.bytes.len(), usize::from(case == 0));
    }
}

#[test]
fn generic_header_mismatch_is_rejected_before_first_output_byte() {
    let profile = profile(8, 1);
    let scratch = vec![0];
    let mut output = Output::new();
    let mut sink = PageOrSink::new(
        profile,
        text_report(profile),
        &scratch,
        &mut output,
        &DEFAULT_LIMITS,
        &NeverCancel,
    )
    .unwrap();
    let mut actual = profile.generic_header();
    actual.info.width = 7;
    actual.pixels = 7;
    assert!(try_arm_from_source(&mut sink, observed_bytes(7, 1), actual).is_err());
    let error = sink.take_failure().unwrap();
    assert!(matches!(error.kind, PageComposeErrorKind::Malformed(_)));
    assert!(sink.write(&[0]).is_err());
    drop(sink);
    assert!(output.bytes.is_empty());
}

#[test]
fn wrong_generic_page_association_with_same_data_is_rejected_before_output() {
    let profile = profile(8, 1);
    let scratch = vec![0];
    let mut output = Output::new();
    let mut sink = PageOrSink::new(
        profile,
        text_report(profile),
        &scratch,
        &mut output,
        &DEFAULT_LIMITS,
        &NeverCancel,
    )
    .unwrap();
    let mut actual = profile.generic_header();
    actual.page_association = 2;
    assert!(
        try_arm_from_source_with_header(&mut sink, observed_bytes(8, 1), actual, |header| header
            .page_association =
            2,)
        .is_err()
    );
    let failure = sink.take_failure().unwrap();
    assert!(matches!(failure.kind, PageComposeErrorKind::Malformed(_)));
    drop(sink);
    assert_eq!(output.write_calls, 0);
    assert!(output.bytes.is_empty());
}

#[test]
fn no_generic_write_or_flush_is_allowed_before_checked_header_arming() {
    for operation in 0..3 {
        let profile = profile(8, 1);
        let scratch = vec![0];
        let mut output = Output::new();
        let mut sink = PageOrSink::new(
            profile,
            text_report(profile),
            &scratch,
            &mut output,
            &DEFAULT_LIMITS,
            &NeverCancel,
        )
        .unwrap();
        let kind = match operation {
            0 => {
                assert!(sink.write(&[]).is_err());
                sink.take_failure().unwrap().kind
            }
            1 => {
                assert!(sink.flush().is_err());
                sink.take_failure().unwrap().kind
            }
            _ => sink.finish(&generic_report(profile)).unwrap_err().kind,
        };
        assert!(matches!(
            kind,
            PageComposeErrorKind::Malformed(_) | PageComposeErrorKind::Incomplete
        ));
        assert!(output.bytes.is_empty());
        assert_eq!(output.flush_calls, 0);
    }
}

#[test]
fn exact_header_span_geometry_and_pixel_count_are_bound_before_output() {
    for field in 0..3 {
        let profile = profile(8, 1);
        let scratch = vec![0];
        let mut output = Output::new();
        let mut sink = PageOrSink::new(
            profile,
            text_report(profile),
            &scratch,
            &mut output,
            &DEFAULT_LIMITS,
            &NeverCancel,
        )
        .unwrap();
        let mut actual = profile.generic_header();
        let bytes = match field {
            0 => {
                actual.data.offset += 1;
                actual.mq_span.offset += 1;
                observed_bytes_with_text_length(8, 1, 24)
            }
            1 => {
                actual.info.width = 7;
                actual.pixels = 7;
                observed_bytes(7, 1)
            }
            2 => {
                actual.info.height = 2;
                actual.pixels = 16;
                observed_bytes(8, 2)
            }
            _ => unreachable!(),
        };
        assert!(try_arm_from_source(&mut sink, bytes, actual).is_err());
        let failure = sink.take_failure().unwrap();
        if field == 0 {
            assert!(matches!(failure.kind, PageComposeErrorKind::InvalidSpan(_)));
        } else {
            assert!(matches!(failure.kind, PageComposeErrorKind::Malformed(_)));
        }
        drop(sink);
        assert!(output.bytes.is_empty());
        assert_eq!(output.write_calls, 0);
    }
}

#[test]
fn a_checked_header_cannot_rearm_the_same_sink() {
    let profile = profile(8, 1);
    let scratch = vec![0];
    let mut output = Output::new();
    let mut sink = PageOrSink::new(
        profile,
        text_report(profile),
        &scratch,
        &mut output,
        &DEFAULT_LIMITS,
        &NeverCancel,
    )
    .unwrap();
    arm_from_real_decoder(&mut sink, profile);
    assert!(
        try_arm_from_source(&mut sink, observed_bytes(8, 1), profile.generic_header(),).is_err()
    );
    assert!(matches!(
        sink.take_failure().unwrap().kind,
        PageComposeErrorKind::Malformed("page output is already armed")
    ));
    drop(sink);
    assert!(output.bytes.is_empty());
}

#[test]
fn incomplete_flush_and_extra_generic_bytes_are_rejected() {
    let profile = profile(8, 1);
    let scratch = vec![0];
    let mut output = Output::new();
    let mut sink = PageOrSink::new(
        profile,
        text_report(profile),
        &scratch,
        &mut output,
        &DEFAULT_LIMITS,
        &NeverCancel,
    )
    .unwrap();
    arm_from_real_decoder(&mut sink, profile);
    assert!(sink.flush().is_err());
    assert!(matches!(
        sink.take_failure().unwrap().kind,
        PageComposeErrorKind::Incomplete
    ));
    assert!(sink.flush().is_err());
    assert!(matches!(
        sink.finish(&generic_report(profile)).unwrap_err().kind,
        PageComposeErrorKind::Incomplete
    ));

    let mut sink = PageOrSink::new(
        profile,
        text_report(profile),
        &scratch,
        &mut output,
        &DEFAULT_LIMITS,
        &NeverCancel,
    )
    .unwrap();
    arm_from_real_decoder(&mut sink, profile);
    feed_all(&mut sink, &[0]).unwrap();
    assert!(sink.write(&[0]).is_err());
    let failure = sink.take_failure().unwrap();
    assert!(matches!(
        failure.kind,
        PageComposeErrorKind::Malformed("extra generic bytes")
    ));
    assert_eq!(failure.offset, 1);
}

#[test]
fn a_cancelled_output_write_is_reported_as_cancellation() {
    let profile = profile(8, 1);
    let scratch = vec![0];
    let mut output = Output::new();
    output.cancel_write = true;
    let mut sink = PageOrSink::new(
        profile,
        text_report(profile),
        &scratch,
        &mut output,
        &DEFAULT_LIMITS,
        &NeverCancel,
    )
    .unwrap();
    arm_from_real_decoder(&mut sink, profile);
    assert!(sink.write(&[0]).is_err());
    assert!(matches!(
        sink.take_failure().unwrap().kind,
        PageComposeErrorKind::Cancelled
    ));
}

#[test]
fn cancellation_before_and_after_final_flush_prevents_completion() {
    for cancel_during_flush in [false, true] {
        let profile = profile(8, 1);
        let flag = Rc::new(Cell::new(false));
        let token = FlagCancel(flag.clone());
        let scratch = vec![0];
        let mut output = Output::new();
        if cancel_during_flush {
            output.cancel_on_flush = Some(flag.clone());
        }
        let mut sink = PageOrSink::new(
            profile,
            text_report(profile),
            &scratch,
            &mut output,
            &DEFAULT_LIMITS,
            &token,
        )
        .unwrap();
        arm_from_real_decoder(&mut sink, profile);
        feed_all(&mut sink, &[0]).unwrap();
        sink.flush().unwrap();
        if !cancel_during_flush {
            flag.set(true);
        }
        let error = sink.finish(&generic_report(profile)).unwrap_err();
        assert!(matches!(error.kind, PageComposeErrorKind::Cancelled));
        assert_eq!(output.flush_calls, usize::from(cancel_during_flush));
    }
}

#[test]
fn narrow_and_byte_aligned_rows_have_exact_packed_boundaries() {
    for (width, text, generic, expected) in [
        (1, vec![0x80, 0], vec![0, 0x80], vec![0x80, 0x80]),
        (7, vec![0x02, 0], vec![0x80, 0x02], vec![0x82, 0x02]),
        (8, vec![0x01, 0x80], vec![0x80, 0x01], vec![0x81, 0x81]),
    ] {
        let profile = profile(width, 2);
        let scratch = text;
        let mut output = Output::new();
        let mut sink = PageOrSink::new(
            profile,
            text_report(profile),
            &scratch,
            &mut output,
            &DEFAULT_LIMITS,
            &NeverCancel,
        )
        .unwrap();
        arm_from_real_decoder(&mut sink, profile);
        feed_all(&mut sink, &generic).unwrap();
        sink.flush().unwrap();
        sink.finish(&generic_report(profile)).unwrap();
        assert_eq!(output.bytes, expected, "width {width}");
    }
}

#[test]
fn empty_text_only_generic_only_and_overlapping_pixels_use_or() {
    for (text_byte, generic_byte, expected) in [
        (0x00, 0x00, 0x00),
        (0x80, 0x00, 0x80),
        (0x00, 0x80, 0x80),
        (0x80, 0x80, 0x80),
        (0x40, 0x80, 0xc0),
    ] {
        let profile = profile(8, 1);
        let scratch = vec![text_byte];
        let mut output = Output::new();
        let mut sink = PageOrSink::new(
            profile,
            text_report(profile),
            &scratch,
            &mut output,
            &DEFAULT_LIMITS,
            &NeverCancel,
        )
        .unwrap();
        arm_from_real_decoder(&mut sink, profile);
        feed_all(&mut sink, &[generic_byte]).unwrap();
        sink.flush().unwrap();
        sink.finish(&generic_report(profile)).unwrap();
        assert_eq!(output.bytes, [expected]);
    }
}

#[test]
fn successful_report_retains_explicit_text_header_anomaly() {
    let mut text = text_header(8, 1);
    text.flags.raw = 0xa40c;
    text.flags.refinement_template = 1;
    text.anomaly = Some(TextHeaderAnomaly::UnusedRefinementTemplate);
    let profile = profile_with_header(8, 1, text);
    let mut text = text_report(profile);
    text.text_flags_raw = 0xa40c;
    text.header_anomaly = Some(TextHeaderAnomaly::UnusedRefinementTemplate);
    let scratch = vec![0];
    let mut output = Output::new();
    let mut sink = PageOrSink::new(
        profile,
        text,
        &scratch,
        &mut output,
        &DEFAULT_LIMITS,
        &NeverCancel,
    )
    .unwrap();
    arm_from_real_decoder(&mut sink, profile);
    feed_all(&mut sink, &[0]).unwrap();
    sink.flush().unwrap();
    let report = sink.finish(&generic_report(profile)).unwrap();
    assert_eq!(report.text_flags_raw, 0xa40c);
    assert_eq!(
        report.text_header_anomaly,
        Some(TextHeaderAnomaly::UnusedRefinementTemplate)
    );
}

#[test]
fn rejects_incomplete_or_inconsistent_inputs_before_final_bytes() {
    let profile = profile(9, 2);
    let scratch = vec![0; 4];
    let mut output = Output::new();
    let mut incomplete = text_report(profile);
    incomplete.progress.stage = TextComposeStage::Instance;
    let error = PageOrSink::new(
        profile,
        incomplete,
        &scratch,
        &mut output,
        &DEFAULT_LIMITS,
        &NeverCancel,
    )
    .err()
    .unwrap();
    assert!(matches!(error.kind, PageComposeErrorKind::Incomplete));
    let mut wrong = text_report(profile);
    wrong.width = 8;
    let error = PageOrSink::new(
        profile,
        wrong,
        &scratch,
        &mut output,
        &DEFAULT_LIMITS,
        &NeverCancel,
    )
    .err()
    .unwrap();
    assert!(matches!(error.kind, PageComposeErrorKind::Malformed(_)));
    for field in 0..7 {
        let mut wrong = text_report(profile);
        match field {
            0 => wrong.header.segment += 1,
            1 => wrong.header.body.offset += 1,
            2 => wrong.header.page_association += 1,
            3 => wrong.header.dictionary_segment += 1,
            4 => wrong.header.region.x += 1,
            5 => wrong.header.region.combination = RegionCombination::Xor,
            6 => wrong.header.instances += 1,
            _ => unreachable!(),
        }
        let error = PageOrSink::new(
            profile,
            wrong,
            &scratch,
            &mut output,
            &DEFAULT_LIMITS,
            &NeverCancel,
        )
        .err()
        .unwrap();
        assert!(matches!(error.kind, PageComposeErrorKind::Malformed(_)));
    }
    let wrong_size = vec![0; 5];
    let error = PageOrSink::new(
        profile,
        text_report(profile),
        &wrong_size,
        &mut output,
        &DEFAULT_LIMITS,
        &NeverCancel,
    )
    .err()
    .unwrap();
    assert!(matches!(error.kind, PageComposeErrorKind::InvalidSpan(_)));
    assert!(output.bytes.is_empty());
}

#[test]
fn rejects_generic_and_text_padding_without_output() {
    for (text, generic) in [([0, 0], [0, 1]), ([0, 1], [0, 0])] {
        let profile = profile(9, 1);
        let scratch = text;
        let mut output = Output::new();
        let mut sink = PageOrSink::new(
            profile,
            text_report(profile),
            &scratch,
            &mut output,
            &DEFAULT_LIMITS,
            &NeverCancel,
        )
        .unwrap();
        arm_from_real_decoder(&mut sink, profile);
        assert!(sink.write(&generic).is_err());
        let failure = sink.take_failure().unwrap();
        assert!(matches!(
            failure.kind,
            PageComposeErrorKind::Malformed("nonzero row padding")
        ));
        assert_eq!(failure.offset, 1);
        drop(sink);
        assert!(output.bytes.is_empty());
    }
}

#[test]
fn output_faults_are_typed() {
    let profile = profile(8, 1);
    let scratch = vec![0x80];
    let mut output = Output::new();
    output.zero_write = true;
    let mut sink = PageOrSink::new(
        profile,
        text_report(profile),
        &scratch,
        &mut output,
        &DEFAULT_LIMITS,
        &NeverCancel,
    )
    .unwrap();
    arm_from_real_decoder(&mut sink, profile);
    assert!(sink.write(&[0]).is_err());
    let failure = sink.take_failure().unwrap();
    assert!(matches!(failure.kind, PageComposeErrorKind::Output(_)));
}

#[test]
fn final_report_must_belong_to_preflighted_generic_segment() {
    for field in 0..3 {
        let profile = profile(8, 1);
        let scratch = vec![0];
        let mut output = Output::new();
        let mut sink = PageOrSink::new(
            profile,
            text_report(profile),
            &scratch,
            &mut output,
            &DEFAULT_LIMITS,
            &NeverCancel,
        )
        .unwrap();
        arm_from_real_decoder(&mut sink, profile);
        feed_all(&mut sink, &[0]).unwrap();
        sink.flush().unwrap();
        let mut report = generic_report(profile);
        if field == 0 {
            report.data.offset += 1;
        } else if field == 1 {
            report.mq_span.offset += 1;
        } else {
            report.progress.info.x = 1;
        }
        let failure = sink.finish(&report).unwrap_err();
        if field < 2 {
            assert!(matches!(failure.kind, PageComposeErrorKind::InvalidSpan(_)));
        } else {
            assert!(matches!(failure.kind, PageComposeErrorKind::Malformed(_)));
        }
        assert_eq!(output.flush_calls, 0);
    }
}

#[test]
fn output_error_and_cancellation_do_not_commit_a_page() {
    let profile = profile(8, 1);
    let scratch = vec![0];
    let mut output = Output::new();
    output.fail_write = true;
    let mut sink = PageOrSink::new(
        profile,
        text_report(profile),
        &scratch,
        &mut output,
        &DEFAULT_LIMITS,
        &NeverCancel,
    )
    .unwrap();
    arm_from_real_decoder(&mut sink, profile);
    assert!(sink.write(&[0x80]).is_err());
    assert!(matches!(
        sink.take_failure().unwrap().kind,
        PageComposeErrorKind::Output(_)
    ));
    drop(sink);
    assert!(output.bytes.is_empty());

    let cancel = Rc::new(Cell::new(false));
    let token = FlagCancel(cancel.clone());
    output.fail_write = false;
    output.cancel_on_write = Some(cancel);
    let mut sink = PageOrSink::new(
        profile,
        text_report(profile),
        &scratch,
        &mut output,
        &DEFAULT_LIMITS,
        &token,
    )
    .unwrap();
    arm_from_real_decoder(&mut sink, profile);
    // The write completes; the next checkpoint observes the cancellation.
    feed_all(&mut sink, &[0x80]).unwrap();
    assert!(sink.flush().is_err());
    let failure = sink.take_failure().unwrap();
    assert!(matches!(failure.kind, PageComposeErrorKind::Cancelled));
    assert_eq!(failure.progress.output_bytes_written, 1);
    drop(sink);
}

#[test]
fn finish_requires_generic_report_and_flushes_output_only_after_validation() {
    let profile = profile(8, 1);
    let scratch = vec![0];
    let mut output = Output::new();
    let mut sink = PageOrSink::new(
        profile,
        text_report(profile),
        &scratch,
        &mut output,
        &DEFAULT_LIMITS,
        &NeverCancel,
    )
    .unwrap();
    arm_from_real_decoder(&mut sink, profile);
    feed_all(&mut sink, &[0]).unwrap();
    sink.flush().unwrap();
    let mut wrong = generic_report(profile);
    wrong.progress.mq.symbols_decoded -= 1;
    let failure = sink.finish(&wrong).unwrap_err();
    assert!(matches!(failure.kind, PageComposeErrorKind::Incomplete));
    assert_eq!(output.flush_calls, 0);

    output.fail_flush = true;
    let mut sink = PageOrSink::new(
        profile,
        text_report(profile),
        &scratch,
        &mut output,
        &DEFAULT_LIMITS,
        &NeverCancel,
    )
    .unwrap();
    arm_from_real_decoder(&mut sink, profile);
    feed_all(&mut sink, &[0]).unwrap();
    sink.flush().unwrap();
    let failure = sink.finish(&generic_report(profile)).unwrap_err();
    assert!(matches!(failure.kind, PageComposeErrorKind::Output(_)));
    assert_eq!(output.flush_calls, 1);
}
