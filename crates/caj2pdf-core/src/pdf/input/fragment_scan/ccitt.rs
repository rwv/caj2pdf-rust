// SPDX-License-Identifier: MIT

//! Bounded Group-4 framing using the MIT fax crate's public Huffman tables.
//! This is PDF CCITT framing, not the project's CAJ-specific JBIG decoder.

use super::super::Dictionary;
use super::*;
use crate::fallible::reserve_exact;
use fax::{
    BitReader, Bits,
    maps::{self, Mode},
};

#[derive(Default)]
struct Buffer {
    data: u32,
    valid: u8,
    consumed: u64,
}

impl BitReader for Buffer {
    type Error = ();
    fn peek(&self, count: u8) -> Option<u16> {
        if count > self.valid || count > 16 {
            return None;
        }
        Some(((self.data >> (self.valid - count)) & ((1_u32 << count) - 1)) as u16)
    }
    fn consume(&mut self, count: u8) -> std::result::Result<(), ()> {
        if count > self.valid {
            return Err(());
        }
        self.consumed = self.consumed.checked_add(u64::from(count)).ok_or(())?;
        self.valid -= count;
        Ok(())
    }
    fn bits_to_byte_boundary(&self) -> u8 {
        ((8 - self.consumed % 8) % 8) as u8
    }
}

struct Input<'a, 'b, S, C> {
    reader: &'a mut Reader<'b, S, C>,
    reference: PdfRef,
    start: u64,
    fetched: u64,
    bits: Buffer,
}

impl<S: RangedSource, C: Cancellation> Input<'_, '_, S, C> {
    fn invalid(&self, reason: &'static str) -> Error {
        self.reader.malformed(
            self.start + self.bits.consumed / 8,
            Some(self.reference),
            reason,
        )
    }
    async fn fill(&mut self) -> Result<()> {
        if self.reader.cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        while self.bits.valid < 16 {
            let byte = self
                .reader
                .byte(self.start + self.fetched)
                .await?
                .ok_or_else(|| self.invalid("truncated CCITT stream"))?;
            self.bits.data = (self.bits.data << 8) | u32::from(byte);
            self.bits.valid += 8;
            self.fetched += 1;
        }
        Ok(())
    }
    fn check_code(&self, before: u16, consumed: u64, code: Bits) -> Result<()> {
        // Verify the canonical code as well as the lookup result. In
        // particular, a zero mode-table prefix is not itself an EOFB code.
        if self.bits.consumed - consumed != u64::from(code.len)
            || before >> (16 - code.len) != code.data
        {
            return Err(self.invalid("invalid CCITT Huffman code"));
        }
        Ok(())
    }
    async fn mode(&mut self) -> Result<Mode> {
        self.fill().await?;
        let before = self.bits.peek(16).expect("filled lookahead");
        let consumed = self.bits.consumed;
        let mode =
            maps::mode::decode(&mut self.bits).ok_or_else(|| self.invalid("invalid CCITT mode"))?;
        self.check_code(
            before,
            consumed,
            maps::mode::encode(mode).expect("table value has code"),
        )?;
        Ok(mode)
    }
    async fn run(&mut self, black: bool, maximum: u32) -> Result<u32> {
        let mut total = 0_u32;
        loop {
            self.fill().await?;
            let before = self.bits.peek(16).expect("filled lookahead");
            let consumed = self.bits.consumed;
            let value = if black {
                maps::black::decode(&mut self.bits)
            } else {
                maps::white::decode(&mut self.bits)
            }
            .ok_or_else(|| self.invalid("invalid CCITT run code"))?;
            let code = if black {
                maps::black::encode(value)
            } else {
                maps::white::encode(value)
            }
            .expect("table value has code");
            self.check_code(before, consumed, code)?;
            if u32::from(value) > maximum - total {
                return Err(self.invalid("CCITT run exceeds row width"));
            }
            total += u32::from(value);
            if value < 64 {
                return Ok(total);
            }
        }
    }
}

fn positive(dictionary: &Dictionary, name: &[u8]) -> Option<u32> {
    dictionary
        .value(name)
        .and_then(exact_unsigned)
        .filter(|&n| n > 0 && n <= i32::MAX as u64)
        .map(|n| n as u32)
}

