// SPDX-License-Identifier: MIT
use super::*;
use crate::native::SeekableSource;
use crate::pdf::input::recovery::{
    PatchedSource, candidate_prefix_end, interrupted_syntax_prefix, replay_end,
};
use crate::read_exact_at;
use crate::test_support::{NEVER, run};
use std::io::{self, Cursor};

/// A source whose bytes from `unreadable_from` onward fail with an I/O
/// error, as a truncated network range or failing disk sector would.
pub(super) struct UnreadableTail {
    pub(super) bytes: Vec<u8>,
    pub(super) unreadable_from: u64,
}

impl RangedSource for UnreadableTail {
    fn size(&self) -> u64 {
        self.bytes.len() as u64
    }

    async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
        if offset >= self.unreadable_from {
            return Err(Error::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "injected unreadable fragment tail",
            )));
        }
        let start = offset as usize;
        let end = (self.unreadable_from.min(self.bytes.len() as u64) as usize)
            .min(start + destination.len());
        destination[..end - start].copy_from_slice(&self.bytes[start..end]);
        Ok(end - start)
    }
}

fn one_byte_reads() -> Limits {
    Limits {
        io_chunk_bytes: 1,
        ..Limits::default()
    }
}

fn expect_injected_io(result: Result<FragmentScan>) {
    let error = result
        .err()
        .expect("unreadable fragment bytes were accepted");
    assert!(
        matches!(
            &error,
            Error::Io(inner) if inner.kind() == io::ErrorKind::UnexpectedEof
                && inner.to_string() == "injected unreadable fragment tail"
        ),
        "{error:?}"
    );
}

#[test]
fn repair_skips_terminators_at_the_declared_end_or_without_endobj() {
    // The first candidate sits exactly at the declared end, the second
    // lacks `endobj`, and only the LF-separated third one is complete.
    let payload = b"0123456789\nendstream junk\nendstream junk2";
    let mut bytes = b"1 0 obj\n<< /Length 10 >>\nstream\n".to_vec();
    let data_at = bytes.len() as u64;
    bytes.extend_from_slice(payload);
    bytes.extend_from_slice(b"\nendstream\nendobj");
    let end = bytes.len() as u64;
    let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
    let scan = run(scan_fragment_with_candidates(
        &mut source,
        0,
        end,
        &Limits::default(),
        &NEVER,
        &mut [],
    ))
    .unwrap();
    assert_eq!(scan.objects.len(), 1);
    assert_eq!(scan.objects[0].object.range.length, end);
    assert_eq!(scan.patches.len(), 1);
    let patch = &scan.patches[0];
    assert_eq!(patch.original, b"10");
    assert_eq!(patch.replacement, payload.len().to_string().into_bytes());
    assert!(patch.offset < data_at);

    let mut patched = PatchedSource::new(&mut source, &scan.patches);
    let mut length = [0_u8; 2];
    run(read_exact_at(
        &mut patched,
        patch.offset,
        &mut length,
        &Limits::default(),
        &NEVER,
    ))
    .unwrap();
    assert_eq!(&length, b"41");
    // An empty read inside a patch overlaps none of its bytes.
    assert_eq!(run(patched.read_at(patch.offset + 1, &mut [])).unwrap(), 0);
}

#[test]
fn repair_accepts_a_terminator_without_a_preceding_end_of_line() {
    let mut bytes = b"1 0 obj\n<< /Length 10 >>\nstream\n0123456789ab".to_vec();
    bytes.extend_from_slice(b"endstream\nendobj");
    let end = bytes.len() as u64;
    let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
    let scan = run(scan_fragment_with_candidates(
        &mut source,
        0,
        end,
        &Limits::default(),
        &NEVER,
        &mut [],
    ))
    .unwrap();
    assert_eq!(scan.objects[0].object.range.length, end);
    assert_eq!(scan.patches.len(), 1);
    assert_eq!(scan.patches[0].replacement, b"12");
}

#[test]
fn source_failure_after_the_declared_stream_extent_is_not_repaired() {
    let mut bytes = b"1 0 obj\n<< /Length 2000 >>\nstream\n".to_vec();
    let data_at = bytes.len() as u64;
    bytes.extend(std::iter::repeat_n(b'x', 2000));
    bytes.extend_from_slice(b"\nendstream\nendobj");
    let size = bytes.len() as u64;
    let mut source = UnreadableTail {
        bytes,
        unreadable_from: data_at + 2000,
    };
    expect_injected_io(run(scan_fragment_with_candidates(
        &mut source,
        0,
        size,
        &one_byte_reads(),
        &NEVER,
        &mut [],
    )));
}

#[test]
fn source_failure_while_checking_a_repair_candidate_is_propagated() {
    let mut bytes = b"1 0 obj\n<< /Length 1000 >>\nstream\n".to_vec();
    bytes.extend(std::iter::repeat_n(b'x', 1002));
    let marker = bytes.len() as u64 + 1;
    bytes.extend_from_slice(b"\nendstream\nendobj");
    let size = bytes.len() as u64;
    let mut source = UnreadableTail {
        bytes,
        unreadable_from: marker + 9,
    };
    expect_injected_io(run(scan_fragment_with_candidates(
        &mut source,
        0,
        size,
        &one_byte_reads(),
        &NEVER,
        &mut [],
    )));

    // The same bytes repair cleanly once the tail is readable.
    source.unreadable_from = u64::MAX;
    let scan = run(scan_fragment_with_candidates(
        &mut source,
        0,
        size,
        &one_byte_reads(),
        &NEVER,
        &mut [],
    ))
    .unwrap();
    assert_eq!(scan.patches.len(), 1);
    assert_eq!(scan.patches[0].original, b"1000");
    assert_eq!(scan.patches[0].replacement, b"1002");
}

#[test]
fn an_object_extending_past_the_page_table_end_is_charged_as_input() {
    let bytes = b"1 0 obj\nnull\nendobj\n2 0 obj\nnull\nendobj".to_vec();
    let end = bytes.len() as u64;
    // The page table ends inside the second object, and every single
    // read (bounded by the 30-byte syntax window) stays within the limit.
    let hint = 30;
    let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
    let limits = Limits {
        io_chunk_bytes: 8,
        max_allocation_bytes: hint * 32,
        max_input_bytes: hint,
        ..Limits::default()
    };
    let error = run(scan_fragment_with_candidates(
        &mut source,
        0,
        hint,
        &limits,
        &NEVER,
        &mut [],
    ))
    .err()
    .expect("an object past the input limit was accepted");
    assert!(
        matches!(
            error,
            Error::PdfLimitExceeded {
                offset,
                object: None,
                resource: "input bytes",
                limit,
                attempted,
            } if offset == end && limit == hint && attempted == end
        ),
        "{error:?}"
    );
}
fn replay_fixture(prefix: &[u8], scalar: &[u8]) -> Vec<u8> {
    let mut bytes =
        b"1 0 obj\n<<\n/Length 1 >>\nstream\nx\nendstream\nendobj\n2 0 obj\n1\nendobj\n".to_vec();
    bytes.extend_from_slice(prefix);
    bytes.extend_from_slice(scalar);
    bytes.extend_from_slice(b"\n3 0 obj\nnull\nendobj\n");
    bytes
}

