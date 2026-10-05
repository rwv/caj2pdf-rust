// SPDX-License-Identifier: MIT

use super::*;
use crate::native::SeekableSource;
use crate::pdf::drawing_font;
use crate::test_support::{NEVER, run};
use crate::{Error, RangedSource};
use std::io::Cursor;
use xberg_ttf_parser::{GlyphId, OutlineBuilder};

type Tables = Vec<([u8; 4], Vec<u8>)>;
/// An expected error reason and the table edit that should cause it.
type Case = (&'static str, fn(&mut Tables));

fn put16(bytes: &mut [u8], at: usize, value: u16) {
    bytes[at..at + 2].copy_from_slice(&value.to_be_bytes());
}

fn get32(bytes: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn tables(font: &[u8]) -> Tables {
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

fn table<'t>(tables: &'t mut Tables, tag: &[u8; 4]) -> &'t mut Vec<u8> {
    &mut tables.iter_mut().find(|table| &table.0 == tag).unwrap().1
}

fn build(mut tables: Tables) -> Vec<u8> {
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

/// One composite component: flags without `MORE_COMPONENTS`, glyph ID and
/// the argument and transform bytes those flags declare.
fn component(flags: u16, glyph: u16) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend(flags.to_be_bytes());
    bytes.extend(glyph.to_be_bytes());
    let arguments = if flags & ARG_1_AND_2_ARE_WORDS != 0 {
        4
    } else {
        2
    };
    let transform = if flags & WE_HAVE_A_SCALE != 0 {
        &[0x40, 0][..] // 1.0 in F2Dot14
    } else if flags & WE_HAVE_AN_X_AND_Y_SCALE != 0 {
        &[0x40, 0, 0x20, 0][..]
    } else if flags & WE_HAVE_A_TWO_BY_TWO != 0 {
        &[0x40, 0, 0, 0, 0, 0, 0x40, 0][..]
    } else {
        &[][..]
    };
    bytes.extend(std::iter::repeat_n(0, arguments));
    bytes.extend(transform);
    bytes
}

fn composite(parts: &[Vec<u8>], instructions: &[u8]) -> Vec<u8> {
    let mut glyph = vec![0xff, 0xff, 0, 0, 0, 0, 0x03, 0x20, 0x02, 0xbc];
    for (index, part) in parts.iter().enumerate() {
        let mut part = part.clone();
        let mut flags = u16::from_be_bytes([part[0], part[1]]);
        if index + 1 < parts.len() {
            flags |= MORE_COMPONENTS;
        } else if !instructions.is_empty() {
            flags |= 0x0100;
        }
        put16(&mut part, 0, flags);
        glyph.extend(part);
    }
    if !instructions.is_empty() {
        glyph.extend((instructions.len() as u16).to_be_bytes());
        glyph.extend(instructions);
    }
    glyph
}

/// The original geometric font extended with composite glyphs 3 (`B`:
/// rectangle, triangle and rectangle) and 4 (`C`: glyph 3 again), an unused glyph 5
/// (`D`) and optional hinting tables. Glyph locations use `long` format
/// when requested. All outlines are the fixture's synthetic shapes.
fn composite_font(long: bool, hinting: bool) -> Vec<u8> {
    let mut tables = tables(&drawing_font());
    let loca = table(&mut tables, b"loca").clone();
    let glyf = table(&mut tables, b"glyf").clone();
    let simple = |id: usize| {
        let at =
            |index| usize::from(u16::from_be_bytes([loca[index * 2], loca[index * 2 + 1]])) * 2;
        glyf[at(id)..at(id + 1)].to_vec()
    };
    let glyphs = [
        Vec::new(),
        simple(1),
        simple(2),
        composite(
            &[
                component(ARG_1_AND_2_ARE_WORDS | WE_HAVE_A_SCALE, 1),
                component(WE_HAVE_AN_X_AND_Y_SCALE, 2),
                component(0, 1),
            ],
            &[],
        ),
        composite(&[component(WE_HAVE_A_TWO_BY_TWO, 3)], &[0; 100]),
        simple(2),
    ];
    let mut outlines = Vec::new();
    let mut offsets = vec![0_u32];
    for glyph in &glyphs {
        outlines.extend(glyph);
        outlines.resize(outlines.len().next_multiple_of(2), 0);
        offsets.push(outlines.len() as u32);
    }
    *table(&mut tables, b"glyf") = outlines;
    *table(&mut tables, b"loca") = offsets
        .iter()
        .flat_map(|offset| {
            if long {
                offset.to_be_bytes().to_vec()
            } else {
                ((offset / 2) as u16).to_be_bytes().to_vec()
            }
        })
        .collect();
    put16(table(&mut tables, b"head"), 50, u16::from(long));
    put16(table(&mut tables, b"maxp"), 4, glyphs.len() as u16);
    put16(table(&mut tables, b"hhea"), 34, glyphs.len() as u16);
    let hmtx = table(&mut tables, b"hmtx");
    hmtx.clear();
    for (advance, bearing) in [
        (500, 0),
        (600, 0),
        (1000, 0),
        (700, 10),
        (800, 20),
        (900, 30),
    ] {
        hmtx.extend(u16::to_be_bytes(advance));
        hmtx.extend(i16::to_be_bytes(bearing));
    }
    let mut cmap = vec![0; 16];
    put16(&mut cmap, 2, 1);
    put16(&mut cmap, 4, 3);
    put16(&mut cmap, 6, 10);
    cmap[8..12].copy_from_slice(&12_u32.to_be_bytes());
    put16(&mut cmap, 12, 12);
    let groups = [(0x41, 1), (0x42, 3), (0x43, 4), (0x44, 5), (0x4e2d, 2)];
    cmap.extend((16 + 12 * groups.len() as u32).to_be_bytes());
    cmap.extend(0_u32.to_be_bytes());
    cmap.extend((groups.len() as u32).to_be_bytes());
    for (code, glyph) in groups {
        cmap.extend(u32::to_be_bytes(code));
        cmap.extend(u32::to_be_bytes(code));
        cmap.extend(u32::to_be_bytes(glyph));
    }
    *table(&mut tables, b"cmap") = cmap;
    if hinting {
        tables.push((*b"cvt ", vec![0, 1, 0, 2]));
        tables.push((*b"fpgm", vec![0xb0, 0x07, 0x2c]));
        tables.push((*b"prep", vec![0xb8, 0x01, 0xff, 0x85]));
    }
    build(tables)
}

fn used(characters: &[char]) -> Vec<u8> {
    let mut bitmap = vec![0; 8192];
    for character in characters {
        let code = *character as usize;
        bitmap[code / 8] |= 1 << (code % 8);
    }
    bitmap
}

#[derive(Default)]
struct Bytes(Vec<u8>);

impl SubsetOutput for Bytes {
    async fn put(&mut self, bytes: &[u8]) -> Result<()> {
        self.0.extend_from_slice(bytes);
        Ok(())
    }
}

fn subset_with<S: RangedSource>(
    source: &mut S,
    characters: &[char],
    limits: &Limits,
) -> Result<Vec<u8>> {
    run(async {
        let mut font = TrueTypeFont::read(source, limits, &NEVER).await?;
        let plan = font
            .plan_subset(&used(characters), u64::MAX, limits, &NEVER)
            .await?;
        let mut output = Bytes::default();
        font.write_subset(&plan, &mut output, limits, &NEVER)
            .await?;
        assert_eq!(output.0.len() as u64, plan.length());
        Ok(output.0)
    })
}

fn subset(font: Vec<u8>, characters: &[char], chunk: usize) -> Result<Vec<u8>> {
    let mut source = SeekableSource::new(Cursor::new(font)).unwrap();
    let limits = Limits {
        io_chunk_bytes: chunk,
        ..Limits::default()
    };
    subset_with(&mut source, characters, &limits)
}

fn checksum(bytes: &[u8]) -> u32 {
    let mut measure = Measure::default();
    measure.add(bytes);
    measure.sum
}

#[derive(Default)]
struct Bounds(Vec<(f32, f32)>);

impl OutlineBuilder for Bounds {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.push((x, y));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.push((x, y));
    }
    fn quad_to(&mut self, _: f32, _: f32, x: f32, y: f32) {
        self.0.push((x, y));
    }
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, x: f32, y: f32) {
        self.0.push((x, y));
    }
    fn close(&mut self) {}
}

