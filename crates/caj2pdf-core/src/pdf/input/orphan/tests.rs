// SPDX-License-Identifier: MIT
//! Independently authored controls, without external document bytes.
use super::*;
use crate::pdf::input::fragment_scan::{FragmentScan, scan_fragment_with_candidates};
use crate::test_support::{CancelAfter, NEVER};
use crate::{ErrorKind, Limits};

const PAGE: &str = "<< /Type /Page /Parent 8 0 R /MediaBox [0 0 100 80] /CropBox [0 0 90 70] /Rotate 0 /Resources << >> /Contents 1 0 R >>";

fn fixture(opener: &str) -> Vec<u8> {
    format!("1 0 obj << /Length 2 0 R >> stream\nabc\nendstream\nendobj\n7 0 obj << /Length 1234 /Type /Metadata /Subtype /XML >> stream\r\n<?xpac\r\n2 0 obj 3 endobj\n{opener}\r\n3 0 obj {PAGE} endobj").into_bytes()
}

struct Source {
    bytes: Vec<u8>,
    reads: usize,
    maximum: usize,
    fail_at: usize,
    change: Option<(usize, usize)>,
    offset_reads: Vec<u64>,
}
impl Source {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            reads: 0,
            maximum: usize::MAX,
            fail_at: usize::MAX,
            change: None,
            offset_reads: Vec::new(),
        }
    }
    fn scan(&mut self, limits: &Limits, cancellation: &impl Cancellation) -> Result<FragmentScan> {
        let end = self.size();
        scan_fragment_with_candidates(self, 0, end, limits, cancellation, &mut [])
    }
}
impl RangedSource for Source {
    fn size(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
        self.offset_reads.push(offset);
        let call = self.reads;
        self.reads += 1;
        if call == self.fail_at {
            return Err(std::io::Error::other("injected ranged read failure").into());
        }
        if let Some((trigger, at)) = self.change
            && call == trigger
        {
            self.bytes[at] ^= 1;
        }
        let at = offset as usize;
        let count = destination
            .len()
            .min(self.maximum)
            .min(self.bytes.len().saturating_sub(at));
        destination[..count].copy_from_slice(&self.bytes[at..at + count]);
        Ok(count)
    }
}
fn replace(bytes: &[u8], from: &str, to: &str) -> Vec<u8> {
    String::from_utf8(bytes.to_vec())
        .unwrap()
        .replace(from, to)
        .into_bytes()
}
fn find(bytes: &[u8], token: &[u8]) -> usize {
    bytes.windows(token.len()).position(|b| b == token).unwrap()
}

#[test]
fn measured_interruptions_preserve_complete_objects_with_short_reads() {
    for opener in ["8 0", "8 0 obj<<"] {
        let bytes = fixture(opener);
        for maximum in [1, 7, 256, usize::MAX] {
            let mut source = Source::new(bytes.clone());
            source.maximum = maximum;
            let result = source.scan(&Limits::default(), &NEVER).unwrap();
            assert_eq!(
                result
                    .objects
                    .iter()
                    .map(|o| o.object.reference.number)
                    .collect::<Vec<_>>(),
                [1, 2, 3]
            );
            assert!(
                result.patches.is_empty()
                    && result.damaged.is_empty()
                    && result.substitutions.is_empty()
            );
            for scanned in result.objects {
                let a = scanned.object.range.offset as usize;
                let b = a + scanned.object.range.length as usize;
                assert!(bytes[a..b].ends_with(b"endobj"));
            }
            assert_eq!(source.bytes, bytes);
        }
    }
}