#[test]
fn skips_only_a_known_partial_header_and_exact_previous_scalar_replay() {
    let bytes = replay_fixture(b"1 0 obj\n<<\n/Length\n", b"2 0 obj\n1\nendobj");
    for chunk in [1, 4096] {
        let end = bytes.len() as u64;
        let mut source = SeekableSource::new(Cursor::new(bytes.clone())).unwrap();
        let limits = Limits {
            io_chunk_bytes: chunk,
            ..Limits::default()
        };
        let scan = run(scan_fragment_with_candidates(
            &mut source,
            0,
            end,
            &limits,
            &NEVER,
            &mut [],
        ))
        .unwrap();
        assert_eq!(
            scan.objects
                .iter()
                .map(|o| o.object.reference.number)
                .collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert!(scan.patches.is_empty());
    }
}

#[test]
fn refuses_unknown_prefixes_changed_scalars_and_unbounded_replays() {
    for (prefix, scalar) in [
        (
            b"9 0 obj\n<<\n/Length\n".as_slice(),
            b"2 0 obj\n1\nendobj".as_slice(),
        ),
        (b"1 1 obj\n<<\n/Length\n", b"2 0 obj\n1\nendobj"),
        (b"1 0 obj\n<<\n/Other\n", b"2 0 obj\n1\nendobj"),
        (b"1 0 obj\n<<\n/Length\n", b"2 0 obj\n2\nendobj"),
        (b"1 0 obj\n<<\n/Length\n", b"2 0 obj\nnull\nendobj"),
        (b"1 0 obj\n<<\n/Length\n", b"2 0 obj 1\nendobj"),
        (b"1 0 obj\n<<\n/Length\n", b"2 0 obj\n1\nendobjJUNK"),
    ] {
        let bytes = replay_fixture(prefix, scalar);
        let end = bytes.len() as u64;
        let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
        assert!(
            run(scan_fragment_with_candidates(
                &mut source,
                0,
                end,
                &Limits::default(),
                &NEVER,
                &mut [],
            ))
            .is_err()
        );
    }
    let mut prefix = b"1 0 obj\n<<\n/Length".to_vec();
    prefix.extend(std::iter::repeat_n(b' ', 256));
    let bytes = replay_fixture(&prefix, b"2 0 obj\n1\nendobj");
    let end = bytes.len() as u64;
    let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
    assert!(
        run(scan_fragment_with_candidates(
            &mut source,
            0,
            end,
            &Limits::default(),
            &NEVER,
            &mut [],
        ))
        .is_err()
    );
}
#[test]
fn refuses_unknown_original_or_scalar_and_changed_prefix() {
    let original = replay_fixture(b"1 0 obj\n<<\n/Length\n", b"2 0 obj\n1\nendobj");
    let text = String::from_utf8(original).unwrap();
    let duplicated = text.replacen("2 0 obj", "1 0 obj\nnull\nendobj\n2 0 obj", 1);
    let non_integer = text.replacen("2 0 obj\n1", "2 0 obj\nnull", 1);
    let changed_prefix = text.replacen("1 0 obj\n<<", "1 0 obj <<", 1);
    let long_integer = text.replacen("2 0 obj\n1", &format!("2 0 obj\n{}1", " ".repeat(256)), 1);
    let non_stream = text.replacen("<<\n/Length 1 >>\nstream\nx\nendstream", "null", 1);
    let no_history = "1 0 obj\n<<\n/Length\n2 0 obj\n1\nendobj\n".to_owned();
    for text in [
        duplicated,
        non_integer,
        changed_prefix,
        long_integer,
        no_history,
        non_stream,
    ] {
        let bytes = text.into_bytes();
        let end = bytes.len() as u64;
        let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
        assert!(
            run(scan_fragment_with_candidates(
                &mut source,
                0,
                end,
                &Limits::default(),
                &NEVER,
                &mut [],
            ))
            .is_err()
        );
    }
}
#[test]
fn recovers_a_partial_filter_name_without_a_sample_specific_rule() {
    let mut bytes = b"1 0 obj\n<< /Length 1 /Filter /FlateDecode >>\nstream\nx\nendstream\nendobj\n2 0 obj\n1\nendobj\n".to_vec();
    bytes.extend_from_slice(
        b"1 0 obj\n<< /Length 1 /Filter /FlateD\n2 0 obj\n1\nendobj\n3 0 obj\nnull\nendobj\n",
    );
    let end = bytes.len() as u64;
    let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
    let scan = run(scan_fragment_with_candidates(
        &mut source,
        0,
        end,
        &Limits::default(),
        &NEVER,
        &mut [],
    ))
    .unwrap();
    assert_eq!(scan.objects.len(), 3);
}
fn scan_bytes(bytes: Vec<u8>) -> Result<FragmentScan> {
    let end = bytes.len() as u64;
    let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
    run(scan_fragment_with_candidates(
        &mut source,
        0,
        end,
        &one_byte_reads(),
        &NEVER,
        &mut [],
    ))
}

#[test]
fn recovers_partial_length_objects_only_when_measured_value_matches() {
    let jpeg = [0xff, 0xd8, 0xff, 0xda, 0, 2, 7, 0xff, 0xd9];
    for prefix in [
        "2",
        "2 0",
        "2 0 obj",
        "2 0 obj\n9",
        "2 0 obj\n9\nendob",
        "3 0",
        "2 1",
        "2 0 obj\n8",
    ] {
        for value in [9, 8] {
            let mut bytes = b"1 0 obj\n<< /Filter /DCTDecode /Length 2 0 R >>\nstream\n".to_vec();
            bytes.extend_from_slice(&jpeg);
            bytes.extend_from_slice(
                format!("\nendstream\nendobj\n{prefix}\n2 0 obj\n{value}\nendobj\n").as_bytes(),
            );
            let expected = value == 9 && !["3 0", "2 1", "2 0 obj\n8"].contains(&prefix);
            assert_eq!(scan_bytes(bytes).is_ok(), expected, "{prefix}, {value}");
        }
    }
}

#[test]
fn interrupted_length_prefixes_preserve_intervening_objects() {
    let jpeg = [0xff, 0xd8, 0xff, 0xda, 0, 2, 7, 0xff, 0xd9];
    for (prefix, following, final_scalar, succeeds) in [
        (
            "2 0 obj 9",
            "3 0 obj 15336 endobj",
            "2 0 obj 9 endobj",
            true,
        ),
        (
            "2 0 obj 9",
            "3 0 obj 15336 endobj",
            "2 0 obj 8 endobj",
            false,
        ),
        ("2 0 obj 9", "3 0 obj 15336 endobj", "", false),
        (
            "2 0 obj 8",
            "3 0 obj 15336 endobj",
            "2 0 obj 9 endobj",
            false,
        ),
        (
            "2 1 obj 9",
            "3 0 obj 15336 endobj",
            "2 0 obj 9 endobj",
            false,
        ),
        (
            "2 0 obj 9 extra",
            "3 0 obj 15336 endobj",
            "2 0 obj 9 endobj",
            false,
        ),
        (
            "2 0 obj 9",
            "3 0 obj << /Broken @ >> endobj",
            "2 0 obj 9 endobj",
            false,
        ),
    ] {
        let mut bytes = b"1 0 obj\n<< /Filter /DCTDecode /Length 2 0 R >>\nstream\n".to_vec();
        bytes.extend_from_slice(&jpeg);
        bytes.extend_from_slice(
            format!("\nendstream\nendobj\n{prefix}\n{following}\n{final_scalar}\n").as_bytes(),
        );
        let result = scan_bytes(bytes);
        assert_eq!(
            result.is_ok(),
            succeeds,
            "{prefix}, {following}, {final_scalar}"
        );
        if let Ok(scan) = result {
            assert_eq!(
                scan.objects
                    .iter()
                    .map(|o| o.object.reference.number)
                    .collect::<Vec<_>>(),
                [1, 3, 2]
            );
        }
    }
}

#[test]
fn compacts_identical_complete_objects_and_keeps_source_order() {
    let first = b"3 0 obj\nnull\nendobj\n2 0 obj\n7\nendobj\n";
    let mut bytes = first.to_vec();
    bytes.extend_from_slice(first);
    bytes.extend_from_slice(b"1 0 obj\n<< /Length 5000 >>\nstream\n");
    bytes.extend(std::iter::repeat_n(b'x', 5000));
    bytes.extend_from_slice(b"\nendstream\nendobj\n");
    let stream = bytes[first.len() * 2..].to_vec();
    bytes.extend_from_slice(&stream);
    let scan = scan_bytes(bytes).unwrap();
    assert_eq!(
        scan.objects
            .iter()
            .map(|o| o.object.reference.number)
            .collect::<Vec<_>>(),
        [3, 2, 1]
    );
}

#[test]
fn rejects_conflicting_complete_replays() {
    for bytes in [
        b"1 0 obj\n7\nendobj\n1 0 obj\n8\nendobj\n".as_slice(),
        b"1 0 obj\n7\nendobj\n1 0 obj\n07\nendobj\n",
        b"1 0 obj\n7\nendobj\n1 0 obj 7\nendobj\n",
        b"1 0 obj\n<< /A 1 >>\nendobj\n1 0 obj\n<< /A 2 >>\nendobj\n",
    ] {
        assert!(scan_bytes(bytes.to_vec()).is_err());
    }
}

#[test]
fn recovers_only_adjacent_same_reference_unfinished_headers() {
    for body in [
        b"10 0 obj<< /Value 37 >>endobj\n".as_slice(),
        b"10 0 obj[3 7 19]endobj\n",
        b"10 0 obj<< /Length 3 >>stream\nabc\nendstream\nendobj\n",
    ] {
        let mut bytes = b"10 0 \r\n".to_vec();
        let offset = bytes.len() as u64;
        bytes.extend_from_slice(body);
        let scan = scan_bytes(bytes).unwrap();
        assert_eq!(scan.objects.len(), 1);
        assert_eq!(scan.objects[0].object.reference.number, 10);
        assert_eq!(scan.objects[0].object.range.offset, offset);
    }
    for bytes in [
        b"10 0 \r\n11 0 obj<<>>endobj\n".as_slice(),
        b"10 1 \r\n10 1 obj<<>>endobj\n",
        b"10 0 \r\n10 1 obj<<>>endobj\n",
        b"10 0 \r\n10 0 obj<<",
        b"10 0 obj garbage\n10 0 obj<<>>endobj\n",
    ] {
        assert!(scan_bytes(bytes.to_vec()).is_err());
    }
}

#[test]
fn adjacent_header_recovery_has_a_fixed_boundary_budget() {
    for length in [64, 65] {
        let mut bytes = b"10 0".to_vec();
        bytes.resize(length, b' ');
        bytes.extend_from_slice(b"10 0 obj<< /Value 37 >>endobj\n");
        assert_eq!(scan_bytes(bytes).is_ok(), length == 64);
    }
}

#[test]
fn unfinished_headers_require_a_unique_identical_known_header() {
    for (prior, prefix, succeeds) in [
        ("7 0 obj 11 endobj\n", "7 0", true),
        ("7 0 obj << /Original 19 >> endobj\n", "7 0", true),
        ("7 0 obj 11 endobj\n", "7\t0", false),
        ("6 0 obj 11 endobj\n", "7 0", false),
        ("7 0 obj 11 endobj\n7 0 obj 11 endobj\n", "7 0", false),
    ] {
        let bytes = format!("{prior}{prefix}\n8 0 obj << /Next 23 >> endobj\n").into_bytes();
        let result = scan_bytes(bytes);
        assert_eq!(result.is_ok(), succeeds, "{prior:?}, {prefix:?}");
        if let Ok(scan) = result {
            assert_eq!(
                scan.objects
                    .iter()
                    .map(|o| o.object.reference.number)
                    .collect::<Vec<_>>(),
                [7, 8]
            );
        }
    }
}

#[test]
fn recovers_known_dictionary_prefix_at_the_syntax_error_boundary() {
    let original = b"1 0 obj<< /A 7 /B << /C 9 >> >>endobj\n";
    for prefix in [
        b"1 0 obj<<\r\n".as_slice(),
        b"1 0 obj<< /A 7 /B <<\n",
        b"1 0 obj<< /A 7 /B << /C 9 >\r\n",
        b"1 0 obj<< /A 7 /B\r\n",
    ] {
        let mut bytes = original.to_vec();
        bytes.extend_from_slice(prefix);
        bytes.extend_from_slice(b"2 0 obj<< /Different 42 >>endobj\n");
        let scan = scan_bytes(bytes).unwrap();
        assert_eq!(scan.objects.len(), 2);
        assert_eq!(scan.objects[0].object.reference.number, 1);
        assert_eq!(
            scan.objects[0].object.range.length,
            (original.len() - 1) as u64
        );
        assert_eq!(scan.objects[1].object.reference.number, 2);
        assert_eq!(
            scan.objects[1].object.range.offset,
            (original.len() + prefix.len()) as u64
        );
    }
}

#[test]
fn recovers_known_dictionary_cut_inside_an_array() {
    let original = b"7 0 obj<< /Box [-1 3 20 40] >>endobj\n";
    for prefix in [
        b"7 0 obj<< /Box [\n".as_slice(),
        b"7 0 obj<< /Box [-1 3\r\n",
    ] {
        let mut bytes = original.to_vec();
        bytes.extend_from_slice(prefix);
        bytes.extend_from_slice(b"9 0 obj<< /New 81 >>endobj\n");
        let scan = scan_bytes(bytes).unwrap();
        assert_eq!(scan.objects.len(), 2);
        assert_eq!(
            scan.objects[1].object.range.offset,
            (original.len() + prefix.len()) as u64
        );
    }
    let mut changed = original.to_vec();
    changed.extend_from_slice(b"7 0 obj<< /Box [99\n9 0 obj<< /New 81 >>endobj\n");
    assert!(scan_bytes(changed).is_err());
}

#[test]
fn deferred_prefix_accepts_exact_prior_arrays_and_bare_headers() {
    let original = b"7 0 obj [11 0 R 12 0 R 13 0 R] endobj\n";
    for prefix in [b"7 0 obj [\n".as_slice(), b"7 0 obj [11 0 R 12\n", b"7\r\n"] {
        let bytes = [original.as_slice(), prefix, b"9 0 obj 42 endobj\n"].concat();
        let result = scan_bytes(bytes).unwrap();
        assert_eq!(result.objects.len(), 2);
        assert_eq!(result.objects[1].object.reference.number, 9);
    }
    for prefix in [b"7 0 obj [99\n".as_slice(), b"7 0 obj [11 0 R 99\n"] {
        assert!(
            scan_bytes([original.as_slice(), prefix, b"9 0 obj 42 endobj\n"].concat()).is_err()
        );
    }
    assert!(scan_bytes([original.as_slice(), b"7 0 obj [11\n9 0 R\n"].concat()).is_err());
}

#[test]
fn dictionary_prefix_recovery_requires_exact_prior_dictionary_bytes() {
    for bytes in [
        b"1 0 obj<< /A 7 >>endobj\n1 0 obj<< /A 8\n2 0 obj<<>>endobj\n".as_slice(),
        b"1 0 obj<< /A 7 >>endobj\n3 0 obj<<\n2 0 obj<<>>endobj\n",
        b"1 0 obj<< /A 7 /Bee 9 >>endobj\n1 0 obj<< /A 7 /Boo 9\n2 0 obj<<>>endobj\n",
        b"4294967296 0 obj<<\n2 0 obj<<>>endobj\n",
        b"1 0 obj<< /A 7 >>endobj\n1 0 obj<<\n2 0 R\n",
        b"1 0 obj<< /A (literal) >>endobj\n1 0 obj<< /A (literal\n2 0 obj<<>>endobj\n",
    ] {
        assert!(scan_bytes(bytes.to_vec()).is_err());
    }
}

#[test]
fn known_integer_prefix_preserves_a_new_following_object() {
    for suffix in ["e", "en", "end", "endo", "endob"] {
        let bytes =
            format!("7 0 obj 137 endobj\n7 0 obj 137 {suffix}\n8 0 obj 19 endobj\n").into_bytes();
        let result = scan_bytes(bytes).unwrap();
        assert_eq!(
            result
                .objects
                .iter()
                .map(|scanned| scanned.object.reference.number)
                .collect::<Vec<_>>(),
            [7, 8]
        );
    }
    for bytes in [
        "7 0 obj 137 endobj\n7 0 obj 138 endob\n8 0 obj 19 endobj\n",
        "7 0 obj 137 endobj\n7 0 obj 137 endox\n8 0 obj 19 endobj\n",
        "7 0 obj 137 endobj\n7 0 obj 137 endobj\n7 0 obj 137 endob\n8 0 obj 19 endobj\n",
    ] {
        assert!(scan_bytes(bytes.as_bytes().to_vec()).is_err(), "{bytes}");
    }
}

#[test]
fn a_known_stream_dictionary_prefix_does_not_discard_its_payload() {
    let original = b"1 0 obj<< /Length 1 >>stream\nx\nendstream\nendobj\n";
    let mut bytes = original.to_vec();
    bytes.extend_from_slice(b"1 0 obj<< /Length\n2 0 obj<< /Next 7 >>endobj\n");
    let scan = scan_bytes(bytes).unwrap();
    assert_eq!(scan.objects.len(), 2);
    assert_eq!(scan.objects[0].object.range.offset, 0);
    assert_eq!(
        scan.objects[0].object.range.length,
        original.len() as u64 - 1
    );
    assert_eq!(scan.objects[1].object.reference.number, 2);
    let bytes = replay_fixture(b"1 0 obj\n<<\n/Length\n", b"4 0 obj\n1\nendobj");
    let scan = scan_bytes(bytes).unwrap();
    assert_eq!(
        scan.objects
            .iter()
            .map(|o| o.object.reference.number)
            .collect::<Vec<_>>(),
        [1, 2, 4, 3]
    );
}

#[test]
fn recovers_a_dictionary_prefix_and_older_integer_copy() {
    let bytes = b"1 0 obj\n<< /Type /Page /A 42 >>\nendobj\n2 0 obj\n7\nendobj\n3 0 obj\nnull\nendobj\n1 0 obj\n<< /Type /Page /A\n2 0 obj\n7\nendobj\n4 0 obj\nnull\nendobj\n";
    let scan = scan_bytes(bytes.to_vec()).unwrap();
    assert_eq!(scan.objects.len(), 4);
}

#[test]
fn recovers_truncated_replayed_stream_but_rejects_changed_prefix() {
    use std::io::Write;
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(b"original stream payload").unwrap();
    let encoded = encoder.finish().unwrap();
    let header = b"1 0 obj\n<< /Length 2 0 R /Filter /FlateDecode >>\nstream\n";
    let scalar = format!("2 0 obj\n{}\nendobj\n", encoded.len());
    let mut original = header.to_vec();
    original.extend_from_slice(&encoded);
    original.extend_from_slice(b"\nendstream\nendobj\n");
    original.extend_from_slice(scalar.as_bytes());
    original.extend_from_slice(b"3 0 obj\nnull\nendobj\n");
    for changed in [false, true] {
        let mut bytes = original.clone();
        bytes.extend_from_slice(header);
        bytes.extend_from_slice(&encoded[..3]);
        if changed {
            *bytes.last_mut().unwrap() ^= 1;
        }
        bytes.extend_from_slice(b"\n");
        bytes.extend_from_slice(scalar.as_bytes());
        bytes.extend_from_slice(b"4 0 obj\nnull\nendobj\n");
        let result = scan_bytes(bytes);
        assert_eq!(result.is_ok(), !changed);
        if let Ok(scan) = result {
            assert_eq!(scan.objects.len(), 4);
        }
    }
}
#[test]
fn refuses_incomplete_candidate_and_ambiguous_embedded_scalar_copies() {
    let mut bytes = b"1 0 obj\n<< /Filter /DCTDecode /Length 2 0 R >>\nstream\n".to_vec();
    bytes.extend_from_slice(&[0xff, 0xd8, 0xff, 0xda, 0, 2, 7, 0xff, 0xd9]);
    bytes.extend_from_slice(b"\nendstream\nendobj\n2 0\n2 0 obj\n9\nendob");
    assert!(scan_bytes(bytes).is_err());

    let scalar = b"2 0 obj\n7\nendobj";
    let mut bytes = scalar.to_vec();
    bytes.push(b'\n');
    let original_at = bytes.len() as u64;
    let payload = b"x\n2 0 obj\n7\nendobj\ny\n2 0 obj\n7\nendobj\nz";
    let header = format!("1 0 obj\n<< /Length {} >>\nstream\n", payload.len());
    bytes.extend_from_slice(header.as_bytes());
    bytes.extend_from_slice(payload);
    bytes.extend_from_slice(b"\nendstream\nendobj\n");
    let original_length = bytes.len() as u64 - original_at - 1;
    let replay_at = bytes.len() as u64;
    bytes.extend_from_slice(header.as_bytes());
    bytes.extend_from_slice(&payload[..payload.len() - 1]);
    let end = bytes.len() as u64;
    let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
    let limits = Limits::default();
    let mut reader = Reader::new(
        &mut source,
        PdfRange {
            offset: 0,
            length: end,
        },
        &limits,
        &NEVER,
    )
    .unwrap();
    let reference = PdfRef {
        number: 2,
        generation: 0,
    };
    let indexed = |object| ScannedObject {
        object,
        inspection: Err(Error::InvalidInput {
            reason: "not inspected",
        }),
    };
    let objects = [
        indexed(FragmentObject {
            reference,
            range: PdfRange {
                offset: 0,
                length: scalar.len() as u64,
            },
        }),
        indexed(FragmentObject {
            reference: PdfRef {
                number: 1,
                generation: 0,
            },
            range: PdfRange {
                offset: original_at,
                length: original_length,
            },
        }),
    ];
    assert_eq!(
        run(replay_end(
            &mut reader,
            replay_at,
            &objects,
            &BTreeMap::from([(reference, 7)])
        ))
        .unwrap(),
        None
    );
}
#[test]
fn a_stream_prefix_followed_by_its_length_replay_is_skipped() {
    use std::io::Write;
    let plain = [b'x'; 120];
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::none());
    encoder.write_all(&plain).unwrap();
    let encoded = encoder.finish().unwrap();
    let header = b"1 0 obj\n<< /Length 2 0 R /Filter /FlateDecode >>\nstream\n";
    let scalar = format!("2 0 obj\n{}\nendobj\n", encoded.len());
    let mut bytes = header.to_vec();
    bytes.extend_from_slice(&encoded);
    bytes.extend_from_slice(b"\nendstream\nendobj\n");
    bytes.extend_from_slice(scalar.as_bytes());
    bytes.extend_from_slice(header);
    bytes.extend_from_slice(&encoded[..50]);
    bytes.push(b'\n');
    bytes.extend_from_slice(scalar.as_bytes());
    bytes.extend_from_slice(b"4 0 obj\n<< /Length 5 0 R /Filter /FlateDecode >>\nstream\n");
    bytes.extend_from_slice(&encoded);
    bytes.extend_from_slice(
        format!("\nendstream\nendobj\n5 0 obj\n{}\nendobj\n", encoded.len()).as_bytes(),
    );
    let scan = scan_bytes(bytes).unwrap();
    assert_eq!(
        scan.objects
            .iter()
            .map(|o| o.object.reference.number)
            .collect::<Vec<_>>(),
        [1, 2, 4, 5]
    );
}

