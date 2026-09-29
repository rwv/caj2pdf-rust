// SPDX-License-Identifier: MIT

//! JPEG marker framing for indirect PDF stream lengths. This does not decode
//! image pixels or replace the stricter HN/C8 JPEG profile validator.

use super::*;

async fn byte<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    cursor: &mut u64,
    reference: PdfRef,
) -> Result<u8> {
    if reader.cancellation.is_cancelled() {
        return Err(Error::Cancelled);
    }
    let value = reader
        .byte(*cursor)
        .await?
        .ok_or_else(|| reader.malformed(*cursor, Some(reference), "truncated JPEG stream"))?;
    *cursor += 1;
    Ok(value)
}

pub(super) async fn extent<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    start: u64,
    reference: PdfRef,
) -> Result<u64> {
    let mut cursor = start;
    if byte(reader, &mut cursor, reference).await? != 0xff
        || byte(reader, &mut cursor, reference).await? != 0xd8
    {
        return Err(reader.malformed(start, Some(reference), "JPEG stream lacks SOI"));
    }
    let mut entropy = false;
    let mut scanned = false;
    loop {
        let at = cursor;
        if byte(reader, &mut cursor, reference).await? != 0xff {
            if entropy {
                continue;
            }
            return Err(reader.malformed(at, Some(reference), "expected JPEG marker"));
        }
        let marker = loop {
            let value = byte(reader, &mut cursor, reference).await?;
            if value != 0xff {
                break value;
            }
        };
        match marker {
            0x00 | 0xd0..=0xd7 if entropy => continue,
            0xd9 if scanned => return Ok(cursor - start),
            0xc0..=0xc7 | 0xc9..=0xcf | 0xda..=0xdf | 0xe0..=0xef | 0xfe => {}
            _ => return Err(reader.malformed(at, Some(reference), "unexpected JPEG marker")),
        }
        let high = byte(reader, &mut cursor, reference).await?;
        let low = byte(reader, &mut cursor, reference).await?;
        let length = u16::from_be_bytes([high, low]);
        if length < 2 {
            return Err(reader.malformed(at, Some(reference), "invalid JPEG segment length"));
        }
        let end = cursor.saturating_add(u64::from(length - 2));
        if end > reader.range.length {
            return Err(reader.malformed(at, Some(reference), "JPEG segment exceeds input"));
        }
        // Segment payloads can contain any marker-like bytes. Seek over them;
        // only entropy-coded bytes use stuffing and restart markers.
        cursor = end;
        if marker == 0xda {
            scanned = true;
            entropy = true;
        } else if marker != 0xdc {
            // DNL may occur inside entropy data without ending the scan.
            entropy = false;
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

    fn measure(bytes: &[u8], chunk: usize) -> Result<u64> {
        let mut source = SeekableSource::new(Cursor::new(bytes.to_vec())).unwrap();
        let limits = Limits {
            io_chunk_bytes: chunk,
            ..Limits::default()
        };
        let mut reader = Reader::new(
            &mut source,
            PdfRange {
                offset: 0,
                length: bytes.len() as u64,
            },
            &limits,
            &NEVER,
        )
        .unwrap();
        run(extent(
            &mut reader,
            0,
            PdfRef {
                number: 1,
                generation: 0,
            },
        ))
    }

    #[test]
    fn traverses_segments_stuffing_restarts_and_multiple_scans() {
        // Synthetic marker framing, not a claim of decodable JPEG pixels.
        let mut bytes = vec![0xff, 0xd8, 0xff, 0xee, 0, 6, 0xff, 0xd9, 0xff, 0xda];
        bytes.extend_from_slice(&[0xff, 0xda, 0, 2, 7, 0xff, 0, 0xd9, 0xff, 0xd0, 8]);
        bytes.extend_from_slice(&[0xff, 0xdc, 0, 4, 0, 3, 9]);
        bytes.extend_from_slice(&[0xff, 0xc4, 0, 2, 0xff, 0xda, 0, 2, 10, 0xff, 0xff, 0xd9]);
        let length = bytes.len() as u64;
        bytes.extend_from_slice(b"\nendstream\nendobj");
        for chunk in [1, 2, 4096] {
            assert_eq!(measure(&bytes, chunk).unwrap(), length);
        }
    }

    #[test]
    fn rejects_truncation_invalid_markers_and_segment_lengths() {
        let valid = [0xff, 0xd8, 0xff, 0xda, 0, 2, 7, 0xff, 0xd9];
        for end in 0..valid.len() {
            assert!(measure(&valid[..end], 1).is_err(), "{end}");
        }
        for bytes in [
            b"no JPEG".as_slice(),
            &[0xff, 0, 0],
            &[0xff, 0xd8, 7],
            &[0xff, 0xd8, 0xff, 0xd9],
            &[0xff, 0xd8, 0xff, 0xd0],
            &[0xff, 0xd8, 0xff, 0xda, 0, 1],
            &[0xff, 0xd8, 0xff, 0xee, 0xff, 0xff],
            &[0xff, 0xd8, 0xff, 0x00],
        ] {
            assert!(measure(bytes, 1).is_err(), "{bytes:?}");
        }
    }

    #[test]
    fn validates_the_indirect_integer_after_framing() {
        let payload = [0xff, 0xd8, 0xff, 0xda, 0, 2, 7, 0xff, 0xd9];
        for declared in [payload.len(), payload.len() + 1] {
            let mut bytes = b"1 0 obj\n<< /Filter /DCTDecode /Length 2 0 R >>\nstream\n".to_vec();
            bytes.extend_from_slice(&payload);
            bytes.extend_from_slice(
                format!("\nendstream\nendobj\n2 0 obj\n{declared}\nendobj\n").as_bytes(),
            );
            let end = bytes.len() as u64;
            let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
            let result = run(scan_fragment_objects(
                &mut source,
                0,
                end,
                &Limits::default(),
                &NEVER,
            ));
            assert_eq!(result.is_ok(), declared == payload.len());
        }
    }

    #[test]
    fn cancellation_is_checked_before_reading_cached_bytes() {
        struct Cancelled;
        impl Cancellation for Cancelled {
            fn is_cancelled(&self) -> bool {
                true
            }
        }
        let mut source = SeekableSource::new(Cursor::new(vec![0xff, 0xd8])).unwrap();
        let limits = Limits::default();
        let mut reader = Reader::new(
            &mut source,
            PdfRange {
                offset: 0,
                length: 2,
            },
            &limits,
            &Cancelled,
        )
        .unwrap();
        assert!(matches!(
            run(extent(
                &mut reader,
                0,
                PdfRef {
                    number: 1,
                    generation: 0
                }
            )),
            Err(Error::Cancelled)
        ));
    }
}
