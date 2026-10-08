// SPDX-License-Identifier: MIT

//! Original synthetic pixels, zlib frames and CAJ metadata; no corpus bytes.

use super::*;
use crate::ConversionOptions;
use crate::caj::{convert_caj, parse_metadata};
use crate::pdf::input::fragment_scan::scan_fragment_with_candidates;
use crate::test_support::{CancelAfter, NEVER};
use std::io::Write;

const TABLE: usize = 0x400;
const BODY: usize = TABLE + 24;

#[derive(Clone)]
struct Source {
    bytes: Vec<u8>,
    maximum: usize,
    largest: usize,
    read: u64,
}

impl Source {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            maximum: 7,
            largest: 0,
            read: 0,
        }
    }
}

impl RangedSource for Source {
    fn size(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn read_at(&mut self, offset: u64, target: &mut [u8]) -> Result<usize> {
        self.largest = self.largest.max(target.len());
        let at = offset as usize;
        let n = target
            .len()
            .min(self.maximum)
            .min(self.bytes.len().saturating_sub(at));
        target[..n].copy_from_slice(&self.bytes[at..at + n]);
        self.read += n as u64;
        Ok(n)
    }
}

fn field(bytes: &mut [u8], at: usize, value: usize) {
    bytes[at..at + 4].copy_from_slice(&(value as u32).to_le_bytes());
}

fn zlib(pixels: &[u8], level: flate2::Compression) -> Vec<u8> {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), level);
    encoder.write_all(pixels).unwrap();
    encoder.finish().unwrap()
}

struct Fixture {
    clean: Vec<u8>,
    damaged: Vec<u8>,
    encoded: Vec<u8>,
    data_at: usize,
}

fn fixture(pixels: &[u8]) -> Fixture {
    let encoded = zlib(pixels, flate2::Compression::none());
    let mut bytes = vec![0; BODY];
    bytes[..4].copy_from_slice(b"CAJ\0");
    // A legitimate identical sequence in container metadata must not change.
    bytes[128..132].copy_from_slice(&EXPANDED);
    field(&mut bytes, 16, 2);
    field(&mut bytes, 20, TABLE);
    bytes.extend_from_slice(b"7 0 obj << /Type /Page /Parent 99 0 R /MediaBox [0 0 32 32] /Resources << /XObject << /Im1 8 0 R >> >> /Contents 10 0 R >> endobj\n");
    bytes.extend_from_slice(format!("8 0 obj << /Type /XObject /Subtype /Image /Width {} /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceGray /Filter /FlateDecode /Length {} >> stream\n", pixels.len(), encoded.len()+1).as_bytes());
    let data_at = bytes.len();
    bytes.extend_from_slice(&encoded);
    bytes.extend_from_slice(b"\n\r\nendstream\nendobj\n");
    let content = b"q 30 0 0 30 1 1 cm /Im1 Do Q";
    bytes
        .extend_from_slice(format!("10 0 obj << /Length {} >> stream\n", content.len()).as_bytes());
    bytes.extend_from_slice(content);
    bytes.extend_from_slice(b"\nendstream\nendobj\n");
    let second = bytes.len();
    bytes.extend_from_slice(
        b"9 0 obj << /Type /Page /Parent 99 0 R /MediaBox [0 0 32 32] /Resources << >> >> endobj\n",
    );
    let end = bytes.len();
    for (i, (at, size, id)) in [(BODY, second - BODY, 7), (second, end - second, 9)]
        .into_iter()
        .enumerate()
    {
        field(&mut bytes, TABLE + i * 12, at);
        field(&mut bytes, TABLE + i * 12 + 4, size);
        field(&mut bytes, TABLE + i * 12 + 8, id);
    }
    let mut damaged = bytes[..data_at].to_vec();
    let mut at = 0;
    while at < encoded.len() {
        if encoded.get(at..at + 2) == Some(&ORIGINAL) {
            damaged.extend_from_slice(&EXPANDED);
            at += 2;
        } else {
            damaged.push(encoded[at]);
            at += 1;
        }
    }
    damaged.extend_from_slice(&bytes[data_at + encoded.len()..]);
    Fixture {
        clean: bytes,
        damaged,
        encoded,
        data_at,
    }
}

