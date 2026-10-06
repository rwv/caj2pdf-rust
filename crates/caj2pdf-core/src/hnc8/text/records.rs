// SPDX-License-Identifier: MIT

//! One streaming parser for every page-text record layout. Only image
//! coordinates are retained; glyph/control payload words and any indexed
//! opaque tail after the terminator are discarded, while still read (and,
//! when compressed, decompressed and checksummed) by the caller.

use super::*;

/// Where the parser is in the ordered uncompressed HN-A record sequence.
#[derive(Clone, Copy)]
enum Stage {
    Start,
    X,
    Y,
    Glyphs,
    Images,
    End,
}

#[derive(Clone, Copy)]
enum Grammar {
    /// Uncompressed HN-A records in the observed order: `8001`, `8070`,
    /// `8071`, glyph words, image records, then `8004`.
    Ordered(Stage),
    /// Four-byte tagged records of the direct `COMPRESSTEXT` frame or of
    /// raw text after the paired `8003` prefix, ending at `8004`.
    Tagged { page_prefix: bool },
    /// The 24-byte legacy header's fixed layout: 16-byte glyph records from
    /// decoded offset 8 to `glyph_end`, then 28-byte image records whose
    /// four-byte markers start at `tail_start - 4`.
    Fixed { glyph_end: u64, tail_start: u64 },
}

/// The next step after one four-byte record.
enum Next {
    Record,
    Image,
    End,
}

pub(super) struct Records {
    grammar: Grammar,
    bytes: [u8; 28],
    filled: usize,
    needed: usize,
    /// The offset of the record being filled.
    start: u64,
    /// The offset after the last consumed input.
    end: u64,
    images: usize,
    count: u32,
    limit: u32,
    ended: bool,
    decode_markers: bool,
}

impl Records {
    fn with(grammar: Grammar, limit: u32) -> Self {
        Self {
            grammar,
            bytes: [0; 28],
            filled: 0,
            needed: 4,
            start: 0,
            end: 0,
            images: 0,
            count: 0,
            limit,
            ended: false,
            decode_markers: false,
        }
    }

    /// Ordered uncompressed HN-A records. Errors are located at the record.
    pub(super) fn ordered() -> Self {
        Self::with(Grammar::Ordered(Stage::Start), u32::MAX)
    }

    /// Tagged records, accepting the paired `8003` page prefix and the `80ce`
    /// control of the raw profile when `page_prefix` is set. Errors are
    /// located at the frame start: a compressed byte offset cannot identify
    /// the corresponding expanded record byte.
    pub(super) fn tagged(page_prefix: bool) -> Self {
        Self::with(Grammar::Tagged { page_prefix }, u32::MAX)
    }

    /// The fixed glyph/tail layout of `record_count` checked glyph records.
    pub(super) fn fixed(glyph_end: u64, tail_start: u64, record_count: u32) -> Self {
        let grammar = Grammar::Fixed {
            glyph_end,
            tail_start,
        };
        Self {
            count: record_count,
            ..Self::with(grammar, record_count)
        }
    }

    /// The verified HN-A profiles decode image marker bits; the others
    /// preserve raw words.
    pub(super) fn decode_markers(mut self, decode: bool) -> Self {
        self.decode_markers = decode;
        self
    }

    /// Consume the text bytes at `offset`, a source offset for raw text and
    /// a decoded offset for compressed text. `loc` locates the frame.
    pub(super) fn consume(
        &mut self,
        offset: u64,
        bytes: &[u8],
        coordinates: &mut [RawTextCoordinate],
        loc: Location,
    ) -> Result<()> {
        self.end = offset + len_u64(bytes.len());
        if let Grammar::Fixed {
            glyph_end,
            tail_start,
        } = self.grammar
        {
            return self.consume_fixed(offset, bytes, coordinates, loc, glyph_end, tail_start);
        }
        for (index, &byte) in bytes.iter().enumerate() {
            if self.ended {
                break; // Remaining indexed bytes are opaque, but still read and hashed.
            }
            if self.filled == 0 {
                self.start = offset + len_u64(index);
            }
            self.bytes[self.filled] = byte;
            self.filled += 1;
            if self.filled != self.needed {
                continue;
            }
            let at = match self.grammar {
                Grammar::Ordered(_) => loc.at(self.start),
                _ => loc,
            };
            if self.needed == 28 {
                let mut coordinate = RawTextCoordinate {
                    x: u16::from_le_bytes([self.bytes[4], self.bytes[5]]),
                    y: u16::from_le_bytes([self.bytes[6], self.bytes[7]]),
                    width: u16::from_le_bytes([self.bytes[8], self.bytes[9]]),
                    height: u16::from_le_bytes([self.bytes[10], self.bytes[11]]),
                };
                if self.decode_markers {
                    let marker = self.bytes[..4].try_into().expect("fixed marker width");
                    decode_image_markers(marker, &mut coordinate);
                }
                coordinates[self.images] = coordinate;
                self.images += 1;
                if let Grammar::Ordered(stage) = &mut self.grammar {
                    *stage = if self.images == coordinates.len() {
                        Stage::End
                    } else {
                        Stage::Images
                    };
                }
            } else {
                if self.count == self.limit {
                    return Err(at.limit(
                        "text records",
                        u64::from(self.limit),
                        u64::from(self.count) + 1,
                    ));
                }
                self.count += 1;
                match self.next(coordinates.len()) {
                    Ok(Next::Record) => (),
                    Ok(Next::Image) => {
                        self.needed = 28;
                        continue;
                    }
                    Ok(Next::End) => self.ended = true,
                    Err(reason) => return Err(at.malformed(reason)),
                }
            }
            self.filled = 0;
            self.needed = 4;
        }
        Ok(())
    }

