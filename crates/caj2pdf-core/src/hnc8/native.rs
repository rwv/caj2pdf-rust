// SPDX-License-Identifier: MIT

//! Framing of the independently observed raw C8 native-page subset.
//! Events preserve uninterpreted words; they do not imply renderability.

use super::{ErrorKind, Hnc8Reader, Location, Result, TextBudget, Variant, read_fixed};
use crate::{Cancellation, RangedSource};

/// One framed native record. Units, style/font words and image words remain raw.
/// Unknown required semantics must be rejected by a renderer, never discarded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeRecord {
    /// A position, style or observed four-byte control, with its original tag.
    Control { tag: u16, value: u16 },
    /// A glyph code with the current run context. This is not Unicode.
    Glyph {
        x: u16,
        y: u16,
        style: u16,
        code: u16,
    },
    /// Two coordinate pairs with a checked, variant-specific record boundary.
    /// Neither stroke style nor physical geometry is assigned here. In particular,
    /// the observed `8010/1` form does not imply a visible `8006` stroke.
    Drawing {
        tag: u16,
        style: u16,
        points: [[u16; 2]; 2],
    },
    /// Thirteen words after the image marker. Do not apply HN-A coordinates.
    Image { words: [u16; 13] },
    /// The final record, including its uninterpreted payload.
    End { value: u16 },
}

/// Decode the admitted C8 native character subset without allocating.
///
/// Ordinary codes use their big-endian two-byte GB18030 value. The independently
/// verified A0-prefixed letters/digits use ASCII plus 0x80 in the low byte.
/// Three independently observed symbol codes have explicit Unicode mappings.
/// Other A0 codes, private-use mappings and malformed sequences return `None`;
/// they must remain explicit unsupported glyphs rather than blank substitutions.
/// This maps characters only: fonts, metrics, drawing/text order and complete
/// native page rendering still require separate validation.
pub fn decode_native_character(code: u16) -> Option<char> {
    match code {
        0xa0a6 => return Some('＆'),
        0xaab3 => return Some('∗'),
        0xaca3 => return Some('►'),
        _ => {}
    }
    let [lead, second] = code.to_be_bytes();
    if lead == 0xa0 {
        let ascii = second.checked_sub(0x80)?;
        return ascii.is_ascii_alphanumeric().then(|| char::from(ascii));
    }
    crate::gb18030::decode_two_byte(lead, second)
        .filter(|c| !c.is_control() && !(0xe000..=0xf8ff).contains(&u32::from(*c)))
}

/// Decode only the established image-coordinate fields of the raw C8 profile.
///
/// `words` is the payload of [`NativeRecord::Image`]. The observed `d300`
/// profile stores x and width with `c000` high bits; y and height are unsigned
/// words. Unknown prefixes and zero extents return `None`. Other payload words
/// remain uninterpreted: this helper does not approve their rendering semantics.
///
/// Coordinates are absolute source units. Subtract [`super::Header::native_origin`]
/// in signed or floating-point arithmetic, without a text-specific margin.
/// Image row orientation depends on the decoded representation; this helper
/// neither flips rows nor supplies a universal PDF image transform.
pub fn decode_native_image_coordinate(words: &[u16; 13]) -> Option<super::RawTextCoordinate> {
    if words[0] != 0xd300 || words[1] & 0xc000 != 0xc000 || words[3] & 0xc000 != 0xc000 {
        return None;
    }
    let width = words[3] & 0x3fff;
    if width == 0 || words[4] == 0 {
        return None;
    }
    Some(super::RawTextCoordinate {
        x: words[1] & 0x3fff,
        y: words[2],
        width,
        height: words[4],
    })
}

/// Receives one record at a time in source order, including all known controls.
/// A callback is awaited before the next record is read. Events delivered before
/// a later error are an incomplete prefix and must not be published as a page.
#[allow(async_fn_in_trait)]
pub trait NativeRecordVisitor {
    async fn visit(&mut self, offset: u64, record: NativeRecord) -> crate::Result<()>;
}

