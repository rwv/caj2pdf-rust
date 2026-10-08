// SPDX-License-Identifier: MIT

//! Original synthetic framing controls; no external document bytes.

use super::*;
use crate::test_support::{CancelAfter, NEVER};
use std::io::Write;

const EPILOGUE: &[u8] = b"xref\r0 1\r0000000000 65535 f \r\n3 1\r0000000001 00000 n \r\ntrailer\r<< /Size 4 /Root 3 0 R /Info 1 0 R /Prev 1 /ID [<0123456789abcdef0123456789abcdef><fedcba9876543210fedcba9876543210>] >>\rstartxref\r1\r%%EOF\r";

#[derive(Clone)]
struct Fixture {
    bytes: Vec<u8>,
    body: u64,
}

fn image_header(number: u32, length: u32) -> String {
    format!(
        "{number} 0 obj << /Type /XObject /Subtype /Image /Name /Im1 /Width 32 /Height 16 /BitsPerComponent 8 /Filter /FlateDecode /ColorSpace /DeviceRGB /Length {length} 0 R >> stream\n"
    )
}

fn fixture(copy_length: usize) -> Fixture {
    let table = 144 + copy_length + 32;
    let body = table + 12;
    let mut bytes = vec![b'p'; body];
    bytes[..4].copy_from_slice(b"CAJ\0");
    bytes[20..24].copy_from_slice(&(table as u32).to_le_bytes());
    bytes[table..table + 4].copy_from_slice(&(body as u32).to_le_bytes());
    let mut pixels: Vec<u8> = (0..1536).map(|i| (i % 251) as u8).collect();
    // A stored zlib block retains false object/stream markers inside data.
    let false_markers = b"\nendstream\nendobj\n99 0 obj null endobj\n";
    pixels[80..80 + false_markers.len()].copy_from_slice(false_markers);
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::none());
    encoder.write_all(&pixels).unwrap();
    let encoded = encoder.finish().unwrap();
    bytes.extend_from_slice(b"1 0 obj << /Length 2 0 R >> stream\nfirst\nendstream\nendobj\n");
    bytes.extend_from_slice(format!("4 0 obj {} endobj\n", encoded.len()).as_bytes());
    bytes.extend_from_slice(image_header(7, 4).as_bytes());
    bytes.extend_from_slice(&encoded[..300]);
    bytes.extend_from_slice(
        b"\r\n2 0 obj 5 endobj\n8 0 obj<\r\n3 0 obj << /Private true >> endobj\n",
    );
    bytes.extend_from_slice(b"11 0 obj << /Length 12 0 R >> stream\nother\nendstream\nendobj\n");
    bytes.extend_from_slice(EPILOGUE);
    bytes.extend((0..copy_length).map(|i| b'p' ^ b"FZHMEI"[(i + 2) % 6]));
    bytes.extend_from_slice(b"\r\n12 0 obj 5 endobj\n");
    bytes.extend_from_slice(image_header(9, 10).as_bytes());
    bytes.extend_from_slice(&encoded);
    bytes.extend_from_slice(b"\nendstream\nendobj\n");
    bytes.extend_from_slice(format!("10 0 obj {} endobj", encoded.len()).as_bytes());
    Fixture {
        bytes,
        body: body as u64,
    }
}

fn find(bytes: &[u8], needle: &[u8]) -> usize {
    bytes
        .windows(needle.len())
        .position(|v| v == needle)
        .unwrap()
}

impl Fixture {
    fn replace(&mut self, from: &[u8], to: &[u8]) {
        let at = find(&self.bytes, from);
        self.bytes.splice(at..at + from.len(), to.iter().copied());
    }

    fn candidate(&self) -> FragmentCandidate {
        let start = find(&self.bytes, b"9 0 obj <<");
        let end = find(&self.bytes[start..], b"\n10 0 obj") + start;
        FragmentCandidate {
            object: FragmentObject {
                reference: PdfRef {
                    number: 9,
                    generation: 0,
                },
                range: PdfRange {
                    offset: start as u64,
                    length: end as u64 - start as u64,
                },
            },
            used: false,
        }
    }

