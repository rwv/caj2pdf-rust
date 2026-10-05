// SPDX-License-Identifier: MIT

//! TrueType subsets of drawn glyphs, read again through the ranged source.
//!
//! A subset keeps `.notdef`, then the glyphs of the used characters in
//! Unicode order, then composite components in discovery order. It contains
//! only the tables that a PDF `FontFile2` program needs (ISO 32000-1 §9.9):
//! `cvt `, `fpgm`, `glyf`, `head`, `hhea`, `hmtx`, `loca`, `maxp` and `prep`.
//! Characters map to glyphs through the PDF `CIDToGIDMap`, so no `cmap` is
//! written. Outline bytes are streamed; only composite glyphs are buffered,
//! because their component IDs are rewritten.

use super::{OUTLINE_TAGS, TrueTypeFont, face_of, invalid, read};
use crate::fallible::reserve_exact;
use crate::{Cancellation, Limits, RangedSource, Result};
use xberg_ttf_parser::{Face, GlyphId, head::IndexToLocationFormat};

const GLYF: usize = 0;
const LOCA: usize = 1;
// Composite glyph component flags, from the OpenType `glyf` table.
const ARG_1_AND_2_ARE_WORDS: u16 = 0x0001;
const WE_HAVE_A_SCALE: u16 = 0x0008;
const MORE_COMPONENTS: u16 = 0x0020;
const WE_HAVE_AN_X_AND_Y_SCALE: u16 = 0x0040;
const WE_HAVE_A_TWO_BY_TWO: u16 = 0x0080;
const CHANGED: &str = "font source changed after its metadata was read";

/// Receives subset font program bytes in order.
pub(crate) trait SubsetOutput {
    async fn put(&mut self, bytes: &[u8]) -> Result<()>;
}

/// Length and checksum of a table, as if zero-padded to a 4-byte boundary.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Measure {
    length: u64,
    sum: u32,
}

impl Measure {
    fn add(&mut self, bytes: &[u8]) {
        for byte in bytes {
            let shift = 24 - 8 * (self.length % 4) as u32;
            self.sum = self.sum.wrapping_add(u32::from(*byte) << shift);
            self.length += 1;
        }
    }

    fn padding(&self) -> usize {
        (self.length.next_multiple_of(4) - self.length) as usize
    }
}