fn points(face: &Face<'_>, glyph: u16) -> Vec<(f32, f32)> {
    let mut points = Bounds::default();
    face.outline_glyph(GlyphId(glyph), &mut points);
    points.0
}

#[test]
fn composite_closure_renumbers_components_with_valid_checksums() {
    for (long, hinting, chunk) in [(false, false, 1), (true, true, 7), (false, true, 4096)] {
        let original = composite_font(long, hinting);
        let program = subset(original.clone(), &['C', 'A'], chunk).unwrap();
        let face = Face::parse(&program, 0).unwrap();
        let source = Face::parse(&original, 0).unwrap();
        // .notdef, A, C, then C's component 3 and its second component 2.
        assert_eq!(face.number_of_glyphs(), 5);
        for (subset, original) in [(1, 1), (2, 4), (3, 3), (4, 2)] {
            assert_eq!(points(&face, subset), points(&source, original), "{subset}");
            assert_eq!(
                face.glyph_hor_advance(GlyphId(subset)),
                source.glyph_hor_advance(GlyphId(original))
            );
            assert_eq!(
                face.glyph_hor_side_bearing(GlyphId(subset)),
                source.glyph_hor_side_bearing(GlyphId(original))
            );
        }
        assert!(!points(&face, 2).is_empty());
        let mut tags = Vec::new();
        for (tag, bytes) in tables(&program) {
            let entry = (0..tags.len() + 1)
                .map(|index| 12 + 16 * index)
                .find(|entry| program[*entry..entry + 4] == tag)
                .unwrap();
            let mut bytes = bytes.clone();
            if &tag == b"head" {
                assert_eq!(u16::from_be_bytes([bytes[50], bytes[51]]), 1);
                bytes[8..12].fill(0);
            }
            assert_eq!(checksum(&bytes), get32(&program, entry + 4));
            tags.push(tag);
        }
        let mut expected = vec![*b"glyf", *b"head", *b"hhea", *b"hmtx", *b"loca", *b"maxp"];
        if hinting {
            expected.extend([*b"cvt ", *b"fpgm", *b"prep"]);
            expected.sort();
            let copied = tables(&program);
            assert_eq!(copied[0].1, [0, 1, 0, 2]);
            assert_eq!(copied[8].1, [0xb8, 0x01, 0xff, 0x85]);
        }
        assert_eq!(tags, expected);
        assert_eq!(checksum(&program), 0xb1b0_afba);
    }
}