#[test]
fn metadata_requires_exact_unused_packet_and_previous_length_boundary() {
    let bytes = fixture("8 0");
    for (from, to) in [
        ("7 0 obj", "7 1 obj"),
        ("/Metadata", "/Other"),
        ("/XML", "/Other"),
        ("/Length 1234", "/Length 8"),
        ("/Length 1234", "/Length 2 0 R"),
        ("/Subtype /XML", "/Subtype /XML /Filter /FlateDecode"),
        ("<?xpac\r\n", "<?xpack\r\n"),
        ("<?xpac\r\n", "<?xpac\n"),
        ("<?xpac\r\n", "<?xpacXYZ\r\n"),
        ("2 0 obj 3", "2 0 obj 4"),
        ("2 0 obj 3", "2 1 obj 3"),
        ("2 0 obj 3", "2 0 obj null"),
        ("2 0 obj 3", "6 0 obj 3"),
        ("/Length 2 0 R", "/Length 3"),
        ("/Resources << >>", "/Resources << /Metadata 7 0 R >>"),
    ] {
        assert!(
            Source::new(replace(&bytes, from, to))
                .scan(&Limits::default(), &NEVER)
                .is_err(),
            "{from} -> {to}"
        );
    }
    for extra in [
        "7 0 obj null endobj",
        "6 0 obj [7 0 R] endobj",
        "6 0 obj << /Type /ObjStm /Length 0 >> stream\nendstream\nendobj",
        "6 0 obj << /Type /XRef /Length 0 >> stream\nendstream\nendobj",
        "6 0 obj << /Type 2 0 R /Length 0 >> stream\nendstream\nendobj",
        "6 0 obj << /Key 1 /Key 2 >> endobj",
    ] {
        let mut changed = bytes.clone();
        changed.extend_from_slice(format!("\n{extra}").as_bytes());
        assert!(
            Source::new(changed)
                .scan(&Limits::default(), &NEVER)
                .is_err(),
            "{extra}"
        );
    }
}

#[test]
fn parent_requires_payload_free_opener_and_explicit_leaf_properties() {
    let bytes = fixture("8 0 obj<<");
    for (from, to) in [
        ("8 0 obj<<\r\n", "8 1 obj<<\r\n"),
        ("8 0 obj<<\r\n", "8 0 obj<< /Rotate 90\r\n"),
        ("8 0 obj<<\r\n", "8 0 obj<<\n"),
        ("8 0 obj<<\r\n", "8 0 o\r\n"),
        ("8 0 obj<<\r\n", "8\r\n"),
        ("/Parent 8 0 R", "/Parent 9 0 R"),
        ("/MediaBox [0 0 100 80]", ""),
        ("/CropBox [0 0 90 70]", ""),
        ("/CropBox [0 0 90 70]", "/CropBox 2 0 R"),
        ("/Rotate 0", ""),
        ("/Rotate 0", "/Rotate 45"),
        ("/Resources << >>", ""),
        ("/Resources << >>", "/Resources null"),
        ("/Resources << >>", "/Resources << /Other 8 0 R >>"),
        ("/Contents 1 0 R", "/Contents 8 0 R"),
    ] {
        assert!(
            Source::new(replace(&bytes, from, to))
                .scan(&Limits::default(), &NEVER)
                .is_err(),
            "{from} -> {to}"
        );
    }
    for extra in [
        "6 0 obj [8 0 R] endobj",
        "6 0 obj << /Type /Pages /Parent 8 0 R /Count 0 /Kids [] >> endobj",
    ] {
        let mut changed = bytes.clone();
        changed.extend_from_slice(format!("\n{extra}").as_bytes());
        assert!(
            Source::new(changed)
                .scan(&Limits::default(), &NEVER)
                .is_err(),
            "{extra}"
        );
    }
    // A complete exact counterpart follows the existing replay proof instead.
    let mut complete = bytes.clone();
    complete.extend_from_slice(b"\n8 0 obj<</Type /Pages /Count 1 /Kids [3 0 R]>>endobj");
    assert!(
        Source::new(complete)
            .scan(&Limits::default(), &NEVER)
            .is_ok()
    );
}