mod candidate_tests {
    use super::*;
    use crate::native::SeekableSource;
    use crate::test_support::{NEVER, run};
    use std::io::Cursor;

    const ORIGINAL: &[u8] = b"7 0 obj\n<< /Type /Example /Values [3 9] >>\nendobj\n";
    const PREFIX: &[u8] = b"7 0 obj\n<< /Type /Exa\n";
    const NEXT: &[u8] = b"8 0 obj\n<< /Value 42 >>\nendobj\n";

    fn candidate(offset: usize) -> FragmentCandidate {
        FragmentCandidate {
            object: FragmentObject {
                reference: PdfRef {
                    number: 7,
                    generation: 0,
                },
                range: PdfRange {
                    offset: offset as u64,
                    length: (ORIGINAL.len() - 1) as u64,
                },
            },
            used: false,
        }
    }

    fn scan(bytes: Vec<u8>, candidates: &mut [FragmentCandidate]) -> Result<FragmentScan> {
        let end = bytes.len() as u64;
        let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
        run(scan_fragment_with_candidates(
            &mut source,
            0,
            end,
            &Limits {
                io_chunk_bytes: 1,
                ..Limits::default()
            },
            &NEVER,
            candidates,
        ))
    }

    #[test]
    fn patched_rows_do_not_supply_recovery_candidates() {
        let bytes = b"1 0 obj << /Length 3 >> stream\nabcdef\nendstream\nendobj\n".to_vec();
        let end = bytes.len() as u64;
        let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
        let scan = run(scan_fragment_with_candidates(
            &mut source,
            0,
            end,
            &Limits::default(),
            &NEVER,
            &mut [],
        ))
        .unwrap();
        assert_eq!(scan.patches.len(), 1);
        assert!(
            run(collect_fragment_candidates(
                &mut source,
                0,
                end,
                &Limits::default(),
                &NEVER,
            ))
            .unwrap()
            .is_empty()
        );
    }