impl SubsetOutput for Measure {
    async fn put(&mut self, bytes: &[u8]) -> Result<()> {
        self.add(bytes);
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct Glyph {
    source: u16,
    offset: u32,
    length: u32,
    composite: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Table {
    Copy(usize),
    Glyf,
    Head,
    Hhea,
    Hmtx,
    Loca,
    Maxp,
}

/// The glyphs and table layout of one subset, measured before it is written.
pub(crate) struct SubsetPlan {
    glyphs: Vec<Glyph>,
    /// Source-to-subset glyph IDs; zero means unused except for `.notdef`.
    map: Vec<u16>,
    tables: Vec<([u8; 4], Table, Measure)>,
}

impl SubsetPlan {
    /// Total bytes of the subset font program.
    pub(crate) fn length(&self) -> u64 {
        let directory = 12 + 16 * self.tables.len() as u64;
        self.tables
            .iter()
            .fold(directory, |total, (_, _, measure)| {
                total + measure.length.next_multiple_of(4)
            })
    }

    /// Subset glyph ID of a planned character, or zero for any other.
    pub(crate) fn glyph(&self, face: &Face<'_>, character: char) -> u16 {
        face.glyph_index(character)
            .and_then(|id| self.map.get(usize::from(id.0)).copied())
            .unwrap_or(0)
    }
}

/// Visit each component glyph ID of composite glyph `data`, in place.
fn components(data: &mut [u8], mut visit: impl FnMut(&mut [u8]) -> Result<()>) -> Result<()> {
    let mut at = 10;
    loop {
        if at + 4 > data.len() {
            return Err(invalid("composite glyph component is truncated"));
        }
        let flags = u16::from_be_bytes([data[at], data[at + 1]]);
        visit(&mut data[at + 2..at + 4])?;
        at += 4 + if flags & ARG_1_AND_2_ARE_WORDS != 0 {
            4
        } else {
            2
        };
        at += if flags & WE_HAVE_A_SCALE != 0 {
            2
        } else if flags & WE_HAVE_AN_X_AND_Y_SCALE != 0 {
            4
        } else if flags & WE_HAVE_A_TWO_BY_TWO != 0 {
            8
        } else {
            0
        };
        if at > data.len() {
            return Err(invalid("composite glyph component is truncated"));
        }
        if flags & MORE_COMPONENTS == 0 {
            return Ok(());
        }
    }
}

impl<S: RangedSource> TrueTypeFont<'_, S> {
    /// Plan a subset of the characters set in `used`, a BMP bitmap.
    ///
    /// Every used character must still map to a glyph. Composite glyphs add
    /// their components. Retained state is two bytes per source glyph plus
    /// twelve per subset glyph; selected outlines are read again to measure.
    pub(crate) async fn plan_subset<C: Cancellation>(
        &mut self,
        used: &[u8],
        limits: &Limits,
        cancellation: &C,
    ) -> Result<SubsetPlan> {
        let face = face_of(&self.tables)?;
        let count = face.number_of_glyphs();
        let long = face.tables().head.index_to_location_format == IndexToLocationFormat::Long;
        let entry = if long { 4 } else { 2 };
        if self.outlines[LOCA].1 < (u64::from(count) + 1) * entry {
            return Err(invalid("TrueType glyph locations are truncated"));
        }
        let mut map = Vec::new();
        let mut glyphs = Vec::new();
        let slots = usize::from(count);
        let bytes = slots as u64 * (2 + size_of::<Glyph>() as u64);
        limits.check_allocation(bytes)?;
        let refused = || limits.allocation_refused("font subset glyph tables", bytes);
        reserve_exact(&mut map, slots, refused())?;
        reserve_exact(&mut glyphs, slots, refused())?;
        map.resize(slots, 0_u16);
        let mut add = |glyphs: &mut Vec<Glyph>, source: u16| {
            if source != 0 && map[usize::from(source)] == 0 {
                map[usize::from(source)] = glyphs.len() as u16;
                glyphs.push(Glyph {
                    source,
                    offset: 0,
                    length: 0,
                    composite: false,
                });
            }
        };
        glyphs.push(Glyph {
            source: 0,
            offset: 0,
            length: 0,
            composite: false,
        });
        for (code, _) in (0..used.len() * 8)
            .map(|code| (code, used[code / 8] & (1 << (code % 8))))
            .filter(|(_, bit)| *bit != 0)
        {
            let id = char::from_u32(code as u32)
                .and_then(|character| face.glyph_index(character))
                .filter(|id| id.0 != 0 && id.0 < count)
                .ok_or(invalid(CHANGED))?;
            add(&mut glyphs, id.0);
        }
        let mut scratch = Vec::new();
        let mut total = 0_u64;
        let mut index = 0;
        while index < glyphs.len() {
            let mut location = [0; 8];
            let location = &mut location[..2 * entry as usize];
            let at = self.outlines[LOCA].0 + u64::from(glyphs[index].source) * entry;
            read(self.source, at, location, limits, cancellation).await?;
            let (start, end) = if long {
                (
                    u32::from_be_bytes(location[..4].try_into().unwrap()),
                    u32::from_be_bytes(location[4..].try_into().unwrap()),
                )
            } else {
                (
                    u32::from(u16::from_be_bytes([location[0], location[1]])) * 2,
                    u32::from(u16::from_be_bytes([location[2], location[3]])) * 2,
                )
            };
            if start > end || u64::from(end) > self.outlines[GLYF].1 {
                return Err(invalid("invalid TrueType glyph location"));
            }
            total += u64::from(end - start).next_multiple_of(4);
            if total > u64::from(u32::MAX) {
                return Err(invalid("TrueType subset glyph data exceeds 4 GiB"));
            }
            let glyph = &mut glyphs[index];
            glyph.offset = start;
            glyph.length = end - start;
            if glyph.length >= 2 {
                let mut contours = [0; 2];
                let at = self.outlines[GLYF].0 + u64::from(start);
                read(self.source, at, &mut contours, limits, cancellation).await?;
                glyph.composite = contours[0] & 0x80 != 0;
            }
            if glyph.composite {
                let glyph = *glyph;
                let data = self
                    .composite(glyph, &mut scratch, limits, cancellation)
                    .await?;
                components(data, |id| {
                    let id = u16::from_be_bytes([id[0], id[1]]);
                    if id >= count {
                        return Err(invalid("composite glyph references an invalid glyph"));
                    }
                    add(&mut glyphs, id);
                    Ok(())
                })?;
            }
            index += 1;
        }
        let mut tables = Vec::new();
        for (slot, tag) in OUTLINE_TAGS.iter().enumerate().skip(2) {
            if self.hinting[slot - 2] {
                tables.push((*tag, Table::Copy(slot), Measure::default()));
            }
        }
        tables.extend([
            (*b"glyf", Table::Glyf, Measure::default()),
            (*b"head", Table::Head, Measure::default()),
            (*b"hhea", Table::Hhea, Measure::default()),
            (*b"hmtx", Table::Hmtx, Measure::default()),
            (*b"loca", Table::Loca, Measure::default()),
            (*b"maxp", Table::Maxp, Measure::default()),
        ]);
        tables.sort_by_key(|table| table.0);
        let mut plan = SubsetPlan {
            glyphs,
            map,
            tables,
        };
        for index in 0..plan.tables.len() {
            let mut measure = Measure::default();
            let table = plan.tables[index].1;
            self.emit(
                table,
                &plan,
                0,
                &mut measure,
                &mut scratch,
                limits,
                cancellation,
            )
            .await?;
            plan.tables[index].2 = measure;
        }
        Ok(plan)
    }

    /// Write the planned subset font program to `output`.
    pub(crate) async fn write_subset<O: SubsetOutput, C: Cancellation>(
        &mut self,
        plan: &SubsetPlan,
        output: &mut O,
        limits: &Limits,
        cancellation: &C,
    ) -> Result<()> {
        let count = plan.tables.len() as u16;
        let selector = count.ilog2() as u16;
        let mut header = [0; 12 + 16 * 9];
        let mut directory = Measure::default();
        let mut put = |bytes: &[u8]| {
            let at = directory.length as usize;
            header[at..at + bytes.len()].copy_from_slice(bytes);
            directory.add(bytes);
        };
        put(&0x0001_0000_u32.to_be_bytes());
        put(&count.to_be_bytes());
        put(&(16_u16 << selector).to_be_bytes());
        put(&selector.to_be_bytes());
        put(&(16 * count - (16_u16 << selector)).to_be_bytes());
        let mut offset = 12 + 16 * u64::from(count);
        let mut sum = 0_u32;
        for (tag, _, measure) in &plan.tables {
            put(tag);
            put(&measure.sum.to_be_bytes());
            put(&(offset as u32).to_be_bytes());
            put(&(measure.length as u32).to_be_bytes());
            offset += measure.length.next_multiple_of(4);
            sum = sum.wrapping_add(measure.sum);
        }
        let length = directory.length as usize;
        let adjustment = 0xb1b0_afba_u32.wrapping_sub(sum.wrapping_add(directory.sum));
        output.put(&header[..length]).await?;
        let mut scratch = Vec::new();
        for (_, table, measure) in &plan.tables {
            self.emit(
                *table,
                plan,
                adjustment,
                output,
                &mut scratch,
                limits,
                cancellation,
            )
            .await?;
            output.put(&[0; 3][..measure.padding()]).await?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn emit<O: SubsetOutput, C: Cancellation>(
        &mut self,
        table: Table,
        plan: &SubsetPlan,
        adjustment: u32,
        output: &mut O,
        scratch: &mut Vec<u8>,
        limits: &Limits,
        cancellation: &C,
    ) -> Result<()> {
        let count = plan.glyphs.len() as u16;
        match table {
            Table::Copy(slot) => {
                let (offset, length) = self.outlines[slot];
                self.copy(offset, length, output, scratch, limits, cancellation)
                    .await?;
            }
            Table::Glyf => {
                for glyph in &plan.glyphs {
                    if glyph.composite {
                        let data = self
                            .composite(*glyph, scratch, limits, cancellation)
                            .await?;
                        components(data, |id| {
                            let source = u16::from_be_bytes([id[0], id[1]]);
                            let target = plan.map.get(usize::from(source)).copied().unwrap_or(0);
                            if target == 0 && source != 0 {
                                return Err(invalid(CHANGED));
                            }
                            id.copy_from_slice(&target.to_be_bytes());
                            Ok(())
                        })?;
                        output.put(data).await?;
                    } else {
                        let offset = self.outlines[GLYF].0 + u64::from(glyph.offset);
                        let length = u64::from(glyph.length);
                        self.copy(offset, length, output, scratch, limits, cancellation)
                            .await?;
                    }
                    let padding = glyph.length.next_multiple_of(4) - glyph.length;
                    output.put(&[0; 3][..padding as usize]).await?;
                }
            }
            Table::Loca => {
                let mut offset = 0_u32;
                output.put(&offset.to_be_bytes()).await?;
                for glyph in &plan.glyphs {
                    offset += glyph.length.next_multiple_of(4);
                    output.put(&offset.to_be_bytes()).await?;
                }
            }
            Table::Hmtx => {
                let face = face_of(&self.tables)?;
                for glyph in &plan.glyphs {
                    let id = GlyphId(glyph.source);
                    // Reading validated `hmtx`; every glyph ID has a metric.
                    let advance = face.glyph_hor_advance(id).unwrap_or(0);
                    let bearing = face.glyph_hor_side_bearing(id).unwrap_or(0);
                    output.put(&advance.to_be_bytes()).await?;
                    output.put(&bearing.to_be_bytes()).await?;
                }
            }
            Table::Head | Table::Hhea | Table::Maxp => {
                // Fixed sizes of `head`, `hhea` and version 1.0 `maxp`. Each
                // field below is replaced: long glyph locations, the subset's
                // metric count and its glyph count.
                let (source, length, field, value) = match table {
                    Table::Head => (0, 54, 50, 1),
                    Table::Hhea => (1, 36, 34, count),
                    _ => (2, 32, 4, count),
                };
                let mut bytes = [0; 54];
                let available = self.tables[source].len().min(length);
                bytes[..available].copy_from_slice(&self.tables[source][..available]);
                bytes[field..field + 2].copy_from_slice(&value.to_be_bytes());
                if table == Table::Head {
                    bytes[8..12].copy_from_slice(&adjustment.to_be_bytes());
                }
                output.put(&bytes[..length]).await?;
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn copy<O: SubsetOutput, C: Cancellation>(
        &mut self,
        offset: u64,
        length: u64,
        output: &mut O,
        scratch: &mut Vec<u8>,
        limits: &Limits,
        cancellation: &C,
    ) -> Result<()> {
        let mut done = 0;
        while done < length {
            let size = (length - done).min(limits.io_chunk_bytes as u64) as usize;
            grow(scratch, size, limits)?;
            read(
                self.source,
                offset + done,
                &mut scratch[..size],
                limits,
                cancellation,
            )
            .await?;
            output.put(&scratch[..size]).await?;
            done += size as u64;
        }
        Ok(())
    }

    async fn composite<'b, C: Cancellation>(
        &mut self,
        glyph: Glyph,
        scratch: &'b mut Vec<u8>,
        limits: &Limits,
        cancellation: &C,
    ) -> Result<&'b mut [u8]> {
        let length = glyph.length as usize;
        grow(scratch, length, limits)?;
        let data = &mut scratch[..length];
        let offset = self.outlines[GLYF].0 + u64::from(glyph.offset);
        read(self.source, offset, data, limits, cancellation).await?;
        Ok(data)
    }
}

fn grow(scratch: &mut Vec<u8>, length: usize, limits: &Limits) -> Result<()> {
    if scratch.len() < length {
        limits.check_allocation(length as u64)?;
        let refused = limits.allocation_refused("font subset buffer", length as u64);
        reserve_exact(scratch, length - scratch.len(), refused)?;
        scratch.resize(length, 0);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
