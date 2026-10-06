// SPDX-License-Identifier: MIT

//! ASCII85 extent validation from ISO 32000 section 7.4.3. No decoded buffer.

use super::*;

pub(super) async fn extent<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    start: u64,
    reference: PdfRef,
    decoded_work: &mut u64,
) -> Result<u64> {
    let mut cursor = start;
    let mut digits = 0_u32;
    let mut value = 0_u64;
    loop {
        if reader.cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let byte = reader
            .byte(cursor)
            .await?
            .ok_or_else(|| reader.malformed(cursor, Some(reference), "truncated ASCII85 stream"))?;
        let failure = reader.malformed(cursor, Some(reference), "invalid ASCII85 stream");
        cursor += 1;
        let produced = match byte {
            0 | 9 | 10 | 12 | 13 | 32 => continue,
            b'!'..=b'u' => {
                value = value * 85 + u64::from(byte - b'!');
                digits += 1;
                if digits != 5 {
                    continue;
                }
                if value > u64::from(u32::MAX) {
                    return Err(failure);
                }
                digits = 0;
                value = 0;
                4
            }
            b'z' if digits == 0 => 4,
            b'~' => {
                if reader.byte(cursor).await? != Some(b'>') || digits == 1 {
                    return Err(failure);
                }
                if digits != 0 {
                    for _ in digits..5 {
                        value = value * 85 + 84;
                    }
                    if value > u64::from(u32::MAX) {
                        return Err(failure);
                    }
                }
                u64::from(digits.saturating_sub(1))
            }
            _ => return Err(failure),
        };
        *decoded_work = decoded_work.saturating_add(produced);
        if *decoded_work > reader.limits.max_output_bytes {
            return Err(Error::LimitExceeded {
                resource: "CAJ ASCII85 scan bytes",
                limit: reader.limits.max_output_bytes,
                attempted: *decoded_work,
            });
        }
        if byte == b'~' {
            return Ok(cursor + 1 - start);
        }
    }
}