fn scan(
    source: &mut Source,
    limits: &Limits,
    cancel: &impl Cancellation,
) -> Result<(CajMetadata, FragmentScan)> {
    let metadata = parse_metadata(source, limits, cancel)?;
    let scan = scan_fragment_with_candidates(
        source,
        metadata.body_start,
        metadata.body_end_hint,
        limits,
        cancel,
        &mut [],
    )?;
    Ok((metadata, scan))
}

fn convert(bytes: Vec<u8>, limits: &Limits) -> Vec<u8> {
    let mut source = Source::new(bytes);
    let mut output = Vec::new();
    let report = convert_caj(
        &mut source,
        &mut output,
        &ConversionOptions::default(),
        limits,
        &NEVER,
    )
    .unwrap();
    assert_eq!(report.pages_converted, 2);
    assert_eq!(report.input_bytes_read, source.read);
    assert!(source.largest <= limits.io_chunk_bytes);
    output
}

#[test]
fn source_redundancy_recovers_pixels_with_bounded_short_reads() {
    for pixels in [ORIGINAL.to_vec(), [ORIGINAL, ORIGINAL, ORIGINAL].concat()] {
        let f = fixture(&pixels);
        for chunk in [1, 7, 256, 4096] {
            let limits = Limits {
                io_chunk_bytes: chunk,
                ..Limits::default()
            };
            let expected = convert(f.clean.clone(), &limits);
            assert_eq!(convert(f.damaged.clone(), &limits), expected);
            let mut source = Source::new(f.damaged.clone());
            let (metadata, mut scan) = scan(&mut source, &limits, &NEVER).unwrap();
            assert_eq!(scan.substitutions.len(), 1);
            let plan = Plan::from_scan(&mut scan, &metadata, &limits)
                .unwrap()
                .unwrap();
            let mut view = plan.source(&mut source, &limits, &NEVER).unwrap();
            let mut restored = vec![0; view.size() as usize];
            for (i, bytes) in restored.chunks_mut(chunk).enumerate() {
                read_exact_at(&mut view, (i * chunk) as u64, bytes, &limits, &NEVER).unwrap();
            }
            assert_eq!(restored, f.clean);
        }
    }
}

#[test]
fn substitution_recognition_crosses_read_windows() {
    let mut pixels = vec![19; WINDOW + 32];
    // Stored DEFLATE starts its pixels at encoded offset 7; the marker
    // begins two bytes before the recognition window ends.
    pixels[WINDOW - 9..WINDOW - 7].copy_from_slice(&ORIGINAL);
    let f = fixture(&pixels);
    assert_eq!(
        convert(f.damaged, &Limits::default()),
        convert(f.clean, &Limits::default())
    );
}

#[test]
fn wrong_candidate_checksum_and_ambiguous_marker_counts_do_not_rewrite() {
    let f = fixture(&ORIGINAL);
    let mut wrong = f.damaged.clone();
    // The candidate must preserve, not replace, the stored Adler checksum.
    wrong[f.data_at + f.encoded.len() + 1] ^= 1;
    let ambiguous = fixture(&[ORIGINAL.as_slice(), EXPANDED.as_slice()].concat());
    for bytes in [wrong, ambiguous.damaged] {
        let mut source = Source::new(bytes.clone());
        let (_, scan) = scan(&mut source, &Limits::default(), &NEVER).unwrap();
        assert!(scan.substitutions.is_empty());
        let output = convert(bytes, &Limits::default());
        assert!(output.windows(4).any(|w| w == EXPANDED));
    }
}

#[test]
fn correct_codec_with_an_understated_length_is_preserved() {
    let mut f = fixture(&EXPANDED);
    let old = format!("/Length {}", f.encoded.len() + 1);
    let new = format!("/Length {}", f.encoded.len() - 1);
    assert_eq!(old.len(), new.len());
    let at = f
        .clean
        .windows(old.len())
        .position(|w| w == old.as_bytes())
        .unwrap();
    f.clean[at..at + old.len()].copy_from_slice(new.as_bytes());
    let mut source = Source::new(f.clean.clone());
    let (_, scan) = scan(&mut source, &Limits::default(), &NEVER).unwrap();
    assert!(scan.substitutions.is_empty());
    assert!(
        convert(f.clean, &Limits::default())
            .windows(f.encoded.len())
            .any(|w| w == f.encoded)
    );
}