    fn scan(&self, limits: &Limits, cancellation: &impl Cancellation) -> Result<FragmentScan> {
        let mut source = ShortReads {
            bytes: self.bytes.clone(),
            maximum: 7,
        };
        self.scan_source(&mut source, limits, cancellation)
    }

    fn scan_source(
        &self,
        source: &mut impl RangedSource,
        limits: &Limits,
        cancellation: &impl Cancellation,
    ) -> Result<FragmentScan> {
        scan_fragment_with_candidates(
            source,
            self.body,
            self.bytes.len() as u64,
            limits,
            cancellation,
            &mut [self.candidate()],
        )
    }
}

struct ShortReads {
    bytes: Vec<u8>,
    maximum: usize,
}

impl RangedSource for ShortReads {
    fn size(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
        let at = offset as usize;
        let n = destination
            .len()
            .min(self.maximum)
            .min(self.bytes.len().saturating_sub(at));
        destination[..n].copy_from_slice(&self.bytes[at..at + n]);
        Ok(n)
    }
}

#[test]
fn redundant_metadata_preserves_all_complete_objects_and_opaque_payloads() {
    for copy in [128, 300, 5000, 65535] {
        let f = fixture(copy);
        for chunk in [1, 256, 4096] {
            let limits = Limits {
                io_chunk_bytes: chunk,
                ..Limits::default()
            };
            let scan = f.scan(&limits, &NEVER).unwrap();
            assert_eq!(
                scan.objects
                    .iter()
                    .map(|o| o.object.reference.number)
                    .collect::<Vec<_>>(),
                [1, 4, 2, 3, 11, 12, 9, 10]
            );
            assert!(
                scan.patches.is_empty() && scan.damaged.is_empty() && scan.source_paths.is_empty()
            );
            let image = scan
                .objects
                .iter()
                .find(|o| o.object.reference.number == 9)
                .unwrap();
            assert_eq!(image.object, f.candidate().object);
            assert!(!scan.objects.iter().any(|o| o.object.reference.number == 99));
        }
    }
}

#[test]
fn redundant_metadata_requires_a_complete_unreferenced_graph() {
    for extra in [
        "20 0 obj 7 0 R endobj",
        "20 0 obj << /Nested [<< /Use 8 0 R >>] >> endobj",
        "20 0 obj << /Length 0 /Private 7 0 R >> stream\nendstream\nendobj",
        "8 0 obj null endobj",
        "7 0 obj null endobj",
        "20 0 obj << /Type /Page >> endobj",
        "20 0 obj << /Key 1 /Key 2 >> endobj",
        "20 0 obj << /Type /ObjStm /Length 5 >> stream\n7 0 R\nendstream\nendobj",
        "20 0 obj << /Type /XRef /Length 0 >> stream\nendstream\nendobj",
        "20 0 obj << /Type 3 0 R /Length 0 >> stream\nendstream\nendobj",
    ] {
        let mut f = fixture(300);
        f.bytes.extend_from_slice(format!("\n{extra}").as_bytes());
        assert!(f.scan(&Limits::default(), &NEVER).is_err(), "{extra}");
    }
    let mut f = fixture(300);
    f.bytes
        .extend_from_slice(b"\n20 0 obj << /String (7 0 R and 8 0 R) /Hex <3720302052> >> endobj");
    assert!(f.scan(&Limits::default(), &NEVER).is_ok());
}