#[test]
fn interruptions_propagate_every_read_failure_and_cancellation() {
    let bytes = fixture("8 0");
    let limits = Limits::default();
    let mut baseline = Source::new(bytes.clone());
    let checkpoints = CancelAfter::never();
    baseline.scan(&limits, &checkpoints).unwrap();
    for allowed in 0..checkpoints.queries() {
        let e = Source::new(bytes.clone())
            .scan(&limits, &CancelAfter::new(allowed))
            .err()
            .unwrap();
        assert!(matches!(e.kind, ErrorKind::Cancelled), "{allowed}: {e:?}");
    }
    for fail_at in 0..baseline.reads {
        let mut source = Source::new(bytes.clone());
        source.fail_at = fail_at;
        let e = source.scan(&limits, &NEVER).err().unwrap();
        assert!(matches!(e.kind, ErrorKind::Io(_)), "{fail_at}: {e:?}");
    }
    let mut zero = Source::new(bytes);
    zero.maximum = 0;
    assert!(zero.scan(&limits, &NEVER).is_err());
}

#[test]
fn interruptions_recheck_metadata_and_parent_source_bytes() {
    let bytes = fixture("8 0");
    let metadata = find(&bytes, b"7 0 obj");
    let integer = find(&bytes, b"2 0 obj");
    let parent = find(&bytes, b"8 0\r\n");
    let page = find(&bytes, b"3 0 obj");
    let crop = find(&bytes, b"/CropBox");
    let mut baseline = Source::new(bytes.clone());
    baseline.scan(&Limits::default(), &NEVER).unwrap();
    let next_integer_read = baseline
        .offset_reads
        .iter()
        .position(|at| *at == integer as u64)
        .unwrap();
    let parent_reads: Vec<_> = baseline
        .offset_reads
        .iter()
        .enumerate()
        .filter(|(_, at)| **at == parent as u64)
        .map(|(i, _)| i)
        .collect();
    let page_recheck = baseline
        .offset_reads
        .iter()
        .rposition(|at| *at == page as u64)
        .unwrap();
    for (trigger, change) in [
        (page_recheck, crop + 1),
        (next_integer_read, metadata + 10),
        (*parent_reads.last().unwrap(), parent),
    ] {
        let mut source = Source::new(bytes.clone());
        source.change = Some((trigger, change));
        assert!(
            source.scan(&Limits::default(), &NEVER).is_err(),
            "{trigger}: {change}"
        );
        assert_ne!(source.bytes, bytes);
    }
}

#[test]
fn parent_interruptions_have_count_and_allocation_bounds() {
    for count in [64, 65] {
        let mut bytes = Vec::new();
        for n in 0..count {
            let parent = n + 100;
            let page = n + 1000;
            let value = PAGE.replace("8 0 R", &format!("{parent} 0 R"));
            bytes.extend_from_slice(
                format!("{parent} 0\r\n{page} 0 obj {value} endobj\n").as_bytes(),
            );
        }
        let result = Source::new(bytes).scan(&Limits::default(), &NEVER);
        if count == 64 {
            assert!(result.is_ok());
        } else {
            assert!(matches!(
                result.err().unwrap().kind,
                ErrorKind::LimitExceeded { .. }
            ));
        }
    }
    let limits = Limits {
        io_chunk_bytes: 64,
        max_allocation_bytes: 64,
        ..Limits::default()
    };
    assert!(matches!(
        Source::new(fixture("8 0"))
            .scan(&limits, &NEVER)
            .err()
            .unwrap()
            .kind,
        ErrorKind::LimitExceeded { .. }
    ));
}

#[test]
fn metadata_header_bound_preserves_the_exact_boundary() {
    let bytes = fixture("8 0");
    let start = find(&bytes, b"7 0 obj");
    let data = find(&bytes, b"<?xpac");
    for length in [256, 257] {
        let padding = " ".repeat(length - (data - start));
        let padded = replace(&bytes, "7 0 obj", &format!("7 0 {padding}obj"));
        let result = Source::new(padded).scan(&Limits::default(), &NEVER);
        assert_eq!(result.is_ok(), length == 256, "{length}");
    }
}
