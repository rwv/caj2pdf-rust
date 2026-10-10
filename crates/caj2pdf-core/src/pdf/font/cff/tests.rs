// SPDX-License-Identifier: MIT

use super::*;
use crate::ErrorKind;
use crate::native::SeekableSource;
use crate::pdf::font::tests::{
    CALLSUBR, CffOptions as Options, ENDCHAR, RETURN, RLINETO, RMOVETO, Tables, build, charstrings,
    entry, get32, num, ops, otf, table, tables,
};
use crate::test_support::NEVER;
use std::io::Cursor;
use xberg_ttf_parser::{Face, GlyphId, OutlineBuilder};

#[derive(Default)]
struct Points(Vec<(i32, i32)>);

impl OutlineBuilder for Points {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.push((x as i32, y as i32));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.push((x as i32, y as i32));
    }
    fn quad_to(&mut self, _: f32, _: f32, x: f32, y: f32) {
        self.0.push((x as i32, y as i32));
    }
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, x: f32, y: f32) {
        self.0.push((x as i32, y as i32));
    }
    fn close(&mut self) {}
}

fn outline(face: &Face<'_>, glyph: u16) -> Vec<(i32, i32)> {
    let mut points = Points::default();
    face.outline_glyph(GlyphId(glyph), &mut points);
    points.0
}

fn used(characters: &[char]) -> Vec<u8> {
    let mut bitmap = vec![0; 8192];
    for character in characters {
        super::super::mark_code(&mut bitmap, *character as usize);
    }
    bitmap
}

/// The CFF subset of `characters` and the source bytes read to build it.
fn plan(font: &[u8], characters: &[char]) -> Result<(CffSubset, u64)> {
    plan_within(font, characters, &Limits::default())
}

fn plan_within(font: &[u8], characters: &[char], limits: &Limits) -> Result<(CffSubset, u64)> {
    let mut source = SeekableSource::new(Cursor::new(font.to_vec())).unwrap();
    (|| {
        let mut font = OpenTypeFont::read(&mut source, 0, &Limits::default(), &NEVER)?;
        let cff = font.cff.clone().unwrap();
        let subset = font.plan_cff(
            &cff,
            crate::pdf::font::Characters::Unicode(&used(characters)),
            u64::MAX,
            limits,
            &NEVER,
        )?;
        Ok((subset, font.subset_bytes_read))
    })()
}

fn subset(font: &[u8], characters: &[char]) -> Result<Vec<u8>> {
    plan(font, characters).map(|(subset, _)| subset.parts().concat())
}

#[test]
fn mapped_cff_capacity_includes_notdef_and_checks_pair_allocation() {
    use crate::pdf::font::{Characters, GlyphCharacters};
    let limits = Limits::default();
    let mut source = SeekableSource::new(Cursor::new(otf(&Options::default()))).unwrap();
    let mut font = OpenTypeFont::read(&mut source, 0, &limits, &NEVER).unwrap();
    let cff = Rc::clone(font.cff.as_ref().unwrap());
    let entries = vec![
        GlyphCharacters {
            glyph: 'A',
            text: 'X'
        };
        usize::from(u16::MAX)
    ];
    let error = font
        .plan_cff(
            &cff,
            Characters::Mapped(&entries),
            u64::MAX,
            &limits,
            &NEVER,
        )
        .err()
        .unwrap();
    assert_eq!(
        error.reason,
        "CFF subset exceeds 65535 glyphs including .notdef"
    );
    let small = Limits {
        max_allocation_bytes: 1,
        ..Limits::default()
    };
    assert!(matches!(
        font.plan_cff(
            &cff,
            Characters::Mapped(&entries[..2]),
            u64::MAX,
            &small,
            &NEVER
        ),
        Err(Error {
            kind: ErrorKind::LimitExceeded { .. },
            ..
        })
    ));
    let plan = font
        .plan_cff(
            &cff,
            Characters::Mapped(&entries[..entries.len() - 1]),
            u64::MAX,
            &limits,
            &NEVER,
        )
        .unwrap();
    let (_, cids, _) = structure(&plan.parts().concat());
    assert_eq!(cids.len(), usize::from(u16::MAX) - 1);
    assert_eq!(cids.first(), Some(&1));
    assert_eq!(cids.last(), Some(&(u16::MAX - 1)));
}

