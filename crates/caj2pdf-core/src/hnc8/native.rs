// SPDX-License-Identifier: MIT

//! Framing of independently observed raw C8 and HN-B native-page subsets.
//! Events preserve uninterpreted words; they do not imply renderability.

use super::{ErrorKind, Hnc8Reader, Location, Result, TextBudget, Variant, read_fixed};
use crate::{Cancellation, RangedSource};

/// One framed native record. Units, style/font words and image words remain raw.
/// Unknown required semantics must be rejected by a renderer, never discarded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeRecord {
    /// A position, style or observed four-byte control, with its original tag.
    Control { tag: u16, value: u16 },
    /// An observed eight-byte control (`81ff/1..=3`, `80cc/0204`,
    /// or the HN-B `c052/a385` prefix).
    /// The two payload words are atomic and uninterpreted. Preserving their
    /// framing does not establish font, layout or resource semantics.
    ExtendedControl {
        tag: u16,
        value: u16,
        words: [u16; 2],
    },
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
    /// The observed `810a/d300` image form with zero flags. Coordinates are
    /// absolute source units without the older image form's high-bit markers.
    /// `reference` locates the raw name bytes, excluding NUL and padding.
    /// It is not a filesystem resource request. Match embedded descriptors in
    /// record order; image orientation remains codec-specific.
    ImageReference {
        coordinate: super::RawTextCoordinate,
        reference: super::Span,
    },
    /// The observed `80cc/01xx` encoded-string record. The low byte of
    /// `value` counts all words, including the two-word header. `payload`
    /// locates 0..=253 validated `e020..=e07e` words in the original source.
    /// Its role is deliberately uninterpreted; this is not visible page text
    /// or permission to discard a required resource reference.
    EncodedString { value: u16, payload: super::Span },
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
    /// Visit admitted C8 or HN-B raw native records without allocating.
    /// HN-B admits independently controlled glyph runs, controls, three drawing
    /// forms and fixed-length image records. Other C8 framing is not inherited.
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
            if !matches!(self.header.variant, Variant::C8 | Variant::HnB) {
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
                if self.header.variant == Variant::HnB
                    && tag >= 0x8000
                    && !matches!(
                        (tag, value),
                        (0x8001 | 0x8002 | 0x8004, _)
                            | (0x801d, 0 | 3 | 4)
                            | (0x801c, 4)
                            | (0x8067, 5 | 6 | 7 | 9)
                            | (0x8069, 0x1084)
                            | (0x80ce, 0 | 1)
                            | (0x8070 | 0x8071, 0x0024 | 0x002b)
                            | (0x8070, 0x001c)
                            | (0x8072, 0 | 0x1084 | 0xc2c7 | 0xcdc1)
                            | (0x8074, 0xb7bd | 0xcfc8)
                            | (0x8073, 0x001e | 0x001f | 0x0029 | 0x002a)
                            | (0x8024, 0x2800 | 0x281d)
                            | (0xc053, _)
                            | (0xffff, 5)
                            | (0x8006, 0xa381 | 0xa383 | 0xa385)
                            | (0xc052, 0xa385)
                            | (0x800a, 0xd300)
                    )
                {
                    return Err(at.error(ErrorKind::Unsupported {
                        field: "HN-B native record tag/value",
                        value: (u64::from(tag) << 16) | u64::from(value),
                    }));
                }
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
                    0x801d if matches!(value, 0 | 3 | 4) => NativeRecord::Control { tag, value },
                    0x8067
                        if matches!(value, 5 | 6 | 8 | 9)
                            || (self.header.variant == Variant::HnB && value == 7) =>
                    {
                        NativeRecord::Control { tag, value }
                    }
                    0x801c | 0x8070 | 0x8071 if value == 4 => NativeRecord::Control { tag, value },
                    0x80ce if value <= 1 => NativeRecord::Control { tag, value },
                    0x8024 if matches!(value, 0x2800 | 0x281d) => {
                        NativeRecord::Control { tag, value }
                    }
                    // Values were checked by the HN-B profile guard above.
                    0x8069 | 0x8070 | 0x8071 | 0x8072 | 0x8073 | 0xc053 | 0xffff
                        if self.header.variant == Variant::HnB =>
                    {
                        NativeRecord::Control { tag, value }
                    }
                    0x8021 if value == 0x2000 => NativeRecord::Control { tag, value },
                    0x80d0 | 0x80d2 if value == 0 => NativeRecord::Control { tag, value },
                    0x80d1 if value == 1 => NativeRecord::Control { tag, value },
                    0x81ff | 0x80cc | 0xc052
                        if matches!((tag, value), (0x81ff, 1..=3) | (0x80cc, 0x0204))
                            || (self.header.variant == Variant::HnB
                                && (tag, value) == (0xc052, 0xa385)) =>
                    {
                        length = 8;
                        self.native_bytes(position + 4, end, &mut bytes[4..8], at)
                            .await?;
                        NativeRecord::ExtendedControl {
                            tag,
                            value,
                            words: [word(&bytes[4..6]), word(&bytes[6..8])],
                        }
                    }
                    0x80cc if (0x0102..=0x01ff).contains(&value) => {
                        length = usize::from(value & 0xff) * 2;
                        let mut consumed = 4;
                        while consumed < length {
                            let count = (length - consumed).min(bytes.len());
                            self.native_bytes(
                                position + consumed as u64,
                                end,
                                &mut bytes[..count],
                                at,
                            )
                            .await?;
                            for (index, pair) in
                                bytes[..count].as_chunks::<2>().0.iter().enumerate()
                            {
                                if !(0xe020..=0xe07e).contains(&word(pair)) {
                                    return Err(at
                                        .at(position + (consumed + index * 2) as u64)
                                        .error(ErrorKind::Unsupported {
                                            field: "native encoded-string word",
                                            value: u64::from(word(pair)),
                                        }));
                                }
                            }
                            consumed += count;
                        }
                        NativeRecord::EncodedString {
                            value,
                            payload: super::Span {
                                offset: position + 4,
                                length: length as u64 - 4,
                            },
                        }
                    }
                    0x8072..=0x8074 | 0xc053 | 0xc054 => NativeRecord::Control { tag, value },
                    0xffff if value == 5 => NativeRecord::Control { tag, value },
                    0x8006 | 0x8010
                        if matches!(
                            (tag, value),
                            (0x8006, 0xa381 | 0xa383 | 0xa385 | 0xa38b) | (0x8010, 1)
                        ) =>
                    {
                        length = 12;
                        self.native_bytes(position + 4, end, &mut bytes[4..length], at)
                            .await?;
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
                    0x810a if value == 0xd300 => {
                        self.native_bytes(position + 4, end, &mut bytes[4..16], at)
                            .await?;
                        let flags = word(&bytes[12..14]);
                        if flags != 0 {
                            return Err(at.at(position + 12).error(ErrorKind::Unsupported {
                                field: "native image-reference flags",
                                value: u64::from(flags),
                            }));
                        }
                        let coordinate = super::RawTextCoordinate {
                            x: word(&bytes[4..6]),
                            y: word(&bytes[6..8]),
                            width: word(&bytes[8..10]),
                            height: word(&bytes[10..12]),
                        };
                        let name_bytes = usize::from(word(&bytes[14..16]));
                        // The length excludes NUL; the complete record has
                        // zero padding to a four-byte boundary. Preserve a
                        // source span instead of allocating or opening a name.
                        length = (16 + name_bytes + 1).next_multiple_of(4);
                        let mut consumed = 16;
                        while consumed < length {
                            let count = (length - consumed).min(bytes.len());
                            self.native_bytes(
                                position + consumed as u64,
                                end,
                                &mut bytes[..count],
                                at,
                            )
                            .await?;
                            for (index, &byte) in bytes[..count].iter().enumerate() {
                                if consumed + index >= 16 + name_bytes && byte != 0 {
                                    return Err(at
                                        .at(position + (consumed + index) as u64)
                                        .malformed(
                                            "native image reference",
                                            "nonzero terminator or padding",
                                        ));
                                }
                            }
                            consumed += count;
                        }
                        if images == page.image_count {
                            return Err(
                                at.malformed("native image records", "exceed declared image count")
                            );
                        }
                        images += 1;
                        NativeRecord::ImageReference {
                            coordinate,
                            reference: super::Span {
                                offset: position + 16,
                                length: name_bytes as u64,
                            },
                        }
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
                        if self.header.variant == Variant::HnB && style.is_none() {
                            return Err(at.error(ErrorKind::Unsupported {
                                field: "HN-B implicit native glyph style",
                                value: 0,
                            }));
                        }
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