#[test]
fn redundant_metadata_rejects_unmeasured_or_ambiguous_neighbors() {
    for (from, to) in [
        ("8 0 obj<\r\n", "8 1 obj<\r\n"),
        ("8 0 obj<\r\n", "8 0 obj<A\r\n"),
        ("8 0 obj<\r\n", "8 0 obj<\n"),
        ("3 0 obj", "3 1 obj"),
        ("7 0 obj", "7 1 obj"),
        ("/Width 32", "/Width 31"),
        ("/Height 16", "/Height 0"),
        ("/ColorSpace /DeviceRGB", "/ColorSpace /DeviceCMYK"),
        ("/Filter /FlateDecode", "/Filter /DCTDecode"),
        ("/BitsPerComponent 8", "/BitsPerComponent 1"),
        ("/Name /Im1", "/Extra true /Name /Im1"),
        ("/Length 4 0 R", "/Length 4 1 R"),
        ("\r\n2 0 obj 5", "\r\n2 0 obj 6"),
        ("\r\n2 0 obj 5", "\r\n2 1 obj 5"),
        ("\r\n2 0 obj 5", "\r\n2 0 obj null"),
        ("\r\n2 0 obj", "\n2 0 obj"),
        ("\r\n12 0 obj 5", "\r\n12 0 obj 6"),
        ("\r\n12 0 obj 5", "\r\n12 1 obj 5"),
        ("/Prev 1", "/Encrypt 1"),
        ("/Prev 1", "/Prev 0"),
        ("/Root 3 0 R", "/Root 3 1 R"),
        ("/Size 4", "/Size 5"),
        ("xref\r0 1", "xref\n0 1"),
        ("xref\r0 1", "xref\r0 0"),
        ("xref\r0 1", "xref\r1 1"),
        ("xref\r0 1", "xref\r0 18446744073709551615"),
        ("\n3 1\r", "\n18446744073709551615 1\r"),
        ("\n3 1\r", "\n3 18446744073709551615\r"),
        ("\n3 1\r", "\n0 1\r"),
        ("\n3 1\r", "\n3 129\r"),
        ("0000000001 00000 n", "0000000001 00002 n"),
        ("0000000001 00000 n", "0000000001 00000 f"),
        ("0000000000 65535 f", "0000000000 00000 n"),
        ("/ID [<0123456789abcdef", "/ID [<0123456789abcdeg"),
        ("/ID [<0123456789abcdef", "/ID [<0123456789abcde"),
        ("/Prev 1", "/Prev 1 /Prev 1"),
        ("startxref\r1", "startxref\r0"),
        ("%%EOF\r", "%%EOF\n"),
    ] {
        let mut f = fixture(300);
        f.replace(from.as_bytes(), to.as_bytes());
        assert!(
            f.scan(&Limits::default(), &NEVER).is_err(),
            "{from:?} -> {to:?}"
        );
    }
    for copy in [0, 127, 65536] {
        assert!(fixture(copy).scan(&Limits::default(), &NEVER).is_err());
    }
    for offset in [0, 143, 144, 160] {
        let mut f = fixture(300);
        f.bytes[offset] ^= 1;
        if offset == 143 {
            assert!(f.scan(&Limits::default(), &NEVER).is_ok());
        } else {
            assert!(f.scan(&Limits::default(), &NEVER).is_err());
        }
    }
    // Candidate discovery does not substitute for a complete forward parse.
    let f = fixture(300);
    let mut source = ShortReads {
        bytes: f.bytes.clone(),
        maximum: 7,
    };
    let error = scan_fragment_with_candidates(
        &mut source,
        f.body,
        f.bytes.len() as u64,
        &Limits::default(),
        &NEVER,
        &mut [f.candidate(), f.candidate()],
    );
    assert!(error.is_err());
    let mut hidden = fixture(300);
    let at = hidden.candidate().object.range.offset as usize;
    let payload = hidden.bytes.split_off(at);
    hidden
        .bytes
        .extend_from_slice(format!("30 0 obj << /Length {} >> stream\n", payload.len()).as_bytes());
    hidden.bytes.extend_from_slice(&payload);
    hidden.bytes.extend_from_slice(b"\nendstream\nendobj");
    let error = hidden.scan(&Limits::default(), &NEVER).err().unwrap();
    assert_eq!(
        error.reason,
        "recovery candidate is not a complete fragment object"
    );
    let mut f = fixture(300);
    for _ in 0..63 {
        f.bytes
            .extend_from_slice(b"\n81 0 obj<\r\n82 0 obj null endobj");
    }
    assert!(matches!(
        f.scan(&Limits::default(), &NEVER).err().unwrap().kind,
        ErrorKind::LimitExceeded {
            resource: "CAJ unused interruptions",
            limit: 64,
            attempted: 65
        }
    ));
}

