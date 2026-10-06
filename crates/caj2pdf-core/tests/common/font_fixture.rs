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
    finish_checksums(&mut bytes);
    bytes
}

/// Original rectangle/triangle relabelled as space and fullwidth colon.
/// A visible space deliberately catches implementations that drop the record.
pub fn symbol_font() -> Vec<u8> {
    let mut bytes = drawing_font();
    let cmap_entry = entry(&bytes, b"cmap");
    let cmap = span(bytes[cmap_entry..cmap_entry + 16].try_into().unwrap()).0 as usize;
    for (index, code) in [0x20, 0xff1a].into_iter().enumerate() {
        put32(&mut bytes, cmap + 28 + index * 12, code);
        put32(&mut bytes, cmap + 32 + index * 12, code);
    }
    finish_checksums(&mut bytes);
    bytes
}

/// Original two-face TrueType collection: face 0 is the geometric font and
/// face 1 the symbol font. Table offsets become file-relative.
pub fn collection_font() -> Vec<u8> {
    let faces = [drawing_font(), symbol_font()];
    let mut bytes = vec![0; 12 + 4 * faces.len()];
    bytes[..4].copy_from_slice(b"ttcf");
    put16(&mut bytes, 4, 1);
    put32(&mut bytes, 8, faces.len() as u32);
    for (index, face) in faces.iter().enumerate() {
        let base = bytes.len();
        put32(&mut bytes, 12 + 4 * index, base as u32);
        let mut face = face.clone();
        let count = u16::from_be_bytes([face[4], face[5]]) as usize;
        for table in 0..count {
            let at = 12 + 16 * table + 8;
            let offset = u32::from_be_bytes(face[at..at + 4].try_into().unwrap());
            put32(&mut face, at, offset + base as u32);
        }
        bytes.extend(face);
    }
    bytes
}

// SFNT table helpers shared by the fixture builders and unit tests.

pub type Tables = Vec<([u8; 4], Vec<u8>)>;

pub fn get32(bytes: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap())
}

pub fn tables(font: &[u8]) -> Tables {
    let count = u16::from_be_bytes([font[4], font[5]]) as usize;
    (0..count)
        .map(|index| {
            let entry = 12 + 16 * index;
            let offset = get32(font, entry + 8) as usize;
            let length = get32(font, entry + 12) as usize;
            let tag = font[entry..entry + 4].try_into().unwrap();
            (tag, font[offset..offset + length].to_vec())
        })
        .collect()
}

pub fn table<'t>(tables: &'t mut Tables, tag: &[u8; 4]) -> &'t mut Vec<u8> {
    &mut tables.iter_mut().find(|table| &table.0 == tag).unwrap().1
}

pub fn build(mut tables: Tables) -> Vec<u8> {
    tables.sort_by_key(|table| table.0);
    let mut font = vec![0; 12 + 16 * tables.len()];
    font[..4].copy_from_slice(&0x0001_0000_u32.to_be_bytes());
    put16(&mut font, 4, tables.len() as u16);
    for (index, (tag, bytes)) in tables.iter().enumerate() {
        let entry = 12 + 16 * index;
        let offset = font.len();
        font[entry..entry + 4].copy_from_slice(tag);
        font[entry + 8..entry + 12].copy_from_slice(&(offset as u32).to_be_bytes());
        font[entry + 12..entry + 16].copy_from_slice(&(bytes.len() as u32).to_be_bytes());
        font.extend(bytes);
        font.resize(font.len().next_multiple_of(4), 0);
    }
    font
}

// Original CFF fixtures: Type 2 charstrings for the same geometric shapes,
// with global and local subroutines and hint masks.

pub const RMOVETO: u8 = 21;
pub const RLINETO: u8 = 5;
pub const ENDCHAR: u8 = 14;
pub const CALLSUBR: u8 = 10;
pub const CALLGSUBR: u8 = 29;
pub const RETURN: u8 = 11;