#[test]
fn all_original_page_anchors_must_agree() {
    let f = fixture(&ORIGINAL);
    let mut source = Source::new(f.damaged.clone());
    let (mut metadata, mut scanned) = scan(&mut source, &Limits::default(), &NEVER).unwrap();
    assert_eq!(scanned.substitutions.len(), 1);
    metadata.page_rows[1].offset += 2;
    assert!(
        Plan::from_scan(&mut scanned, &metadata, &Limits::default())
            .unwrap()
            .is_none()
    );
    let mut bytes = f.damaged;
    let size = u32::from_le_bytes(bytes[TABLE + 4..TABLE + 8].try_into().unwrap()) as usize;
    let second = u32::from_le_bytes(bytes[TABLE + 12..TABLE + 16].try_into().unwrap()) as usize;
    field(&mut bytes, TABLE + 4, size + 2);
    field(&mut bytes, TABLE + 12, second + 2);
    assert!(
        convert(bytes, &Limits::default())
            .windows(4)
            .any(|w| w == EXPANDED)
    );
}

#[test]
fn checksums_mapping_and_rechecks_preserve_cancellation_and_limits() {
    let f = fixture(&ORIGINAL);
    let limits = Limits {
        io_chunk_bytes: 256,
        ..Limits::default()
    };
    let signal = CancelAfter::never();
    let mut source = Source::new(f.damaged.clone());
    let (metadata, mut scanned) = scan(&mut source, &limits, &signal).unwrap();
    let plan = Plan::from_scan(&mut scanned, &metadata, &limits)
        .unwrap()
        .unwrap();
    plan.verify(&mut source, &limits, &signal).unwrap();
    for allowed in 0..signal.queries() {
        let result = (|| {
            let cancel = CancelAfter::new(allowed);
            let mut source = Source::new(f.damaged.clone());
            let (metadata, mut scanned) = scan(&mut source, &limits, &cancel)?;
            let plan = Plan::from_scan(&mut scanned, &metadata, &limits)?.unwrap();
            plan.verify(&mut source, &limits, &cancel)
        })();
        assert!(
            matches!(
                result,
                Err(Error {
                    kind: ErrorKind::Cancelled,
                    ..
                })
            ),
            "checkpoint {allowed}"
        );
    }
    source.bytes[f.data_at + 3] ^= 1;
    assert!(matches!(
        plan.verify(&mut source, &limits, &NEVER),
        Err(Error {
            kind: ErrorKind::Malformed,
            ..
        })
    ));
    let tight = Limits {
        max_allocation_bytes: 32768,
        ..limits
    };
    assert!(matches!(
        scan(&mut Source::new(f.damaged), &tight, &NEVER),
        Err(Error {
            kind: ErrorKind::LimitExceeded { .. },
            ..
        })
    ));
}

#[test]
fn inflate_refuses_trailing_truncation_and_excess_work() {
    let limits = Limits::default();
    let compressed = zlib(b"original control", flate2::Compression::fast());
    let mut source = Source::new(compressed.clone());
    assert!(matches!(
        inflate(
            &mut source,
            PdfRange {
                offset: 0,
                length: compressed.len() as u64 - 1
            },
            &limits,
            &NEVER
        )
        .unwrap(),
        Codec::Invalid
    ));
    source.bytes.extend_from_slice(b"junk");
    let length = source.size();
    assert!(
        matches!(inflate(&mut source,PdfRange{offset:0,length},&limits,&NEVER).unwrap(),Codec::Valid(n) if n==compressed.len() as u64)
    );
    let compressed = zlib(
        &vec![0; MAX_DECODED as usize + 1],
        flate2::Compression::fast(),
    );
    let length = compressed.len() as u64;
    assert!(matches!(
        inflate(
            &mut Source::new(compressed),
            PdfRange { offset: 0, length },
            &limits,
            &NEVER
        )
        .unwrap(),
        Codec::OutsideProfile
    ));
}

