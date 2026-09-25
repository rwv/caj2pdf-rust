// SPDX-License-Identifier: MIT

//! Invented text-region segments test the public header parser only; they
//! are not CAJ/HN text-placement or pixel compatibility evidence.

use caj2pdf_core::{
    Cancellation, Error, Limits, NeverCancel, RangedSource,
    jbig2::{
        HeaderLimits, SegmentHeader, SegmentSpan, read_segment_header,
        text::{
            ReferenceCorner, RegionCombination, SymbolCombination, TextRegionBudget,
            TextRegionError, TextRegionErrorKind, TextRegionHeader, read_text_region_header,
        },
    },
};
use std::{
    cell::Cell,
    future::{Future, pending},
    io,
    pin::pin,
    task::{Context, Poll, Waker},
};

fn ready<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("unexpected pending test I/O"),
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Fault {
    None,
    ZeroAt(u64),
    Overreport,
    Fail,
    Cancelled,
    PendingAt(u64),
}

struct Source {
    bytes: Vec<u8>,
    advertised: u64,
    max_read: usize,
    reads: Vec<(u64, usize)>,
    fault: Fault,
}

impl Source {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            advertised: bytes.len() as u64,
            bytes,
            max_read: usize::MAX,
            reads: Vec::new(),
            fault: Fault::None,
        }
    }
}

impl RangedSource for Source {
    fn size(&self) -> u64 {
        self.advertised
    }

    async fn read_at(
        &mut self,
        offset: u64,
        destination: &mut [u8],
    ) -> caj2pdf_core::Result<usize> {
        self.reads.push((offset, destination.len()));
        match self.fault {
            Fault::Overreport => return Ok(destination.len() + 1),
            Fault::Fail => return Err(Error::Io(io::Error::other("test source failure"))),
            Fault::Cancelled => return Err(Error::Cancelled),
            Fault::ZeroAt(at) if offset >= at => return Ok(0),
            Fault::PendingAt(at) if offset >= at => pending::<()>().await,
            _ => {}
        }
        let start = offset as usize;
        let count = self
            .bytes
            .len()
            .saturating_sub(start)
            .min(destination.len())
            .min(self.max_read);
        destination[..count].copy_from_slice(&self.bytes[start..start + count]);
        Ok(count)
    }
}

/// Cancels once the source has served `after` read calls.
struct CancelAfter<'a> {
    reads: &'a Cell<usize>,
    after: usize,
}

impl Cancellation for CancelAfter<'_> {
    fn is_cancelled(&self) -> bool {
        self.reads.get() >= self.after
    }
}

/// Segment framing (T.88 §7.2) with one-byte references and page association.
fn frame(number: u32, kind: u8, references: &[u8], page: u8, data: &[u8]) -> Vec<u8> {
    let mut bytes = number.to_be_bytes().to_vec();
    bytes.push(kind);
    bytes.push((references.len() as u8) << 5);
    bytes.extend_from_slice(references);
    bytes.push(page);
    bytes.extend_from_slice(&(data.len() as u32).to_be_bytes());
    bytes.extend_from_slice(data);
    bytes
}

fn parse_header(bytes: &[u8]) -> SegmentHeader {
    let mut source = Source::new(bytes.to_vec());
    ready(read_segment_header(
        &mut source,
        SegmentSpan {
            offset: 0,
            length: bytes.len() as u64,
        },
        &Limits::default(),
        HeaderLimits::default(),
        &NeverCancel,
    ))
    .unwrap()
}

fn referred(number: u32, kind: u8, page: u8) -> SegmentHeader {
    // The parser uses only the referred segment's framing metadata.
    parse_header(&frame(number, kind, &[], page, &[0; 10]))
}

fn dictionary(number: u32, page: u8) -> SegmentHeader {
    referred(number, 0, page)
}

struct Region {
    width: u32,
    height: u32,
    x: u32,
    y: u32,
    region_flags: u8,
    flags: u16,
    huffman_flags: Option<u16>,
    refinement_at: Option<[i8; 4]>,
    instances: u32,
    body: Vec<u8>,
}

