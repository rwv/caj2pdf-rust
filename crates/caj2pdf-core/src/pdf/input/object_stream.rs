// SPDX-License-Identifier: MIT

//! Bounded metadata from the measured, direct-length Flate ObjStm profile.

use super::*;

pub(super) struct CompressedObject {
    reference: PdfRef,
    container: ObjectLocation,
    // One complete synthetic object frame, never a source-file span. Content
    // streams cannot be members; original compressed bytes remain in the PDF.
    bytes: Vec<u8>,
}

impl PdfIndex {
    pub(super) fn compressed_object(&self, reference: PdfRef) -> Option<&CompressedObject> {
        self.compressed_objects
            .binary_search_by_key(&reference.number, |object| object.reference.number)
            .ok()
            .map(|position| &self.compressed_objects[position])
            .filter(|object| object.reference == reference)
    }

    // Diagnostics for decoded metadata point to the owning physical stream.
    // Public object_location never pretends this is the member's source span.
    pub(super) fn metadata_location(&self, reference: PdfRef) -> Result<ObjectLocation> {
        match self.compressed_object(reference) {
            Some(object) => Ok(object.container),
            None => self.object_location(reference),
        }
    }
}

impl<S: RangedSource, C: Cancellation> Reader<'_, S, C> {
    pub(super) fn load_indexed_object(
        &mut self,
        at: u64,
        reference: PdfRef,
        slots: &[Option<XrefSlot>],
        index: &PdfIndex,
    ) -> Result<(ObjectHead, ObjectLocation)> {
        if let Some(object) = index.compressed_object(reference) {
            if self.cancellation.is_cancelled() {
                return Err(ErrorKind::Cancelled.into());
            }
            let mut bytes = Vec::new();
            reserve_exact(
                &mut bytes,
                object.bytes.len(),
                self.allocation_limit(
                    at,
                    Some(reference),
                    "PDF compressed metadata copy",
                    object.bytes.len() as u64,
                ),
            )?;
            bytes.extend_from_slice(&object.bytes);
            let head = parse_object_head(bytes).map_err(|issue| {
                self.parse_issue(at, Some(reference), issue)
                    .at(self.absolute(at))
            })?;
            Ok((head, object.container))
        } else {
            self.load_object(at, reference, slots)
        }
    }

    pub(super) fn read_compressed_objects(
        &mut self,
        slots: &[Option<XrefSlot>],
        index: &mut PdfIndex,
    ) -> Result<()> {
        let cap = self.limits.max_allocation_bytes / 8;
        let mut streams = Vec::new();
        for (position, slot) in slots.iter().enumerate() {
            if position % 1024 == 0 && self.cancellation.is_cancelled() {
                return Err(ErrorKind::Cancelled.into());
            }
            if let Some(XrefSlot {
                kind: XrefKind::Compressed { stream, .. },
                ..
            }) = slot
            {
                push_bounded(&mut streams, *stream, cap, "PDF object stream index")
                    .map_err(self.locator(index.xref_offset, None))?;
            }
        }
        streams.sort_unstable();
        streams.dedup();
        let mut retained = 0_u64;
        for number in streams {
            let reference = PdfRef {
                number,
                generation: 0,
            };
            let at = match slots.get(number as usize).and_then(|slot| *slot) {
                Some(XrefSlot {
                    generation: 0,
                    kind: XrefKind::InUse(at),
                }) => at,
                _ => {
                    return Err(self.malformed(
                        index.xref_offset,
                        Some(reference),
                        "object stream must be a standalone generation-zero object",
                    ));
                }
            };
            let (head, location) = self.load_object(at, reference, slots)?;
            check_live_object_end(self.range, at, reference, location, index.logical_end)?;
            let unsupported = || {
                self.problem(
                    at,
                    Some(reference),
                    ErrorKind::UnsupportedFormat,
                    "object stream is outside the direct-length Flate profile",
                )
            };
            let dictionary = head.dictionary.as_ref().ok_or_else(unsupported)?;
            self.reject_duplicate_names(dictionary, at, Some(reference))?;
            let ObjectTail::Stream { data_start } = head.tail else {
                return Err(unsupported());
            };
            if dictionary.value(b"Type").and_then(exact_name).as_deref() != Some(b"ObjStm")
                || dictionary.value(b"Filter").and_then(exact_name).as_deref()
                    != Some(b"FlateDecode")
                || [b"DecodeParms".as_slice(), b"F", b"Extends"]
                    .iter()
                    .any(|key| dictionary.value(key).is_some())
            {
                return Err(unsupported());
            }
            let length = dictionary
                .value(b"Length")
                .and_then(exact_unsigned)
                .ok_or_else(unsupported)?;
            let n = dictionary
                .value(b"N")
                .and_then(exact_unsigned)
                .ok_or(self.malformed(at, Some(reference), "object stream has invalid N"))?;
            let first = dictionary
                .value(b"First")
                .and_then(exact_unsigned)
                .ok_or(self.malformed(at, Some(reference), "object stream has invalid First"))?;
            let stream_cap = self.syntax_limit();
            if n == 0 {
                return Err(self.malformed(
                    at,
                    Some(reference),
                    "live object stream has no members",
                ));
            }
            for (resource, limit, attempted) in [
                ("PDF object stream members", u64::from(MAX_PDF_OBJECTS), n),
                ("PDF object stream header bytes", stream_cap, first),
                ("PDF object stream encoded bytes", stream_cap, length),
            ] {
                if attempted > limit {
                    return Err(self.locate_limit(
                        at,
                        Some(reference),
                        Error::limit(resource, limit, attempted),
                    ));
                }
            }
            let data_at = at + data_start as u64;
            let encoded = self.bytes(data_at, length as usize)?;
            let decoded = inflate_pdf_stream(
                &encoded,
                stream_cap as usize,
                false,
                self.limits.io_chunk_bytes,
                self.cancellation,
            )
            .map_err(self.locator(at, Some(reference)))?;
            let header = decoded.get(..first as usize).ok_or(self.malformed(
                at,
                Some(reference),
                "object stream First exceeds decoded data",
            ))?;
            let mut syntax = Syntax::new(header);
            let mut entries = Vec::new();
            for position in 0..n {
                if position % 1024 == 0 && self.cancellation.is_cancelled() {
                    return Err(ErrorKind::Cancelled.into());
                }
                let member = syntax.unsigned().map_err(|issue| {
                    self.parse_issue(at, Some(reference), issue)
                        .at(self.absolute(at))
                })?;
                let offset = syntax.unsigned().map_err(|issue| {
                    self.parse_issue(at, Some(reference), issue)
                        .at(self.absolute(at))
                })?;
                if member == 0
                    || member > u64::from(MAX_PDF_OBJECTS)
                    || offset >= (decoded.len() - header.len()) as u64
                    || (position == 0 && offset != 0)
                    || entries
                        .last()
                        .is_some_and(|(_, previous)| offset <= *previous)
                {
                    return Err(self.malformed(
                        at,
                        Some(reference),
                        "object stream member index is invalid",
                    ));
                }
                push_bounded(
                    &mut entries,
                    (member as u32, offset),
                    cap,
                    "PDF object stream members",
                )
                .map_err(self.locator(at, Some(reference)))?;
            }
            if !syntax.at_end() {
                return Err(self.malformed(
                    at,
                    Some(reference),
                    "object stream header has trailing data",
                ));
            }
            let mut numbers = Vec::new();
            for &(member, _) in &entries {
                push_bounded(
                    &mut numbers,
                    member,
                    cap,
                    "PDF object stream member numbers",
                )
                .map_err(self.locator(at, Some(reference)))?;
            }
            numbers.sort_unstable();
            if numbers.windows(2).any(|pair| pair[0] == pair[1]) {
                return Err(self.malformed(
                    at,
                    Some(reference),
                    "object stream repeats a member number",
                ));
            }
            for (position, &(member, offset)) in entries.iter().enumerate() {
                if self.cancellation.is_cancelled() {
                    return Err(ErrorKind::Cancelled.into());
                }
                let end = entries
                    .get(position + 1)
                    .map_or(decoded.len(), |(_, offset)| {
                        first as usize + *offset as usize
                    });
                let body = &decoded[first as usize + offset as usize..end];
                let mut syntax = Syntax::new(body);
                syntax.skip_value(0).map_err(|issue| {
                    self.parse_issue(at, Some(reference), issue)
                        .at(self.absolute(at))
                })?;
                if !syntax.at_end() {
                    return Err(self.malformed(
                        at,
                        Some(reference),
                        "object stream member is not one direct value",
                    ));
                }
                // Old object-stream members can be superseded by later xref
                // revisions. Only the final live slot controls membership.
                let Some(XrefSlot {
                    kind:
                        XrefKind::Compressed {
                            stream,
                            index: ordinal,
                        },
                    ..
                }) = slots.get(member as usize).and_then(|slot| *slot)
                else {
                    continue;
                };
                if stream != number {
                    continue;
                }
                if ordinal as usize != position {
                    return Err(self.malformed(
                        at,
                        Some(reference),
                        "xref compressed member index disagrees with object stream",
                    ));
                }
                let prefix = format!("{member} 0 obj\n");
                let needed = prefix.len().saturating_add(body.len()).saturating_add(8);
                if needed as u64 > stream_cap {
                    return Err(self.locate_limit(
                        at,
                        Some(reference),
                        Error::limit("PDF object syntax bytes", stream_cap, needed as u64),
                    ));
                }
                retained = retained.saturating_add(needed as u64);
                if retained > cap {
                    return Err(self.locate_limit(
                        at,
                        Some(reference),
                        Error::limit("PDF compressed metadata bytes", cap, retained),
                    ));
                }
                let mut bytes = Vec::new();
                let refused = self.allocation_limit(
                    at,
                    Some(reference),
                    "PDF compressed metadata allocation",
                    needed as u64,
                );
                reserve_exact(&mut bytes, needed, refused)?;
                bytes.extend_from_slice(prefix.as_bytes());
                bytes.extend_from_slice(body);
                bytes.extend_from_slice(b"\nendobj\n");
                push_bounded(
                    &mut index.compressed_objects,
                    CompressedObject {
                        reference: PdfRef {
                            number: member,
                            generation: 0,
                        },
                        container: location,
                        bytes,
                    },
                    cap,
                    "PDF compressed metadata index",
                )
                .map_err(self.locator(at, Some(reference)))?;
            }
        }
        index
            .compressed_objects
            .sort_unstable_by_key(|object| object.reference.number);
        for (number, slot) in slots.iter().enumerate() {
            if number % 1024 == 0 && self.cancellation.is_cancelled() {
                return Err(ErrorKind::Cancelled.into());
            }
            if let Some(XrefSlot {
                kind: XrefKind::Compressed { .. },
                ..
            }) = slot
                && index
                    .compressed_object(PdfRef {
                        number: number as u32,
                        generation: 0,
                    })
                    .is_none()
            {
                return Err(self.malformed(
                    index.xref_offset,
                    Some(PdfRef {
                        number: number as u32,
                        generation: 0,
                    }),
                    "xref compressed object has no matching stream member",
                ));
            }
        }
        Ok(())
    }
}