    /// Classify the completed four-byte record against the grammar.
    fn next(&mut self, descriptors: usize) -> std::result::Result<Next, &'static str> {
        let tag = u16::from_le_bytes([self.bytes[0], self.bytes[1]]);
        let images_left = self.images < descriptors;
        match &mut self.grammar {
            Grammar::Ordered(stage) => {
                *stage = match (*stage, tag) {
                    (Stage::Start | Stage::Glyphs, 0x8001) => Stage::X,
                    (Stage::X, 0x8070) => Stage::Y,
                    (Stage::Y, 0x8071) => Stage::Glyphs,
                    (Stage::Glyphs, 0..=0x7fff) => Stage::Glyphs,
                    (Stage::Glyphs | Stage::Images, 0x800a) if images_left => {
                        return Ok(Next::Image);
                    }
                    (Stage::Start | Stage::Glyphs | Stage::End, 0x8004) if !images_left => {
                        return Ok(Next::End);
                    }
                    _ => return Err("raw text record: unexpected tag or record order"),
                };
                Ok(Next::Record)
            }
            &mut Grammar::Tagged { page_prefix } => match tag {
                0x8003 if page_prefix && self.count <= 2 => Ok(Next::Record),
                // Only observed in the raw profile, with a zero payload.
                // No glyph semantics are assigned to the control.
                0x80ce if page_prefix && self.bytes[2..4] == [0, 0] => Ok(Next::Record),
                0x800a if images_left => Ok(Next::Image),
                0x800a => Err("decoded image records: more image records than source descriptors"),
                0x8004 => Ok(Next::End),
                0..=0x7fff | 0x8001 | 0x801c | 0x801d | 0x80ff | 0x8070 | 0x8071 => {
                    Ok(Next::Record)
                }
                _ => Err("decoded text record: unknown control tag"),
            },
            Grammar::Fixed { .. } => unreachable!("the fixed layout is positional"),
        }
    }

    /// Check the fixed glyph markers byte by byte and retain each image
    /// record's marker and coordinate across chunk boundaries.
    fn consume_fixed(
        &mut self,
        offset: u64,
        bytes: &[u8],
        coordinates: &mut [RawTextCoordinate],
        loc: Location,
        glyph_end: u64,
        tail_start: u64,
    ) -> Result<()> {
        for (index, byte) in bytes.iter().copied().enumerate() {
            let at = offset + len_u64(index);
            if at >= 8 && at < glyph_end {
                let within = (at - 8) % 16;
                let expected = match within {
                    0 | 1 => Some(0x8070_u16.to_le_bytes()[within as usize]),
                    4 | 5 => Some(0x8071_u16.to_le_bytes()[(within - 4) as usize]),
                    8 | 9 => Some(0x8001_u16.to_le_bytes()[(within - 8) as usize]),
                    _ => None,
                };
                if expected.is_some_and(|expected| expected != byte) {
                    return Err(
                        loc.malformed("decoded text marker: differs from observed record marker")
                    );
                }
            } else if at >= tail_start - 4 {
                let relative = at - (tail_start - 4);
                let within = relative % 28;
                if within < 4 {
                    self.bytes[within as usize] = byte;
                } else if within < 12 {
                    let coordinate = &mut coordinates[(relative / 28) as usize];
                    let word = match (within - 4) / 2 {
                        0 => &mut coordinate.x,
                        1 => &mut coordinate.y,
                        2 => &mut coordinate.width,
                        _ => &mut coordinate.height,
                    };
                    *word |= u16::from(byte) << ((within % 2) * 8);
                    if within == 11 && self.decode_markers {
                        let marker = self.bytes[..4].try_into().expect("fixed marker width");
                        decode_image_markers(marker, coordinate);
                    }
                }
            }
        }
        Ok(())
    }

    /// The record count once the text is complete. Unused descriptors drop
    /// their coordinates; `loc` locates the frame.
    pub(super) fn finish(
        self,
        coordinates: &mut Vec<RawTextCoordinate>,
        loc: Location,
    ) -> Result<u32> {
        match self.grammar {
            Grammar::Fixed { .. } => return Ok(self.count),
            Grammar::Ordered(_) if !self.ended => {
                return Err(loc
                    .at(self.end)
                    .malformed("raw text records: missing image records or end marker"));
            }
            Grammar::Tagged { .. } if !self.ended => {
                return Err(loc.malformed("decoded text records: missing complete terminator"));
            }
            _ => (),
        }
        coordinates.truncate(self.images);
        Ok(self.count)
    }
}

/// Decode the independently verified HN-A image marker, only for composition.
/// Other record prefixes and partial marker patterns retain their raw values.
fn decode_image_markers(marker: [u8; 4], coordinate: &mut RawTextCoordinate) {
    if marker == [0x0a, 0x80, 0, 0xd3]
        && coordinate.x & 0xc000 == 0xc000
        && coordinate.width & 0xc000 == 0xc000
    {
        coordinate.x &= 0x3fff;
        coordinate.width &= 0x3fff;
    }
}