impl Default for Region {
    fn default() -> Self {
        Self {
            width: 64,
            height: 32,
            x: 0,
            y: 0,
            region_flags: 0,
            // The observed arithmetic profile: SBREFINE, 8 strips,
            // SBDSOFFSET 4, SBRTEMPLATE 1.
            flags: 0x900e,
            huffman_flags: None,
            refinement_at: None,
            instances: 6,
            body: vec![0xa5, 0x5a, 0xff, 0xac],
        }
    }
}

impl Region {
    fn data(&self) -> Vec<u8> {
        let mut data = Vec::new();
        for value in [self.width, self.height, self.x, self.y] {
            data.extend_from_slice(&value.to_be_bytes());
        }
        data.push(self.region_flags);
        data.extend_from_slice(&self.flags.to_be_bytes());
        if let Some(flags) = self.huffman_flags {
            data.extend_from_slice(&flags.to_be_bytes());
        }
        if let Some(at) = self.refinement_at {
            data.extend(at.map(|value| value as u8));
        }
        data.extend_from_slice(&self.instances.to_be_bytes());
        data.extend_from_slice(&self.body);
        data
    }

    fn segment(&self) -> Vec<u8> {
        frame(3, 6, &[2], 1, &self.data())
    }
}

fn parse_with(
    source: &mut Source,
    budget: TextRegionBudget,
    cancellation: &impl Cancellation,
) -> Result<TextRegionHeader, TextRegionError> {
    let header = parse_header(&source.bytes);
    ready(read_text_region_header(
        source,
        &header,
        &dictionary(2, 1),
        &Limits::default(),
        budget,
        cancellation,
    ))
}

fn parse(region: &Region) -> Result<TextRegionHeader, TextRegionError> {
    parse_with(
        &mut Source::new(region.segment()),
        TextRegionBudget::default(),
        &NeverCancel,
    )
}

fn error(region: &Region) -> TextRegionError {
    parse(region).expect_err("header must be rejected")
}

const DATA_OFFSET: u64 = 12;

#[test]
fn parses_observed_arithmetic_header_and_exposes_exact_body() {
    let region = Region {
        x: 7,
        y: 9,
        ..Region::default()
    };
    let parsed = parse(&region).unwrap();
    assert_eq!(parsed.region.width, 64);
    assert_eq!(parsed.region.height, 32);
    assert_eq!((parsed.region.x, parsed.region.y), (7, 9));
    assert_eq!(parsed.region.combination, RegionCombination::Or);
    let flags = parsed.flags;
    assert_eq!(flags.raw, 0x900e);
    assert!(!flags.huffman && flags.refine && !flags.transposed && !flags.default_pixel);
    assert_eq!((flags.log_strips, flags.strips()), (3, 8));
    assert_eq!(flags.reference_corner, ReferenceCorner::BottomLeft);
    assert_eq!(flags.combination, SymbolCombination::Or);
    assert_eq!((flags.ds_offset, flags.refinement_template), (4, 1));
    assert_eq!((parsed.huffman_flags, parsed.refinement_at), (None, None));
    assert_eq!(parsed.instances, 6);
    assert_eq!(parsed.header_bytes, 23);
    assert_eq!(
        parsed.body,
        SegmentSpan {
            offset: DATA_OFFSET + 23,
            length: 4
        }
    );
    assert_eq!(parsed.unsupported_feature(), None);
}

#[test]
fn decodes_every_observed_flag_value() {
    for (raw, ds_offset) in [
        (0x840e, 1),
        (0x880e, 2),
        (0x8c0e, 3),
        (0x900e, 4),
        (0x940e, 5),
        (0x980e, 6),
        (0x9c0e, 7),
        (0xa00e, 8),
        (0xa40e, 9),
        (0xa80e, 10),
        (0xac0e, 11),
        (0xb00e, 12),
        (0xb80e, 14),
        (0xbc0e, 15),
    ] {
        let flags = parse(&Region {
            flags: raw,
            ..Region::default()
        })
        .unwrap()
        .flags;
        assert_eq!(flags.ds_offset, ds_offset, "{raw:#06x}");
        assert_eq!((flags.strips(), flags.refinement_template), (8, 1));
    }
}

