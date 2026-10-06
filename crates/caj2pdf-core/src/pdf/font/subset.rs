// SPDX-License-Identifier: MIT

//! Subsets of drawn glyphs, read again through the ranged source: TrueType
//! subsets here, CFF subsets in [`super::cff`].
//!
//! A subset keeps `.notdef`, then the glyphs of the used characters in
//! Unicode order, then composite components in discovery order. It contains
//! only the tables that a PDF `FontFile2` program needs (ISO 32000-1 §9.9):
//! `cvt `, `fpgm`, `glyf`, `head`, `hhea`, `hmtx`, `loca`, `maxp` and `prep`.
//! Characters map to glyphs through the PDF `CIDToGIDMap`, so no `cmap` is
//! written. Each selected glyph is read whole, once while planning (to find
//! components and measure checksums) and once while writing.

use super::{CHANGED, OUTLINE_TAGS, OpenTypeFont, invalid, read};
use crate::fallible::reserve_exact;
use crate::{Cancellation, Error, Limits, RangedSource, Result};
use sha2::{Digest, Sha256};
use xberg_ttf_parser::{Face, GlyphId, head::IndexToLocationFormat};

const GLYF: usize = 0;
const LOCA: usize = 1;
// Composite glyph component flags, from the OpenType `glyf` table.
const ARG_1_AND_2_ARE_WORDS: u16 = 0x0001;
const WE_HAVE_A_SCALE: u16 = 0x0008;
const MORE_COMPONENTS: u16 = 0x0020;
const WE_HAVE_AN_X_AND_Y_SCALE: u16 = 0x0040;
const WE_HAVE_A_TWO_BY_TWO: u16 = 0x0080;

/// Receives subset font program bytes in order.
pub(crate) trait SubsetOutput {
    fn put(&mut self, bytes: &[u8]) -> Result<()>;
}

/// Length and checksum of a table, as if zero-padded to a 4-byte boundary.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Measure {
    length: u64,
    sum: u32,
}

impl Measure {
    fn add(&mut self, mut bytes: &[u8]) {
        while !self.length.is_multiple_of(4) && !bytes.is_empty() {
            self.byte(bytes[0]);
            bytes = &bytes[1..];
        }
        let (words, rest) = bytes.as_chunks::<4>();
        for word in words {
            self.sum = self.sum.wrapping_add(u32::from_be_bytes(*word));
        }
        self.length += 4 * words.len() as u64;
        for byte in rest {
            self.byte(*byte);
        }
    }

    fn byte(&mut self, byte: u8) {
        let shift = 24 - 8 * (self.length % 4) as u32;
        self.sum = self.sum.wrapping_add(u32::from(byte) << shift);
        self.length += 1;
    }

    fn padding(&self) -> usize {
        (self.length.next_multiple_of(4) - self.length) as usize
    }
}