#[test]
fn shared_glyphs_and_unused_fonts_keep_only_needed_outlines() {
    // B and C share component glyphs; .notdef alone is a valid subset.
    let program = subset(composite_font(false, false), &['B', 'C'], 64).unwrap();
    assert_eq!(Face::parse(&program, 0).unwrap().number_of_glyphs(), 5);
    let program = subset(composite_font(false, false), &[], 64).unwrap();
    let face = Face::parse(&program, 0).unwrap();
    assert_eq!(face.number_of_glyphs(), 1);
    assert!(points(&face, 0).is_empty());
}

#[test]
fn plan_maps_used_characters_to_subset_glyphs() {
    let mut source = SeekableSource::new(Cursor::new(composite_font(false, false))).unwrap();
    let limits = Limits::default();
    run(async {
        let mut font = TrueTypeFont::read(&mut source, &limits, &NEVER)
            .await
            .unwrap();
        let plan = font
            .plan_subset(&used(&['D', 'A']), u64::MAX, &limits, &NEVER)
            .await
            .unwrap();
        let face = font.face().unwrap();
        assert_eq!(plan.glyph(&face, 'A'), 1);
        assert_eq!(plan.glyph(&face, 'D'), 2);
        assert_eq!(plan.glyph(&face, '中'), 0);
        assert_eq!(plan.glyph(&face, 'Z'), 0);
    });
}

