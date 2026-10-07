// SPDX-License-Identifier: MIT

//! Observed CAJ download metadata after a PDF's logical EOF. This recognizes
//! only bounded suffixes already read by the tail scanner; it never interprets
//! metadata as PDF syntax or follows a metadata URL.

pub(super) fn recognized(suffix: &[u8]) -> bool {
    // Retain the existing ambiguity guard even inside recognized metadata.
    if [
        b"startxref".as_slice(),
        b"%%EOF",
        b"xref",
        b"trailer",
        b"obj",
    ]
    .iter()
    .any(|marker| suffix.windows(marker.len()).any(|window| window == *marker))
    {
        return false;
    }
    if suffix.starts_with(b"WebFastLoadP") || suffix.starts_with(b"WebFastLoadW") {
        return true;
    }
    let suffix = suffix.strip_prefix(b"WebFastLoad").unwrap_or(suffix);
    if suffix.is_empty() {
        return true;
    }
    // The measured FileProperty profile is UTF-8 with a BOM, optionally
    // preceded by WebFastLoad. Do not treat an arbitrary XML/HTML tail as one.
    let Some(xml) = suffix.strip_prefix(b"\xef\xbb\xbf") else {
        return false;
    };
    let Ok(xml) = std::str::from_utf8(xml) else {
        return false;
    };
    let Some(mut fields) = xml
        .strip_prefix("<FileProperty>")
        .and_then(|value| value.strip_suffix("</FileProperty>"))
    else {
        return false;
    };
    for tag in ["Doi", "FileName", "TableName", "Type"] {
        let Some(rest) = fields.strip_prefix('<').and_then(|s| s.strip_prefix(tag)) else {
            return false;
        };
        if let Some(rest) = rest.strip_prefix(" />") {
            fields = rest;
            continue;
        }
        let Some(rest) = rest.strip_prefix('>') else {
            return false;
        };
        let Some((text, closing)) = rest.split_once("</") else {
            return false;
        };
        // Leaf values in this measured profile are plain XML character data.
        // Entities, nested markup and declarations need separate evidence.
        if text.chars().any(|c| {
            matches!(c, '<' | '>' | '&' | '\u{fffe}' | '\u{ffff}')
                || (c < ' ' && !matches!(c, '\t' | '\r' | '\n'))
        }) {
            return false;
        }
        let Some(rest) = closing.strip_prefix(tag).and_then(|s| s.strip_prefix('>')) else {
            return false;
        };
        fields = rest;
    }
    fields.is_empty()
}