/// A Type 2 / DICT integer in its shortest encoding.
pub fn num(value: i32) -> Vec<u8> {
    match value {
        -107..=107 => vec![(value + 139) as u8],
        108..=1131 => vec![
            ((value - 108) / 256 + 247) as u8,
            ((value - 108) % 256) as u8,
        ],
        -1131..=-108 => vec![
            ((-value - 108) / 256 + 251) as u8,
            ((-value - 108) % 256) as u8,
        ],
        _ => {
            let mut bytes = vec![28];
            bytes.extend((value as i16).to_be_bytes());
            bytes
        }
    }
}

pub fn ops(parts: &[&[i32]]) -> Vec<u8> {
    // Each part is operands followed by one operator; operators >= 1200 are
    // two-byte escapes, and -1 marks a raw byte (hint mask data).
    let mut out = Vec::new();
    for part in parts {
        let (operator, operands) = part.split_last().unwrap();
        for value in operands {
            out.extend(num(*value));
        }
        match *operator {
            raw if raw < 0 => out.push((-raw - 1) as u8),
            escape if escape >= 1200 => out.extend([12, (escape - 1200) as u8]),
            operator => out.push(operator as u8),
        }
    }
    out
}

pub fn index(items: &[Vec<u8>]) -> Vec<u8> {
    let mut out = (items.len() as u16).to_be_bytes().to_vec();
    if items.is_empty() {
        return out;
    }
    let data = items.iter().map(Vec::len).sum::<usize>() + 1;
    let size = (1..4).find(|size| data < 1 << (8 * size)).unwrap_or(4);
    out.push(size as u8);
    let mut offset = 1_usize;
    for item in std::iter::once(&Vec::new()).chain(items) {
        offset += item.len();
        out.extend(&(offset as u32).to_be_bytes()[4 - size..]);
    }
    for item in items {
        out.extend(item);
    }
    out
}

fn int5(out: &mut Vec<u8>, value: u64) {
    out.push(29);
    out.extend((value as i32).to_be_bytes());
}

pub fn dict_int(value: i64, op: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    int5(&mut out, value as u64);
    out.extend(op);
    out
}

/// Original charstrings: .notdef, a rectangle (`A`) through a local subr, a
/// triangle (`中`) through a global subr that calls a local one, and a hinted
/// square (`B`) with hintmask, cntrmask, implicit stems and a dotsection.
pub fn charstrings() -> Vec<Vec<u8>> {
    let local = |number: i32| number - 107;
    vec![
        ops(&[&[ENDCHAR as i32]]),
        [
            ops(&[&[500, 0, 0, RMOVETO as i32], &[local(0), CALLSUBR as i32]]),
            ops(&[&[ENDCHAR as i32]]),
        ]
        .concat(),
        [
            ops(&[&[0, 0, RMOVETO as i32], &[-107, CALLGSUBR as i32]]),
            ops(&[&[ENDCHAR as i32]]),
        ]
        .concat(),
        [ops(&[
            // One hstem and two vstems, then cntrmask with an implied
            // fourth (vertical) stem: masks are one byte.
            &[0, 10, 1],
            &[0, 10, 20, 10, 23],
            &[0, 5, 20],
            &[-0b1111_0001],
            &[19],
            &[-0b1110_0001],
            &[0, 0, RMOVETO as i32],
            &[100, 6],
            &[100, 7],
            &[-100, 6],
            &[1200],
            &[ENDCHAR as i32],
        ])]
        .concat(),
    ]
}

pub fn local_subrs() -> Vec<Vec<u8>> {
    vec![
        ops(&[
            &[400, 0, RLINETO as i32],
            &[0, 700, RLINETO as i32],
            &[-400, 0, RLINETO as i32],
            &[RETURN as i32],
        ]),
        ops(&[&[-400, 700, RLINETO as i32], &[RETURN as i32]]),
    ]
}

pub fn global_subrs() -> Vec<Vec<u8>> {
    vec![ops(&[
        &[800, 0, RLINETO as i32],
        &[-106, CALLSUBR as i32],
        &[RETURN as i32],
    ])]
}

/// FontMatrix [0.001 0 0 0.001 0 0] as real and integer operands.
pub fn matrix() -> Vec<u8> {
    let thousandth = [30, 0x1c, 0x3f];
    [
        &thousandth[..],
        &[139, 139],
        &thousandth,
        &[139, 139, 12, 7],
    ]
    .concat()
}

