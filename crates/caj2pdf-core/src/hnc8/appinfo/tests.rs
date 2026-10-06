// SPDX-License-Identifier: MIT

//! Original synthetic C8 sources with generated application-info tails.
//! The XML text is invented; no document values or bytes are used.

use super::*;
use crate::hnc8::Hnc8Reader;
use crate::test_support::{CancelAfter, NEVER};
use crate::test_support::{field_of, kind_name};
use crate::{Limits, native::SeekableSource};
use flate2::{Compression, write::ZlibEncoder};
use std::io::{Cursor, Write};

const VALID: &str = "\u{feff}<?xml version='1.0' encoding='utf-8' ?> \n\
<!-- invented -->\n<Package>\n<Note-Package>\n\
<NoteItems Author=\"a > b\" ReadOnly='0'>\n\
<Item Type=\"1\"><RC l=\"1\" t=\"2\"/><Item Type=\"2\"/></Item>\n\
<Item Type=\"1\"><RC l=\"1\"/><Item Type=\"2\"/></Item>\n\
</NoteItems>\n<NoteItems><Item/></NoteItems>\n</Note-Package >\n\
<FileProperty-Package>\n<DOI> X&amp;Y&#x41;&#66;&lt;&gt;&quot;&apos; </DOI>\
<SCODE><![CDATA[S]]></SCODE><PCODE>P</PCODE>\n<DURL><![CDATA[ http://example.invalid/a?b&c ]]></DURL>\n\
</FileProperty-Package>\n<Item/><DOI>ignored path</DOI>\n</Package>\n<?trailing pi?>\n";

/// One-page C8 header and index, then a short invented body.
fn c8() -> Vec<u8> {
    let mut bytes = vec![0; 0x50 + 20];
    bytes[0] = 0xc8;
    bytes[8] = 1;
    bytes.extend(b"invented page body");
    bytes
}

fn deflate(xml: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(xml).unwrap();
    encoder.finish().unwrap()
}

/// Append `[decoded][compressed][stream]APPINFOSIGN <start>` to `bytes`.
fn framed(mut bytes: Vec<u8>, decoded: u32, stream: &[u8]) -> Vec<u8> {
    let start = bytes.len();
    bytes.extend(decoded.to_le_bytes());
    bytes.extend((stream.len() as u32).to_le_bytes());
    bytes.extend(stream);
    bytes.extend(format!("APPINFOSIGN {start}").as_bytes());
    bytes
}

fn package(xml: &str) -> Vec<u8> {
    framed(c8(), xml.len() as u32, &deflate(xml.as_bytes()))
}

fn read_with(
    bytes: Vec<u8>,
    limits: &Limits,
    cancellation: &CancelAfter,
) -> Result<Option<ApplicationInfo>> {
    (|| {
        let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
        let mut reader = Hnc8Reader::open(&mut source, limits, cancellation)?;
        reader.application_info()
    })()
}

fn read(bytes: Vec<u8>) -> Result<Option<ApplicationInfo>> {
    read_with(bytes, &Limits::default(), &NEVER)
}

fn defect(result: Result<Option<ApplicationInfo>>) -> ApplicationInfoDefect {
    ApplicationInfoDefect::from(&result.unwrap_err())
}

fn xml_error(xml: &str) -> &'static str {
    let found = defect(read(package(xml)));
    assert_eq!(found.field, "application-info XML", "{xml:?}");
    found.reason
}

#[test]
fn valid_package_yields_selected_fields_only() {
    let info = read(package(VALID)).unwrap().unwrap();
    assert_eq!(info.doi.as_deref(), Some("X&YAB<>\"'"));
    assert_eq!(info.url.as_deref(), Some("http://example.invalid/a?b&c"));
    assert_eq!(info.note_count, 3);
    // Small I/O chunks split both the trailer and the stream reads.
    let limits = Limits {
        io_chunk_bytes: 7,
        ..Limits::default()
    };
    assert_eq!(
        read_with(package(VALID), &limits, &NEVER).unwrap(),
        Some(info)
    );
}

#[test]
fn empty_and_missing_values_are_none() {
    for xml in [
        "<Package/>",
        "<?xml version=\"1.0\"?><Package><FileProperty-Package><DOI/><DURL> \t</DURL>\
         </FileProperty-Package></Package>",
    ] {
        assert_eq!(
            read(package(xml)).unwrap(),
            Some(ApplicationInfo::default())
        );
    }
}

#[test]
fn absent_trailers_and_other_variants_have_no_package() {
    assert_eq!(read(c8()).unwrap(), None);
    let mut digits = c8();
    digits.extend([b'7'; 40]);
    assert_eq!(read(digits).unwrap(), None);
    // HN-A is never read, even with a well-formed tail.
    let mut hna = vec![0; 0x15c + 20];
    hna[..8].copy_from_slice(b"HN\0\0\x90\x01\0\0");
    hna[0x90] = 1;
    let hna = framed(hna, VALID.len() as u32, &deflate(VALID.as_bytes()));
    assert_eq!(read(hna).unwrap(), None);
}