#[test]
fn filter_parameters_and_nonzero_generations_never_enable_substitution() {
    let f = fixture(&ORIGINAL);
    for replacement in [
        b"/Filter [/FlateDecode]".as_slice(),
        b"/Filter /FlateDecode /DecodeParms null",
        b"/Filter /FlateDecode /F (external)",
        b"/Filter /FlateDecode /FFilter /FlateDecode",
        b"/Filter /FlateDecode /FDecodeParms null",
        b"/Filter /ASCIIHexDecode",
    ] {
        let mut bytes = f.damaged.clone();
        let needle = b"/Filter /FlateDecode";
        let at = bytes
            .windows(needle.len())
            .position(|w| w == needle)
            .unwrap();
        bytes.splice(at..at + needle.len(), replacement.iter().copied());
        let (_, scanned) = scan(&mut Source::new(bytes), &Limits::default(), &NEVER).unwrap();
        assert!(scanned.substitutions.is_empty(), "{replacement:?}");
    }
    let mut bytes = f.damaged;
    let at = bytes.windows(7).position(|w| w == b"8 0 obj").unwrap();
    bytes[at + 2] = b'1';
    assert!(matches!(
        scan(&mut Source::new(bytes), &Limits::default(), &NEVER),
        Err(Error {
            kind: ErrorKind::UnsupportedFormat,
            ..
        })
    ));
}

#[test]
fn missing_lf_and_incomplete_candidates_remain_opaque() {
    let f = fixture(&[ORIGINAL, ORIGINAL].concat());
    let mut missing_lf = f.damaged.clone();
    missing_lf[f.data_at + f.encoded.len() + 4] = b' ';
    let mut incomplete = f.damaged;
    let at = incomplete[f.data_at..]
        .windows(4)
        .position(|w| w == EXPANDED)
        .unwrap();
    incomplete[f.data_at + at] ^= 1;
    for bytes in [missing_lf, incomplete] {
        let (_, scanned) = scan(&mut Source::new(bytes), &Limits::default(), &NEVER).unwrap();
        assert!(scanned.substitutions.is_empty());
    }
}

#[test]
fn large_encoded_frames_are_not_probed() {
    let mut pixels = vec![17; MAX_ENCODED as usize];
    pixels[..2].copy_from_slice(&ORIGINAL);
    let f = fixture(&pixels);
    let mut source = Source::new(f.damaged);
    source.maximum = WINDOW;
    let (_, scanned) = scan(&mut source, &Limits::default(), &NEVER).unwrap();
    assert!(scanned.substitutions.is_empty());
}

#[test]
fn unresolved_or_unselected_objects_prevent_a_global_rewrite() {
    let f = fixture(&ORIGINAL);
    let limits = Limits::default();
    for mode in 0..6 {
        let (mut metadata, mut scanned) =
            scan(&mut Source::new(f.damaged.clone()), &limits, &NEVER).unwrap();
        match mode {
            0 => scanned.damaged.push((None, BODY as u64)),
            1 => scanned.objects[0].inspection = Err(Error::malformed(BODY as u64, "control")),
            2 => scanned.objects.retain(|o| o.object.reference.number != 8),
            3 => metadata.page_rows.pop().map(|_| ()).unwrap(),
            4 => metadata.page_rows[1].page_object_id = 50,
            5 => {
                scanned
                    .objects
                    .iter_mut()
                    .find(|o| o.object.reference.number == 9)
                    .unwrap()
                    .object
                    .reference
                    .generation = 1
            }
            _ => unreachable!(),
        }
        assert!(
            Plan::from_scan(&mut scanned, &metadata, &limits)
                .unwrap()
                .is_none(),
            "mode {mode}"
        );
    }
}