    #[test]
    fn candidate_stream_lengths_are_still_verified_by_the_complete_scan() {
        let mut bytes = b"8 0 obj << /Length 12 0 R /Filter /DCTDecode >> stream\n".to_vec();
        bytes.extend_from_slice(&[0xff, 0xd8, 0xff, 0xda, 0, 2, 7, 0xff, 0xd9]);
        bytes.extend_from_slice(b"\nendstream\nendobj\n");
        let end = bytes.len() as u64;
        let mut source = SeekableSource::new(Cursor::new(bytes.clone())).unwrap();
        assert_eq!(
            run(collect_fragment_candidates(
                &mut source,
                0,
                end,
                &Limits::default(),
                &NEVER,
            ))
            .unwrap()
            .len(),
            1
        );
        assert!(scan(bytes.clone(), &mut []).is_err());
        let mut valid = bytes.clone();
        valid.extend_from_slice(b"12 0 obj 9 endobj\n");
        assert_eq!(scan(valid, &mut []).unwrap().objects.len(), 2);
        bytes.extend_from_slice(b"12 0 obj 8 endobj\n");
        assert!(scan(bytes, &mut []).is_err());
    }

    #[test]
    fn anchored_candidates_can_depend_on_a_later_row_prefix_proof() {
        let header = b"8 0 obj << /Length 600 >> stream\n";
        let mut bytes = header.to_vec();
        bytes.extend_from_slice(b"ZZZZZ\n");
        let row_start = bytes.len() as u64;
        bytes.extend_from_slice(b"9 0 obj 42 endobj\n");
        bytes.extend_from_slice(PREFIX);
        bytes.extend_from_slice(b"10 0 obj 13 endobj\n");
        bytes.extend_from_slice(header);
        bytes.extend_from_slice(&[b'Z'; 600]);
        bytes.extend_from_slice(b"\nendstream\nendobj\n");
        let row_end = bytes.len() as u64;
        bytes.extend_from_slice(ORIGINAL);
        let mut source = SeekableSource::new(Cursor::new(bytes.clone())).unwrap();
        // A row alone is not a complete verified document: object 7 is later.
        assert!(
            run(scan_fragment_with_candidates(
                &mut source,
                row_start,
                row_end,
                &Limits::default(),
                &NEVER,
                &mut [],
            ))
            .is_err()
        );
        let objects = run(collect_fragment_candidates(
            &mut source,
            row_start,
            row_end,
            &Limits::default(),
            &NEVER,
        ))
        .unwrap();
        assert_eq!(
            objects
                .iter()
                .map(|o| o.reference.number)
                .collect::<Vec<_>>(),
            [9, 10, 8]
        );
        let mut candidates: Vec<_> = objects
            .into_iter()
            .map(|object| FragmentCandidate {
                object,
                used: false,
            })
            .collect();
        // The interrupted stream's replay bounds it; the deferred prefix is
        // proved by the whole fragment.
        assert_eq!(
            scan(bytes.clone(), &mut candidates).unwrap().objects.len(),
            4
        );
        // Collection must never waive proof on the final whole fragment.
        bytes.truncate(row_end as usize);
        assert!(scan(bytes, &mut candidates).is_err());
    }