#[test]
fn malformed_trailers_are_located() {
    let base = c8().len() as u64;
    for (suffix, reason) in [
        (
            &b"APPINFOSIGN x"[..],
            "marker is not followed by a final decimal offset",
        ),
        (
            b"APPINFOSIGN7",
            "marker is not followed by a final decimal offset",
        ),
        (
            b"APPINFOSIGN 99999999999999999999",
            "offset overflows 64 bits",
        ),
    ] {
        let mut bytes = c8();
        bytes.extend(suffix);
        let found = defect(read(bytes));
        assert_eq!(found.offset, base);
        assert_eq!(found.field, "application-info trailer");
        assert_eq!(found.reason, reason);
    }
}

#[test]
fn declared_offsets_and_lengths_are_checked_against_the_file() {
    let lengths = "application-info lengths";
    let stream = deflate(VALID.as_bytes());
    let mut inside = c8();
    inside.extend(b"APPINFOSIGN 10");
    let mut beyond = c8();
    beyond.extend(b"APPINFOSIGN 999");
    let mut short = c8();
    let start = short.len();
    short.extend([0; 7]);
    short.extend(format!("APPINFOSIGN {start}").as_bytes());
    let mut gap = framed(c8(), VALID.len() as u32, &stream);
    let marker = gap.len() - format!("APPINFOSIGN {}", c8().len()).len();
    gap.insert(marker, 0);
    gap[c8().len() + 4..c8().len() + 8].copy_from_slice(&(stream.len() as u32).to_le_bytes());
    let cases = [
        (inside, "malformed"),
        (beyond, "truncated"),
        (short, "truncated"),
        (gap, "malformed"),
        (framed(c8(), 0, &stream), "malformed"),
    ];
    for (bytes, kind) in cases {
        let error = read(bytes).unwrap_err();
        assert_eq!(field_of(&error), lengths);
        assert_eq!(kind_name(&error), kind, "{error}");
    }
    let mut huge = framed(c8(), 1, &stream);
    huge[c8().len()..c8().len() + 4]
        .copy_from_slice(&(MAX_APPLICATION_INFO_BYTES + 1).to_le_bytes());
    let found = defect(read(huge));
    assert_eq!(
        (found.field, found.reason),
        ("application-info decoded bytes", "limit")
    );
    let mut wide = c8();
    let start = wide.len();
    wide.extend(1_u32.to_le_bytes());
    wide.extend((MAX_APPLICATION_INFO_BYTES + 1).to_le_bytes());
    wide.extend(format!("APPINFOSIGN {start}").as_bytes());
    let found = defect(read(wide));
    assert_eq!(
        (found.offset, found.field),
        (start as u64, "application-info compressed bytes")
    );
}

#[test]
fn stream_defects_are_rejected_without_partial_values() {
    let field = "application-info zlib stream";
    let xml = VALID.as_bytes();
    let stream = deflate(xml);
    let mut corrupt = stream.clone();
    corrupt[2] = 0xff;
    corrupt[3] = 0xff;
    let mut trailing = stream.clone();
    trailing.push(0);
    for (bytes, reason) in [
        (
            framed(c8(), xml.len() as u32, &corrupt),
            "invalid stream or checksum",
        ),
        (
            framed(c8(), xml.len() as u32, &stream[..stream.len() - 6]),
            "truncated stream",
        ),
        (framed(c8(), 4, &[]), "truncated stream"),
        (
            framed(c8(), xml.len() as u32 - 1, &stream),
            "output exceeds the declared length",
        ),
        (
            framed(c8(), xml.len() as u32 + 1, &stream),
            "stream ends before the declared lengths",
        ),
        (
            framed(c8(), xml.len() as u32, &trailing),
            "stream ends before the declared lengths",
        ),
    ] {
        let found = defect(read(bytes));
        assert_eq!((found.field, found.reason), (field, reason));
        assert!(found.offset >= c8().len() as u64 + 8);
    }
}

#[test]
fn allocation_limits_and_cancellation_are_reported() {
    let limits = Limits {
        io_chunk_bytes: 4096,
        max_allocation_bytes: TEXT_DECODER_RESERVATION_BYTES + 64,
        ..Limits::default()
    };
    let found = defect(read_with(package(VALID), &limits, &NEVER));
    assert_eq!(
        (found.field, found.reason),
        ("application-info allocation bytes", "limit")
    );
    // Count the queries of a complete read, then cancel at each one.
    let counter = CancelAfter::never();
    read_with(package(VALID), &Limits::default(), &counter).unwrap();
    let total = counter.queries();
    let mut cancelled = 0;
    for allowed in 0..total {
        let error = read_with(
            package(VALID),
            &Limits::default(),
            &CancelAfter::new(allowed),
        );
        if let Err(error) = error {
            assert_eq!(kind_name(&error), "cancelled");
            cancelled += 1;
        }
    }
    assert!(cancelled > 2, "{cancelled} of {total}");
    let error = {
        let mut source = SeekableSource::new(Cursor::new(package(VALID))).unwrap();
        let limits = Limits::default();
        let mut reader = Hnc8Reader::open(&mut source, &limits, &NEVER).unwrap();
        reader.cancellation = &ALWAYS;
        reader.application_info()
    }
    .unwrap_err();
    assert_eq!((error.offset, kind_name(&error)), (Some(0), "cancelled"));
}

