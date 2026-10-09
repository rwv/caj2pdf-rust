// SPDX-License-Identifier: MIT

//! Format detection and the facade every adapter calls: [`convert`],
//! [`inspect`], [`inspect_pages`], [`needs_fonts`], [`read_outline`] and
//! [`index_pdf`] detect the input and dispatch to the format engines.

mod caa;
mod convert;
mod inspect;
mod observe;

pub use convert::{
    ConversionOptions, ConversionReport, FONTS_REQUIRE_HNC8, Fonts, ImageCounts, OmittedPage,
    convert, convert_with_ttkn_response, needs_fonts,
};
pub use inspect::{
    DocumentInfo, InspectOptions, PageVisitor, Structure, index_pdf, inspect, inspect_pages,
    read_outline,
};
pub use observe::Progress;

use crate::{Cancellation, Limits, RangedSource, Result, read_exact_at};
use std::ops::Range;

/// Recognized input families. Recognition does not imply conversion support.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputFormat {
    Pdf,
    Caj,
    Kdh,
    Nh,
    Hn,
    C8,
    Teb,
    /// An observed CAA target descriptor, without document pages.
    Caa,
}

impl InputFormat {
    /// The family's name in upper case, as in `"PDF"` or `"HN"`.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Pdf => "PDF",
            Self::Caj => "CAJ",
            Self::Kdh => "KDH",
            Self::Nh => "NH",
            Self::Hn => "HN",
            Self::C8 => "C8",
            Self::Teb => "TEB",
            Self::Caa => "CAA",
        }
    }

    /// Whether [`convert`] converts this family; [`inspect`] reads every
    /// family, but reports no page count for the others.
    pub const fn is_convertible(self) -> bool {
        matches!(
            self,
            Self::Pdf | Self::Caj | Self::Kdh | Self::Hn | Self::C8
        )
    }
}

/// Leading bytes that [`detect_format`] needs to recognize every signature
/// that must start at byte 0.
pub const SIGNATURE_BYTES: usize = 5;

/// Leading bytes searched for a `%PDF-` header that does not start at byte 0.
/// The whole five-byte marker must lie within this prefix.
pub const PDF_HEADER_SEARCH_BYTES: usize = 1024;

const PDF_SIGNATURE: &[u8] = b"%PDF-";

/// A recognized input family and where its header starts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Detection {
    pub format: InputFormat,
    /// Offset of the `%PDF-` header; always 0 for the other families. A PDF
    /// reader should treat the input as starting here (see `docs/pdf-input.md`).
    pub header_offset: u64,
    /// Leading input bytes read to reach this result.
    pub bytes_read: u64,
}

/// Recognize an input family from its leading bytes, never from a file name:
/// some observed `.caj` files are plain PDFs.
///
/// `prefix` should hold the first `min(PDF_HEADER_SEARCH_BYTES, size)` bytes;
/// `SIGNATURE_BYTES` suffice for every signature that starts at byte 0, and
/// bytes beyond `PDF_HEADER_SEARCH_BYTES` are ignored. CAA requires its
/// complete observed descriptor fields in the same bounded prefix.
/// Other signatures are recorded in `tests/fixtures/README.md`, `docs/research/caj-format.md`,
/// `docs/research/kdh-format.md`, and `docs/research/hnc8-container.md`; CAJ, KDH, HN, C8, and
/// TEB must start at byte 0. If none matches, a `%PDF-` marker elsewhere in
/// the searched prefix still selects PDF, as observed for files with a leading
/// newline, byte-order mark, or junk line. Recognition does not imply
/// support; the selected reader validates the complete header.
pub fn detect_format(prefix: &[u8]) -> Option<InputFormat> {
    leading_format(prefix).or_else(|| searched_format(prefix).map(|(format, _)| format))
}

fn searched_format(prefix: &[u8]) -> Option<(InputFormat, usize)> {
    let prefix = &prefix[..prefix.len().min(PDF_HEADER_SEARCH_BYTES)];
    if caa::recognizes(prefix) {
        Some((InputFormat::Caa, 0))
    } else {
        prefix
            .windows(PDF_SIGNATURE.len())
            .position(|window| window == PDF_SIGNATURE)
            .map(|offset| (InputFormat::Pdf, offset))
    }
}

fn leading_format(prefix: &[u8]) -> Option<InputFormat> {
    const SIGNATURES: [(&[u8], InputFormat); 6] = [
        (PDF_SIGNATURE, InputFormat::Pdf),
        (b"CAJ", InputFormat::Caj),
        (b"KDH", InputFormat::Kdh),
        (b"HN", InputFormat::Hn),
        (b"\xc8\0\0\0", InputFormat::C8),
        (b"TEB", InputFormat::Teb),
    ];
    SIGNATURES
        .iter()
        .find(|(signature, _)| prefix.starts_with(signature))
        .map(|&(_, format)| format)
}