fn malformed(edit: impl Fn(&mut Tables)) -> Vec<u8> {
    let mut tables = tables(&composite_font(true, false));
    edit(&mut tables);
    build(tables)
}

fn set_location(tables: &mut Tables, glyph: usize, offset: u32) {
    table(tables, b"loca")[glyph * 4..glyph * 4 + 4].copy_from_slice(&offset.to_be_bytes());
}

fn component_offset(tables: &mut Tables, glyph: usize) -> usize {
    get32(table(tables, b"loca"), glyph * 4) as usize
}

#[test]
fn malformed_locations_and_components_fail_closed() {
    let cases: [Case; 6] = [
        ("TrueType glyph locations are truncated", |tables| {
            table(tables, b"loca").truncate(20);
        }),
        ("invalid TrueType glyph location", |tables| {
            set_location(tables, 2, 0xffff_0000);
        }),
        ("invalid TrueType glyph location", |tables| {
            let end = table(tables, b"glyf").len() as u32 + 4;
            set_location(tables, 6, end);
            set_location(tables, 5, end);
        }),
        ("composite glyph component is truncated", |tables| {
            let start = component_offset(tables, 3);
            // Claim another component after the last one.
            table(tables, b"glyf")[start + 31] |= MORE_COMPONENTS as u8;
        }),
        ("composite glyph component is truncated", |tables| {
            // End glyph 4 inside its component's eight-byte 2x2 transform.
            let start = component_offset(tables, 4);
            set_location(tables, 5, start as u32 + 16);
        }),
        ("composite glyph references an invalid glyph", |tables| {
            let start = component_offset(tables, 4);
            put16(table(tables, b"glyf"), start + 12, 6);
        }),
    ];
    for (index, (reason, edit)) in cases.into_iter().enumerate() {
        let result = subset(malformed(edit), &['C', 'D'], 64);
        assert!(
            matches!(result, Err(Error::InvalidInput { reason: actual }) if actual == reason),
            "case {index}: {result:?}"
        );
    }
}

#[test]
fn unmapped_used_characters_report_a_changed_source() {
    for character in ['Z', '\u{d7ff}'] {
        let result = subset(composite_font(false, false), &[character], 64);
        assert!(matches!(
            result,
            Err(Error::InvalidInput {
                reason: "font source changed after its metadata was read"
            })
        ));
    }
}

#[test]
fn allocation_limits_bound_glyph_tables_and_composite_buffers() {
    let limits = Limits {
        io_chunk_bytes: 1,
        max_allocation_bytes: 32,
        ..Limits::default()
    };
    let mut source = SeekableSource::new(Cursor::new(composite_font(false, false))).unwrap();
    let mut font = run(TrueTypeFont::read(&mut source, &Limits::default(), &NEVER)).unwrap();
    // Six glyphs need 6 * (2 + 12) bytes of glyph tables.
    assert!(matches!(
        run(font.plan_subset(&used(&['A']), u64::MAX, &limits, &NEVER)),
        Err(Error::LimitExceeded { attempted: 84, .. })
    ));
    let limits = Limits {
        max_allocation_bytes: 96,
        ..limits
    };
    // Glyph 4 with its instructions is 10 + 14 + 2 + 100 bytes.
    assert!(matches!(
        run(font.plan_subset(&used(&['C']), u64::MAX, &limits, &NEVER)),
        Err(Error::LimitExceeded { attempted: 126, .. })
    ));
    run(font.plan_subset(&used(&['B']), u64::MAX, &limits, &NEVER)).unwrap();
}

/// A large virtual source of zeros after its real bytes.
struct Virtual {
    bytes: Vec<u8>,
    size: u64,
}

impl RangedSource for Virtual {
    fn size(&self) -> u64 {
        self.size
    }
    async fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize> {
        for (index, byte) in out.iter_mut().enumerate() {
            *byte = self
                .bytes
                .get(offset as usize + index)
                .copied()
                .unwrap_or(0);
        }
        Ok(out.len())
    }
}

