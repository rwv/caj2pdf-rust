// SPDX-License-Identifier: MIT

//! Measured duplicate direct opacity resources, resolved through the live xref.

use super::*;

const MAX_OPACITY_OBJECT: usize = 256;
const OPACITY_DIGITS: usize = 64;

// Integer digit plus 64 decimal places: equality is exact, never rounded by a
// floating-point conversion. The measured values are in the closed unit range.
fn opacity(value: &[u8]) -> Option<[u8; OPACITY_DIGITS + 1]> {
    let value = value.strip_prefix(b"+").unwrap_or(value);
    if value.is_empty() || value.len() > OPACITY_DIGITS + 2 {
        return None;
    }
    let mut parts = value.splitn(2, |&byte| byte == b'.');
    let integer = parts.next()?;
    let fraction = parts.next().unwrap_or_default();
    if integer.is_empty() && fraction.is_empty() || fraction.len() > OPACITY_DIGITS {
        return None;
    }
    let mut result = [0; OPACITY_DIGITS + 1];
    for &byte in integer {
        if !byte.is_ascii_digit() {
            return None;
        }
        result[0] = result[0] * 10 + (byte - b'0');
        if result[0] > 1 {
            return None;
        }
    }
    for (position, &byte) in fraction.iter().enumerate() {
        if !byte.is_ascii_digit() || result[0] == 1 && byte != b'0' {
            return None;
        }
        result[position + 1] = byte - b'0';
    }
    Some(result)
}

impl<S: RangedSource, C: Cancellation> Reader<'_, S, C> {
    pub(super) fn repair_resource_duplicate(
        &mut self,
        at: u64,
        expected: PdfRef,
        slots: &[Option<XrefSlot>],
    ) -> Result<ObjectHead> {
        let mut head = self.load_head_mode(at, Some(expected), true)?;
        let range = self.range;
        let failure = || {
            located_problem(
                range,
                at,
                Some(expected),
                ErrorKind::Malformed,
                "unproved duplicate ExtGState resource",
            )
            .ambiguous_repair()
        };
        let candidate = head.resource_duplicate.take().ok_or_else(failure)?;
        if !matches!(head.tail, ObjectTail::EndObject { .. })
            || head
                .dictionary
                .as_ref()
                .and_then(|dictionary| dictionary.value(b"Type"))
                .and_then(exact_name)
                .as_deref()
                != Some(b"Page")
        {
            return Err(failure());
        }
        let first = self.opacity_resource(candidate.first, slots, at, expected)?;
        let second = self.opacity_resource(candidate.second, slots, at, expected)?;
        if first != second {
            return Err(failure());
        }
        head.bytes[candidate.blank.clone()].fill(b' ');
        // Require the normalized object to pass the ordinary strict parser.
        let mut normalized = parse_object_head(head.bytes)
            .map_err(|issue| self.parse_issue(at, Some(expected), issue))?;
        normalized.resource_duplicate = Some(candidate);
        Ok(normalized)
    }

    fn opacity_resource(
        &mut self,
        reference: PdfRef,
        slots: &[Option<XrefSlot>],
        owner_at: u64,
        owner: PdfRef,
    ) -> Result<[[u8; OPACITY_DIGITS + 1]; 2]> {
        let range = self.range;
        let failure = || {
            located_problem(
                range,
                owner_at,
                Some(owner),
                ErrorKind::Malformed,
                "unproved duplicate ExtGState target",
            )
            .ambiguous_repair()
        };
        let Some(XrefSlot {
            generation: 0,
            kind: XrefKind::InUse(offset),
        }) = slots.get(reference.number as usize).and_then(|slot| *slot)
        else {
            return Err(failure());
        };
        if reference.generation != 0 {
            return Err(failure());
        }
        let count =
            (self.range.length.saturating_sub(offset)).min(MAX_OPACITY_OBJECT as u64) as usize;
        let bytes = self.bytes(offset, count)?;
        let head = parse_object_head(bytes).map_err(|_| failure())?;
        if head.reference != reference || !matches!(head.tail, ObjectTail::EndObject { .. }) {
            return Err(failure());
        }
        let dictionary = head.dictionary.ok_or_else(failure)?;
        if dictionary.entries.len() != 2 {
            return Err(failure());
        }
        Ok([
            dictionary
                .value(b"CA")
                .and_then(opacity)
                .ok_or_else(failure)?,
            dictionary
                .value(b"ca")
                .and_then(opacity)
                .ok_or_else(failure)?,
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::opacity;

    #[test]
    fn opacity_equality_keeps_every_decimal_digit() {
        for (left, right) in [
            ("0.08", "0.08000"),
            (".08", "+00.080"),
            ("0", "0.0"),
            ("1", "01.000"),
        ] {
            assert_eq!(opacity(left.as_bytes()), opacity(right.as_bytes()));
            assert!(opacity(left.as_bytes()).is_some());
        }
        assert_ne!(opacity(b"0.08"), opacity(b"0.080000000000000001"));
        assert_ne!(
            opacity(b"0.08"),
            opacity(b"0.0800000000000000000000000000000000000000000000000000000000000001")
        );
        for value in [
            "", ".", "-0.08", "2", "1.01", "0.08e0", "0.0.8", "null", "+",
        ] {
            assert_eq!(opacity(value.as_bytes()), None, "{value}");
        }
        assert!(opacity(format!("0.{}", "0".repeat(65)).as_bytes()).is_none());
    }
}
