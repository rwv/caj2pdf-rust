// SPDX-License-Identifier: MIT

//! Original controls for constraints hidden by provisional stream extents.

use super::*;
use crate::test_support::{CancelAfter, NEVER};

struct Source {
    bytes: Vec<u8>,
    maximum: usize,
    starts: usize,
    mutate: bool,
}
impl Source {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            maximum: 7,
            starts: 0,
            mutate: false,
        }
    }
}
impl RangedSource for Source {
    fn size(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize> {
        let at = offset as usize;
        if at == 0 {
            self.starts += 1;
            // skip_space first fills its window at zero, then load_head reads
            // it again. The next read at zero belongs to the repeated pass.
            if self.mutate && self.starts == 3 {
                let needle = b"20 0 obj 6";
                let at = self
                    .bytes
                    .windows(needle.len())
                    .position(|b| b == needle)
                    .unwrap();
                self.bytes[at + needle.len() - 1] = b'8';
            }
        }
        let count = out
            .len()
            .min(self.maximum)
            .min(self.bytes.len().saturating_sub(at));
        out[..count].copy_from_slice(&self.bytes[at..at + count]);
        Ok(count)
    }
}
fn scan(
    source: &mut impl RangedSource,
    limits: &Limits,
    cancel: &impl Cancellation,
) -> Result<FragmentScan> {
    let end = source.size();
    scan_fragment_with_candidates(source, 0, end, limits, cancel, &mut [])
}
fn hidden_integer() -> Vec<u8> {
    // The first stream's Length 10 is swallowed by stream 2's provisional
    // extent. Resolving the available Length 20 must expose it on a new pass.
    b"1 0 obj<</Length 10 0 R>>stream\nabc\nendstream\nendobj\n\
2 0 obj<</Length 20 0 R>>stream\nxy\r\n\
10 0 obj 3 endobj\n5 0 obj null endobj\n\
2 0 obj<</Length 20 0 R>>stream\nxyZabc\nendstream\nendobj\n\
20 0 obj 6 endobj\n"
        .to_vec()
}
#[test]
fn known_constraints_expose_a_missing_earlier_length() {
    for maximum in [1, 7, 8192] {
        let mut source = Source::new(hidden_integer());
        source.maximum = maximum;
        let result = scan(&mut source, &Limits::default(), &NEVER).unwrap();
        assert!(result.patches.is_empty() && result.damaged.is_empty());
        assert_eq!(
            result
                .objects
                .iter()
                .map(|s| s.object.reference.number)
                .collect::<Vec<_>>(),
            [1, 10, 5, 2, 20]
        );
        let copy = &result.objects[3].object;
        assert_eq!(&source.bytes[copy.range.offset as usize..][..7], b"2 0 obj");
        assert!(copy.range.offset > 100);
    }
}

fn many_constraints(count: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut lengths = Vec::new();
    for n in 0..count {
        let stream = 3 * n + 1;
        let length = stream + 1;
        let other = stream + 2;
        let header = format!("{stream} 0 obj<</Length {length} 0 R>>stream\n");
        bytes.extend_from_slice(header.as_bytes());
        bytes.extend_from_slice(b"ab\r\n");
        bytes.extend_from_slice(
            format!("{other} 0 obj<</Length 1>>stream\nX\nendstream\nendobj\n").as_bytes(),
        );
        bytes.extend_from_slice(header.as_bytes());
        bytes.extend_from_slice(b"abcdef\nendstream\nendobj\n");
        lengths.extend_from_slice(format!("{length} 0 obj 6 endobj\n").as_bytes());
    }
    bytes.extend(lengths);
    bytes
}
#[test]
fn provisional_duplicates_resolve_in_one_batch_beyond_the_retry_count() {
    let count = 40;
    let mut source = Source::new(many_constraints(count));
    let result = scan(&mut source, &Limits::default(), &NEVER).unwrap();
    assert_eq!(result.objects.len(), count as usize * 3);
    assert!(result.patches.is_empty() && result.damaged.is_empty());
    for object in &result.objects {
        if object.object.reference.number % 3 == 1 {
            let range = object.object.range;
            assert!(
                source.bytes[range.offset as usize..(range.offset + range.length) as usize]
                    .windows(6)
                    .any(|b| b == b"abcdef")
            );
        }
    }
}
#[test]
fn every_final_length_and_complete_duplicate_remains_strict() {
    let original = String::from_utf8(hidden_integer()).unwrap();
    for changed in [
        original.replace("10 0 obj 3 endobj", "10 0 obj null endobj"),
        original.replace("20 0 obj 6 endobj", "20 0 obj 8 endobj"),
        original.replace("20 0 obj 6 endobj", "20 0 obj null endobj"),
        format!("{original}20 0 obj 7 endobj\n"),
        format!("{original}2 0 obj<</Length 20 0 R>>stream\nxyZabQ\nendstream\nendobj\n"),
    ] {
        assert!(
            scan(
                &mut Source::new(changed.into_bytes()),
                &Limits::default(),
                &NEVER
            )
            .is_err()
        );
    }
    let mut source = Source::new(hidden_integer());
    source.mutate = true;
    assert!(scan(&mut source, &Limits::default(), &NEVER).is_err());
    assert!(source.starts >= 3);
}
#[test]
fn length_batch_preserves_cancellation_io_and_metadata_limits() {
    let bytes = hidden_integer();
    let checkpoints = CancelAfter::new(u64::MAX);
    scan(
        &mut Source::new(bytes.clone()),
        &Limits::default(),
        &checkpoints,
    )
    .unwrap();
    for allowed in 0..checkpoints.queries() {
        let error = scan(
            &mut Source::new(bytes.clone()),
            &Limits::default(),
            &CancelAfter::new(allowed),
        )
        .err()
        .unwrap();
        assert!(matches!(error.kind, ErrorKind::Cancelled), "{error:?}");
    }
    let limits = Limits {
        io_chunk_bytes: 256,
        max_allocation_bytes: 4096,
        ..Limits::default()
    };
    // Each dictionary fits; the batch itself does not.
    let error = scan(&mut Source::new(many_constraints(257)), &limits, &NEVER)
        .err()
        .unwrap();
    assert!(
        matches!(
            error.kind,
            ErrorKind::LimitExceeded {
                attempted: 4112,
                ..
            }
        ),
        "{error:?}"
    );
    let unreadable_from = bytes.len() as u64 - 12;
    let mut source = super::tests::UnreadableTail {
        bytes,
        unreadable_from,
    };
    let error = scan(&mut source, &Limits::default(), &NEVER).err().unwrap();
    assert!(matches!(error.kind, ErrorKind::Io(_)), "{error:?}");
}
