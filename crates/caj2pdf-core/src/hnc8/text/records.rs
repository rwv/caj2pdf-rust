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
        }
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
                };
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
                match u16::from_le_bytes([self.bytes[0], self.bytes[1]]) {
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

    pub(super) fn finish(self, images: usize, loc: Location) -> Result<u32> {
        if !self.ended {
            return Err(loc.malformed("decoded text records", "missing complete terminator"));
        }
        if self.images != images {
            return Err(loc.malformed(
                "decoded image records",
                "image count differs from source descriptors",
            ));
        }
        Ok(self.count)
    }
}
