// SPDX-License-Identifier: MIT

//! Recognition only for the independently observed CAA descriptor profile.
//! See the CAA/NH discovery reference in docs/provenance.md. Opaque targets
//! are neither decoded nor resolved; this module allocates nothing.

pub(super) fn recognizes(prefix: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(prefix) else {
        return false;
    };
    // Require a line terminator for every field, including DOCTYPE, so a
    // truncated prefix cannot turn DOCTYPE=NH... into a complete field.
    let mut lines = text.split_inclusive('\n');
    fn complete_line(line: &str) -> Option<&str> {
        line.strip_suffix('\n')
            .map(|line| line.strip_suffix('\r').unwrap_or(line))
    }
    if lines.next().and_then(complete_line) != Some("[TARGET]") {
        return false;
    }
    for key in [
        "A1=", "A2=", "B1=", "B2=", "C1=", "C2=", "D1=", "D2=", "DOCTYPE=",
    ] {
        let Some(value) = lines
            .next()
            .and_then(complete_line)
            .and_then(|line| line.strip_prefix(key))
        else {
            return false;
        };
        let valid = match key {
            "A1=" | "D1=" => !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()),
            "A2=" | "D2=" => {
                !value.is_empty()
                    && value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
            }
            "B1=" | "C1=" => value == "0",
            "B2=" | "C2=" => value.is_empty(),
            _ => matches!(value, "NH" | "KDH"),
        };
        if !valid {
            return false;
        }
    }
    lines.all(|line| {
        line.bytes()
            .all(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
    })
}
