// SPDX-License-Identifier: MIT

//! Minimal JSON string encoding for the inspection report (RFC 8259 §7).

use std::io::{self, Write};

/// Write `value` as a quoted JSON string. Quotes, backslashes, and control
/// characters are escaped; all other characters are emitted as UTF-8.
pub fn write_string<W: Write>(out: &mut W, value: &str) -> io::Result<()> {
    out.write_all(b"\"")?;
    let mut start = 0;
    for (index, character) in value.char_indices() {
        let escape = match character {
            '"' => "\\\"",
            '\\' => "\\\\",
            '\n' => "\\n",
            '\r' => "\\r",
            '\t' => "\\t",
            c if c < ' ' => "",
            _ => continue,
        };
        out.write_all(&value.as_bytes()[start..index])?;
        if escape.is_empty() {
            write!(out, "\\u{:04x}", u32::from(character))?;
        } else {
            out.write_all(escape.as_bytes())?;
        }
        start = index + character.len_utf8();
    }
    out.write_all(&value.as_bytes()[start..])?;
    out.write_all(b"\"")
}

/// Write an optional unsigned number, or `null`.
pub fn write_number<W: Write>(out: &mut W, value: Option<u32>) -> io::Result<()> {
    match value {
        Some(value) => write!(out, "{value}"),
        None => out.write_all(b"null"),
    }
}