/// A damaged ASCII85 prefix can be followed immediately by a complete replay.
/// Derive that boundary backwards from the first codec EOD and the immediately
/// following referenced Length scalar. Never search for an object header.
pub(super) async fn adjacent_replay<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    start: u64,
    data_at: u64,
    reference: PdfRef,
    target: PdfRef,
    work: &mut u64,
) -> Result<Option<u64>> {
    let mut end = data_at;
    loop {
        if reader.cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let Some(byte) = reader.byte(end).await? else {
            return Ok(None);
        };
        end += 1;
        *work = work.saturating_add(1);
        if *work > reader.limits.max_output_bytes {
            return Err(Error::LimitExceeded {
                resource: "CAJ ASCII85 boundary scan bytes",
                limit: reader.limits.max_output_bytes,
                attempted: *work,
            });
        }
        if byte == b'~' {
            if reader.byte(end).await? != Some(b'>') {
                return Ok(None);
            }
            end += 1;
            break;
        }
    }
    let mut after = reader.check_stream_tail(end, Some(reference)).await?;
    reader.skip_space(&mut after).await?;
    let length_head = reader.load_head(after, Some(target)).await?;
    let Some(length) = length_head
        .scalar
        .as_ref()
        .and_then(|range| exact_unsigned(&length_head.bytes[range.clone()]))
    else {
        return Ok(None);
    };
    let header_bytes = data_at - start;
    let Some(candidate) = end
        .checked_sub(length)
        .and_then(|data| data.checked_sub(header_bytes))
    else {
        return Ok(None);
    };
    if candidate <= start || candidate - start > 4096 || header_bytes > 256 {
        return Ok(None);
    }
    let prefix = reader.bytes(start, (candidate - start) as usize).await?;
    let prefix = prefix.trim_ascii_end();
    if prefix.len() <= header_bytes as usize
        || reader.bytes(candidate, prefix.len()).await? != prefix
    {
        return Ok(None);
    }
    // The candidate starts after data_at, so its first EOD is the one above.
    // Its extent equals Length by construction; still validate every group.
    extent(reader, candidate + header_bytes, reference, work).await?;
    Ok(Some(candidate))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        native::SeekableSource,
        test_support::{NEVER, run},
    };
    use std::io::Cursor;

    fn measure(bytes: &[u8], limit: u64, work: &mut u64, cancel: bool) -> Result<u64> {
        struct Cancel(bool);
        impl Cancellation for Cancel {
            fn is_cancelled(&self) -> bool {
                self.0
            }
        }
        let mut source = SeekableSource::new(Cursor::new(bytes.to_vec())).unwrap();
        let limits = Limits {
            io_chunk_bytes: 1,
            max_output_bytes: limit,
            ..Limits::default()
        };
        let cancellation = Cancel(cancel);
        let mut reader = Reader::new(
            &mut source,
            PdfRange {
                offset: 0,
                length: bytes.len() as u64,
            },
            &limits,
            &cancellation,
        )
        .unwrap();
        run(extent(
            &mut reader,
            0,
            PdfRef {
                number: 1,
                generation: 0,
            },
            work,
        ))
    }

    #[test]
    fn validates_groups_whitespace_zero_runs_and_exact_end() {
        for (bytes, decoded) in [
            (b"~>tail".as_slice(), 0),
            (b"z~>", 4),
            (b"!!!!!~>", 4),
            (b"!!~>", 1),
            (b"!!!~>", 2),
            (b"!!!!~>", 3),
            (b"s8W-!~>", 4),
            (b"\0\t\n\x0c\r z ! ! ! ! ! ~>", 8),
        ] {
            let mut work = 0;
            assert_eq!(
                measure(bytes, 100, &mut work, false).unwrap(),
                (bytes.iter().position(|b| *b == b'~').unwrap() + 2) as u64
            );
            assert_eq!(work, decoded);
        }
    }

    #[test]
    fn rejects_impossible_and_truncated_encodings() {
        for bytes in [
            b"".as_slice(),
            b"!!!!!",
            b"~",
            b"~ >",
            b"!~>",
            b"!z~>",
            b"uuuuu~>",
            b"uu~>",
            b"v~>",
            b"\x0b~>",
            b"<~z~>",
        ] {
            let result = measure(bytes, 100, &mut 0, false);
            assert!(is_malformed(&result), "{bytes:?}: {result:?}");
        }
    }

    #[test]
    fn preserves_work_budget_and_cancellation() {
        let mut work = 0;
        measure(b"z~>", 6, &mut work, false).unwrap();
        assert!(matches!(
            measure(b"z~>", 6, &mut work, false),
            Err(Error::LimitExceeded {
                resource: "CAJ ASCII85 scan bytes",
                attempted: 8,
                ..
            })
        ));
        assert!(matches!(
            measure(b"z~>", 100, &mut 0, true),
            Err(Error::Cancelled)
        ));
    }

    #[test]
    fn derives_adjacent_replay_only_from_eod_and_referenced_length() {
        let header = "1 0 obj << /Length 2 0 R /Filter /ASCII85Decode >> stream\n";
        let payload = "z!!!!!~>";
        let prefix = format!("{header}z!!!\n");
        for (replay_header, replay_payload, scalar, succeeds) in [
            (header.to_owned(), payload, "2 0 obj 8 endobj", true),
            (
                header.replace("1 0 obj", "3 0 obj"),
                payload,
                "2 0 obj 8 endobj",
                false,
            ),
            (header.to_owned(), "!!!!!z~>", "2 0 obj 8 endobj", false),
            (header.to_owned(), payload, "3 0 obj 8 endobj", false),
            (header.to_owned(), payload, "2 0 obj 7 endobj", false),
            (header.to_owned(), payload, "2 0 obj 999999 endobj", false),
            (header.to_owned(), payload, "2 0 obj null endobj", false),
            (header.to_owned(), "z!!!!v~>", "2 0 obj 8 endobj", false),
        ] {
            let bytes =
                format!("{prefix}{replay_header}{replay_payload}\nendstream\nendobj\n{scalar}\n")
                    .into_bytes();
            let end = bytes.len() as u64;
            let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
            let result = run(scan_fragment_with_candidates(
                &mut source,
                0,
                end,
                &Limits::default(),
                &NEVER,
                &mut [],
                &mut 0,
            ));
            assert_eq!(
                result.is_ok(),
                succeeds,
                "{replay_header} {replay_payload} {scalar}"
            );
            if let Ok(scan) = result {
                assert_eq!(scan.objects.len(), 2);
                assert_eq!(scan.objects[0].range.offset, prefix.len() as u64);
                assert!(scan.patches.is_empty());
            }
        }
    }

    #[test]
    fn longer_ascii85_prefix_uses_the_same_derived_boundary_proof() {
        let header = "1 0 obj << /Length 2 0 R /Filter /ASCII85Decode >> stream\n";
        let payload = format!("{}~>", "!!!!!".repeat(200));
        let prefix = format!("{header}{}\r\n", &payload[..520]);
        let bytes = format!(
            "{prefix}{header}{payload}\nendstream\nendobj\n2 0 obj {} endobj\n",
            payload.len()
        )
        .into_bytes();
        let length = bytes.len() as u64;
        let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
        let limits = Limits {
            io_chunk_bytes: 1,
            ..Limits::default()
        };
        let result = run(scan_fragment_with_candidates(
            &mut source,
            0,
            length,
            &limits,
            &NEVER,
            &mut [],
            &mut 0,
        ))
        .unwrap();
        assert_eq!(result.objects[0].range.offset, prefix.len() as u64);
        assert_eq!(result.objects.len(), 2);
    }

    #[test]
    fn adjacent_replay_scan_is_bounded_and_cancellable() {
        struct Cancel(bool);
        impl Cancellation for Cancel {
            fn is_cancelled(&self) -> bool {
                self.0
            }
        }
        let header = "1 0 obj << /Length 2 0 R /Filter /ASCII85Decode >> stream\n";
        let valid =
            format!("{header}z!!!\n{header}z!!!!!~>\nendstream\nendobj\n2 0 obj 8 endobj\n");
        let mut source = SeekableSource::new(Cursor::new(valid.as_bytes().to_vec())).unwrap();
        let limits = Limits {
            max_output_bytes: 60,
            ..Limits::default()
        };
        let result = run(scan_fragment_with_candidates(
            &mut source,
            0,
            valid.len() as u64,
            &limits,
            &NEVER,
            &mut [],
            &mut 0,
        ));
        assert!(matches!(
            result,
            Err(Error::LimitExceeded {
                resource: "CAJ ASCII85 boundary scan bytes",
                ..
            })
        ));
        for (bytes, limit, cancelled) in [
            (valid.clone(), 10000, true),
            (valid.clone(), 1, false),
            (format!("{header}unterminated"), 10000, false),
            (format!("{header}~!"), 10000, false),
            (
                format!("{header}~>endstream endobj 2 0 obj 999999 endobj"),
                10000,
                false,
            ),
            (
                format!("{header}~>endstream endobj 2 0 obj 2 endobj"),
                10000,
                false,
            ),
            (
                format!(
                    "{header}{}\n{header}z!!!!!~>\nendstream\nendobj\n2 0 obj 8 endobj",
                    "z".repeat(4097)
                ),
                10000,
                false,
            ),
            (
                format!("{header}{header}z!!!!!~>\nendstream\nendobj\n2 0 obj 8 endobj"),
                10000,
                false,
            ),
        ] {
            let length = bytes.len() as u64;
            let mut source = SeekableSource::new(Cursor::new(bytes.into_bytes())).unwrap();
            let limits = Limits {
                io_chunk_bytes: 1,
                max_output_bytes: limit,
                ..Limits::default()
            };
            let cancellation = Cancel(cancelled);
            let mut reader = Reader::new(
                &mut source,
                PdfRange { offset: 0, length },
                &limits,
                &cancellation,
            )
            .unwrap();
            let result = run(adjacent_replay(
                &mut reader,
                0,
                header.len() as u64,
                PdfRef {
                    number: 1,
                    generation: 0,
                },
                PdfRef {
                    number: 2,
                    generation: 0,
                },
                &mut 0,
            ));
            if cancelled {
                assert!(matches!(result, Err(Error::Cancelled)));
            } else if limit == 1 {
                assert!(matches!(
                    result,
                    Err(Error::LimitExceeded {
                        resource: "CAJ ASCII85 boundary scan bytes",
                        ..
                    })
                ));
            } else {
                assert!(matches!(result, Ok(None)), "{result:?}");
            }
        }
    }

    #[test]
    fn indirect_lengths_are_checked_after_framing() {
        for length in [3, 4] {
            let bytes = format!("1 0 obj << /Length 2 0 R /Filter /ASCII85Decode >> stream\nz~>\nendstream\nendobj\n2 0 obj {length} endobj\n").into_bytes();
            let end = bytes.len() as u64;
            let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
            assert_eq!(
                run(scan_fragment_with_candidates(
                    &mut source,
                    0,
                    end,
                    &Limits::default(),
                    &NEVER,
                    &mut [],
                    &mut 0
                ))
                .is_ok(),
                length == 3
            );
        }
    }
}
