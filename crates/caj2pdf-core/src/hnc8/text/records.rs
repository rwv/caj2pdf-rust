// SPDX-License-Identifier: MIT

//! Records inside the directly prefixed COMPRESSTEXT frame. Only image
//! coordinates are retained; glyph/control payload words and the indexed
//! opaque tail after the terminator are discarded, while still decompressed
//! and checksummed by the caller.

use super::*;

pub(super) struct Records {
    bytes: [u8; 28],
    filled: usize,
    needed: usize,
    images: usize,
    count: u32,
    limit: u32,
    ended: bool,
    page_prefix: bool,
    decode_raw_hna_markers: bool,
}

impl Records {
    pub(super) fn new(limit: u32) -> Self {
        Self {
            bytes: [0; 28],
            filled: 0,
            needed: 4,
            images: 0,
            count: 0,
            limit,
            ended: false,
            page_prefix: false,
            decode_raw_hna_markers: false,
        }
    }

    pub(super) fn with_page_prefix(limit: u32) -> Self {
        Self {
            page_prefix: true,
            ..Self::new(limit)
        }
    }

    /// The verified raw HN-A profile decodes image marker bits; compressed
    /// and unverified profiles preserve raw words.
    pub(super) fn decode_raw_hna_markers(mut self) -> Self {
        self.decode_raw_hna_markers = true;
        self
    }

    pub(super) fn consume(
        &mut self,
        bytes: &[u8],
        coordinates: &mut [RawTextCoordinate],
        loc: Location,
    ) -> Result<()> {
        for &byte in bytes {
            if self.ended {
                break;
            }
            self.bytes[self.filled] = byte;
            self.filled += 1;
            if self.filled != self.needed {
                continue;
            }
            if self.needed == 28 {
                coordinates[self.images] = RawTextCoordinate {
                    x: u16::from_le_bytes([self.bytes[4], self.bytes[5]]),
                    y: u16::from_le_bytes([self.bytes[6], self.bytes[7]]),
                    width: u16::from_le_bytes([self.bytes[8], self.bytes[9]]),
                    height: u16::from_le_bytes([self.bytes[10], self.bytes[11]]),
                };
                let coordinate = &mut coordinates[self.images];
                if self.decode_raw_hna_markers {
                    decode_image_markers(
                        self.bytes[..4].try_into().expect("fixed marker width"),
                        coordinate,
                    );
                }
                self.images += 1;
            } else {
                if self.count == self.limit {
                    return Err(loc.limit(
                        "text records",
                        u64::from(self.limit),
                        u64::from(self.count) + 1,
                    ));
                }
                self.count += 1;
                let tag = u16::from_le_bytes([self.bytes[0], self.bytes[1]]);
                match tag {
                    0x8003 if self.page_prefix && self.count <= 2 => (),
                    // Only observed in this raw profile, with a zero payload.
                    // No glyph semantics are assigned to the control.
                    0x80ce if self.page_prefix && self.bytes[2..4] == [0, 0] => (),
                    0x800a => {
                        if self.images == coordinates.len() {
                            return Err(loc.malformed(
                                "decoded image records",
                                "more image records than source descriptors",
                            ));
                        }
                        self.needed = 28;
                        continue;
                    }
                    0x8004 => self.ended = true,
                    0..=0x7fff | 0x8001 | 0x801c | 0x801d | 0x80ff | 0x8070 | 0x8071 => (),
                    _ => return Err(loc.malformed("decoded text record", "unknown control tag")),
                }
            }
            self.filled = 0;
            self.needed = 4;
        }
        Ok(())
    }

    pub(super) fn finish(
        self,
        coordinates: &mut Vec<RawTextCoordinate>,
        loc: Location,
    ) -> Result<u32> {
        if !self.ended {
            return Err(loc.malformed("decoded text records", "missing complete terminator"));
        }
        coordinates.truncate(self.images);
        Ok(self.count)
    }
}

/// Decode the independently verified HN-A image marker, only for composition.
/// Other record prefixes and partial marker patterns retain their raw values.
pub(super) fn decode_image_markers(marker: [u8; 4], coordinate: &mut RawTextCoordinate) {
    if marker == [0x0a, 0x80, 0, 0xd3]
        && coordinate.x & 0xc000 == 0xc000
        && coordinate.width & 0xc000 == 0xc000
    {
        coordinate.x &= 0x3fff;
        coordinate.width &= 0x3fff;
    }
}