#[test]
fn signed_ds_offset_edges_and_every_field_value() {
    for (bits, expected) in [(0x0f, 15), (0x10, -16), (0x1f, -1), (0, 0)] {
        let flags = parse(&Region {
            flags: bits << 10,
            ..Region::default()
        })
        .unwrap()
        .flags;
        assert_eq!(flags.ds_offset, expected);
        assert!(!flags.refine);
        assert_eq!(flags.refinement_template, 0);
    }
    for (log, strips) in [(0, 1), (1, 2), (2, 4), (3, 8)] {
        let flags = parse(&Region {
            flags: log << 2,
            ..Region::default()
        })
        .unwrap()
        .flags;
        assert_eq!((flags.log_strips, flags.strips()), (log as u8, strips));
    }
    for (value, corner) in [
        (0, ReferenceCorner::BottomLeft),
        (1, ReferenceCorner::TopLeft),
        (2, ReferenceCorner::BottomRight),
        (3, ReferenceCorner::TopRight),
    ] {
        let flags = parse(&Region {
            flags: value << 4,
            ..Region::default()
        })
        .unwrap()
        .flags;
        assert_eq!(flags.reference_corner, corner);
    }
    for (value, combination) in [
        (0, SymbolCombination::Or),
        (1, SymbolCombination::And),
        (2, SymbolCombination::Xor),
        (3, SymbolCombination::Xnor),
    ] {
        let flags = parse(&Region {
            flags: value << 7 | 0x0240,
            ..Region::default()
        })
        .unwrap()
        .flags;
        assert_eq!(flags.combination, combination);
        assert!(flags.transposed && flags.default_pixel);
    }
    for (value, combination) in [
        (1, RegionCombination::And),
        (2, RegionCombination::Xor),
        (3, RegionCombination::Xnor),
        (4, RegionCombination::Replace),
    ] {
        let region = parse(&Region {
            region_flags: value,
            ..Region::default()
        })
        .unwrap()
        .region;
        assert_eq!(region.combination, combination);
    }
}

#[test]
fn measured_refinement_template_conflict_is_located_with_raw_flags() {
    let error = error(&Region {
        flags: 0xa40c,
        ..Region::default()
    });
    assert_eq!(error.segment, 3);
    assert_eq!(error.offset, DATA_OFFSET + 17);
    assert_eq!(error.bytes_fetched, 19);
    assert!(matches!(
        error.kind,
        TextRegionErrorKind::MalformedFlags {
            field: "SBRTEMPLATE without SBREFINE",
            raw: 0xa40c
        }
    ));
    assert_eq!(
        error.to_string(),
        "JBIG2 text region segment 3 at source byte 29: malformed SBRTEMPLATE without SBREFINE (flags 0xa40c)"
    );
}

#[test]
fn refinement_template_zero_reads_adaptive_pixels_and_is_classified() {
    let parsed = parse(&Region {
        flags: 0x000e,
        refinement_at: Some([-1, -1, -2, 3]),
        ..Region::default()
    })
    .unwrap();
    assert_eq!(parsed.refinement_at, Some([(-1, -1), (-2, 3)]));
    assert_eq!(parsed.header_bytes, 27);
    assert_eq!(parsed.body.offset, DATA_OFFSET + 27);
    assert_eq!(
        parsed.unsupported_feature(),
        Some(("refinement template 0 with adaptive pixels", 0))
    );
}

#[test]
fn huffman_headers_are_parsed_then_classified_unsupported() {
    let parsed = parse(&Region {
        flags: 0x0003,
        huffman_flags: Some(0x7ffd),
        refinement_at: Some([-1, -1, -1, -1]),
        body: Vec::new(),
        ..Region::default()
    })
    .unwrap();
    assert_eq!(parsed.huffman_flags, Some(0x7ffd));
    assert_eq!(parsed.body.length, 0);
    assert_eq!(
        parsed.unsupported_feature(),
        Some(("Huffman text region", 0x7ffd))
    );
    let direct = parse(&Region {
        flags: 0x0001,
        huffman_flags: Some(0x003d),
        ..Region::default()
    })
    .unwrap();
    assert_eq!(
        direct.unsupported_feature(),
        Some(("Huffman text region", 0x3d))
    );
}