/// Wrap a subset CFF in the source's metadata so it can be outlined.
fn wrap(program: &[u8], source: &[u8]) -> Vec<u8> {
    let mut tables: Tables = tables(source)
        .into_iter()
        .filter(|(tag, _)| tag != b"CFF ")
        .collect();
    tables.push((*b"CFF ", program.to_vec()));
    let mut font = build(tables);
    font[..4].copy_from_slice(b"OTTO");
    font
}

/// Top DICT entries as operators with their operand values.
type TopDict = Vec<(u16, Vec<i64>)>;

/// Top DICT entries, charset CIDs and FDSelect of a subset program.
fn structure(program: &[u8]) -> (TopDict, Vec<u16>, Vec<u8>) {
    let mut source = SeekableSource::new(Cursor::new(program.to_vec())).unwrap();
    let limits = Limits::default();
    {
        let mut reader = Reader {
            source: &mut source,
            limits: &limits,
            cancellation: &NEVER,
            start: 0,
            end: program.len() as u64,
            read: 0,
        };
        let names = reader.index(4).unwrap();
        let tops = reader.index(names.end).unwrap();
        let top = reader.object_bytes(&tops, 0, MAX_DICT_BYTES).unwrap();
        let top: TopDict = dict(&top)
            .unwrap()
            .into_iter()
            .map(|entry| (entry.op, entry.values))
            .collect();
        let at = |op| top.iter().find(|(entry, _)| *entry == op).unwrap().1[0] as usize;
        let charstrings = reader.index(at(OP_CHARSTRINGS) as u64).unwrap();
        let count = charstrings.count as usize;
        let charset = program[at(15)..at(15) + 1 + 2 * (count - 1)].to_vec();
        assert_eq!(charset[0], 0);
        let cids = charset[1..]
            .chunks(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
            .collect();
        let select = program[at(OP_FD_SELECT)..].to_vec();
        (top, cids, select)
    }
}

#[test]
fn name_keyed_fonts_become_desubroutinized_cid_subsets() {
    let options = Options::default();
    let font = otf(&options);
    let subset = subset(&font, &['中', 'B', 'A']).unwrap();
    let source = Face::parse(&font, 0).unwrap();
    let wrapped = wrap(&subset, &font);
    let face = Face::parse(&wrapped, 0).unwrap();
    // .notdef, then A, B and 中 in Unicode order.
    assert_eq!(face.number_of_glyphs(), 4);
    for (subset, original) in [(1, 1), (2, 3), (3, 2)] {
        assert_eq!(
            outline(&face, subset),
            outline(&source, original),
            "{subset}"
        );
        assert!(!outline(&face, subset).is_empty());
    }
    let (top, cids, select) = structure(&subset);
    assert_eq!(cids, [0x41, 0x42, 0x4e2d]);
    assert!(top.contains(&(OP_ROS, vec![391, 392, 0])));
    assert!(top.contains(&(1234, vec![0x4e2e])));
    // One font DICT for every glyph.
    assert_eq!(&select[..8], [3, 0, 1, 0, 0, 0, 0, 4]);
    // The subset is measured and tagged by its program bytes.
    let (planned, _) = plan(&font, &['中', 'B', 'A']).unwrap();
    let planned = super::super::subset::Subset::Cff(planned);
    assert_eq!(planned.length(), subset.len() as u64);
    assert_eq!(planned.tag("Fixture").len(), 6);
}

#[test]
fn cid_keyed_fonts_keep_used_font_dicts_and_matrices() {
    for select_format in [0, 3] {
        let options = Options {
            cid: true,
            select_format,
            ..Options::default()
        };
        let font = otf(&options);
        let source = Face::parse(&font, 0).unwrap();
        // Only FD 1 is used: 中 and B. .notdef uses FD 0.
        let subset = subset(&font, &['中', 'B']).unwrap();
        let wrapped = wrap(&subset, &font);
        let face = Face::parse(&wrapped, 0).unwrap();
        assert_eq!(outline(&face, 1), outline(&source, 3));
        assert_eq!(outline(&face, 2), outline(&source, 2));
        let (top, cids, select) = structure(&subset);
        assert_eq!(cids, [0x42, 0x4e2d]);
        assert!(top.iter().any(|(op, _)| *op == OP_FONT_MATRIX));
        assert_eq!(&select[..11], [3, 0, 2, 0, 0, 0, 0, 1, 1, 0, 3]);
    }
    // An empty subset keeps only .notdef.
    let font = otf(&Options {
        cid: true,
        ..Options::default()
    });
    let (_, cids, _) = structure(&subset(&font, &[]).unwrap());
    assert!(cids.is_empty());
}

fn reason(result: Result<Vec<u8>>) -> &'static str {
    match result {
        Err(Error {
            kind: ErrorKind::Malformed,
            reason,
            ..
        }) => reason,
        Err(Error {
            kind: ErrorKind::LimitExceeded { resource, .. },
            ..
        }) => resource,
        Err(other) => panic!("unexpected {other:?}"),
        Ok(_) => "ok",
    }
}

