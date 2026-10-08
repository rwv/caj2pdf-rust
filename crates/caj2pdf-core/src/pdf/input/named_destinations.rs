// SPDX-License-Identifier: MIT

//! Measured byte-string /Names/Dests trees, ISO 32000-1 §§7.9.6, 12.3.2.3.
//! Validate once in lexical order, then resolve outline keys by binary search.
//! No document content or replacement destination is retained or emitted.

use super::*;
use parser::{MAX_SYNTAX_DEPTH, array_values, xyz_destination_page};

pub(super) struct NamedDestination {
    pub name: Vec<u8>,
    pub page: PdfRef,
}

enum Step {
    Visit(PdfRef, u32),
    Finish {
        owner: PdfRef,
        at: u64,
        first: usize,
        low: Vec<u8>,
        high: Vec<u8>,
    },
}

/// Decode a PDF byte string, not Unicode text. Capacity is checked before
/// allocation; a decoded string cannot exceed its original lexical length.
pub(super) fn name_bytes(raw: &[u8], cap: u64) -> Result<Option<Vec<u8>>> {
    if !valid_text_string(raw) {
        return Ok(None);
    }
    let mut syntax = Syntax::new(raw);
    syntax.skip_space();
    let start = syntax.pos;
    // Validation above established a complete string.
    syntax.skip_value(0).expect("validated PDF string");
    let raw = &raw[start..syntax.pos];
    let attempted = raw.len() as u64;
    let refused = Error::limit("PDF destination name bytes", cap, attempted);
    if attempted > cap {
        return Err(refused);
    }
    let mut bytes = Vec::new();
    reserve_exact(&mut bytes, raw.len(), refused)?;
    let mut i = 1;
    if raw[0] == b'<' {
        let mut high = None;
        for &byte in &raw[1..raw.len() - 1] {
            let digit = match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                b'A'..=b'F' => byte - b'A' + 10,
                _ => continue, // Only whitespace remains after validation.
            };
            if let Some(first) = high.take() {
                bytes.push(first * 16 + digit);
            } else {
                high = Some(digit);
            }
        }
        if let Some(first) = high {
            bytes.push(first * 16);
        }
    } else {
        while i < raw.len() - 1 {
            let byte = raw[i];
            i += 1;
            match byte {
                b'\\' => {
                    let escaped = raw[i];
                    i += 1;
                    match escaped {
                        b'n' => bytes.push(b'\n'),
                        b'r' => bytes.push(b'\r'),
                        b't' => bytes.push(b'\t'),
                        b'b' => bytes.push(8),
                        b'f' => bytes.push(12),
                        b'\n' => {}
                        b'\r' => i += usize::from(raw.get(i) == Some(&b'\n')),
                        b'0'..=b'7' => {
                            let mut value = escaped - b'0';
                            for _ in 0..2 {
                                match raw.get(i) {
                                    Some(next @ b'0'..=b'7') => {
                                        value = value.wrapping_mul(8).wrapping_add(next - b'0');
                                        i += 1;
                                    }
                                    _ => break,
                                }
                            }
                            bytes.push(value);
                        }
                        _ => bytes.push(escaped),
                    }
                }
                b'\r' => {
                    bytes.push(b'\n');
                    i += usize::from(raw.get(i) == Some(&b'\n'));
                }
                _ => bytes.push(byte),
            }
        }
    }
    Ok(Some(bytes))
}

