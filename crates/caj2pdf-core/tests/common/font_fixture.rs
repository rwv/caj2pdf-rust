// SPDX-License-Identifier: MIT

pub fn put16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}
pub fn put32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

// Original minimal metadata. Glyph slots have empty outlines; this fixture
// proves character/metric access, not rendered font validity or C8 fidelity.
pub fn metadata_font() -> (Vec<u8>, u64) {
    let mut head = vec![0; 54];
    put32(&mut head, 0, 0x10000);
    put32(&mut head, 12, 0x5f0f3cf5);
    put16(&mut head, 18, 1000);
    let mut hhea = vec![0; 36];
    put32(&mut hhea, 0, 0x10000);
    put16(&mut hhea, 4, 800);
    put16(&mut hhea, 6, (-200_i16) as u16);
    put16(&mut hhea, 34, 3);
    let mut maxp = vec![0; 32];
    put32(&mut maxp, 0, 0x10000);
    put16(&mut maxp, 4, 3);
    let mut cmap = vec![0; 52];
    put16(&mut cmap, 2, 1);
    put16(&mut cmap, 4, 3);
    put16(&mut cmap, 6, 10);
    put32(&mut cmap, 8, 12);
    put16(&mut cmap, 12, 12);
    put32(&mut cmap, 16, 40);
    put32(&mut cmap, 24, 2);
    for (index, code) in [65, 0x4e2d].into_iter().enumerate() {
        let at = 28 + index * 12;
        put32(&mut cmap, at, code);
        put32(&mut cmap, at + 4, code);
        put32(&mut cmap, at + 8, index as u32 + 1);
    }
    let mut hmtx = vec![0; 12];
    for (index, advance) in [500, 600, 1000].into_iter().enumerate() {
        put16(&mut hmtx, index * 4, advance);
    }
    let mut post = vec![0; 32];
    put32(&mut post, 0, 0x30000);
    let mut os2 = vec![0; 96];
    put16(&mut os2, 0, 3);
    let ps_name = "CajFixture";
    let mut name = vec![0; 18];
    put16(&mut name, 2, 1);
    put16(&mut name, 4, 18);
    put16(&mut name, 6, 3);
    put16(&mut name, 8, 1);
    put16(&mut name, 10, 0x409);
    put16(&mut name, 12, 6);
    put16(&mut name, 14, ps_name.len() as u16 * 2);
    for unit in ps_name.encode_utf16() {
        name.extend(unit.to_be_bytes());
    }
    let mut tables = vec![
        (*b"head", head),
        (*b"hhea", hhea),
        (*b"maxp", maxp),
        (*b"cmap", cmap),
        (*b"hmtx", hmtx),
        (*b"OS/2", os2),
        (*b"post", post),
        (*b"name", name),
        (*b"loca", vec![0; 8]),
        (*b"glyf", vec![]),
    ];
    tables.sort_by_key(|table| table.0);
    let mut bytes = vec![0; 12 + 16 * tables.len()];
    put32(&mut bytes, 0, 0x10000);
    put16(&mut bytes, 4, tables.len() as u16);
    for (index, (tag, payload)) in tables.iter().enumerate() {
        bytes.resize(bytes.len().next_multiple_of(4), 0);
        let offset = bytes.len() as u32;
        bytes[12 + index * 16..16 + index * 16].copy_from_slice(tag);
        put32(&mut bytes, 20 + index * 16, offset);
        put32(&mut bytes, 24 + index * 16, payload.len() as u32);
        bytes.extend(payload);
    }
    bytes.resize(bytes.len().next_multiple_of(4), 0);
    let outline = bytes.len() as u64;
    let glyf = entry(&bytes, b"glyf");
    put32(&mut bytes, glyf + 8, outline as u32);
    put32(&mut bytes, glyf + 12, 2 * 1024 * 1024);
    bytes.resize(outline as usize + 2 * 1024 * 1024, 0);
    (bytes, outline)
}

pub fn entry(bytes: &[u8], tag: &[u8; 4]) -> usize {
    let n = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
    (0..n)
        .map(|i| 12 + i * 16)
        .find(|&i| &bytes[i..i + 4] == tag)
        .unwrap()
}