#[test]
fn malformed_charstrings_fail_closed() {
    let a = |glyph: Vec<u8>| {
        let mut charstrings = charstrings();
        charstrings[1] = glyph;
        Options {
            charstrings,
            ..Options::default()
        }
    };
    let deep = Options {
        local: vec![ops(&[&[-107, CALLSUBR as i32], &[RETURN as i32]])],
        ..a(ops(&[&[-107, CALLSUBR as i32], &[ENDCHAR as i32]]))
    };
    let big = Options {
        local: vec![[ops(&[&[1, 1, RLINETO as i32]]).repeat(3000), vec![RETURN]].concat()],
        ..a([ops(&[&[-107, CALLSUBR as i32]]).repeat(40), vec![ENDCHAR]].concat())
    };
    // Subroutine `i` calls subroutine `i + 1` six times: about 6^9 calls
    // from a few hundred bytes.
    let mut nested: Vec<Vec<u8>> = (1..10)
        .map(|next| {
            [
                ops(&[&[next - 107, CALLSUBR as i32]]).repeat(6),
                vec![RETURN],
            ]
            .concat()
        })
        .collect();
    nested.push(vec![RETURN]);
    let amplified = Options {
        local: nested,
        ..a(ops(&[&[-107, CALLSUBR as i32], &[ENDCHAR as i32]]))
    };
    let cases = [
        (
            a(ops(&[&[0, 0, RMOVETO as i32]])),
            "CFF charstring ends without endchar or return",
        ),
        (a(vec![255, 0, 1]), "CFF charstring is truncated"),
        (
            a(ops(&[&[RETURN as i32]])),
            "CFF charstring returns outside a subroutine",
        ),
        (
            a(ops(&[&[0, 0, 0, 0, ENDCHAR as i32]])),
            "CFF accented-character endchar is not supported",
        ),
        (
            a(ops(&[&[CALLSUBR as i32]])),
            "CFF subroutine call has no number",
        ),
        (
            a(vec![255, 0, 0, 0, 0, CALLSUBR]),
            "CFF subroutine number is not an integer",
        ),
        (
            a(ops(&[&[-1000, CALLSUBR as i32]])),
            "CFF subroutine number is out of range",
        ),
        (
            a(ops(&[&[100, CALLSUBR as i32]])),
            "CFF INDEX object is out of range",
        ),
        (
            a(ops(&[&[2000, CALLSUBR as i32]])),
            "CFF INDEX object is out of range",
        ),
        (a(ops(&[&[1210]])), "unsupported CFF charstring operator"),
        (a(ops(&[&[0]])), "invalid CFF charstring operator"),
        (
            a([num(1).repeat(49), vec![ENDCHAR]].concat()),
            "CFF charstring argument stack overflows",
        ),
        (deep, "CFF subroutines nest too deeply"),
        (big, "CFF charstring bytes"),
        (amplified, "CFF charstring bytes"),
        (
            Options {
                subrs: false,
                ..Options::default()
            },
            "CFF charstring calls missing subroutines",
        ),
    ];
    for (index, (options, expected)) in cases.into_iter().enumerate() {
        let font = otf(&options);
        assert_eq!(reason(subset(&font, &['A'])), expected, "case {index}");
    }
}