#[test]
fn malformed_huffman_flags_are_located() {
    for (flags, huffman, field) in [
        (0x0001, 0x8000, "reserved Huffman flag"),
        (0x0001, 0x0002, "reserved Huffman table selector"),
        (0x8003, 0x0080, "reserved Huffman table selector"),
        (0x8003, 0x0200, "reserved Huffman table selector"),
        (0x8003, 0x0800, "reserved Huffman table selector"),
        (0x8003, 0x2000, "reserved Huffman table selector"),
        (0x0001, 0x0040, "refinement Huffman tables without SBREFINE"),
        (0x0001, 0x4000, "refinement Huffman tables without SBREFINE"),
    ] {
        let error = error(&Region {
            flags,
            huffman_flags: Some(huffman),
            ..Region::default()
        });
        assert_eq!(error.offset, DATA_OFFSET + 19, "{huffman:#06x}");
        match error.kind {
            TextRegionErrorKind::MalformedFlags { field: got, raw } => {
                assert_eq!((got, raw), (field, huffman));
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}

#[test]
fn region_information_is_validated() {
    let reserved = error(&Region {
        region_flags: 0x08,
        ..Region::default()
    });
    assert_eq!(reserved.offset, DATA_OFFSET + 16);
    assert!(matches!(
        reserved.kind,
        TextRegionErrorKind::Malformed("reserved region segment flags")
    ));
    let operator = error(&Region {
        region_flags: 5,
        ..Region::default()
    });
    assert!(matches!(
        operator.kind,
        TextRegionErrorKind::Malformed("region combination operator")
    ));
    for (width, height) in [(0, 5), (5, 0)] {
        let empty = error(&Region {
            width,
            height,
            ..Region::default()
        });
        assert!(matches!(
            empty.kind,
            TextRegionErrorKind::Unsupported {
                feature: "empty text region",
                value: 0
            }
        ));
    }
}

#[test]
fn dimensions_pixels_instances_and_body_respect_budget() {
    let budget = TextRegionBudget {
        max_width: 64,
        max_height: 32,
        max_pixels: 64 * 32,
        max_instances: 6,
        max_body_bytes: 4,
        ..TextRegionBudget::default()
    };
    let check = |region: Region, resource: &str, offset: u64, attempted: u64| {
        let error = parse_with(&mut Source::new(region.segment()), budget, &NeverCancel)
            .expect_err("budget must be enforced");
        assert_eq!(error.offset, offset, "{resource}");
        match error.kind {
            TextRegionErrorKind::LimitExceeded {
                resource: got,
                attempted: value,
                ..
            } => assert_eq!((got, value), (resource, attempted)),
            other => panic!("unexpected {other:?}"),
        }
    };
    parse_with(
        &mut Source::new(Region::default().segment()),
        budget,
        &NeverCancel,
    )
    .unwrap();
    check(
        Region {
            width: 65,
            height: 1,
            ..Region::default()
        },
        "text region width",
        DATA_OFFSET,
        65,
    );
    check(
        Region {
            width: 1,
            height: 33,
            ..Region::default()
        },
        "text region height",
        DATA_OFFSET + 4,
        33,
    );
    let tight = TextRegionBudget {
        max_pixels: 64 * 32 - 1,
        ..budget
    };
    let error = parse_with(
        &mut Source::new(Region::default().segment()),
        tight,
        &NeverCancel,
    )
    .unwrap_err();
    assert!(matches!(
        error.kind,
        TextRegionErrorKind::LimitExceeded {
            resource: "text region pixels",
            attempted: 2048,
            ..
        }
    ));
    check(
        Region {
            instances: 7,
            ..Region::default()
        },
        "text region symbol instances",
        DATA_OFFSET + 19,
        7,
    );
    check(
        Region {
            body: vec![0; 5],
            ..Region::default()
        },
        "text region body bytes",
        DATA_OFFSET + 23,
        5,
    );
    let small_header = TextRegionBudget {
        max_data_header_bytes: 22,
        ..budget
    };
    let error = parse_with(
        &mut Source::new(Region::default().segment()),
        small_header,
        &NeverCancel,
    )
    .unwrap_err();
    assert!(matches!(
        error.kind,
        TextRegionErrorKind::LimitExceeded {
            resource: "text region header bytes",
            limit: 22,
            attempted: 23
        }
    ));
    assert_eq!(error.bytes_fetched, 19);
    assert!(
        error
            .to_string()
            .ends_with("text region header bytes limit 22 exceeded by 23")
    );
}

#[test]
fn display_propagates_formatter_errors() {
    struct Refuse;
    impl std::fmt::Write for Refuse {
        fn write_str(&mut self, _: &str) -> std::fmt::Result {
            Err(std::fmt::Error)
        }
    }
    let error = error(&Region {
        flags: 0xa40c,
        ..Region::default()
    });
    assert!(std::fmt::Write::write_fmt(&mut Refuse, format_args!("{error}")).is_err());
}

#[test]
fn truncated_fields_and_mq_terminal_pair() {
    let data = Region::default().data();
    for (length, field) in [
        (0, "text region header"),
        (18, "text region header"),
        (19, "SBNUMINSTANCES"),
        (22, "SBNUMINSTANCES"),
        (23, "MQ body terminal pair"),
        (24, "MQ body terminal pair"),
    ] {
        let segment = frame(3, 6, &[2], 1, &data[..length]);
        let error = parse_with(
            &mut Source::new(segment),
            TextRegionBudget::default(),
            &NeverCancel,
        )
        .unwrap_err();
        match error.kind {
            TextRegionErrorKind::Truncated(got) => assert_eq!(got, field, "{length}"),
            other => panic!("unexpected {other:?} at {length}"),
        }
    }
    let huffman = Region {
        flags: 0x0001,
        huffman_flags: Some(0),
        ..Region::default()
    }
    .data();
    let error = parse_with(
        &mut Source::new(frame(3, 6, &[2], 1, &huffman[..20])),
        TextRegionBudget::default(),
        &NeverCancel,
    )
    .unwrap_err();
    assert!(matches!(
        error.kind,
        TextRegionErrorKind::Truncated("text region Huffman flags")
    ));
    let adaptive = Region {
        flags: 0x0002,
        refinement_at: Some([0; 4]),
        ..Region::default()
    }
    .data();
    let error = parse_with(
        &mut Source::new(frame(3, 6, &[2], 1, &adaptive[..22])),
        TextRegionBudget::default(),
        &NeverCancel,
    )
    .unwrap_err();
    assert!(matches!(
        error.kind,
        TextRegionErrorKind::Truncated("text region refinement AT")
    ));
}

fn with_header(
    source_bytes: Vec<u8>,
    edit: impl FnOnce(&mut SegmentHeader, &mut SegmentHeader),
) -> TextRegionError {
    let mut region = parse_header(&source_bytes);
    let mut dict = dictionary(2, 1);
    edit(&mut region, &mut dict);
    let mut source = Source::new(source_bytes);
    let error = ready(read_text_region_header(
        &mut source,
        &region,
        &dict,
        &Limits::default(),
        TextRegionBudget::default(),
        &NeverCancel,
    ))
    .unwrap_err();
    assert!(source.reads.is_empty(), "framing errors must precede reads");
    assert_eq!(error.bytes_fetched, 0);
    error
}

#[test]
fn segment_type_reference_and_page_are_checked_before_reads() {
    let valid = Region::default().segment();
    for (kind, expected) in [
        (4, "unsupported text region segment type (4)"),
        (7, "unsupported text region segment type (7)"),
        (0, "malformed segment type is not a text region"),
    ] {
        let error = with_header(
            frame(3, kind, &[2], 1, &Region::default().data()),
            |_, _| {},
        );
        assert!(error.to_string().ends_with(expected), "{error}");
    }
    // The framing reader already refuses page 0 for region types; the
    // parser still refuses a caller-constructed header without a page.
    let error = with_header(valid.clone(), |region, _| region.page_association = 0);
    assert!(
        error
            .to_string()
            .ends_with("immediate region without a page")
    );
    let error = with_header(
        frame(3, 6, &[1, 2], 1, &Region::default().data()),
        |_, _| {},
    );
    assert!(
        error
            .to_string()
            .ends_with("unsupported text region reference count (2)")
    );
    let error = with_header(frame(3, 6, &[], 1, &Region::default().data()), |_, _| {});
    assert!(
        error
            .to_string()
            .ends_with("unsupported text region reference count (0)")
    );
    let error = with_header(frame(3, 6, &[1], 1, &Region::default().data()), |_, _| {});
    assert!(
        error
            .to_string()
            .ends_with("reference differs from the supplied dictionary")
    );
    let error = with_header(valid.clone(), |_, dict| *dict = referred(2, 6, 1));
    assert!(
        error
            .to_string()
            .ends_with("referred segment is not a symbol dictionary")
    );
    let error = with_header(valid.clone(), |region, _| region.number = 2);
    assert!(
        error
            .to_string()
            .ends_with("dictionary does not precede region"),
        "{error}"
    );
    let error = with_header(valid.clone(), |_, dict| *dict = dictionary(2, 2));
    assert!(
        error
            .to_string()
            .ends_with("dictionary page association differs")
    );
    // A global dictionary (page association 0) is accepted.
    let mut source = Source::new(valid.clone());
    let region = parse_header(&valid);
    ready(read_text_region_header(
        &mut source,
        &region,
        &dictionary(2, 0),
        &Limits::default(),
        TextRegionBudget::default(),
        &NeverCancel,
    ))
    .unwrap();
}

#[test]
fn spans_are_checked_before_reads() {
    let valid = Region::default().segment();
    let error = with_header(valid.clone(), |region, _| region.data.offset = u64::MAX - 2);
    assert!(
        error
            .to_string()
            .ends_with("invalid span: data end overflow")
    );
    let error = with_header(valid.clone(), |region, _| region.data.length += 1);
    assert!(
        error
            .to_string()
            .ends_with("invalid span: data outside source")
    );
    let error = with_header(valid.clone(), |region, _| {
        region.header_length = DATA_OFFSET + 1
    });
    assert!(
        error
            .to_string()
            .ends_with("segment header start underflow")
    );
    let mut source = Source::new(valid.clone());
    let region = parse_header(&valid);
    let error = ready(read_text_region_header(
        &mut source,
        &region,
        &dictionary(2, 1),
        &Limits {
            max_input_bytes: 10,
            ..Limits::default()
        },
        TextRegionBudget::default(),
        &NeverCancel,
    ))
    .unwrap_err();
    assert!(matches!(
        error.kind,
        TextRegionErrorKind::LimitExceeded {
            resource: "text region data bytes",
            limit: 10,
            ..
        }
    ));
    assert!(source.reads.is_empty());
}

#[test]
fn invalid_limits_and_zero_request_bound_are_rejected() {
    let bytes = Region::default().segment();
    let region = parse_header(&bytes);
    let mut source = Source::new(bytes);
    let error = ready(read_text_region_header(
        &mut source,
        &region,
        &dictionary(2, 1),
        &Limits {
            io_chunk_bytes: 0,
            ..Limits::default()
        },
        TextRegionBudget::default(),
        &NeverCancel,
    ))
    .unwrap_err();
    assert!(matches!(error.kind, TextRegionErrorKind::Source(_)));
    assert!(std::error::Error::source(&error).is_some());
    let error = parse_with(
        &mut source,
        TextRegionBudget {
            max_source_request_bytes: 0,
            ..TextRegionBudget::default()
        },
        &NeverCancel,
    )
    .unwrap_err();
    assert!(matches!(
        error.kind,
        TextRegionErrorKind::Malformed("zero I/O request bound")
    ));
    assert!(std::error::Error::source(&error).is_none());
    assert!(source.reads.is_empty());
}

#[test]
fn requests_are_bounded_and_short_reads_resume() {
    let mut source = Source::new(Region::default().segment());
    source.max_read = 3;
    let budget = TextRegionBudget {
        max_source_request_bytes: 5,
        ..TextRegionBudget::default()
    };
    let parsed = parse_with(&mut source, budget, &NeverCancel).unwrap();
    assert_eq!(parsed.header_bytes, 23);
    assert!(source.reads.iter().all(|&(_, length)| length <= 5));
    let fetched: u64 = source
        .reads
        .iter()
        .map(|&(_, length)| length.min(3) as u64)
        .sum();
    assert_eq!(fetched, 23, "the body is never read");
    assert!(
        source
            .reads
            .iter()
            .all(|&(offset, _)| offset < DATA_OFFSET + 23)
    );
}

#[test]
fn source_faults_are_typed_and_located() {
    let cases = [
        (
            Fault::ZeroAt(DATA_OFFSET + 5),
            "truncated text region header",
            5,
        ),
        (
            Fault::ZeroAt(DATA_OFFSET + 19),
            "truncated SBNUMINSTANCES",
            19,
        ),
        (Fault::Overreport, "malformed source read length", 0),
        (Fault::Fail, "source: ", 0),
        (Fault::Cancelled, "cancelled", 0),
    ];
    for (fault, message, fetched) in cases {
        let mut source = Source::new(Region::default().segment());
        source.fault = fault;
        source.max_read = 5;
        let error = parse_with(&mut source, TextRegionBudget::default(), &NeverCancel).unwrap_err();
        assert!(error.to_string().contains(message), "{error}");
        assert_eq!(error.bytes_fetched, fetched);
        assert_eq!(error.offset, DATA_OFFSET + fetched);
    }
}

#[test]
fn cancellation_is_checked_before_and_between_reads() {
    let bytes = Region::default().segment();
    let region = parse_header(&bytes);
    for after in 0..=3 {
        let reads = Cell::new(0);
        let cancellation = CancelAfter {
            reads: &reads,
            after,
        };
        let mut source = CountingSource {
            inner: Source::new(bytes.clone()),
            reads: &reads,
        };
        source.inner.max_read = 7;
        let error = ready(read_text_region_header(
            &mut source,
            &region,
            &dictionary(2, 1),
            &Limits::default(),
            TextRegionBudget::default(),
            &cancellation,
        ))
        .unwrap_err();
        assert!(
            matches!(error.kind, TextRegionErrorKind::Cancelled),
            "{after}"
        );
        assert_eq!(source.inner.reads.len(), after);
    }
}

struct CountingSource<'a> {
    inner: Source,
    reads: &'a Cell<usize>,
}

impl RangedSource for CountingSource<'_> {
    fn size(&self) -> u64 {
        self.inner.size()
    }

    async fn read_at(
        &mut self,
        offset: u64,
        destination: &mut [u8],
    ) -> caj2pdf_core::Result<usize> {
        self.reads.set(self.reads.get() + 1);
        self.inner.read_at(offset, destination).await
    }
}

#[test]
fn dropped_pending_read_leaves_no_partial_result() {
    let bytes = Region::default().segment();
    let region = parse_header(&bytes);
    let dict = dictionary(2, 1);
    let mut source = Source::new(bytes);
    source.fault = Fault::PendingAt(DATA_OFFSET + 19);
    {
        let limits = Limits::default();
        let mut future = pin!(read_text_region_header(
            &mut source,
            &region,
            &dict,
            &limits,
            TextRegionBudget::default(),
            &NeverCancel,
        ));
        assert!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
    }
    // The parser holds no state between calls: a fresh call succeeds.
    source.fault = Fault::None;
    let parsed = ready(read_text_region_header(
        &mut source,
        &region,
        &dict,
        &Limits::default(),
        TextRegionBudget::default(),
        &NeverCancel,
    ))
    .unwrap();
    assert_eq!(parsed.instances, 6);
}

#[test]
fn fixed_budget_mutations_never_panic_or_read_past_header() {
    let original = Region::default().segment();
    let region = parse_header(&original);
    let dict = dictionary(2, 1);
    let mut state = 0x2545_f491_u32;
    let mut outcomes = [0usize; 2];
    for _ in 0..4096 {
        let mut bytes = original.clone();
        for _ in 0..1 + (state % 3) {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let index =
                DATA_OFFSET as usize + (state as usize % (bytes.len() - DATA_OFFSET as usize));
            bytes[index] ^= (state >> 24) as u8 | 1;
        }
        let mut source = Source::new(bytes);
        let result = ready(read_text_region_header(
            &mut source,
            &region,
            &dict,
            &Limits::default(),
            TextRegionBudget::default(),
            &NeverCancel,
        ));
        outcomes[usize::from(result.is_ok())] += 1;
        let end = source
            .reads
            .iter()
            .map(|&(offset, length)| offset + length as u64)
            .max()
            .unwrap_or(0);
        assert!(end <= DATA_OFFSET + 29, "header reads only: {end}");
        if let Ok(parsed) = result {
            assert_eq!(
                parsed.body.offset + parsed.body.length,
                region.data.offset + region.data.length
            );
            assert!(parsed.region.width > 0 && parsed.region.height > 0);
        }
    }
    assert!(outcomes[0] > 0 && outcomes[1] > 0, "{outcomes:?}");
}