    #[test]
    fn later_dictionary_must_be_reached_at_its_exact_boundary() {
        let mut bytes = [PREFIX, NEXT].concat();
        let mut candidates = [candidate(bytes.len())];
        bytes.extend_from_slice(ORIGINAL);
        let result = scan(bytes, &mut candidates).unwrap();
        assert!(candidates[0].used);
        assert_eq!(result.objects.len(), 2);
        assert_eq!(result.objects[1].object.range, candidates[0].object.range);

        // An apparently valid candidate embedded inside a real opaque stream
        // must not justify discarding an earlier interrupted object.
        let mut bytes = PREFIX.to_vec();
        bytes.extend_from_slice(
            format!("8 0 obj\n<< /Length {} >>\nstream\n", ORIGINAL.len()).as_bytes(),
        );
        let mut candidates = [candidate(bytes.len())];
        bytes.extend_from_slice(ORIGINAL);
        bytes.extend_from_slice(b"\nendstream\nendobj\n");
        assert!(matches!(
            scan(bytes, &mut candidates),
            Err(Error::Pdf {
                kind: PdfErrorKind::AmbiguousRepair,
                reason: "recovery candidate is not a complete fragment object",
                ..
            })
        ));
    }

    #[test]
    fn candidate_recovery_rejects_changed_prefixes_and_conflicting_copies() {
        let mut bytes = [PREFIX, NEXT].concat();
        let offset = bytes.len();
        bytes.extend_from_slice(ORIGINAL);
        let mut conflicting = [candidate(offset), candidate(offset + 1)];
        assert!(scan(bytes.clone(), &mut conflicting).is_err());
        assert!(conflicting.iter().all(|c| !c.used));
        for altered in *b"b!\0" {
            let mut changed = bytes.clone();
            changed[PREFIX.len() - 2] = altered;
            let mut candidates = [candidate(offset)];
            assert!(scan(changed, &mut candidates).is_err());
            assert!(!candidates[0].used);
        }
    }
    #[test]
    fn later_flate_copy_replaces_only_the_matching_interruption() {
        use std::io::Write;
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder
            .write_all(b"Original bounded candidate recovery stream.")
            .unwrap();
        let encoded = encoder.finish().unwrap();
        let header = b"7 0 obj\n<< /Length 9 0 R /Filter /FlateDecode >>\nstream\n";
        let mut complete = header.to_vec();
        complete.extend_from_slice(&encoded);
        complete.extend_from_slice(b"\nendstream\nendobj");
        let mut bytes = header.to_vec();
        bytes.extend_from_slice(&encoded[..5]);
        bytes.push(b'\n');
        bytes.extend_from_slice(NEXT);
        let mut candidates = [candidate(bytes.len())];
        candidates[0].object.range.length = complete.len() as u64;
        bytes.extend_from_slice(&complete);
        bytes.extend_from_slice(format!("\n9 0 obj\n{}\nendobj\n", encoded.len()).as_bytes());
        let result = scan(bytes, &mut candidates).unwrap();
        assert!(candidates[0].used);
        assert_eq!(result.objects.len(), 3);
    }

