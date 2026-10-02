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
    fn indirect_lengths_are_checked_after_framing() {
        for length in [3, 4] {
            let bytes = format!("1 0 obj << /Length 2 0 R /Filter /ASCII85Decode >> stream\nz~>\nendstream\nendobj\n2 0 obj {length} endobj\n").into_bytes();
            let end = bytes.len() as u64;
            let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
            assert_eq!(
                run(scan_fragment_objects(
                    &mut source,
                    0,
                    end,
                    &Limits::default(),
                    &NEVER
                ))
                .is_ok(),
                length == 3
            );
        }
    }
}