#[test]
fn sparse_view_handles_every_split_and_large_offsets() {
    let limits = Limits {
        io_chunk_bytes: 3,
        ..Limits::default()
    };
    let bytes = [b"a".as_slice(), &EXPANDED, &EXPANDED, b"xyz"].concat();
    let expected = [b"a".as_slice(), &ORIGINAL, &ORIGINAL, b"xyz"].concat();
    let map = sites(&[1, 5], &limits).unwrap();
    for at in 0..=expected.len() {
        for size in 0..=expected.len() - at {
            let mut source = Source::new(bytes.clone());
            let mut view = SubstitutedSource::new(&mut source, &map, &limits, &NEVER).unwrap();
            let mut actual = vec![0; size];
            for (i, chunk) in actual.chunks_mut(limits.io_chunk_bytes).enumerate() {
                read_exact_at(
                    &mut view,
                    (at + i * limits.io_chunk_bytes) as u64,
                    chunk,
                    &limits,
                    &NEVER,
                )
                .unwrap();
            }
            assert_eq!(actual, expected[at..at + size]);
            assert_eq!(view.read_at(u64::MAX, &mut [0; 1]).unwrap(), 0);
        }
    }
    struct Large;
    impl RangedSource for Large {
        fn size(&self) -> u64 {
            (1_u64 << 33) + 100
        }
        fn read_at(&mut self, at: u64, out: &mut [u8]) -> Result<usize> {
            for (i, byte) in out.iter_mut().enumerate() {
                *byte = (at + i as u64) as u8;
            }
            Ok(out.len())
        }
    }
    let map = sites(&[1], &limits).unwrap();
    let mut source = Large;
    let mut view = SubstitutedSource::new(&mut source, &map, &limits, &NEVER).unwrap();
    let offset = (1_u64 << 32) + 10;
    let mut actual = [0; 3];
    assert_eq!(view.read_at(offset, &mut actual).unwrap(), 3);
    assert_eq!(actual, [12, 13, 14]);
    assert_eq!(
        view.locate(Error::malformed(offset, "control")).offset,
        Some(offset + 2)
    );
}

#[test]
fn sparse_view_rejects_mutation_and_overread_and_preserves_source_errors() {
    let limits = Limits::default();
    let map = sites(&[1], &limits).unwrap();
    let bytes = [b"a".as_slice(), &EXPANDED, b"z"].concat();
    let mut source = Source::new(bytes);
    let mut view = SubstitutedSource::new(&mut source, &map, &limits, &NEVER).unwrap();
    view.source.bytes[2] ^= 1;
    let error = view.read_at(1, &mut [0; 2]).unwrap_err();
    assert_eq!(view.locate(error).offset, Some(1));
    view.source.bytes.push(0);
    assert!(view.read_at(0, &mut [0; 1]).is_err());
    struct Broken {
        overread: bool,
    }
    impl RangedSource for Broken {
        fn size(&self) -> u64 {
            100
        }
        fn read_at(&mut self, at: u64, out: &mut [u8]) -> Result<usize> {
            if self.overread {
                Ok(out.len() + 1)
            } else {
                Err(Error::truncated(at, out.len() as u64, 0))
            }
        }
    }
    for overread in [false, true] {
        for at in [1, 50] {
            let mut source = Broken { overread };
            let mut view = SubstitutedSource::new(&mut source, &map, &limits, &NEVER).unwrap();
            let error = view.read_at(at, &mut [0; 2]).unwrap_err();
            let error = view.locate(error);
            if overread {
                assert!(matches!(error.kind, ErrorKind::Malformed));
            } else {
                assert!(matches!(error.kind, ErrorKind::Truncated { .. }));
                assert_eq!(error.offset, Some(if at == 1 { 1 } else { 52 }));
            }
        }
    }
    let cancel = CancelAfter::new(0);
    let mut source = Broken { overread: true };
    let mut view = SubstitutedSource::new(&mut source, &map, &limits, &cancel).unwrap();
    assert!(matches!(
        view.read_at(0, &mut [0; 1]),
        Err(Error {
            kind: ErrorKind::Cancelled,
            ..
        })
    ));
}