/// Read the leading bytes of `source` and recognize its input family as
/// [`detect_format`] does, also locating a displaced PDF header.
///
/// Only `min(SIGNATURE_BYTES, size)` bytes are read when a signature starts
/// at byte 0; otherwise CAA fields and a displaced PDF header are searched in
/// at most `PDF_HEADER_SEARCH_BYTES` in total, in reads
/// no larger than `limits.io_chunk_bytes`. `Ok(None)` means unrecognized.
pub fn detect_source<S: RangedSource, C: Cancellation>(
    source: &mut S,
    limits: &Limits,
    cancellation: &C,
) -> Result<Option<Detection>> {
    limits.validate()?;
    let mut prefix = [0; PDF_HEADER_SEARCH_BYTES];
    let size = source.size();
    let leading = size.min(SIGNATURE_BYTES as u64) as usize;
    read_prefix(source, &mut prefix, 0..leading, limits, cancellation)?;
    if let Some(format) = leading_format(&prefix[..leading]) {
        return Ok(Some(Detection {
            format,
            header_offset: 0,
            bytes_read: leading as u64,
        }));
    }
    let searched = size.min(PDF_HEADER_SEARCH_BYTES as u64) as usize;
    read_prefix(source, &mut prefix, leading..searched, limits, cancellation)?;
    Ok(
        searched_format(&prefix[..searched]).map(|(format, offset)| Detection {
            format,
            header_offset: offset as u64,
            bytes_read: searched as u64,
        }),
    )
}

fn read_prefix<S: RangedSource, C: Cancellation>(
    source: &mut S,
    prefix: &mut [u8],
    range: Range<usize>,
    limits: &Limits,
    cancellation: &C,
) -> Result<()> {
    let start = range.start;
    for (index, chunk) in prefix[range].chunks_mut(limits.io_chunk_bytes).enumerate() {
        let offset = start + index * limits.io_chunk_bytes;
        read_exact_at(source, offset as u64, chunk, limits, cancellation)?;
    }
    Ok(())
}

/// The PDF viewed from its `%PDF-` header, which may follow leading bytes.
fn pdf_range(size: u64, header_offset: u64) -> crate::pdf::PdfRange {
    crate::pdf::PdfRange {
        offset: header_offset,
        length: size - header_offset,
    }
}

/// One outline entry in document order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Bookmark {
    /// Zero means a root entry; each child increments the depth by one.
    pub depth: u32,
    pub title: String,
    /// Zero-based page index.
    pub page_index: u32,
}

/// A recipient for streamed bookmark entries, called once per entry.
pub trait BookmarkVisitor {
    fn visit(&mut self, bookmark: Bookmark) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::{
        Detection, InputFormat, PDF_HEADER_SEARCH_BYTES, SIGNATURE_BYTES, detect_format,
        detect_source,
    };
    use crate::ErrorKind;
    use crate::test_support::{CancelAfter, NEVER};
    use crate::{Error, Limits, RangedSource, Result};

    #[test]
    fn detects_offset_zero_signatures_and_a_displaced_pdf_header() {
        let cases: [(&[u8], Option<InputFormat>); 16] = [
            (b"%PDF-1.7", Some(InputFormat::Pdf)),
            (b"CAJ\0", Some(InputFormat::Caj)),
            (b"KDH 2.00", Some(InputFormat::Kdh)),
            (b"HN\0\0", Some(InputFormat::Hn)),
            (b"\xc8\0\0\0\x01", Some(InputFormat::C8)),
            (b"TEB", Some(InputFormat::Teb)),
            (b" %PDF-", Some(InputFormat::Pdf)),
            (b"\xef\xbb\xbf%PDF-1.4", Some(InputFormat::Pdf)),
            (b"%PDF", None),
            (b"\xc8\0\0", None),
            (b"NH\0\0", None),
            (b" CAJ", None),
            (b"\nKDH 2.00", None),
            (b" TEB", None),
            (b"caj", None),
            (b"", None),
        ];
        for (prefix, expected) in cases {
            assert_eq!(detect_format(prefix), expected, "{prefix:?}");
        }
    }

    #[test]
    fn displaced_pdf_header_must_fit_in_the_searched_prefix() {
        let last = PDF_HEADER_SEARCH_BYTES - b"%PDF-".len();
        let mut prefix = vec![b'x'; last];
        prefix.extend_from_slice(b"%PDF-1.7");
        assert_eq!(detect_format(&prefix), Some(InputFormat::Pdf));
        prefix.insert(0, b'x');
        assert_eq!(detect_format(&prefix), None);
    }