    #[test]
    fn later_direct_length_stream_has_a_bounded_prefix_recovery() {
        let header = b"7 0 obj\n<< /Length 600 >>\nstream\n";
        let mut complete = header.to_vec();
        complete.extend_from_slice(&[b'Z'; 600]);
        complete.extend_from_slice(b"\nendstream\nendobj");
        let mut bytes = header.to_vec();
        bytes.extend_from_slice(b"ZZZZZ\n");
        bytes.extend_from_slice(NEXT);
        let mut candidates = [candidate(bytes.len())];
        candidates[0].object.range.length = complete.len() as u64;
        bytes.extend_from_slice(&complete);
        // The later copy repeats the stream header at the offset its
        // endstream and Length imply, so the replay rule bounds the prefix.
        assert_eq!(scan(bytes, &mut candidates).unwrap().objects.len(), 2);
    }

    #[test]
    fn deferred_prefixes_require_real_later_counterparts() {
        let too_short =
            b"7 0 obj << /LongDictionaryName 3 >\n8 0 obj 42 endobj\n7 0 obj null endobj\n";
        assert!(matches!(
            scan(too_short.to_vec(), &mut []),
            Err(Error::Pdf {
                reason: "interrupted prefix has no exact complete counterpart",
                ..
            })
        ));
        let bytes = [PREFIX, NEXT, ORIGINAL].concat();
        assert_eq!(scan(bytes, &mut []).unwrap().objects.len(), 2);
        let bytes = b"7 0 obj << /Box [1 3\n8 0 obj 42 endobj\n7 0 obj << /Box [1 3 9] >> endobj\n"
            .to_vec();
        assert_eq!(scan(bytes, &mut []).unwrap().objects.len(), 2);
        let cut = b"7 0 obj << /Value 3 >\r\n8 0 obj 42 endobj\n7 0 obj << /Value 3 >> endobj\n";
        assert_eq!(scan(cut.to_vec(), &mut []).unwrap().objects.len(), 2);
        let changed =
            b"7 0 obj << /Value 3 >\r\n8 0 obj 42 endobj\n7 0 obj << /Value 4 >> endobj\n";
        assert!(scan(changed.to_vec(), &mut []).is_err());
        let mut padded = b"7 0 obj << /Value 3 >".to_vec();
        padded.extend_from_slice(&[b' '; 257]);
        padded.extend_from_slice(&cut[22..]);
        assert!(scan(padded, &mut []).is_err());
        let mut fake = PREFIX.to_vec();
        fake.extend_from_slice(
            format!("8 0 obj << /Length {} >> stream\n", ORIGINAL.len()).as_bytes(),
        );
        fake.extend_from_slice(ORIGINAL);
        fake.extend_from_slice(b"\nendstream\nendobj\n");
        assert!(matches!(
            scan(fake, &mut []),
            Err(Error::Pdf {
                reason: "interrupted prefix has no exact complete counterpart",
                ..
            })
        ));
        let changed =
            b"7 0 obj << /Box [1 3\n8 0 obj 42 endobj\n7 0 obj << /Box [1 4 9] >> endobj\n"
                .to_vec();
        assert!(scan(changed, &mut []).is_err());
    }

    #[test]
    fn prior_indirect_lengths_frame_streams_and_later_lengths_confirm_them() {
        let payload = b"opaque endstream endobj 99 0 obj";
        let mut bytes = format!(
            "2 0 obj {} endobj\n1 0 obj << /Length 2 0 R /Filter /RunLengthDecode >> stream\n",
            payload.len()
        )
        .into_bytes();
        bytes.extend_from_slice(payload);
        bytes.extend_from_slice(b"\nendstream\nendobj\n");
        assert_eq!(scan(bytes.clone(), &mut []).unwrap().objects.len(), 2);
        bytes.extend_from_slice(b"2 0 obj 1 endobj\n");
        assert!(scan(bytes, &mut []).is_err());
        let short = b"2 0 obj 1 endobj\n1 0 obj << /Length 2 0 R /Filter /RunLengthDecode >> stream\nlong\nendstream\nendobj\n";
        assert!(scan(short.to_vec(), &mut []).is_err());
        // Whatever the filter, the later Length object confirms the endstream.
        let later = b"1 0 obj << /Length 2 0 R /Filter /RunLengthDecode >> stream\nx\nendstream\nendobj\n2 0 obj 1 endobj\n";
        assert_eq!(scan(later.to_vec(), &mut []).unwrap().objects.len(), 2);
    }

    #[test]
    fn deferred_stream_prefix_requires_a_real_later_copy() {
        use std::io::Write;
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::none());
        encoder.write_all(&[b'Q'; 1024]).unwrap();
        let encoded = encoder.finish().unwrap();
        let scalar = format!("6 0 obj {} endobj\n", encoded.len());
        let header = b"7 0 obj << /Length 6 0 R /Filter /FlateDecode >> stream\n";
        let prefix = [header.as_slice(), &encoded[..12], b"\n"].concat();
        let complete = [header.as_slice(), &encoded, b"\nendstream\nendobj\n"].concat();
        let intervening = b"8 0 obj << /Value 42 >> endobj\n";
        let valid = [
            scalar.as_bytes(),
            &prefix,
            scalar.as_bytes(),
            intervening,
            &complete,
        ]
        .concat();
        let result = scan(valid, &mut []).unwrap();
        assert_eq!(
            result
                .objects
                .iter()
                .map(|scanned| scanned.object.reference.number)
                .collect::<Vec<_>>(),
            [6, 8, 7]
        );
        assert!(result.patches.is_empty());