impl<S: RangedSource, C: Cancellation> Reader<'_, S, C> {
    fn destination_object(
        &mut self,
        reference: PdfRef,
        slots: &[Option<XrefSlot>],
        index: &PdfIndex,
        work: &mut u64,
    ) -> Result<(ObjectHead, u64)> {
        let location = index.metadata_location(reference)?;
        let (head, _) = self.load_indexed_object(location.offset, reference, slots, index)?;
        *work = work.saturating_add(head.bytes.len() as u64);
        if *work > self.limits.max_allocation_bytes {
            return Err(Error::limit(
                "PDF destination metadata work",
                self.limits.max_allocation_bytes,
                *work,
            )
            .at(self.absolute(location.offset))
            .in_pdf(Some((reference.number, reference.generation))));
        }
        if !matches!(head.tail, ObjectTail::EndObject { .. }) {
            return Err(self.malformed(
                location.offset,
                Some(reference),
                "destination metadata must not be a stream",
            ));
        }
        Ok((head, location.offset))
    }

    pub(super) fn read_named_destinations(
        &mut self,
        slots: &[Option<XrefSlot>],
        index: &PdfIndex,
        page_targets: &[Option<u16>],
    ) -> Result<Vec<NamedDestination>> {
        let catalog_at = index.metadata_location(index.catalog)?.offset;
        let reference = index
            .catalog_dict
            .value(b"Names")
            .and_then(exact_reference)
            .ok_or_else(|| {
                self.problem(
                    catalog_at,
                    Some(index.catalog),
                    ErrorKind::UnsupportedFormat,
                    "named destinations require an indirect Catalog Names dictionary",
                )
            })?;
        let mut work = 0;
        let (head, at) = self.destination_object(reference, slots, index, &mut work)?;
        let root = head
            .dictionary
            .as_ref()
            .and_then(|d| d.value(b"Dests"))
            .and_then(exact_reference)
            .ok_or_else(|| {
                self.problem(
                    at,
                    Some(reference),
                    ErrorKind::UnsupportedFormat,
                    "named destinations require an indirect Dests name tree",
                )
            })?;
        drop(head);
        // Every slot was already admitted by PdfIndex's larger slot budget.
        let mut visited = Vec::new();
        reserve_exact(
            &mut visited,
            slots.len(),
            self.allocation_limit(
                at,
                Some(root),
                "PDF destination visited index",
                slots.len() as u64,
            ),
        )?;
        visited.resize(slots.len(), false);
        let cap = self.limits.max_allocation_bytes / 8;
        let mut names: Vec<NamedDestination> = Vec::new();
        let mut name_bytes_used = 0_u64;
        let mut stack = Vec::new();
        push_bounded(
            &mut stack,
            Step::Visit(root, 0),
            cap,
            "PDF destination tree stack",
        )
        .map_err(self.locator(at, Some(root)))?;
        while let Some(step) = stack.pop() {
            if self.cancellation.is_cancelled() {
                return Err(ErrorKind::Cancelled.into());
            }
            let (owner, depth) = match step {
                Step::Visit(owner, depth) => (owner, depth),
                Step::Finish {
                    owner,
                    at,
                    first,
                    low,
                    high,
                } => {
                    if names.len() == first
                        || names[first].name != low
                        || names.last().is_none_or(|entry| entry.name != high)
                    {
                        return Err(self.malformed(
                            at,
                            Some(owner),
                            "destination name-tree Limits disagree with descendants",
                        ));
                    }
                    continue;
                }
            };
            let at = index.metadata_location(owner)?.offset;
            if depth > MAX_SYNTAX_DEPTH {
                return Err(Error::limit(
                    "PDF destination tree depth",
                    u64::from(MAX_SYNTAX_DEPTH),
                    u64::from(depth),
                )
                .at(self.absolute(at))
                .in_pdf(Some((owner.number, owner.generation))));
            }
            if std::mem::replace(&mut visited[owner.number as usize], true) {
                return Err(self.malformed(
                    at,
                    Some(owner),
                    "destination name tree contains a cycle or shared node",
                ));
            }
            let (head, _) = self.destination_object(owner, slots, index, &mut work)?;
            let dictionary = head.dictionary.ok_or_else(|| {
                self.malformed(
                    at,
                    Some(owner),
                    "destination name-tree node is not a dictionary",
                )
            })?;
            let range = self.range;
            let absolute = self.absolute(at);
            let bad = || {
                located_problem(
                    range,
                    at,
                    Some(owner),
                    ErrorKind::Malformed,
                    "invalid destination name-tree node",
                )
            };
            let (kids, values) = (dictionary.value(b"Kids"), dictionary.value(b"Names"));
            if kids.is_some() == values.is_some()
                || dictionary.entries.len() != if depth == 0 { 1 } else { 2 }
            {
                return Err(bad());
            }
            let decode = |raw: &[u8], used: &mut u64| -> Result<Vec<u8>> {
                let value = name_bytes(raw, cap.saturating_sub(*used))
                    .map_err(|error| {
                        error
                            .at(absolute)
                            .in_pdf(Some((owner.number, owner.generation)))
                    })?
                    .ok_or_else(bad)?;
                // Charge reserved lexical capacity, including temporary Limits strings.
                *used = used.saturating_add(value.capacity() as u64);
                if *used > cap {
                    return Err(Error::limit("PDF destination name bytes", cap, *used)
                        .at(absolute)
                        .in_pdf(Some((owner.number, owner.generation))));
                }
                Ok(value)
            };
            if depth != 0 {
                let mut bounds = array_values(dictionary.value(b"Limits").ok_or_else(bad)?)
                    .map_err(|_| bad())?;
                let low = decode(
                    bounds
                        .next()
                        .transpose()
                        .map_err(|_| bad())?
                        .ok_or_else(bad)?,
                    &mut name_bytes_used,
                )?;
                let high = decode(
                    bounds
                        .next()
                        .transpose()
                        .map_err(|_| bad())?
                        .ok_or_else(bad)?,
                    &mut name_bytes_used,
                )?;
                if low > high || bounds.next().is_some() {
                    return Err(bad());
                }
                push_bounded(
                    &mut stack,
                    Step::Finish {
                        owner,
                        at,
                        first: names.len(),
                        low,
                        high,
                    },
                    cap,
                    "PDF destination tree stack",
                )
                .map_err(self.locator(at, Some(owner)))?;
            }
            if let Some(kids) = kids {
                let children = reference_array(kids, slots.len())
                    .filter(|v| !v.is_empty())
                    .ok_or_else(bad)?;
                for child in children.into_iter().rev() {
                    push_bounded(
                        &mut stack,
                        Step::Visit(child, depth + 1),
                        cap,
                        "PDF destination tree stack",
                    )
                    .map_err(self.locator(at, Some(owner)))?;
                }
            } else {
                let mut entries =
                    array_values(values.expect("exactly one Names or Kids")).map_err(|_| bad())?;
                let first = names.len();
                while let Some(raw) = entries.next() {
                    if self.cancellation.is_cancelled() {
                        return Err(ErrorKind::Cancelled.into());
                    }
                    let name = decode(raw.map_err(|_| bad())?, &mut name_bytes_used)?;
                    if names.last().is_some_and(|previous| previous.name >= name) {
                        return Err(self.malformed(
                            at,
                            Some(owner),
                            "destination names are duplicated or out of order",
                        ));
                    }
                    let value = entries
                        .next()
                        .transpose()
                        .map_err(|_| bad())?
                        .ok_or_else(bad)?;
                    let target = exact_reference(value).ok_or_else(|| {
                        self.problem(
                            at,
                            Some(owner),
                            ErrorKind::UnsupportedFormat,
                            "named destination must be an indirect XYZ array",
                        )
                    })?;
                    let (destination, target_at) =
                        self.destination_object(target, slots, index, &mut work)?;
                    let page = destination
                        .scalar
                        .as_ref()
                        .and_then(|span| xyz_destination_page(&destination.bytes[span.clone()]))
                        .ok_or_else(|| {
                            self.problem(
                                target_at,
                                Some(target),
                                ErrorKind::UnsupportedFormat,
                                "named destination must be an indirect XYZ array",
                            )
                        })?;
                    if page_targets.get(page.number as usize).copied().flatten()
                        != Some(page.generation)
                    {
                        return Err(self.malformed(
                            target_at,
                            Some(target),
                            "named destination does not target a page",
                        ));
                    }
                    push_bounded(
                        &mut names,
                        NamedDestination { name, page },
                        cap,
                        "PDF named destination index",
                    )
                    .map_err(self.locator(at, Some(owner)))?;
                }
                if names.len() == first {
                    return Err(self.malformed(
                        at,
                        Some(owner),
                        "destination name-tree leaf is empty",
                    ));
                }
            }
        }
        Ok(names)
    }
}