static ALWAYS: CancelAfter = CancelAfter::new(0);

#[test]
fn package_markup_outside_the_admitted_subset_is_rejected() {
    let cases = [
        ("x<Package/>", "text outside the root element"),
        ("<Package/>x", "text outside the root element"),
        ("<?xml version='1.0'", "unterminated processing instruction"),
        (
            "<?xml encoding='GBK'?><Package/>",
            "declared encoding is not UTF-8",
        ),
        (
            "<?xml encoding=GBK?><Package/>",
            "declared encoding is not UTF-8",
        ),
        (
            "<?xml encoding?><Package/>",
            "declared encoding is not UTF-8",
        ),
        (
            "<?xml encoding='utf-8?><Package/>",
            "declared encoding is not UTF-8",
        ),
        ("<!-- open <Package/>", "unterminated comment"),
        (
            "<Package><![CDATA[x</Package>",
            "unterminated character data",
        ),
        (
            "<![CDATA[x]]><Package/>",
            "character data outside the root element",
        ),
        ("<!DOCTYPE p><Package/>", "declarations are not accepted"),
        ("<Package></Package", "unterminated end tag"),
        (
            "<Package></Other>",
            "end tag does not match the open element",
        ),
        ("</Package>", "end tag does not match the open element"),
        ("<Other/>", "root element is not one Package"),
        ("<Package/><Package/>", "root element is not one Package"),
        (
            "<Package><FileProperty-Package><DOI><b/></DOI></FileProperty-Package></Package>",
            "unexpected element inside DOI or DURL",
        ),
        (
            "<Package><FileProperty-Package><DURL/><DURL/></FileProperty-Package></Package>",
            "duplicate DOI or DURL",
        ),
        (" ", "root element is missing or unclosed"),
        ("<Package>", "root element is missing or unclosed"),
    ];
    for (xml, reason) in cases {
        assert_eq!(xml_error(xml), reason, "{xml:?}");
    }
    let invalid = framed(c8(), 3, &deflate(b"<\xff>"));
    assert_eq!(defect(read(invalid)).reason, "package is not UTF-8");
    let deep = format!("<Package>{}", "<a>".repeat(MAX_DEPTH));
    assert_eq!(xml_error(&deep), "elements nest too deeply");
    for tag in [
        "<>",
        "< Package/>",
        "<Package",
        "<Package a>",
        "<Package a=b>",
        "<Package =\"b\">",
        "<Package a=\"x<y\">",
        "<Package a=\"x\"b=\"y\">",
        "<Package a=\"open>",
        "<Package/ >",
        "<Package a = >",
    ] {
        assert_eq!(xml_error(tag), "malformed start tag", "{tag:?}");
    }
}

#[test]
fn entities_and_field_sizes_are_bounded() {
    let doi = |text: &str| {
        format!("<Package><FileProperty-Package><DOI>{text}</DOI></FileProperty-Package></Package>")
    };
    assert_eq!(xml_error(&doi("a&amp")), "unterminated entity reference");
    for reference in [
        "&nbsp;",
        "&#;",
        "&#x;",
        "&#0;",
        "&#+5;",
        "&#xD800;",
        "&#x+1;",
        "&#99999999999;",
    ] {
        assert_eq!(
            xml_error(&doi(reference)),
            "unknown entity or character reference",
            "{reference}"
        );
    }
    let longest = "d".repeat(MAX_APPLICATION_INFO_FIELD_BYTES);
    let info = read(package(&doi(&longest))).unwrap().unwrap();
    assert_eq!(info.doi.unwrap().len(), MAX_APPLICATION_INFO_FIELD_BYTES);
    for text in [
        format!("{longest}e"),
        format!("{longest}&amp;"),
        format!("<![CDATA[{longest}e]]>"),
    ] {
        let found = defect(read(package(&doi(&text))));
        assert_eq!(
            (found.field, found.reason),
            ("application-info field bytes", "limit")
        );
    }
}

#[test]
fn defects_display_their_location_and_reason() {
    let found = defect(read(package("<Other/>")));
    assert_eq!(
        found.to_string(),
        format!(
            "ignored C8 application-info package at byte {}: application-info XML: root element is not one Package",
            c8().len() + 8
        )
    );
    assert_eq!(
        ApplicationInfoStatus::default(),
        ApplicationInfoStatus::Absent
    );
}