#[test]
fn subset_glyph_data_is_bounded_by_long_locations() {
    let mut tables = tables(&composite_font(true, false));
    let huge: u32 = 3 << 30;
    // Glyphs 1 (A) and 5 (D) both claim the same 3 GiB range.
    for (glyph, offset) in [0, 0, huge, huge, huge, 0, huge].into_iter().enumerate() {
        set_location(&mut tables, glyph, offset);
    }
    let mut bytes = build(tables);
    let glyf = (0..9)
        .map(|index| 12 + 16 * index)
        .find(|entry| &bytes[*entry..entry + 4] == b"glyf")
        .unwrap();
    // Place the virtual glyph data after every real table.
    let offset = bytes.len() as u32;
    bytes[glyf + 8..glyf + 12].copy_from_slice(&offset.to_be_bytes());
    bytes[glyf + 12..glyf + 16].copy_from_slice(&huge.to_be_bytes());
    let mut source = Virtual {
        size: u64::from(offset) + u64::from(huge),
        bytes,
    };
    let result = subset_with(&mut source, &['A', 'D'], &Limits::default());
    assert!(
        matches!(
            result,
            Err(Error::LimitExceeded {
                resource: "font subset program bytes",
                limit: 0xffff_ffff,
                ..
            })
        ),
        "{result:?}"
    );
}

/// Serves `patched` bytes for reads at `trigger` after `after` such reads.
struct Changing {
    bytes: Vec<u8>,
    patched: Vec<u8>,
    trigger: u64,
    after: usize,
    seen: usize,
}

impl RangedSource for Changing {
    fn size(&self) -> u64 {
        self.bytes.len() as u64
    }
    async fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize> {
        if offset == self.trigger {
            self.seen += 1;
        }
        let bytes = if self.seen > self.after {
            &self.patched
        } else {
            &self.bytes
        };
        let start = offset as usize;
        out.copy_from_slice(&bytes[start..start + out.len()]);
        Ok(out.len())
    }
}

#[test]
fn components_changed_between_reads_are_rejected() {
    let original = composite_font(false, false);
    let mut tables = tables(&original);
    let start = 2 * usize::from(u16::from_be_bytes([
        table(&mut tables, b"loca")[8],
        table(&mut tables, b"loca")[9],
    ]));
    let glyf = (0..9)
        .map(|index| 12 + 16 * index)
        .find(|entry| &original[*entry..entry + 4] == b"glyf")
        .unwrap();
    let trigger = u64::from(get32(&original, glyf + 8)) + start as u64;
    let mut patched = original.clone();
    // Glyph 4's only component becomes the unplanned glyph 5.
    put16(&mut patched, trigger as usize + 12, 5);
    let mut source = Changing {
        bytes: original,
        patched,
        trigger,
        after: 2,
        seen: 0,
    };
    let limits = Limits::default();
    let result = subset_with(&mut source, &['C'], &limits);
    assert!(matches!(
        result,
        Err(Error::InvalidInput {
            reason: "font source changed after its metadata was read"
        })
    ));
}

#[test]
fn projected_program_length_is_bounded_before_measuring() {
    let mut source = SeekableSource::new(Cursor::new(composite_font(false, true))).unwrap();
    let limits = Limits::default();
    run(async {
        let mut font = TrueTypeFont::read(&mut source, &limits, &NEVER)
            .await
            .unwrap();
        let used = used(&['C']);
        let plan = font
            .plan_subset(&used, u64::MAX, &limits, &NEVER)
            .await
            .unwrap();
        let length = plan.length();
        let read = font.subset_bytes_read();
        assert!(read > 0);
        font.plan_subset(&used, length, &limits, &NEVER)
            .await
            .unwrap();
        assert_eq!(font.subset_bytes_read(), 2 * read);
        // Glyph data alone fits; the other tables push the program over.
        let error = font.plan_subset(&used, length - 1, &limits, &NEVER).await;
        assert!(matches!(
            error,
            Err(Error::LimitExceeded { limit, attempted, .. })
                if limit == length - 1 && attempted == length
        ));
        // The glyph data bound applies while components are still found.
        let error = font.plan_subset(&used, 40, &limits, &NEVER).await;
        assert!(matches!(error, Err(Error::LimitExceeded { limit: 40, .. })));
    });
}