#[test]
fn each_subroutine_is_read_once_per_subset() {
    // A calls local subroutine 1 two hundred times; 中 calls it through a
    // global subroutine as well.
    let mut charstrings = charstrings();
    charstrings[1] = [
        ops(&[&[0, 0, RMOVETO as i32]]),
        ops(&[&[-106, CALLSUBR as i32]]).repeat(200),
        vec![ENDCHAR],
    ]
    .concat();
    let font = otf(&Options {
        charstrings,
        ..Options::default()
    });
    let (subset, read) = plan(&font, &['A', '中']).unwrap();
    assert!(subset.length() > 200 * 4);
    assert!(read < font.len() as u64, "{read}");
    // Charstrings and cached subroutines count toward the allocation limit.
    let limits = Limits {
        max_allocation_bytes: 600,
        ..Limits::default()
    };
    assert!(matches!(
        plan_within(&font, &['A', '中'], &limits),
        Err(Error {
            kind: ErrorKind::LimitExceeded {
                resource: "allocation bytes",
                ..
            },
            ..
        })
    ));
}

fn top_edit(edit: impl Fn(&mut Options)) -> Vec<u8> {
    let mut options = Options::default();
    edit(&mut options);
    otf(&options)
}

#[test]
fn malformed_structures_fail_when_the_font_is_read() {
    let read = |font: Vec<u8>| -> &'static str {
        let mut source = SeekableSource::new(Cursor::new(font)).unwrap();
        match OpenTypeFont::read(&mut source, 0, &Limits::default(), &NEVER) {
            Err(Error {
                kind: ErrorKind::Malformed,
                reason,
                ..
            }) => reason,
            Err(Error {
                kind: ErrorKind::LimitExceeded { resource, .. },
                ..
            }) => resource,
            Err(other) => panic!("unexpected {other:?}"),
            Ok(_) => "ok",
        }
    };
    let patched = |edit: fn(&mut [u8])| {
        let mut font = otf(&Options::default());
        let at = get32(&font, entry(&font, b"CFF ") + 8) as usize;
        edit(&mut font[at..]);
        font
    };
    assert_eq!(read(otf(&Options::default())), "ok");
    assert_eq!(read(patched(|cff| cff[0] = 2)), "unsupported CFF header");
    assert_eq!(
        read(patched(|cff| cff[6] = 5)),
        "invalid CFF INDEX offset size"
    );
    assert_eq!(read(patched(|cff| cff[7] = 0)), "invalid CFF INDEX offsets");
    assert_eq!(
        read(top_edit(|options| options.top_extra = vec![139, 139, 17])),
        "CFF DICT operator has the wrong operands"
    );
    assert_eq!(
        read(top_edit(
            |options| options.top_extra = [num(1), vec![12, 6]].concat()
        )),
        "only Type 2 CFF charstrings are supported"
    );
    let mut fewer = tables(&otf(&Options::default()));
    table(&mut fewer, b"maxp")[4..6].copy_from_slice(&3_u16.to_be_bytes());
    let mut fewer = build(fewer);
    fewer[..4].copy_from_slice(b"OTTO");
    assert_eq!(
        read(fewer),
        "CFF CharStrings count differs from the glyph count"
    );
    assert_eq!(
        read(top_edit(|options| options.top_extra = vec![22])),
        "invalid CFF DICT byte"
    );
    assert_eq!(
        read(top_edit(|options| options.top_extra = vec![139; 49])),
        "CFF DICT has too many operands"
    );
    assert_eq!(
        read(top_edit(
            |options| options.top_extra = [num(-1), vec![15]].concat()
        )),
        "ok"
    );
    assert_eq!(
        read(patched(|cff| cff[17] = 0)),
        "a CFF table must hold exactly one font"
    );
    assert_eq!(
        read(top_edit(
            |options| options.top_extra = vec![29, 0, 2, 0, 0, 139, 18]
        )),
        "CFF structure bytes"
    );
    assert_eq!(
        read(top_edit(
            |options| options.top_extra = vec![29, 0, 0x10, 0, 0, 17]
        )),
        "CFF structure is outside its table"
    );
    assert_eq!(
        read(top_edit(
            |options| options.top_extra = [&num(391)[..], &num(392), &num(0), &[12, 30]].concat()
        )),
        "CID-keyed CFF needs an FDArray and FDSelect"
    );
    // Private DICT entries other than Subrs are kept.
    assert_eq!(
        read(top_edit(
            |options| options.private_extra = [num(1), vec![10]].concat()
        )),
        "ok"
    );
    let select = |select: &[u8]| {
        otf(&Options {
            cid: true,
            select: Some(select.to_vec()),
            ..Options::default()
        })
    };
    assert_eq!(read(select(&[0, 0, 0, 1, 1])), "ok");
    assert_eq!(read(select(&[3, 0, 1, 0, 0, 0, 0, 4])), "ok");
    for bad in [
        &[0, 0, 0, 1, 2][..],
        &[3, 0, 0, 0, 4],
        &[3, 0, 1, 0, 1, 0, 0, 4],
        &[3, 0, 2, 0, 0, 0, 0, 0, 1, 0, 4],
        &[3, 0, 1, 0, 0, 2, 0, 4],
        &[3, 0, 1, 0, 0, 0, 0, 3],
    ] {
        assert_eq!(read(select(bad)), "invalid CFF FDSelect", "{bad:?}");
    }
    assert_eq!(read(select(&[1])), "unsupported CFF FDSelect format");
}