#[test]
fn cancellation_is_observed_during_rescan_and_emission() {
    let f = fixture(&ORIGINAL);
    let limits = Limits {
        io_chunk_bytes: 256,
        ..Limits::default()
    };
    let signal = CancelAfter::never();
    convert_caj(
        &mut Source::new(f.damaged.clone()),
        &mut Vec::new(),
        &ConversionOptions::default(),
        &limits,
        &signal,
    )
    .unwrap();
    for allowed in 0..signal.queries() {
        let result = convert_caj(
            &mut Source::new(f.damaged.clone()),
            &mut Vec::new(),
            &ConversionOptions::default(),
            &limits,
            &CancelAfter::new(allowed),
        );
        assert!(
            matches!(
                result,
                Err(Error {
                    kind: ErrorKind::Cancelled,
                    ..
                })
            ),
            "checkpoint {allowed}"
        );
    }
}

#[test]
fn non_marker_mutation_during_output_is_reported() {
    use std::{cell::RefCell, rc::Rc};
    struct Shared(Rc<RefCell<Source>>);
    impl RangedSource for Shared {
        fn size(&self) -> u64 {
            self.0.borrow().size()
        }
        fn read_at(&mut self, at: u64, out: &mut [u8]) -> Result<usize> {
            self.0.borrow_mut().read_at(at, out)
        }
    }
    struct MutatingSink {
        source: Rc<RefCell<Source>>,
        at: usize,
        changed: bool,
    }
    impl Write for MutatingSink {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if !self.changed {
                self.source.borrow_mut().bytes[self.at] ^= 1;
                self.changed = true;
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let f = fixture(&ORIGINAL);
    let source = Rc::new(RefCell::new(Source::new(f.damaged)));
    let mut sink = MutatingSink {
        source: source.clone(),
        at: f.data_at + 3,
        changed: false,
    };
    let error = convert_caj(
        &mut Shared(source),
        &mut sink,
        &ConversionOptions::default(),
        &Limits::default(),
        &NEVER,
    )
    .unwrap_err();
    assert!(sink.changed);
    assert!(matches!(error.kind, ErrorKind::Malformed));
    assert_eq!(error.offset, Some(f.data_at as u64));
    assert_eq!(
        error.reason,
        "source stream changed after substitution validation"
    );
}

#[test]
fn without_a_moved_page_anchor_local_codec_evidence_is_insufficient() {
    let f = fixture(&ORIGINAL);
    let second = u32::from_le_bytes(f.clean[TABLE + 12..TABLE + 16].try_into().unwrap()) as usize;
    let mut bytes = f.damaged[..second + 2].to_vec();
    field(&mut bytes, 16, 1);
    let (metadata, mut scanned) =
        scan(&mut Source::new(bytes), &Limits::default(), &NEVER).unwrap();
    assert_eq!(scanned.substitutions.len(), 1);
    assert!(
        Plan::from_scan(&mut scanned, &metadata, &Limits::default())
            .unwrap()
            .is_none()
    );
}

#[test]
fn excessive_candidates_stop_at_the_document_budget() {
    let f = fixture(&ORIGINAL);
    let image_at = f.clean.windows(7).position(|w| w == b"8 0 obj").unwrap();
    let second = u32::from_le_bytes(f.clean[TABLE + 12..TABLE + 16].try_into().unwrap()) as usize;
    let end = f.clean[f.data_at..]
        .windows(6)
        .position(|w| w == b"endobj")
        .unwrap()
        + f.data_at
        + 6;
    let mut added = Vec::new();
    for id in 20..20 + MAX_CANDIDATES {
        added.extend_from_slice(format!("\n{id}").as_bytes());
        added.extend_from_slice(&f.damaged[image_at + 1..end + 2]);
    }
    let mut bytes = f.damaged;
    bytes.splice(second + 2..second + 2, added.iter().copied());
    field(&mut bytes, TABLE + 4, second - BODY + added.len());
    field(&mut bytes, TABLE + 12, second + added.len());
    let error = match scan(&mut Source::new(bytes), &Limits::default(), &NEVER) {
        Ok(_) => panic!("excessive candidate streams accepted"),
        Err(error) => error,
    };
    assert!(
        matches!(
            &error.kind,
            ErrorKind::LimitExceeded {
                resource: "CAJ stream substitution candidates",
                limit: 64,
                attempted: 65
            }
        ),
        "{error:?}"
    );
}