impl<S: RangedSource, C: Cancellation> Hnc8Reader<'_, S, C> {
    /// Visit the current C8 page's observed raw native records without allocating.
    /// Call `next_page` first. This does not consume image descriptors, decode
    /// characters or enable conversion. Unknown framing stops at its source byte.
    /// A failed/dropped operation poisons the reader, just like image traversal.
    ///
    /// `TextBudget` caps span bytes, records, images and fixed working storage.
    /// Raw span bytes also count against its decoded-byte ceiling. The 4 KiB
    /// reservation accounts for fixed parser state, not process/visitor memory.
    pub async fn visit_native_records<V: NativeRecordVisitor>(
        &mut self,
        budget: TextBudget,
        visitor: &mut V,
    ) -> Result<u32> {
        let loc = Location {
            variant: Some(self.header.variant),
            offset: self
                .current
                .map_or(self.header.page_index.offset, |p| p.page.text.offset),
            page: self.current.map(|p| p.page.page_number),
            image: None,
        };
        if self.poisoned {
            return Err(loc.error(ErrorKind::Poisoned));
        }
        let page = self
            .current
            .ok_or_else(|| loc.error(ErrorKind::NoCurrentPage))?
            .page;
        self.poisoned = true;
        let result = async {
            if self.header.variant != Variant::C8 {
                return Err(loc.error(ErrorKind::Unsupported {
                    field: "native record variant",
                    value: 0,
                }));
            }
            if self.cancellation.is_cancelled() {
                return Err(loc.error(ErrorKind::Cancelled));
            }
            // Metadata came from this cursor, so its span and counts are checked.
            // Unlike compressed text, a native page can be just one end record.
            if page.text.offset < self.header.page_index.checked_end().expect("checked index") {
                return Err(loc.malformed("native text span", "overlaps protected container index"));
            }
            for (resource, limit, attempted) in [
                (
                    "native text bytes",
                    budget.max_span_bytes.min(budget.max_decoded_bytes),
                    page.text.length,
                ),
                (
                    "native text images",
                    u64::from(budget.max_images),
                    u64::from(page.image_count),
                ),
                (
                    "native parser working bytes",
                    budget.max_working_bytes,
                    4096,
                ),
            ] {
                if attempted > limit {
                    return Err(loc.limit(resource, limit, attempted));
                }
            }
            let end = page.text.checked_end().expect("checked text span");
            let mut position = page.text.offset;
            let mut count = 0;
            let mut images = 0;
            let (mut y, mut style) = (None, None);
            while position < end {
                let at = loc.at(position);
                if count == budget.max_records {
                    return Err(at.limit(
                        "native records",
                        u64::from(budget.max_records),
                        u64::from(count) + 1,
                    ));
                }
                let mut bytes = [0_u8; 28];
                self.native_bytes(position, end, &mut bytes[..4], at)
                    .await?;
                let tag = word(&bytes[..2]);
                let value = word(&bytes[2..4]);
                let mut length = 4;
                let record = match tag {
                    0x8001 => {
                        y = Some(value);
                        NativeRecord::Control { tag, value }
                    }
                    0x8002 => {
                        style = Some(value);
                        NativeRecord::Control { tag, value }
                    }
                    0x801d if matches!(value, 0 | 4) => NativeRecord::Control { tag, value },
                    0x8067 if matches!(value, 5 | 6 | 8 | 9) => {
                        NativeRecord::Control { tag, value }
                    }
                    0x8072..=0x8074 | 0xc053 | 0xc054 => NativeRecord::Control { tag, value },
                    0x8006 | 0x8010
                        if matches!(
                            (tag, value),
                            (0x8006, 0xa381 | 0xa383 | 0xa38b) | (0x8010, 1)
                        ) =>
                    {
                        length = if value == 0xa383 { 12 } else { 16 };
                        self.native_bytes(position + 4, end, &mut bytes[4..length], at)
                            .await?;
                        if length == 16 && bytes[12..16] != [0xff, 0xff, 5, 0] {
                            return Err(at
                                .at(position + 12)
                                .malformed("native drawing end", "expected ffff/0005"));
                        }
                        NativeRecord::Drawing {
                            tag,
                            style: value,
                            points: [
                                [word(&bytes[4..6]), word(&bytes[6..8])],
                                [word(&bytes[8..10]), word(&bytes[10..12])],
                            ],
                        }
                    }
                    0x800a if value == 0xd300 => {
                        length = 28;
                        self.native_bytes(position + 4, end, &mut bytes[4..length], at)
                            .await?;
                        if images == page.image_count {
                            return Err(
                                at.malformed("native image records", "exceed declared image count")
                            );
                        }
                        images += 1;
                        let mut words = [0; 13];
                        for (out, pair) in words.iter_mut().zip(bytes[2..].as_chunks::<2>().0) {
                            *out = word(pair);
                        }
                        NativeRecord::Image { words }
                    }
                    0x8004 => {
                        if position + 4 != end {
                            return Err(at.malformed(
                                "native page end",
                                "trailing bytes in indexed text span",
                            ));
                        }
                        if images != page.image_count {
                            return Err(at.malformed(
                                "native image records",
                                "differ from declared image count",
                            ));
                        }
                        NativeRecord::End { value }
                    }
                    x if x < 0x8000 => {
                        let (Some(y), Some(style)) = (y, style) else {
                            return Err(
                                at.malformed("native glyph", "missing run position or style")
                            );
                        };
                        NativeRecord::Glyph {
                            x,
                            y,
                            style,
                            code: value,
                        }
                    }
                    _ => {
                        return Err(at.error(ErrorKind::Unsupported {
                            field: "native record tag/value",
                            value: (u64::from(tag) << 16) | u64::from(value),
                        }));
                    }
                };
                visitor.visit(position, record).await.map_err(|source| {
                    at.error(ErrorKind::Source {
                        field: "native record visitor",
                        source,
                    })
                })?;
                if self.cancellation.is_cancelled() {
                    return Err(at.error(ErrorKind::Cancelled));
                }
                count += 1;
                position += length as u64;
                if matches!(record, NativeRecord::End { .. }) {
                    return Ok(count);
                }
            }
            Err(loc
                .at(end)
                .malformed("native page end", "missing end record"))
        }
        .await;
        if result.is_ok() {
            self.poisoned = false;
        }
        result
    }

    async fn native_bytes(
        &mut self,
        offset: u64,
        end: u64,
        bytes: &mut [u8],
        loc: Location,
    ) -> Result<()> {
        // Every requested offset is within the already checked page interval.
        if bytes.len() as u64 > end - offset {
            return Err(loc.at(offset).error(ErrorKind::Truncated {
                field: "native record",
                expected: bytes.len() as u64,
                available: end - offset,
            }));
        }
        read_fixed(
            self.source,
            self.limits,
            self.cancellation,
            offset,
            bytes,
            loc.at(offset),
            "native record",
        )
        .await
    }
}

fn word(bytes: &[u8]) -> u16 {
    u16::from_le_bytes([bytes[0], bytes[1]])
}

#[cfg(test)]
mod tests;