    #[test]
    fn signature_bytes_cover_the_longest_signature() {
        assert_eq!(
            detect_format(&b"%PDF-1.7"[..SIGNATURE_BYTES]),
            Some(InputFormat::Pdf)
        );
        assert_eq!(detect_format(&b"%PDF-"[..SIGNATURE_BYTES - 1]), None);
    }

    /// Serves `bytes` but claims `size`, recording every read.
    struct Source {
        bytes: Vec<u8>,
        size: u64,
        reads: Vec<(u64, usize)>,
    }

    impl Source {
        fn new(bytes: Vec<u8>) -> Self {
            Self {
                size: bytes.len() as u64,
                bytes,
                reads: Vec::new(),
            }
        }
    }

    impl RangedSource for Source {
        fn size(&self) -> u64 {
            self.size
        }

        fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
            self.reads.push((offset, destination.len()));
            let start = (offset as usize).min(self.bytes.len());
            let available = &self.bytes[start..];
            let count = available.len().min(destination.len());
            destination[..count].copy_from_slice(&available[..count]);
            Ok(count)
        }
    }

    fn detect(source: &mut Source, limits: &Limits) -> Result<Option<Detection>> {
        detect_source(source, limits, &NEVER)
    }

    fn detection(format: InputFormat, header_offset: u64, bytes_read: u64) -> Option<Detection> {
        Some(Detection {
            format,
            header_offset,
            bytes_read,
        })
    }

    #[test]
    fn offset_zero_signatures_read_only_the_signature_prefix() {
        let mut source = Source::new(b"CAJ\0 and %PDF- later".to_vec());
        let found = detect(&mut source, &Limits::default()).unwrap();
        assert_eq!(found, detection(InputFormat::Caj, 0, 5));
        assert_eq!(source.reads, [(0, SIGNATURE_BYTES)]);

        let mut short = Source::new(b"HN".to_vec());
        let found = detect(&mut short, &Limits::default()).unwrap();
        assert_eq!(found, detection(InputFormat::Hn, 0, 2));
    }

    #[test]
    fn displaced_pdf_headers_are_located_within_the_bounded_prefix() {
        let mut junk = vec![b'#'; 100];
        junk.push(b'\n');
        for (prefix, offset) in [
            (b"\n".as_slice(), 1),
            (b"\xef\xbb\xbf".as_slice(), 3),
            (junk.as_slice(), 101),
        ] {
            let mut bytes = prefix.to_vec();
            bytes.extend_from_slice(&[b"%PDF-1.7\n".as_slice(), &[b' '; 4096]].concat());
            let mut source = Source::new(bytes);
            let found = detect(&mut source, &Limits::default()).unwrap();
            let searched = PDF_HEADER_SEARCH_BYTES as u64;
            assert_eq!(found, detection(InputFormat::Pdf, offset, searched));
            let tail = PDF_HEADER_SEARCH_BYTES - SIGNATURE_BYTES;
            assert_eq!(source.reads, [(0, SIGNATURE_BYTES), (5, tail)]);
        }
    }

    #[test]
    fn the_search_reads_in_io_chunks_and_stops_at_the_source_end() {
        let limits = Limits {
            io_chunk_bytes: 4,
            ..Limits::default()
        };
        let mut source = Source::new(b"\r\n%PDF-1.0".to_vec());
        let found = detect(&mut source, &limits).unwrap();
        assert_eq!(found, detection(InputFormat::Pdf, 2, 10));
        assert_eq!(source.reads, [(0, 4), (4, 1), (5, 4), (9, 1)]);
    }

    #[test]
    fn a_header_beyond_the_searched_prefix_or_an_empty_source_is_unrecognized() {
        let mut bytes = vec![0; PDF_HEADER_SEARCH_BYTES - 4];
        bytes.extend_from_slice(b"%PDF-1.7\n");
        let mut late = Source::new(bytes);
        assert_eq!(detect(&mut late, &Limits::default()).unwrap(), None);
        assert_eq!(late.reads.last(), Some(&(5, PDF_HEADER_SEARCH_BYTES - 5)));

        let mut empty = Source::new(Vec::new());
        assert_eq!(detect(&mut empty, &Limits::default()).unwrap(), None);
        assert!(empty.reads.is_empty());
    }

    #[test]
    fn invalid_limits_short_sources_and_cancellation_are_errors() {
        let mut source = Source::new(b"%PDF-1.7".to_vec());
        let zero_chunk = Limits {
            io_chunk_bytes: 0,
            ..Limits::default()
        };
        assert!(detect(&mut source, &zero_chunk).is_err());
        assert!(source.reads.is_empty());

        let mut short = Source::new(b"\n\n\n\n\n%PD".to_vec());
        short.size = 64;
        let error = detect(&mut short, &Limits::default()).unwrap_err();
        assert!(
            matches!(
                error,
                Error {
                    kind: ErrorKind::Truncated { .. },
                    offset: Some(5),
                    ..
                }
            ),
            "{error:?}"
        );

        let mut source = Source::new(b"%PDF-1.7".to_vec());
        let error =
            detect_source(&mut source, &Limits::default(), &CancelAfter::always()).unwrap_err();
        assert!(
            matches!(
                error,
                Error {
                    kind: ErrorKind::Cancelled,
                    ..
                }
            ),
            "{error:?}"
        );
    }

    const CAA: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/target_descriptor.caa"
    ));

    #[test]
    fn caa_requires_complete_observed_fields_within_the_probe_bound() {
        let text = std::str::from_utf8(CAA).unwrap();
        for bytes in [
            CAA.to_vec(),
            text.replace("DOCTYPE=NH", "DOCTYPE=KDH")
                .replace('\n', "\r\n")
                .into_bytes(),
            [CAA, b"\r\n \t"].concat(),
        ] {
            assert_eq!(detect_format(&bytes), Some(InputFormat::Caa));
        }
        for end in 0..CAA.len() {
            assert_eq!(detect_format(&CAA[..end]), None, "prefix {end}");
        }
        for (from, to) in [
            ("[TARGET]", "[OTHER]"),
            ("A1=1", "A1="),
            ("D1=1", "D1=-1"),
            ("A2=QQ==", "A2="),
            ("A2=QQ==", "A2=Q!"),
            ("D2=Qg==", "D2=你好"),
            ("B1=0", "B1=1"),
            ("C2=", "C2=unknown"),
            ("B2=\n", ""),
            ("A1=1", "A1=1\nA1=1"),
            ("DOCTYPE=NH", "DOCTYPE=CAS"),
            ("DOCTYPE=NH", "DOCTYPE=NHX"),
        ] {
            assert_eq!(
                detect_format(text.replace(from, to).as_bytes()),
                None,
                "{to}"
            );
        }
        assert_eq!(detect_format(&[CAA, b"X=extra\n"].concat()), None);
        assert_eq!(detect_format(&[CAA, &[0xff]].concat()), None);
        let long = text.replace("A2=QQ==", &format!("A2={}", "A".repeat(1024)));
        assert_eq!(detect_format(long.as_bytes()), None);
    }

    #[test]
    fn caa_detection_is_chunked_cancellable_and_bounded() {
        let limits = Limits {
            io_chunk_bytes: 3,
            ..Limits::default()
        };
        let mut source = Source::new(CAA.to_vec());
        assert_eq!(
            detect(&mut source, &limits).unwrap(),
            detection(InputFormat::Caa, 0, CAA.len() as u64)
        );
        assert!(source.reads.iter().all(|(_, len)| *len <= 3));
        assert_eq!(
            source.reads.iter().map(|(_, len)| len).sum::<usize>(),
            CAA.len()
        );
        let error = detect_source(&mut source, &limits, &CancelAfter::new(2)).unwrap_err();
        assert!(matches!(error.kind, ErrorKind::Cancelled));

        let mut source = Source::new([b"[TARGET]\n".as_slice(), &[b'A'; 2048]].concat());
        assert_eq!(detect(&mut source, &limits).unwrap(), None);
        assert_eq!(
            source.reads.iter().map(|(_, len)| len).sum::<usize>(),
            PDF_HEADER_SEARCH_BYTES
        );
    }

    #[test]
    fn caa_inspection_has_unknown_counts_and_conversion_writes_nothing() {
        let limits = Limits::default();
        let mut source = CAA;
        let info = super::inspect(
            &mut source,
            &super::InspectOptions::default(),
            &limits,
            &mut crate::NeverCancel,
        )
        .unwrap();
        assert_eq!(info.format, InputFormat::Caa);
        assert_eq!(info.page_count, None);
        assert_eq!(info.bookmark_count, None);
        assert_eq!(info.has_outline, None);
        assert_eq!(info.input_bytes_read, CAA.len() as u64);
        assert!(!info.format.is_convertible());
        let mut output = Vec::new();
        let error = super::convert(
            &mut source,
            &mut output,
            super::ConversionOptions::default(),
            &limits,
            &mut crate::NeverCancel,
        )
        .unwrap_err();
        assert!(matches!(error.kind, ErrorKind::UnsupportedFormat));
        assert!(output.is_empty());
    }
}