#[derive(Clone)]
pub struct CffOptions {
    pub cid: bool,
    pub select_format: u8,
    pub charstrings: Vec<Vec<u8>>,
    pub global: Vec<Vec<u8>>,
    pub local: Vec<Vec<u8>>,
    /// Whether Private DICTs reference the local subroutines.
    pub subrs: bool,
    /// Entries placed before the Top and Private DICTs' own.
    pub top_extra: Vec<u8>,
    pub private_extra: Vec<u8>,
    /// Raw FDSelect of a CID-keyed font instead of one in `select_format`.
    pub select: Option<Vec<u8>>,
}

impl Default for CffOptions {
    fn default() -> Self {
        Self {
            cid: false,
            select_format: 3,
            charstrings: charstrings(),
            global: global_subrs(),
            local: local_subrs(),
            subrs: true,
            top_extra: Vec::new(),
            private_extra: Vec::new(),
            select: None,
        }
    }
}

/// Assemble a CFF table. Top DICT offsets are five-byte integers, so the
/// layout is computed with placeholder offsets first.
pub fn cff(options: &CffOptions) -> Vec<u8> {
    let glyphs = options.charstrings.len();
    let local = index(&options.local);
    let private = |subrs_at: i64| {
        let mut private = options.private_extra.clone();
        if options.subrs {
            private.extend(dict_int(subrs_at, &[19]));
        }
        private
    };
    let private_length = private(0).len();
    let layout = |[charstrings_at, private_at, array_at, select_at, charset_at]: [i64; 5]| {
        let mut top = options.top_extra.clone();
        if options.cid {
            top.extend([&num(391)[..], &num(392), &num(0), &[12, 30]].concat());
            top.extend(matrix());
            top.extend(dict_int(array_at, &[12, 36]));
            top.extend(dict_int(select_at, &[12, 37]));
        } else {
            let mut entry = Vec::new();
            int5(&mut entry, private_length as u64);
            int5(&mut entry, private_at as u64);
            entry.push(18);
            top.extend(entry);
        }
        top.extend(dict_int(charstrings_at, &[17]));
        top.extend(dict_int(charset_at, &[15]));
        let mut head = vec![1, 0, 4, 1];
        head.extend(index(&[b"Fixture".to_vec()]));
        head.extend(index(&[top]));
        let strings = if options.cid {
            vec![b"Adobe".to_vec(), b"Identity".to_vec()]
        } else {
            Vec::new()
        };
        head.extend(index(&strings));
        head.extend(index(&options.global));
        head
    };
    let head_length = layout([0; 5]).len() as i64;
    let charstrings = index(&options.charstrings);
    let charstrings_at = head_length;
    let private_at = charstrings_at + charstrings.len() as i64;
    // A CID font has two font DICTs with identical Private DICTs.
    let font_dict = |at: i64| {
        let mut dict = Vec::new();
        int5(&mut dict, private_length as u64);
        int5(&mut dict, at as u64);
        dict.push(18);
        dict
    };
    let array_at = private_at + 2 * (private_length + local.len()) as i64;
    let array = index(&[font_dict(0), font_dict(0)]);
    // A name-keyed font has no FDArray or FDSelect.
    let select_at = array_at + if options.cid { array.len() as i64 } else { 0 };
    let select = match (options.cid, &options.select, options.select_format) {
        (false, _, _) => Vec::new(),
        (true, Some(select), _) => select.clone(),
        (true, None, 0) => [
            vec![0],
            (0..glyphs).map(|glyph| u8::from(glyph >= 2)).collect(),
        ]
        .concat(),
        (true, None, _) => [
            &[3, 0, 2, 0, 0, 0, 0, 2, 1][..],
            &(glyphs as u16).to_be_bytes(),
        ]
        .concat(),
    };
    let charset_at = select_at + select.len() as i64;
    let mut bytes = layout([charstrings_at, private_at, array_at, select_at, charset_at]);
    bytes.extend(&charstrings);
    for _ in 0..2 {
        bytes.extend(private(private_length as i64));
        bytes.extend(&local);
    }
    if options.cid {
        let mut array = Vec::new();
        let dicts: Vec<Vec<u8>> = (0..2)
            .map(|fd| font_dict(private_at + fd * (private_length + local.len()) as i64))
            .collect();
        array.extend(index(&dicts));
        bytes.extend(array);
    }
    bytes.extend(select);
    // Format 0 charset: glyph n has CID n, or standard string ID n.
    bytes.push(0);
    for glyph in 1..glyphs as u16 {
        bytes.extend(glyph.to_be_bytes());
    }
    bytes
}