#[test]
fn index_lengths_match_written_indexes() {
    for lengths in [&[][..], &[0, 0], &[255], &[300], &[0x1_0000], &[0x100_0000]] {
        let items: Vec<Vec<u8>> = lengths.iter().map(|length| vec![0; *length]).collect();
        let items: Vec<&[u8]> = items.iter().map(|item| &item[..]).collect();
        let mut written = Vec::new();
        index(&mut written, &items);
        assert_eq!(index_length(lengths), written.len(), "{lengths:?}");
    }
}

#[test]
fn dict_parsing_handles_every_operand_form() {
    let bytes = [
        &num(5)[..],
        &num(500),
        &num(-500),
        &num(30_000),
        &[29, 0, 1, 0, 0],
        &[30, 0x1a, 0x5f],
        &[12, 7],
        &[17],
    ]
    .concat();
    let entries = dict(&bytes).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].op, OP_FONT_MATRIX);
    assert_eq!(entries[0].values, [5, 500, -500, 30_000, 65536, 0]);
    assert_eq!(entries[1].op, OP_CHARSTRINGS);
    assert!(matches!(
        dict(&[30, 0x11]),
        Err(Error {
            kind: ErrorKind::Malformed,
            reason: "CFF real number is truncated",
            ..
        })
    ));
    for bad in [&[28, 0][..], &[29][..], &[247][..], &[12][..], &[139][..]] {
        assert!(dict(bad).is_err(), "{bad:?}");
    }
    let entry = &dict(&[&num(-1)[..], &[17]].concat()).unwrap()[0];
    assert!(operands::<1>(entry).is_err());
    assert_eq!([bias(0), bias(1240), bias(33900)], [107, 1131, 32768]);
}