#[test]
fn changed_recovery_framing_is_never_silently_omitted() {
    struct Changing {
        bytes: Vec<u8>,
        trigger: usize,
        change: usize,
        fired: bool,
    }
    impl RangedSource for Changing {
        fn size(&self) -> u64 {
            self.bytes.len() as u64
        }
        fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
            let at = offset as usize;
            if !self.fired && at == self.trigger {
                self.bytes[self.change] ^= 1;
                self.fired = true;
            }
            let n = destination.len().min(self.bytes.len().saturating_sub(at));
            destination[..n].copy_from_slice(&self.bytes[at..at + n]);
            Ok(n)
        }
    }
    let f = fixture(6000);
    let xref = find(&f.bytes, b"xref\r");
    let copy = xref + EPILOGUE.len();
    let image = find(&f.bytes, b"9 0 obj <<");
    let payload = image + image_header(9, 10).len();
    let orphan = find(&f.bytes, b"7 0 obj <<");
    let open = find(&f.bytes, b"8 0 obj<");
    let after_open = find(&f.bytes, b"3 0 obj <<");
    let after_image_prefix = find(&f.bytes, b"2 0 obj 5");
    for (trigger, change) in [
        (144, xref + 10),
        (144, 20),
        (144, f.body as usize - 12),
        (144 + 5120, copy + 4500),
        (payload, orphan + 20),
        (after_image_prefix, orphan + image_header(7, 4).len() + 200),
        (after_open, open + 7),
    ] {
        let mut source = Changing {
            bytes: f.bytes.clone(),
            trigger,
            change,
            fired: false,
        };
        let result = f.scan_source(&mut source, &Limits::default(), &NEVER);
        assert!(source.fired, "trigger {trigger} did not run");
        assert!(
            result.is_err(),
            "accepted change at {change}, triggered at {trigger}"
        );
    }
}

#[test]
fn redundant_metadata_propagates_every_cancellation_and_resource_failure() {
    let f = fixture(300);
    let limits = Limits::default();
    let checkpoints = CancelAfter::never();
    f.scan(&limits, &checkpoints).unwrap();
    assert!(checkpoints.queries() > 100);
    for allowed in 0..checkpoints.queries() {
        let error = f
            .scan(&limits, &CancelAfter::new(allowed))
            .err()
            .expect("cancelled scan passed");
        assert!(
            matches!(error.kind, ErrorKind::Cancelled),
            "checkpoint {allowed}: {error:?}"
        );
    }
    for limits in [
        Limits {
            max_input_bytes: 100,
            ..Limits::default()
        },
        Limits {
            max_allocation_bytes: 128,
            ..Limits::default()
        },
        Limits {
            io_chunk_bytes: 0,
            ..Limits::default()
        },
    ] {
        assert!(f.scan(&limits, &NEVER).is_err());
    }
    let at = find(&f.bytes, b"xref\r") as u64;
    let mut source = super::tests::UnreadableTail {
        bytes: f.bytes.clone(),
        unreadable_from: at + 20,
    };
    assert!(matches!(
        f.scan_source(&mut source, &limits, &NEVER)
            .err()
            .unwrap()
            .kind,
        ErrorKind::Io(_)
    ));
}