/// Original three-slot font: empty .notdef, a rectangle, and a triangle.
/// Character labels are deliberately synthetic; no commercial glyph design
/// or external font bytes are used. Shared by independent PDF output tests.
pub fn drawing_font() -> Vec<u8> {
    let (mut bytes, outline) = metadata_font();
    let mut outlines = vec![0; 10];
    let mut offsets = vec![0_u16, 5];
    for points in [
        &[(0_i16, 0_i16), (400, 0), (400, 700), (0, 700)][..],
        &[(0, 0), (800, 0), (400, 700)][..],
    ] {
        let mut glyph = vec![0; 14];
        put16(&mut glyph, 0, 1);
        put16(
            &mut glyph,
            6,
            points.iter().map(|point| point.0).max().unwrap() as u16,
        );
        put16(&mut glyph, 8, 700);
        put16(&mut glyph, 10, points.len() as u16 - 1);
        glyph.extend(std::iter::repeat_n(1, points.len()));
        let (mut x, mut y) = (0, 0);
        for point in points {
            glyph.extend((point.0 - x).to_be_bytes());
            x = point.0;
        }
        for point in points {
            glyph.extend((point.1 - y).to_be_bytes());
            y = point.1;
        }
        glyph.resize(glyph.len().next_multiple_of(2), 0);
        outlines.extend(glyph);
        offsets.push(outlines.len() as u16 / 2);
    }
    let glyf = entry(&bytes, b"glyf");
    put32(&mut bytes, glyf + 12, outlines.len() as u32);
    bytes.truncate(outline as usize);
    bytes.extend(outlines);
    bytes.resize(bytes.len().next_multiple_of(4), 0);
    let table_at = |bytes: &[u8], tag| {
        let i = entry(bytes, tag);
        span(bytes[i..i + 16].try_into().unwrap()).0 as usize
    };
    let loca = table_at(&bytes, b"loca");
    for (i, offset) in offsets.into_iter().enumerate() {
        put16(&mut bytes, loca + i * 2, offset);
    }
    let head = table_at(&bytes, b"head");
    // Fixed 2020-01-01 timestamps in seconds since the TrueType 1904 epoch.
    put32(&mut bytes, head + 24, 3_660_681_600);
    put32(&mut bytes, head + 32, 3_660_681_600);
    put16(&mut bytes, head + 40, 800);
    put16(&mut bytes, head + 42, 700);
    put16(&mut bytes, head + 46, 8);
    put16(&mut bytes, head + 48, 2);
    let maxp = table_at(&bytes, b"maxp");
    put16(&mut bytes, maxp + 6, 4);
    put16(&mut bytes, maxp + 8, 1);
    put16(&mut bytes, maxp + 14, 1);
    let hhea = table_at(&bytes, b"hhea");
    put16(&mut bytes, hhea + 10, 1000);
    put16(&mut bytes, hhea + 16, 800);
    put16(&mut bytes, hhea + 18, 1);
    let os2 = table_at(&bytes, b"OS/2");
    put16(&mut bytes, os2 + 4, 400);
    put16(&mut bytes, os2 + 6, 5);
    put16(&mut bytes, os2 + 64, 65);
    put16(&mut bytes, os2 + 66, 0x4e2d);
    put16(&mut bytes, os2 + 68, 800);
    put16(&mut bytes, os2 + 70, (-200_i16) as u16);
    put16(&mut bytes, os2 + 74, 800);
    put16(&mut bytes, os2 + 76, 200);
    put16(&mut bytes, os2 + 88, 700);
    let sum = |bytes: &[u8]| {
        bytes.chunks(4).fold(0_u32, |sum, chunk| {
            let mut word = [0; 4];
            word[..chunk.len()].copy_from_slice(chunk);
            sum.wrapping_add(u32::from_be_bytes(word))
        })
    };
    let count = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
    let power = 1 << count.ilog2();
    put16(&mut bytes, 6, power * 16);
    put16(&mut bytes, 8, count.ilog2() as u16);
    put16(&mut bytes, 10, count as u16 * 16 - power * 16);
    for i in 0..count {
        let at = 12 + i * 16;
        let (offset, length) = span(bytes[at..at + 16].try_into().unwrap());
        let checksum = sum(&bytes[offset as usize..(offset + length) as usize]);
        put32(&mut bytes, at + 4, checksum);
    }
    let adjustment = 0xb1b0afba_u32.wrapping_sub(sum(&bytes));
    put32(&mut bytes, head + 8, adjustment);
    bytes
}

fn span(entry: &[u8; 16]) -> (u64, u64) {
    (u64::from(u32::from_be_bytes(entry[8..12].try_into().unwrap())),
     u64::from(u32::from_be_bytes(entry[12..16].try_into().unwrap())))
}

// Also usable as a standalone generator for cross-runtime test fixtures.
fn main() {
    use std::io::Write;
    std::io::stdout().write_all(&drawing_font()).unwrap();
}
