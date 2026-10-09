// SPDX-License-Identifier: MIT

//! Observed CAJ download metadata after a PDF's logical EOF. This recognizes
//! only bounded suffixes already read by the tail scanner; it never interprets
//! metadata as PDF syntax or follows a metadata URL.

use crate::{Cancellation, ErrorKind, Result};
use flate2::{Decompress, FlushDecompress, Status};

pub(super) fn recognized<C: Cancellation>(
    suffix: &[u8],
    offset: u64,
    cancellation: &C,
) -> Result<bool> {
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
        return Ok(false);
    }
    if suffix.starts_with(b"WebFastLoadP") || suffix.starts_with(b"WebFastLoadW") {
        return Ok(true);
    }
    if let Some(payload) = suffix.strip_prefix(b"WebFastLoad")
        && !payload.is_empty()
        && !payload.starts_with(b"\xef\xbb\xbf")
    {
        return framed(payload, offset + 11, cancellation);
    }
    Ok(property(suffix))
}

fn property(suffix: &[u8]) -> bool {
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

// Only the observed length-framed zlib profile is admitted. The decoded
// download metadata is discarded, never parsed as PDF or followed as a URL.
fn framed<C: Cancellation>(payload: &[u8], position: u64, cancellation: &C) -> Result<bool> {
    let Some(header) = payload.get(..8) else {
        return Ok(false);
    };
    let decoded = u32::from_le_bytes(header[..4].try_into().unwrap()) as u64;
    let encoded = u32::from_le_bytes(header[4..].try_into().unwrap()) as usize;
    // Match the existing 4 MiB PDF metadata bound without allocating decoded data.
    if decoded == 0 || decoded > 4 * 1024 * 1024 || encoded > payload.len() - 8 {
        return Ok(false);
    }
    let Some(signpost) = payload[8 + encoded..].strip_prefix(b"APPINFOSIGN ") else {
        return Ok(false);
    };
    if signpost != position.to_string().as_bytes() {
        return Ok(false);
    }
    let compressed = &payload[8..8 + encoded];
    let mut inflater = Decompress::new(true);
    let mut scratch = [0_u8; 4096];
    loop {
        if cancellation.is_cancelled() {
            return Err(ErrorKind::Cancelled.into());
        }
        let before = (inflater.total_in(), inflater.total_out());
        let status = inflater.decompress(
            &compressed[before.0 as usize..],
            &mut scratch,
            FlushDecompress::None,
        );
        let Ok(status) = status else {
            return Ok(false);
        };
        if inflater.total_out() > decoded {
            return Ok(false);
        }
        if status == Status::StreamEnd {
            return Ok(inflater.total_in() == encoded as u64 && inflater.total_out() == decoded);
        }
        if before == (inflater.total_in(), inflater.total_out()) {
            return Ok(false);
        }
    }
}