        let mut changed = prefix.clone();
        changed[header.len() + 8] ^= 1;
        let mut corrupt = complete.clone();
        corrupt[header.len() + encoded.len() - 1] ^= 1;
        let decoy_header = format!("9 0 obj << /Length {} >> stream\n", complete.len());
        let embedded = [decoy_header.as_bytes(), &complete, b"\nendstream\nendobj\n"].concat();
        // Complete objects of any kind may sit between the prefix and its
        // copy, and payload bytes are not decoded: a corrupt copy still frames.
        for bytes in [
            [
                scalar.as_bytes(),
                &prefix,
                scalar.as_bytes(),
                intervening,
                &corrupt,
            ]
            .concat(),
            [
                scalar.as_bytes(),
                &prefix,
                scalar.as_bytes(),
                scalar.as_bytes(),
                intervening,
                &complete,
            ]
            .concat(),
            [
                scalar.as_bytes(),
                intervening,
                &prefix,
                scalar.as_bytes(),
                &complete,
            ]
            .concat(),
        ] {
            assert_eq!(scan(bytes, &mut []).unwrap().objects.len(), 3);
        }
        for bytes in [
            [scalar.as_bytes(), &prefix, scalar.as_bytes(), intervening].concat(),
            [
                scalar.as_bytes(),
                &changed,
                scalar.as_bytes(),
                intervening,
                &complete,
            ]
            .concat(),
            [
                scalar.as_bytes(),
                &prefix,
                scalar.as_bytes(),
                intervening,
                &embedded,
            ]
            .concat(),
            [
                scalar.as_bytes(),
                &prefix,
                b"6 0 obj 999 endobj\n",
                intervening,
                &complete,
            ]
            .concat(),
        ] {
            assert!(scan(bytes, &mut []).is_err());
        }
    }

    #[test]
    fn adjacent_replays_need_an_exact_prefix_and_a_framed_copy() {
        use std::io::Write;
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::none());
        encoder.write_all(&[b'Q'; 1024]).unwrap();
        let encoded = encoder.finish().unwrap();
        let fixture = |cut: usize, padding: &[u8], extra: usize, corrupt: bool| {
            let header = format!(
                "7 0 obj << /Length {} /Filter /FlateDecode >> stream\n",
                encoded.len() + padding.len() + extra
            );
            let mut payload = encoded.clone();
            if corrupt {
                let last = payload.len() - 1;
                payload[last] ^= 1;
            }
            [
                header.as_bytes(),
                &payload[..cut],
                b"\n",
                header.as_bytes(),
                &payload,
                padding,
                b"\nendstream\nendobj\n",
            ]
            .concat()
        };
        let short = format!(
            "7 0 obj << /Length {} /Filter /FlateDecode >> stream\n",
            encoded.len() - 1
        );
        let repaired = [short.as_bytes(), &encoded, b"\nendstream\nendobj\n"].concat();
        assert_eq!(scan(repaired, &mut []).unwrap().patches.len(), 1);
        // Payload bytes are not decoded: a byte past the codec end is data.
        let junk = [short.as_bytes(), &encoded, b"X\nendstream\nendobj\n"].concat();
        assert_eq!(scan(junk, &mut []).unwrap().patches.len(), 1);
        // The replay needs no codec and no fixed distance: the copy's header
        // sits where its endstream and Length put it.
        for bytes in [
            fixture(1, b"", 0, false),
            fixture(12, b"", 0, false),
            fixture(12, b"\n", 0, false),
            fixture(12, b"\r", 0, false),
            fixture(12, b"\r\n", 0, false),
            fixture(12, b"X", 0, false),
            fixture(12, b"\n\n\n", 0, false),
            fixture(12, b"", 0, true),
            fixture(512, b"", 0, false),
        ] {
            let result = scan(bytes, &mut []).unwrap();
            assert_eq!(result.objects.len(), 1);
            assert!(result.patches.is_empty());
        }
        // Without a payload byte the prefix is no evidence; a copy that its own
        // Length cannot frame, or a cut-off copy, is still an error.
        let mut truncated = fixture(12, b"", 0, false);
        truncated.truncate(truncated.len() - 4);
        for bytes in [
            fixture(0, b"", 0, false),
            fixture(12, b"", 2, false),
            truncated,
        ] {
            assert!(scan(bytes, &mut []).is_err());
        }
    }

    #[test]
    fn replays_after_intervening_objects_need_an_exact_prefix_and_header() {
        use std::io::Write;
        let plain: Vec<_> = (0..1024).map(|value| value as u8).collect();
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::none());
        encoder.write_all(&plain).unwrap();
        let encoded = encoder.finish().unwrap();
        let scalar = b"6 0 obj 91 endobj\n";
        let header = format!(
            "7 0 obj << /Length {} /Filter /FlateDecode >> stream\n",
            encoded.len()
        );
        let interrupted = [header.as_bytes(), &encoded[..12], b"\n"].concat();
        let complete = [header.as_bytes(), &encoded, b"\nendstream\nendobj\n"].concat();
        let valid = [scalar.as_slice(), &interrupted, scalar, &complete].concat();
        let result = scan(valid, &mut []).unwrap();
        assert_eq!(result.objects.len(), 2);
        assert_eq!(
            result.objects[1].object.range.offset,
            (scalar.len() * 2 + interrupted.len()) as u64
        );
        assert!(result.patches.is_empty());
        let array = b"6 0 obj[/ICCBased 7 0 R] endobj\n";
        let prior_stream = b"6 0 obj << /Length 1 >> stream\nX\nendstream\nendobj\n";
        let large_scalar = format!("6 0 obj {}91 endobj\n", " ".repeat(260));
        let mut corrupt = complete.clone();
        corrupt[header.len() + encoded.len() - 1] ^= 1;
        // The intervening objects are ordinary complete objects: new, repeated,
        // streams, or arbitrarily long. Payload bytes are never decoded.
        for bytes in [
            [array.as_slice(), &interrupted, array, &complete].concat(),
            [
                prior_stream.as_slice(),
                &interrupted,
                prior_stream,
                &complete,
            ]
            .concat(),
            [&interrupted[..], scalar, &complete].concat(),
            [scalar.as_slice(), scalar, &interrupted, scalar, &complete].concat(),
            [
                large_scalar.as_bytes(),
                &interrupted,
                large_scalar.as_bytes(),
                &complete,
            ]
            .concat(),
            [scalar.as_slice(), &interrupted, scalar, scalar, &complete].concat(),
            [scalar.as_slice(), &interrupted, scalar, &corrupt].concat(),
        ] {
            assert_eq!(scan(bytes, &mut []).unwrap().objects.len(), 2);
        }
        let mut changed_prefix = interrupted.clone();
        changed_prefix[header.len() + 2] ^= 1;
        let wrong_length =
            header.replace(&encoded.len().to_string(), &(encoded.len() + 2).to_string());
        let bad_extent_prefix = [wrong_length.as_bytes(), &encoded[..12], b"\n"].concat();
        let bad_extent_copy =
            [wrong_length.as_bytes(), &encoded, b"\nendstream\nendobj\n"].concat();
        let mut changed_header = complete.clone();
        changed_header[0] = b'8';
        for bytes in [
            [
                b"6 0 obj null endobj\n".as_slice(),
                &interrupted,
                scalar,
                &complete,
            ]
            .concat(),
            [
                scalar.as_slice(),
                &interrupted,
                b"6 0 obj 92 endobj\n",
                &complete,
            ]
            .concat(),
            [scalar.as_slice(), &changed_prefix, scalar, &complete].concat(),
            [scalar.as_slice(), &interrupted, scalar, &changed_header].concat(),
            [
                scalar.as_slice(),
                &bad_extent_prefix,
                scalar,
                &bad_extent_copy,
            ]
            .concat(),
            [
                scalar.as_slice(),
                header.as_bytes(),
                b"\n",
                scalar,
                &complete,
            ]
            .concat(),
        ] {
            assert!(scan(bytes, &mut []).is_err());
        }
    }

    #[test]
    fn cut_reference_generation_requires_an_exact_later_dictionary() {
        for space in [" ", "\r\n"] {
            let prefix = format!("7 0 obj << /Probe 11 0{space}");
            let suffix = "8 0 obj 42 endobj\n7 0 obj << /Probe 11 0 R >> endobj\n";
            let bytes = format!("{prefix}{suffix}").into_bytes();
            assert_eq!(scan(bytes, &mut []).unwrap().objects.len(), 2);
            for changed in ["11", "12 0 R"] {
                let bytes =
                    format!("{prefix}8 0 obj 42 endobj\n7 0 obj << /Probe {changed} >> endobj\n");
                assert!(scan(bytes.into_bytes(), &mut []).is_err());
            }
        }
        for token in ["0x", "00", "1"] {
            let bytes = format!(
                "7 0 obj << /Probe 11 {token}\n8 0 obj 42 endobj\n7 0 obj << /Probe 11 0 R >> endobj\n"
            );
            assert!(scan(bytes.into_bytes(), &mut []).is_err());
        }
        let bytes = format!(
            "7 0 obj << /Probe 11 0{}8 0 obj 42 endobj\n7 0 obj << /Probe 11 0 R >> endobj\n",
            " ".repeat(257)
        );
        assert!(scan(bytes.into_bytes(), &mut []).is_err());
        assert!(
            scan(
                b"7 0 obj << /Probe 11 0\n8 0 obj 42 endobj\n".to_vec(),
                &mut []
            )
            .is_err()
        );
    }

    #[test]
    fn unfinished_tail_keywords_require_an_exact_complete_counterpart() {
        for (value, keyword, suffix) in [
            ("<< /Length 1 >>", "stream", "\nX\nendstream\nendobj"),
            ("42", "endobj", ""),
        ] {
            let complete = format!("7 0 obj {value} {keyword}{suffix}\n");
            for count in 1..keyword.len() {
                let prefix = format!("7 0 obj {value} {}\r\n", &keyword[..count]);
                let bytes = format!("{prefix}8 0 obj null endobj\n{complete}");
                assert_eq!(scan(bytes.into_bytes(), &mut []).unwrap().objects.len(), 2);
                // Neither a changed value nor a missing counterpart is proof.
                let changed = complete.replace(value, "<< /Length 2 >>");
                for tail in [changed.as_str(), ""] {
                    let bytes = format!("{prefix}8 0 obj null endobj\n{tail}");
                    assert!(scan(bytes.into_bytes(), &mut []).is_err());
                }
            }
        }
        for token in ["strx", "ends", "streamX", "stream", "endobjX"] {
            let bytes = format!(
                "7 0 obj << /Length 1 >> {token}\n8 0 obj null endobj\n7 0 obj << /Length 1 >> stream\nX\nendstream\nendobj\n"
            );
            assert!(scan(bytes.into_bytes(), &mut []).is_err(), "{token}");
        }
        let bytes = format!(
            "7 0 obj 42 endo{}8 0 obj null endobj\n7 0 obj 42 endobj\n",
            " ".repeat(257)
        );
        assert!(scan(bytes.into_bytes(), &mut []).is_err());
    }

    #[test]
    fn unfinished_headers_require_a_real_later_object() {
        for prefix in ["7", "7 0", "7 0 o", "7 0 ob"] {
            let bytes = format!("{prefix}\n8 0 obj 42 endobj\n7 0 obj << /Value 19 >> endobj\n")
                .into_bytes();
            let result = scan(bytes, &mut []).unwrap();
            assert_eq!(
                result
                    .objects
                    .iter()
                    .map(|scanned| scanned.object.reference.number)
                    .collect::<Vec<_>>(),
                [8, 7]
            );
        }
        for bytes in [
            "7\n8 0 obj 42 endobj\n",
            "7 1\n8 0 obj 42 endobj\n7 0 obj 19 endobj\n",
            "7 0 nonsense\n8 0 obj 42 endobj\n7 0 obj 19 endobj\n",
            "7 0 ox\n8 0 obj 42 endobj\n7 0 obj 19 endobj\n",
            "7 1 o\n8 0 obj 42 endobj\n7 0 obj 19 endobj\n",
            "7\n8 0 obj 42 endobj\n7 0 obj 19 endobj\n7 0 obj 20 endobj\n",
            "7 0\n8 0 obj 42 endobj\n7 1 obj 19 endobj\n",
        ] {
            assert!(scan(bytes.as_bytes().to_vec(), &mut []).is_err(), "{bytes}");
        }
        let mut padded = b"7 0 ob".to_vec();
        padded.extend_from_slice(&[b' '; 65]);
        padded.extend_from_slice(b"8 0 obj 42 endobj\n7 0 obj 19 endobj\n");
        assert!(scan(padded, &mut []).is_err());
        let payload = b"7 0 obj 19 endobj";
        let mut fake = format!("7\n8 0 obj << /Length {} >> stream\n", payload.len()).into_bytes();
        fake.extend_from_slice(payload);
        fake.extend_from_slice(b"\nendstream\nendobj\n");
        assert!(scan(fake, &mut []).is_err());
    }

    #[test]
    fn unfinished_header_candidate_propagates_syntax_limits() {
        let mut bytes = b"7 0\n8 0 obj << /Long (".to_vec();
        bytes.extend_from_slice(&[b'A'; 2000]);
        bytes.extend_from_slice(b") >> endobj\n7 0 obj 42 endobj\n");
        let size = bytes.len() as u64;
        let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
        let limits = Limits {
            max_allocation_bytes: 4096,
            io_chunk_bytes: 1,
            ..Limits::default()
        };
        assert!(matches!(
            run(scan_fragment_with_candidates(
                &mut source,
                0,
                size,
                &limits,
                &NEVER,
                &mut [],
            )),
            Err(Error::PdfLimitExceeded { .. })
        ));
    }

    #[test]
    fn deferred_boundary_probe_preserves_syntax_limits() {
        fn probe(
            bytes: Vec<u8>,
            boundary: u64,
            limit: u64,
        ) -> Result<Option<(u64, FragmentObject)>> {
            let size = bytes.len() as u64;
            let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
            let limits = Limits {
                max_allocation_bytes: limit,
                io_chunk_bytes: 1,
                ..Limits::default()
            };
            let mut reader = Reader::new(
                &mut source,
                PdfRange {
                    offset: 0,
                    length: size,
                },
                &limits,
                &NEVER,
            )
            .unwrap();
            let error = reader.malformed(boundary, None, "expected PDF name");
            run(interrupted_syntax_prefix(&mut reader, 0, &error))
        }
        // A short, malformed header cannot create a pending object or loop
        // when walking back to the first token exhausts the prefix.
        assert!(probe(b"7 ?".to_vec(), 2, 4096).unwrap().is_none());
        let mut bytes = PREFIX.to_vec();
        bytes.extend_from_slice(b"8 0 obj << /Long (");
        bytes.extend_from_slice(&[b'A'; 2000]);
        bytes.extend_from_slice(b") >> endobj");
        assert!(matches!(
            probe(bytes, PREFIX.len() as u64, 4096),
            Err(Error::PdfLimitExceeded { .. })
        ));
    }

    #[test]
    fn candidate_bounds_and_following_syntax_are_checked() {
        fn probe(
            bytes: Vec<u8>,
            start: u64,
            mut item: FragmentCandidate,
            limit: u64,
        ) -> Result<Option<u64>> {
            let size = bytes.len() as u64;
            let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
            let limits = Limits {
                io_chunk_bytes: 1,
                max_allocation_bytes: limit,
                ..Limits::default()
            };
            let mut reader = Reader::new(
                &mut source,
                PdfRange {
                    offset: start,
                    length: size - start,
                },
                &limits,
                &NEVER,
            )
            .unwrap();
            run(candidate_prefix_end(
                &mut reader,
                0,
                std::slice::from_mut(&mut item),
            ))
        }
        let mut bytes = [PREFIX, NEXT].concat();
        let at = bytes.len();
        bytes.extend_from_slice(ORIGINAL);
        let mut unknown = candidate(at);
        unknown.object.reference.number = 99;
        assert_eq!(probe(bytes.clone(), 0, unknown, 4096).unwrap(), None);
        assert_eq!(probe(bytes.clone(), 0, candidate(0), 4096).unwrap(), None);
        let mut shifted = vec![b' '; 10];
        shifted.extend_from_slice(&bytes);
        assert_eq!(probe(shifted, 10, candidate(0), 4096).unwrap(), None);
        let mut no_header = bytes.clone();
        no_header[0] = b'?';
        assert_eq!(probe(no_header, 0, candidate(at), 4096).unwrap(), None);
        let mut header_only = bytes.clone();
        header_only[1] = b'\t';
        assert_eq!(probe(header_only, 0, candidate(at), 4096).unwrap(), None);
        let mut bad_next = bytes.clone();
        bad_next[PREFIX.len() + 4] = b'x';
        assert_eq!(probe(bad_next, 0, candidate(at), 4096).unwrap(), None);
        let mut long = PREFIX.to_vec();
        long.extend_from_slice(b"8 0 obj\n<< /Long (");
        long.extend_from_slice(&[b'A'; 2000]);
        long.extend_from_slice(b") >>\nendobj\n");
        let at = long.len();
        long.extend_from_slice(ORIGINAL);
        assert!(matches!(
            probe(long, 0, candidate(at), 512),
            Err(Error::PdfLimitExceeded { .. })
        ));
    }
}