/// The original geometric font's metadata with CFF outlines: glyphs
/// .notdef, `A`, `中` and `B`.
pub fn otf(options: &CffOptions) -> Vec<u8> {
    let mut tables: Tables = tables(&drawing_font())
        .into_iter()
        .filter(|(tag, _)| tag != b"glyf" && tag != b"loca")
        .collect();
    let glyphs = options.charstrings.len() as u16;
    let maxp = table(&mut tables, b"maxp");
    maxp.truncate(6);
    maxp[..4].copy_from_slice(&0x5000_u32.to_be_bytes());
    maxp[4..6].copy_from_slice(&glyphs.to_be_bytes());
    table(&mut tables, b"hhea")[34..36].copy_from_slice(&glyphs.to_be_bytes());
    let hmtx = table(&mut tables, b"hmtx");
    hmtx.clear();
    for advance in [500_u16, 500, 1000, 600]
        .into_iter()
        .cycle()
        .take(usize::from(glyphs))
    {
        hmtx.extend(advance.to_be_bytes());
        hmtx.extend([0, 0]);
    }
    let cmap = table(&mut tables, b"cmap");
    let groups = [(0x41_u32, 1_u32), (0x42, 3), (0x4e2d, 2)];
    cmap.truncate(16);
    cmap[20 - 4..].fill(0);
    cmap.extend((16 + 12 * groups.len() as u32).to_be_bytes());
    cmap.extend(0_u32.to_be_bytes());
    cmap.extend((groups.len() as u32).to_be_bytes());
    for (code, glyph) in groups {
        cmap.extend(code.to_be_bytes());
        cmap.extend(code.to_be_bytes());
        cmap.extend(glyph.to_be_bytes());
    }
    tables.push((*b"CFF ", cff(options)));
    let mut font = build(tables);
    font[..4].copy_from_slice(b"OTTO");
    font
}

fn finish_checksums(bytes: &mut [u8]) {
    let head_entry = entry(bytes, b"head");
    let head = span(bytes[head_entry..head_entry + 16].try_into().unwrap()).0 as usize;
    put32(bytes, head + 8, 0);
    let sum = |bytes: &[u8]| {
        bytes.chunks(4).fold(0_u32, |sum, chunk| {
            let mut word = [0; 4];
            word[..chunk.len()].copy_from_slice(chunk);
            sum.wrapping_add(u32::from_be_bytes(word))
        })
    };
    let count = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
    let power = 1 << count.ilog2();
    put16(bytes, 6, power * 16);
    put16(bytes, 8, count.ilog2() as u16);
    put16(bytes, 10, count as u16 * 16 - power * 16);
    for i in 0..count {
        let at = 12 + i * 16;
        let (offset, length) = span(bytes[at..at + 16].try_into().unwrap());
        let checksum = sum(&bytes[offset as usize..(offset + length) as usize]);
        put32(bytes, at + 4, checksum);
    }
    let adjustment = 0xb1b0afba_u32.wrapping_sub(sum(bytes));
    put32(bytes, head + 8, adjustment);
}

fn span(entry: &[u8; 16]) -> (u64, u64) {
    (
        u64::from(u32::from_be_bytes(entry[8..12].try_into().unwrap())),
        u64::from(u32::from_be_bytes(entry[12..16].try_into().unwrap())),
    )
}

// Also usable as a standalone generator for cross-runtime test fixtures.
fn main() {
    use std::io::Write;
    let bytes = match std::env::args().nth(1).as_deref() {
        None => drawing_font(),
        Some("symbols") => symbol_font(),
        Some("collection") => collection_font(),
        Some("cff") => otf(&CffOptions::default()),
        _ => panic!("expected no argument, symbols, collection or cff"),
    };
    std::io::stdout().write_all(&bytes).unwrap();
}
