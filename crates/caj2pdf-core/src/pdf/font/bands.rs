// SPDX-License-Identifier: MIT

//! Recognize horizontally constant TrueType ornaments without a full outline
//! loader. Only small, simple, instruction-free rectangle contours qualify.
//! Other glyphs retain ordinary repeated placement. OpenType `glyf` / `loca`:
//! <https://learn.microsoft.com/en-us/typography/opentype/spec/glyf>

use super::*;
use xberg_ttf_parser::{GlyphId, head::IndexToLocationFormat};

const MAX_GLYPH_BYTES: usize = 512;
const MAX_CONTOURS: usize = 8;

#[cfg(test)]
mod tests;

impl<S: RangedSource> OpenTypeFont<'_, S> {
    pub(crate) fn horizontal_bands<C: Cancellation>(
        &mut self,
        character: char,
        limits: &Limits,
        cancellation: &C,
    ) -> Result<bool> {
        if self.is_cff() {
            return Ok(false);
        }
        let glyph = self.glyph(character)?;
        let face = self.face()?;
        let em = face.units_per_em();
        let Some(bearing) = face.glyph_hor_side_bearing(GlyphId(glyph.id)) else {
            return Ok(false);
        };
        let long = face.tables().head.index_to_location_format == IndexToLocationFormat::Long;
        let entry = if long { 4 } else { 2 };
        let (locations, location_bytes) =
            self.outlines[1].expect("checked TrueType outline tables");
        let at = u64::from(glyph.id) * entry;
        if at + 2 * entry > location_bytes {
            return Err(invalid("TrueType glyph locations are truncated"));
        }
        let mut offsets = [0; 8];
        self.fetch(
            locations + at,
            &mut offsets[..2 * entry as usize],
            limits,
            cancellation,
        )?;
        let (first, last) = if long {
            (
                u32::from_be_bytes(offsets[..4].try_into().unwrap()),
                u32::from_be_bytes(offsets[4..].try_into().unwrap()),
            )
        } else {
            (
                u32::from(u16::from_be_bytes([offsets[0], offsets[1]])) * 2,
                u32::from(u16::from_be_bytes([offsets[2], offsets[3]])) * 2,
            )
        };
        let (outline, outline_bytes) = self.outlines[0].expect("checked TrueType outline tables");
        if first > last || u64::from(last) > outline_bytes {
            return Err(invalid("invalid TrueType glyph location"));
        }
        let length = (last - first) as usize;
        if !(10..=MAX_GLYPH_BYTES).contains(&length) {
            return Ok(false);
        }
        let mut data = [0; MAX_GLYPH_BYTES];
        self.fetch(
            outline + u64::from(first),
            &mut data[..length],
            limits,
            cancellation,
        )?;
        Ok(rectangles(&data[..length], em, bearing).unwrap_or(false))
    }
}

fn byte(data: &mut &[u8]) -> Option<u8> {
    let (head, rest) = data.split_first()?;
    *data = rest;
    Some(*head)
}

fn word(data: &mut &[u8]) -> Option<i32> {
    let first = byte(data)?;
    let second = byte(data)?;
    Some(i32::from(i16::from_be_bytes([first, second])))
}

/// Parse at most 32 on-curve points. Bounds/flags/contours are validated before
/// a glyph can be stretched; curves, hints and composite glyphs never qualify.
fn rectangles(mut data: &[u8], em: u16, bearing: i16) -> Option<bool> {
    let contours = usize::try_from(word(&mut data)?).ok()?;
    if !(1..=MAX_CONTOURS).contains(&contours) {
        return Some(false);
    }
    let bounds = [
        word(&mut data)?,
        word(&mut data)?,
        word(&mut data)?,
        word(&mut data)?,
    ];
    if bounds[0] != i32::from(bearing) {
        return Some(false);
    }
    for contour in 0..contours {
        if word(&mut data)? != (4 * contour + 3) as i32 {
            return Some(false);
        }
    }
    if word(&mut data)? != 0 {
        return Some(false);
    } // glyph instructions
    let count = 4 * contours;
    let mut flags = [0; 4 * MAX_CONTOURS];
    let mut at = 0;
    while at < count {
        let flag = byte(&mut data)?;
        if flag & 1 == 0 || flag & 0x80 != 0 {
            return Some(false);
        }
        let run = if flag & 8 == 0 {
            1
        } else {
            1 + usize::from(byte(&mut data)?)
        };
        if at + run > count {
            return Some(false);
        }
        flags[at..at + run].fill(flag);
        at += run;
    }
    let mut points = [[0; 2]; 4 * MAX_CONTOURS];
    for axis in 0..2 {
        let mut value = 0;
        let short = 2 << axis;
        let same = 16 << axis;
        for (point, flag) in points[..count].iter_mut().zip(&flags[..count]) {
            let delta = if flag & short != 0 {
                let magnitude = i32::from(byte(&mut data)?);
                if flag & same != 0 {
                    magnitude
                } else {
                    -magnitude
                }
            } else if flag & same != 0 {
                0
            } else {
                word(&mut data)?
            };
            value += delta;
            point[axis] = value;
        }
    }
    let minimum = |axis| points[..count].iter().map(|p| p[axis]).min().unwrap();
    let maximum = |axis| points[..count].iter().map(|p| p[axis]).max().unwrap();
    if bounds != [minimum(0), minimum(1), maximum(0), maximum(1)] {
        return Some(false);
    }
    let mut vertical = [(0, 0); MAX_CONTOURS];
    for (index, contour) in points[..count].as_chunks::<4>().0.iter().enumerate() {
        let x0 = contour.iter().map(|p| p[0]).min()?;
        let x1 = contour.iter().map(|p| p[0]).max()?;
        let y0 = contour.iter().map(|p| p[1]).min()?;
        let y1 = contour.iter().map(|p| p[1]).max()?;
        if x0 > 0 || x1 < i32::from(em) || y0 >= y1 {
            return Some(false);
        }
        // Opposite winding in overlapping contours could create holes; only
        // separated bands have a coverage proof independent of winding.
        if vertical[..index].iter().any(|&(a, b)| y0 < b && a < y1) {
            return Some(false);
        }
        vertical[index] = (y0, y1);
        // Four distinct corners connected along edges, never a bow-tie.
        let mut seen = 0u8;
        for (index, point) in contour.iter().enumerate() {
            if ![x0, x1].contains(&point[0]) || ![y0, y1].contains(&point[1]) {
                return Some(false);
            }
            let corner = u8::from(point[0] == x1) + 2 * u8::from(point[1] == y1);
            seen |= 1 << corner;
            let next = contour[(index + 1) % 4];
            if point[0] != next[0] && point[1] != next[1] {
                return Some(false);
            }
        }
        if seen != 15 {
            return Some(false);
        }
    }
    Some(true)
}