/// Measure an image stream with negative K, unaligned rows and explicit EOFB.
/// All other CCITT profiles remain explicit unsupported cases.
pub(super) async fn extent<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    data_at: u64,
    reference: PdfRef,
    dictionary: &Dictionary,
    dictionary_at: u64,
    decoded_bytes: &mut u64,
) -> Result<u64> {
    let unsupported = || {
        reader.problem(
            data_at,
            Some(reference),
            PdfErrorKind::UnsupportedFeature,
            "indirect CCITT Length requires a bounded Group-4 image with EOFB",
        )
    };
    let width = positive(dictionary, b"Width").ok_or_else(unsupported)?;
    let height = positive(dictionary, b"Height").ok_or_else(unsupported)?;
    let params = dictionary.entry(b"DecodeParms").ok_or_else(unsupported)?;
    let value = params.value(&dictionary.bytes);
    if !value.starts_with(b"<<") {
        return Err(unsupported());
    }
    let (params, _) = reader
        .dictionary_at(
            dictionary_at + params.value.start as u64,
            value.len() as u64,
        )
        .await?;
    let unsupported = || {
        reader.problem(
            data_at,
            Some(reference),
            PdfErrorKind::UnsupportedFeature,
            "indirect CCITT Length requires a bounded Group-4 image with EOFB",
        )
    };
    let negative_k = params
        .value(b"K")
        .and_then(|v| v.strip_prefix(b"-"))
        .and_then(exact_unsigned)
        .is_some_and(|v| v > 0);
    let columns = params
        .value(b"Columns")
        .map(exact_unsigned)
        .unwrap_or(Some(1728));
    let rows = params.value(b"Rows").map(exact_unsigned).unwrap_or(Some(0));
    if !negative_k
        || columns != Some(u64::from(width))
        || !matches!(rows, Some(0)) && rows != Some(u64::from(height))
        || params.value(b"EndOfBlock").is_some_and(|v| v != b"true")
        || params.value(b"EndOfLine").is_some_and(|v| v != b"false")
        || params
            .value(b"EncodedByteAlign")
            .is_some_and(|v| v != b"false")
        || params
            .value(b"DamagedRowsBeforeError")
            .is_some_and(|v| exact_unsigned(v) != Some(0))
    {
        return Err(unsupported());
    }
    let charge = u64::from(width).div_ceil(8) * u64::from(height);
    let attempted = decoded_bytes.saturating_add(charge);
    if attempted > reader.limits.max_output_bytes {
        return Err(Error::LimitExceeded {
            resource: "CAJ decoded scan bytes",
            limit: reader.limits.max_output_bytes,
            attempted,
        });
    }
    *decoded_bytes = attempted;
    let capacity = width as usize + 2;
    let allocation = capacity as u64 * 8;
    reader.limits.check_allocation(allocation)?;
    let mut previous = Vec::<u32>::new();
    let mut current = Vec::<u32>::new();
    for row in [&mut previous, &mut current] {
        let allocation_error = Error::LimitExceeded {
            resource: "CCITT row storage",
            limit: reader.limits.max_allocation_bytes,
            attempted: allocation,
        };
        reserve_exact(row, capacity, allocation_error)?;
    }
    let mut input = Input {
        reader,
        reference,
        start: data_at,
        fetched: 0,
        bits: Buffer::default(),
    };
    for _ in 0..height {
        current.clear();
        let mut a0 = 0;
        let mut black = false;
        let mut first = true;
        while a0 < width {
            let mut index = previous.partition_point(|&x| if first { x < a0 } else { x <= a0 });
            if index % 2 != usize::from(black) {
                index += 1;
            }
            let b1 = previous.get(index).copied().unwrap_or(width);
            let b2 = previous.get(index + 1).copied().unwrap_or(width);
            let mut append = |position| -> std::result::Result<(), &'static str> {
                if position < width {
                    if current.len() == capacity {
                        return Err("too many CCITT row transitions");
                    }
                    current.push(position);
                }
                Ok(())
            };
            match input.mode().await? {
                Mode::Pass => {
                    if b2 <= a0 {
                        return Err(input.invalid("CCITT pass makes no progress"));
                    }
                    a0 = b2;
                }
                Mode::Vertical(delta) => {
                    let next = i64::from(b1) + i64::from(delta);
                    if next < i64::from(a0) || next > i64::from(width) {
                        return Err(input.invalid("CCITT vertical transition is outside row"));
                    }
                    a0 = next as u32;
                    append(a0).map_err(|reason| input.invalid(reason))?;
                    black = !black;
                }
                Mode::Horizontal => {
                    let a1 = a0 + input.run(black, width - a0).await?;
                    let a2 = a1 + input.run(!black, width - a1).await?;
                    append(a1).map_err(|reason| input.invalid(reason))?;
                    append(a2).map_err(|reason| input.invalid(reason))?;
                    a0 = a2;
                }
                _ => return Err(input.invalid("unsupported or early CCITT end code")),
            }
            first = false;
        }
        std::mem::swap(&mut previous, &mut current);
    }
    if !matches!(input.mode().await?, Mode::EOF) {
        return Err(input.invalid("CCITT image lacks EOFB"));
    }
    input.fill().await?;
    if input.bits.peek(12) != Some(1) {
        return Err(input.invalid("CCITT EOFB second code is invalid"));
    }
    input.bits.consume(12).expect("filled lookahead");
    let padding = input.bits.bits_to_byte_boundary();
    if input.bits.peek(padding) != Some(0) {
        return Err(input.invalid("CCITT EOFB padding is nonzero"));
    }
    input.bits.consume(padding).expect("remaining byte bits");
    Ok(input.bits.consumed / 8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        native::SeekableSource,
        test_support::{NEVER, run},
    };
    use std::io::Cursor;

    fn encoded(width: u16, rows: &[Vec<bool>]) -> Vec<u8> {
        let mut encoder = fax::encoder::Encoder::new(fax::VecWriter::new());
        for row in rows {
            encoder
                .encode_line(
                    row.iter().map(|&black| {
                        if black {
                            fax::Color::Black
                        } else {
                            fax::Color::White
                        }
                    }),
                    u32::from(width),
                )
                .unwrap();
        }
        encoder.finish().unwrap().finish()
    }

    fn scan(
        payload: &[u8],
        width: u16,
        height: usize,
        params: &str,
        declared: usize,
        limits: Limits,
    ) -> Result<usize> {
        let mut bytes = format!("1 0 obj\n<< /Width {width} /Height {height} /Filter /CCITTFaxDecode /DecodeParms << {params} >> /Length 2 0 R >>\nstream\n").into_bytes();
        bytes.extend_from_slice(payload);
        bytes.extend_from_slice(
            format!("\nendstream\nendobj\n2 0 obj\n{declared}\nendobj\n").as_bytes(),
        );
        let end = bytes.len() as u64;
        let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
        run(scan_fragment_objects(&mut source, 0, end, &limits, &NEVER))
            .map(|scan| scan.objects.len())
    }

    #[test]
    fn measures_encoded_rows_across_small_read_boundaries() {
        for width in [1, 17, 129, 1728] {
            let rows = vec![
                vec![false; width],
                (0..width).map(|x| x % 7 < 3).collect(),
                vec![true; width],
                vec![false; width],
            ];
            let payload = encoded(width as u16, &rows);
            for chunk in [1, 2, 3, 4096] {
                let params = format!(
                    "/K -1 /Columns {width} /Rows 4 /EndOfBlock true /EndOfLine false /EncodedByteAlign false /DamagedRowsBeforeError 0"
                );
                assert_eq!(
                    scan(
                        &payload,
                        width as u16,
                        rows.len(),
                        &params,
                        payload.len(),
                        Limits {
                            io_chunk_bytes: chunk,
                            ..Limits::default()
                        }
                    )
                    .unwrap(),
                    2
                );
            }
        }
    }

    #[test]
    fn rejects_unsupported_profiles_and_wrong_lengths() {
        let payload = encoded(17, &[vec![false; 17]]);
        for params in [
            "/K 0 /Columns 17",
            "/K -1 /Columns 18",
            "/K -1 /Columns 17 /Rows 2",
            "/K -1 /Columns 17 /EndOfBlock false",
            "/K -1 /Columns 17 /EndOfLine true",
            "/K -1 /Columns 17 /EncodedByteAlign true",
            "/K -1 /Columns 17 /DamagedRowsBeforeError 1",
        ] {
            assert!(
                scan(&payload, 17, 1, params, payload.len(), Limits::default()).is_err(),
                "{params}"
            );
        }
        assert!(
            scan(
                &payload,
                17,
                1,
                "/K -1 /Columns 17",
                payload.len() + 1,
                Limits::default()
            )
            .is_err()
        );
        assert!(
            scan(
                &payload,
                17,
                0,
                "/K -1 /Columns 17",
                payload.len(),
                Limits::default()
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_truncation_marker_bytes_and_nonzero_padding() {
        let payload = encoded(17, &[vec![false; 17]]);
        for length in 0..payload.len() {
            assert!(
                scan(
                    &payload[..length],
                    17,
                    1,
                    "/K -1 /Columns 17",
                    length,
                    Limits::default()
                )
                .is_err()
            );
        }
        assert!(
            scan(
                b"endstream\nendobj",
                17,
                1,
                "/K -1 /Columns 17",
                16,
                Limits::default()
            )
            .is_err()
        );
        let mut bad = payload.clone();
        *bad.last_mut().unwrap() |= 1;
        assert!(
            scan(
                &bad,
                17,
                1,
                "/K -1 /Columns 17",
                bad.len(),
                Limits::default()
            )
            .is_err()
        );
    }

    #[test]
    fn bounds_decoded_work_and_transition_storage() {
        let payload = encoded(4096, &[vec![false; 4096]]);
        for limits in [
            Limits {
                max_output_bytes: 511,
                ..Limits::default()
            },
            Limits {
                max_allocation_bytes: 8192,
                ..Limits::default()
            },
        ] {
            assert!(matches!(
                scan(
                    &payload,
                    4096,
                    1,
                    "/K -1 /Columns 4096",
                    payload.len(),
                    limits
                ),
                Err(Error::LimitExceeded { .. })
            ));
        }
    }
    #[test]
    fn rejects_invalid_modes_runs_and_excess_transitions() {
        // Original bit patterns: zero prefix, extension, early EOFB,
        // vertical-right outside a one-pixel row, and repeated zero-width
        // vertical transitions. None may be accepted as framed image data.
        for bits in [
            "0000000000000000",
            "0000001",
            "000000000001",
            "011",
            "010010010010",
            "0010000000000000000",
        ] {
            let mut payload = vec![0_u8; bits.len().div_ceil(8) + 4];
            for (index, bit) in bits.bytes().enumerate() {
                if bit == b'1' {
                    payload[index / 8] |= 1 << (7 - index % 8);
                }
            }
            assert!(
                scan(
                    &payload,
                    1,
                    1,
                    "/K -1 /Columns 1",
                    payload.len(),
                    Limits::default()
                )
                .is_err(),
                "{bits}"
            );
        }
    }
    #[test]
    fn bit_reader_checks_bounds_and_cancellation_before_cached_reads() {
        let mut bits = Buffer::default();
        assert_eq!(bits.peek(1), None);
        assert_eq!(bits.peek(17), None);
        assert!(bits.consume(1).is_err());
        bits.valid = 1;
        bits.consumed = u64::MAX;
        assert!(bits.consume(1).is_err());

        struct Cancelled;
        impl Cancellation for Cancelled {
            fn is_cancelled(&self) -> bool {
                true
            }
        }
        let mut source = SeekableSource::new(Cursor::new(vec![0; 4])).unwrap();
        let limits = Limits::default();
        let mut reader = Reader::new(
            &mut source,
            PdfRange {
                offset: 0,
                length: 4,
            },
            &limits,
            &Cancelled,
        )
        .unwrap();
        let mut input = Input {
            reader: &mut reader,
            reference: PdfRef {
                number: 1,
                generation: 0,
            },
            start: 0,
            fetched: 0,
            bits: Buffer {
                data: 0,
                valid: 16,
                consumed: 0,
            },
        };
        assert!(matches!(run(input.fill()), Err(Error::Cancelled)));
    }
    #[test]
    fn rejects_overlong_runs_and_stalled_passes() {
        for (bits, rows) in [("0010111", 1), ("01001010001", 2)] {
            let mut payload = vec![0_u8; bits.len().div_ceil(8) + 4];
            for (index, bit) in bits.bytes().enumerate() {
                if bit == b'1' {
                    payload[index / 8] |= 1 << (7 - index % 8);
                }
            }
            let error = scan(
                &payload,
                1,
                rows,
                "/K -1 /Columns 1",
                payload.len(),
                Limits::default(),
            )
            .unwrap_err();
            let message = error.to_string();
            assert!(
                message.contains(if rows == 1 {
                    "run exceeds row width"
                } else {
                    "pass makes no progress"
                }),
                "{message}"
            );
        }
    }

    #[test]
    fn rejects_indirect_decode_parameters() {
        let bytes = b"1 0 obj\n<< /Width 1 /Height 1 /Filter /CCITTFaxDecode /DecodeParms 3 0 R /Length 2 0 R >>\nstream\ninvalid\nendstream\nendobj".to_vec();
        let end = bytes.len() as u64;
        let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
        let error = run(scan_fragment_objects(
            &mut source,
            0,
            end,
            &Limits::default(),
            &NEVER,
        ))
        .err()
        .unwrap();
        assert!(
            error
                .to_string()
                .contains("requires a bounded Group-4 image")
        );
    }
}