impl SubsetOutput for Measure {
    fn put(&mut self, bytes: &[u8]) -> Result<()> {
        self.add(bytes);
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct Glyph {
    source: u16,
    offset: u32,
    length: u32,
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

/// A planned subset program of either outline format.
pub(crate) enum Subset {
    /// A TrueType program; characters map to glyphs through a PDF
    /// `CIDToGIDMap`.
    TrueType(SubsetPlan),
    /// A CID-keyed CFF program whose CIDs are the drawn characters.
    Cff(super::cff::CffSubset),
}

impl Subset {
    /// Total bytes of the subset font program.
    pub(crate) fn length(&self) -> u64 {
        match self {
            Self::TrueType(plan) => plan.length(),
            Self::Cff(subset) => subset.length(),
        }
    }

    /// Six uppercase letters identifying this subset program (ISO 32000-1
    /// §9.6.4), derived from the PostScript name and the program's content.
    /// Equal programs get equal tags.
    pub(crate) fn tag(&self, name: &str) -> String {
        let mut hash = Sha256::new().chain_update(name);
        match self {
            Self::TrueType(plan) => {
                for (tag, _, measure) in &plan.tables {
                    hash.update(tag);
                    hash.update(measure.length.to_be_bytes());
                    hash.update(measure.sum.to_be_bytes());
                }
            }
            Self::Cff(subset) => {
                for part in subset.parts() {
                    hash.update(part);
                }
            }
        }
        hash.finalize()[..6]
            .iter()
            .map(|byte| char::from(b'A' + byte % 26))
            .collect()
    }
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
fn is_composite(data: &[u8]) -> bool {
    data.len() >= 2 && data[0] & 0x80 != 0
}

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

impl<S: RangedSource> OpenTypeFont<'_, S> {
    /// Plan the subset of the characters set in `used`, a BMP bitmap, whose
    /// program is at most `max_length` bytes, in the font's outline format.
    pub(crate) fn subset<C: Cancellation>(
        &mut self,
        used: &[u8],
        max_length: u64,
        limits: &Limits,
        cancellation: &C,
    ) -> Result<Subset> {
        if let Some(cff) = self.cff.clone() {
            let subset = self.plan_cff(&cff, used, max_length, limits, cancellation)?;
            return Ok(Subset::Cff(subset));
        }
        let plan = self.plan_subset(used, max_length, limits, cancellation)?;
        Ok(Subset::TrueType(plan))
    }

    /// Write a planned subset program to `output`.
    pub(crate) fn write<O: SubsetOutput, C: Cancellation>(
        &mut self,
        subset: &Subset,
        output: &mut O,
        limits: &Limits,
        cancellation: &C,
    ) -> Result<()> {
        match subset {
            Subset::TrueType(plan) => self.write_subset(plan, output, limits, cancellation),
            Subset::Cff(subset) => {
                for part in subset.parts() {
                    output.put(part)?;
                }
                Ok(())
            }
        }
    }

    /// Range of a required outline table, checked present by `read`.
    fn table(&self, slot: usize) -> (u64, u64) {
        self.outlines[slot].unwrap_or_default()
    }

    /// Outline bytes read again by subset planning and writing so far.
    pub(crate) fn subset_bytes_read(&self) -> u64 {
        self.subset_bytes_read
    }

    fn fetch<C: Cancellation>(
        &mut self,
        offset: u64,
        bytes: &mut [u8],
        limits: &Limits,
        cancellation: &C,
    ) -> Result<()> {
        read(self.source, offset, bytes, limits, cancellation)?;
        self.subset_bytes_read += bytes.len() as u64;
        Ok(())
    }

    /// Plan a subset of the characters set in `used`, a BMP bitmap, whose
    /// program is at most `max_length` bytes (and at most 4 GiB).
    ///
    /// Every used character must still map to a glyph. Composite glyphs add
    /// their components. Retained state is two bytes per source glyph plus
    /// twelve per subset glyph; each selected outline is read once here.
    pub(crate) fn plan_subset<C: Cancellation>(
        &mut self,
        used: &[u8],
        max_length: u64,
        limits: &Limits,
        cancellation: &C,
    ) -> Result<SubsetPlan> {
        let face = self.face()?;
        let count = face.number_of_glyphs();
        let long = face.tables().head.index_to_location_format == IndexToLocationFormat::Long;
        let entry = if long { 4 } else { 2 };
        if self.table(LOCA).1 < (u64::from(count) + 1) * entry {
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
        // Return the subset ID of a source glyph, adding it when first seen.
        let mut add = |glyphs: &mut Vec<Glyph>, source: u16| {
            if source != 0 && map[usize::from(source)] == 0 {
                map[usize::from(source)] = glyphs.len() as u16;
                glyphs.push(Glyph {
                    source,
                    offset: 0,
                    length: 0,
                });
            }
            map[usize::from(source)]
        };
        glyphs.push(Glyph {
            source: 0,
            offset: 0,
            length: 0,
        });
        for (glyph, _) in self.used_glyphs(used)? {
            add(&mut glyphs, glyph);
        }
        let mut scratch = Vec::new();
        let mut glyf = Measure::default();
        let mut total = 0_u64;
        let mut index = 0;
        while index < glyphs.len() {
            let mut location = [0; 8];
            let location = &mut location[..2 * entry as usize];
            let at = self.table(LOCA).0 + u64::from(glyphs[index].source) * entry;
            self.fetch(at, location, limits, cancellation)?;
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
            if start > end || u64::from(end) > self.table(GLYF).1 {
                return Err(invalid("invalid TrueType glyph location"));
            }
            total += u64::from(end - start).next_multiple_of(4);
            too_long(total, max_length)?;
            glyphs[index].offset = start;
            glyphs[index].length = end - start;
            let data = self.glyph_data(glyphs[index], &mut scratch, limits, cancellation)?;
            if is_composite(data) {
                components(data, |id| {
                    let source = u16::from_be_bytes([id[0], id[1]]);
                    if source >= count {
                        return Err(invalid("composite glyph references an invalid glyph"));
                    }
                    id.copy_from_slice(&add(&mut glyphs, source).to_be_bytes());
                    Ok(())
                })?;
            }
            glyf.add(data);
            glyf.add(&[0; 3][..glyf.padding()]);
            index += 1;
        }
        let mut tables = Vec::new();
        for (slot, tag) in OUTLINE_TAGS.iter().enumerate().skip(2) {
            if self.outlines[slot].is_some() {
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
        // Every table length is known before any is measured.
        let glyphs_bytes = 4 * glyphs.len() as u64;
        for (_, table, _) in &tables {
            total += match table {
                Table::Copy(slot) => self.table(*slot).1.next_multiple_of(4),
                Table::Glyf => 0,
                Table::Head => 56,
                Table::Hhea => 36,
                Table::Hmtx => glyphs_bytes,
                Table::Loca => glyphs_bytes + 4,
                Table::Maxp => 32,
            };
        }
        too_long(total + 12 + 16 * tables.len() as u64, max_length)?;
        let mut plan = SubsetPlan {
            glyphs,
            map,
            tables,
        };
        // Glyph data was measured above, with component IDs rewritten.
        for index in 0..plan.tables.len() {
            let table = plan.tables[index].1;
            if table == Table::Glyf {
                plan.tables[index].2 = glyf;
                continue;
            }
            let mut measure = Measure::default();
            self.emit(
                table,
                &plan,
                0,
                &mut measure,
                &mut scratch,
                limits,
                cancellation,
            )?;
            plan.tables[index].2 = measure;
        }
        Ok(plan)
    }

    /// Write the planned subset font program to `output`.
    pub(crate) fn write_subset<O: SubsetOutput, C: Cancellation>(
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
        output.put(&header[..length])?;
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
            )?;
            output.put(&[0; 3][..measure.padding()])?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn emit<O: SubsetOutput, C: Cancellation>(
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
                let (offset, length) = self.table(slot);
                self.copy(offset, length, output, scratch, limits, cancellation)?;
            }
            Table::Glyf => {
                for glyph in &plan.glyphs {
                    let data = self.glyph_data(*glyph, scratch, limits, cancellation)?;
                    if is_composite(data) {
                        components(data, |id| {
                            let source = u16::from_be_bytes([id[0], id[1]]);
                            let target = plan.map.get(usize::from(source)).copied().unwrap_or(0);
                            if target == 0 && source != 0 {
                                return Err(invalid(CHANGED));
                            }
                            id.copy_from_slice(&target.to_be_bytes());
                            Ok(())
                        })?;
                    }
                    output.put(data)?;
                    let padding = glyph.length.next_multiple_of(4) - glyph.length;
                    output.put(&[0; 3][..padding as usize])?;
                }
            }
            Table::Loca => {
                let mut offset = 0_u32;
                output.put(&offset.to_be_bytes())?;
                for glyph in &plan.glyphs {
                    offset += glyph.length.next_multiple_of(4);
                    output.put(&offset.to_be_bytes())?;
                }
            }
            Table::Hmtx => {
                let face = self.face()?;
                for glyph in &plan.glyphs {
                    let id = GlyphId(glyph.source);
                    // Reading validated `hmtx`; every glyph ID has a metric.
                    let advance = face.glyph_hor_advance(id).unwrap_or(0);
                    let bearing = face.glyph_hor_side_bearing(id).unwrap_or(0);
                    output.put(&advance.to_be_bytes())?;
                    output.put(&bearing.to_be_bytes())?;
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
                output.put(&bytes[..length])?;
            }
        }
        Ok(())
    }

    fn copy<O: SubsetOutput, C: Cancellation>(
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
            self.fetch(offset + done, &mut scratch[..size], limits, cancellation)?;
            output.put(&scratch[..size])?;
            done += size as u64;
        }
        Ok(())
    }

    fn glyph_data<'b, C: Cancellation>(
        &mut self,
        glyph: Glyph,
        scratch: &'b mut Vec<u8>,
        limits: &Limits,
        cancellation: &C,
    ) -> Result<&'b mut [u8]> {
        let length = glyph.length as usize;
        grow(scratch, length, limits)?;
        let data = &mut scratch[..length];
        let offset = self.table(GLYF).0 + u64::from(glyph.offset);
        self.fetch(offset, data, limits, cancellation)?;
        Ok(data)
    }
}

pub(super) fn too_long(length: u64, max_length: u64) -> Result<()> {
    let limit = max_length.min(u64::from(u32::MAX));
    if length > limit {
        return Err(Error::limit("font subset program bytes", limit, length));
    }
    Ok(())
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
