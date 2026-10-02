// SPDX-License-Identifier: MIT

//! Bounded object scanning for headerless CAJ PDF fragments.

use super::{ObjectTail, Reader, exact_name, exact_reference, exact_unsigned};
use crate::fallible::{checked_read_count, reserve};
use crate::pdf::writer::MAX_PDF_OBJECTS;
use crate::pdf::{FragmentObject, PdfRange, PdfRef};
use crate::{Cancellation, Error, Limits, PdfErrorKind, RangedSource, Result};
use flate2::{Decompress, FlushDecompress, Status};

mod ascii85;
mod ccitt;
mod jpeg;

/// An equal-width correction to an observed, understated direct `/Length`.
/// The PDF bytes remain at their original offsets when the patch is applied.
#[derive(Clone, Debug)]
pub(crate) struct LengthPatch {
    pub(super) offset: u64,
    pub(super) original: Vec<u8>,
    pub(super) replacement: Vec<u8>,
}

/// Validated object boundaries within a headerless PDF fragment. The CAJ
/// page table supplies the minimum body end and page order, never object spans.
pub(crate) struct FragmentScan {
    pub objects: Vec<FragmentObject>,
    pub patches: Vec<LengthPatch>,
    lengths: Vec<(PdfRef, u64)>,
}

impl FragmentScan {
    pub fn resolve_length(&self, reference: PdfRef) -> Option<u64> {
        self.lengths
            .binary_search_by_key(&reference, |entry| entry.0)
            .ok()
            .map(|index| self.lengths[index].1)
    }
}

const OVERREAD: &str = "source reported more bytes than requested";

/// Apply verified same-width stream length repairs while forwarding ranged
/// reads. This adapter never buffers an object or stream payload.
pub(crate) struct PatchedSource<'a, S> {
    source: &'a mut S,
    patches: &'a [LengthPatch],
}

impl<'a, S> PatchedSource<'a, S> {
    pub fn new(source: &'a mut S, patches: &'a [LengthPatch]) -> Self {
        Self { source, patches }
    }
}

impl<S: RangedSource> RangedSource for PatchedSource<'_, S> {
    fn size(&self) -> u64 {
        self.source.size()
    }

    async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
        let count = self.source.read_at(offset, destination).await?;
        let count = checked_read_count(count, destination.len(), OVERREAD)?;
        let end = offset.saturating_add(count as u64);
        // The fragment scan records patches in ascending source order. Most
        // reads overlap no patch, so seek directly to the first possible one.
        let first_patch = self.patches.partition_point(|patch| {
            patch.offset.saturating_add(patch.replacement.len() as u64) <= offset
        });
        for patch in &self.patches[first_patch..] {
            if patch.offset >= end {
                break;
            }
            let patch_end = patch.offset.saturating_add(patch.replacement.len() as u64);
            let first = offset.max(patch.offset);
            let last = end.min(patch_end);
            if first < last {
                let source_start = (first - patch.offset) as usize;
                let target_start = (first - offset) as usize;
                let length = (last - first) as usize;
                if destination[target_start..target_start + length]
                    != patch.original[source_start..source_start + length]
                {
                    return Err(Error::Pdf {
                        offset: first,
                        object: None,
                        kind: PdfErrorKind::Malformed,
                        reason: "source changed after stream Length validation",
                    });
                }
                destination[target_start..target_start + length]
                    .copy_from_slice(&patch.replacement[source_start..source_start + length]);
            }
        }
        Ok(count)
    }
}

const MAX_FRAGMENT_TAIL_EXTENSION: u64 = 64 * 1024;
const MAX_STREAM_LENGTH_REPAIR: u64 = 64;

/// Independently framed object; `used` requires confirmation by the full scan.
#[derive(Clone, Copy)]
pub(crate) struct FragmentCandidate {
    pub object: FragmentObject,
    pub used: bool,
}

#[cfg(test)]
pub(crate) async fn scan_fragment_objects<S: RangedSource, C: Cancellation>(
    source: &mut S,
    body_start: u64,
    minimum_end: u64,
    limits: &Limits,
    cancellation: &C,
) -> Result<FragmentScan> {
    scan_fragment_with_candidates(
        source,
        body_start,
        minimum_end,
        limits,
        cancellation,
        &mut [],
        &mut 0,
    )
    .await
}

/// Collect locally framed candidates from an anchored row. Deferred prefixes
/// and indirect Length references are proved by the final whole-fragment scan,
/// never by this index. Framed stream extents still require codec validation.
pub(crate) async fn collect_fragment_candidates<S: RangedSource, C: Cancellation>(
    source: &mut S,
    start: u64,
    end: u64,
    limits: &Limits,
    cancellation: &C,
    inflated_bytes: &mut u64,
) -> Result<Vec<FragmentObject>> {
    let scan = scan_fragment(
        source,
        start,
        end,
        limits,
        cancellation,
        ScanMode::Candidates,
        inflated_bytes,
    )
    .await?;
    Ok(if scan.patches.is_empty() {
        scan.objects
    } else {
        Vec::new()
    })
}

enum ScanMode<'a> {
    Complete(&'a mut [FragmentCandidate]),
    Candidates,
}

/// Scan indirect objects with the existing PDF syntax parser, advancing over
/// stream payloads by `/Length` rather than searching them for object markers.
/// Supported indirect lengths are measured from zlib, JPEG or Group-4 framing
/// and verified against the referenced scalar after indexing. Other filters
/// may use an already parsed scalar, with normal tail and duplicate checks.
/// A narrowly bounded repair accepts a unique nearby `endstream`/`endobj`
/// delimiter when a direct length is understated. An ambiguous marker is an
/// error. Bytes after the complete final object are excluded from the plan.
pub(crate) async fn scan_fragment_with_candidates<S: RangedSource, C: Cancellation>(
    source: &mut S,
    body_start: u64,
    minimum_end: u64,
    limits: &Limits,
    cancellation: &C,
    candidates: &mut [FragmentCandidate],
    inflated_bytes: &mut u64,
) -> Result<FragmentScan> {
    scan_fragment(
        source,
        body_start,
        minimum_end,
        limits,
        cancellation,
        ScanMode::Complete(candidates),
        inflated_bytes,
    )
    .await
}

async fn scan_fragment<S: RangedSource, C: Cancellation>(
    source: &mut S,
    body_start: u64,
    minimum_end: u64,
    limits: &Limits,
    cancellation: &C,
    mode: ScanMode<'_>,
    inflated_bytes: &mut u64,
) -> Result<FragmentScan> {
    let (candidates, verify_document) = match mode {
        ScanMode::Complete(candidates) => (candidates, true),
        ScanMode::Candidates => (&mut [][..], false),
    };
    limits.validate()?;
    if body_start >= minimum_end || minimum_end > source.size() {
        return Err(Error::Caj {
            offset: body_start,
            record: None,
            reason: "CAJ PDF fragment body range is invalid",
        });
    }
    let minimum_length = minimum_end - body_start;
    limits
        .check_input_size(minimum_length)
        .map_err(|error| error.locate_caj_limit(body_start, None))?;
    let scan_end = minimum_end
        .saturating_add(MAX_FRAGMENT_TAIL_EXTENSION)
        .min(source.size());
    let range = PdfRange {
        offset: body_start,
        length: scan_end - body_start,
    };
    let mut reader = Reader::new(source, range, limits, cancellation)?;
    let mut objects = Vec::new();
    let mut patches = Vec::new();
    let mut lengths = Vec::new();
    let mut pending_lengths = Vec::new();
    let mut pending_prefixes = Vec::new();
    let mut cursor = 0_u64;
    let mut final_object_repaired = false;
    let minimum_relative = minimum_end - body_start;
    // `minimum_end <= source.size()` was checked above, so the bounded scan
    // range always reaches the page table's body end. A cursor that has not
    // reached that end is therefore still inside the range; `load_head`
    // rejects syntax that would run past it.
    debug_assert!(minimum_relative <= range.length);
    let logical_end = 'objects: loop {
        reader.skip_space(&mut cursor).await?;
        if cursor >= minimum_relative {
            break cursor;
        }
        let start = cursor;
        let head = match reader.load_head(start, None).await {
            Ok(head) => head,
            Err(
                error @ Error::Pdf {
                    kind: PdfErrorKind::Malformed,
                    ..
                },
            ) => {
                if let Some(end) = replay_end(&mut reader, start, &objects, &lengths).await? {
                    cursor = end;
                    continue;
                }
                if let Some(end) =
                    orphan_length_end(&mut reader, start, pending_lengths.last().copied()).await?
                {
                    cursor = end;
                    continue;
                }
                if let Some(end) = known_prefix_end(&mut reader, start, &error, &objects).await? {
                    cursor = end;
                    continue;
                }
                if let Some(end) = adjacent_header_end(&mut reader, start, &error, &objects).await?
                {
                    cursor = end;
                    continue;
                }
                if let Some(end) = candidate_prefix_end(&mut reader, start, candidates).await? {
                    cursor = end;
                    continue;
                }
                if let Some((end, prefix)) =
                    interrupted_syntax_prefix(&mut reader, start, &error).await?
                {
                    if candidates
                        .iter()
                        .filter(|candidate| candidate.object.reference == prefix.reference)
                        .nth(1)
                        .is_some()
                    {
                        return Err(error);
                    }
                    let count = next_object_count(pending_prefixes.len())?;
                    let allocation = (count as u64) * std::mem::size_of::<FragmentObject>() as u64;
                    limits.check_allocation(allocation)?;
                    let refused =
                        limits.allocation_refused("CAJ interrupted prefix index", allocation);
                    reserve(&mut pending_prefixes, 1, refused)?;
                    pending_prefixes.push(prefix);
                    cursor = end;
                    continue;
                }
                return Err(error);
            }
            Err(error) => return Err(error),
        };
        let mut object_repaired = false;
        let reference = head.reference;
        if reference.generation != 0 {
            return Err(reader.problem(
                start,
                Some(reference),
                PdfErrorKind::UnsupportedFeature,
                "CAJ PDF fragment has a nonzero object generation",
            ));
        }
        let end_overflow =
            reader.malformed(start, Some(reference), "fragment object end overflows");
        let end = match head.tail {
            ObjectTail::EndObject { end } => {
                if let Some(value) = head
                    .scalar
                    .as_ref()
                    .and_then(|range| exact_unsigned(&head.bytes[range.clone()]))
                {
                    retain_length(&mut lengths, (reference, value), limits)?;
                }
                start.checked_add(end as u64)
            }
            ObjectTail::Stream { data_start } => {
                let failure = reader.malformed(start, Some(reference), "stream has no dictionary");
                let dictionary = head.dictionary.as_ref().ok_or(failure)?;
                let failure = reader.malformed(start, Some(reference), "stream lacks Length");
                let entry = dictionary.entry(b"Length").ok_or(failure)?;
                let value = entry.value(&dictionary.bytes);
                let data_at = start
                    .checked_add(data_start as u64)
                    .ok_or(reader.malformed(start, Some(reference), "stream offset overflows"))?;
                let indirect = exact_reference(value);
                let length = if let Some(length) = exact_unsigned(value) {
                    length
                } else if let Some(target) = indirect {
                    let measured = match dictionary.value(b"Filter").and_then(exact_name).as_deref()
                    {
                        Some(b"FlateDecode") => {
                            flate_extent(&mut reader, data_at, reference, inflated_bytes).await
                        }
                        Some(b"DCTDecode") => jpeg::extent(&mut reader, data_at, reference).await,
                        Some(b"ASCII85Decode") => ascii85::extent(&mut reader, data_at, reference, inflated_bytes).await,
                        Some(b"CCITTFaxDecode") => {
                            ccitt::extent(
                                &mut reader,
                                data_at,
                                reference,
                                dictionary,
                                start + head.dictionary_start.expect("stream dictionary") as u64,
                                inflated_bytes,
                            )
                            .await
                        }
                        _ => lengths
                            .iter()
                            .find_map(|(reference, value)| (*reference == target).then_some(*value))
                            .ok_or_else(|| reader.problem(
                                start,
                                Some(reference),
                                PdfErrorKind::UnsupportedFeature,
                                "indirect CAJ stream Length requires a known scalar or supported framed filter",
                            )),
                    };
                    let length = match measured {
                        Ok(length) => length,
                        Err(
                            error @ Error::Pdf {
                                kind: PdfErrorKind::Malformed,
                                ..
                            },
                        ) => {
                            if let Some(end) =
                                replay_end(&mut reader, start, &objects, &lengths).await?
                            {
                                cursor = end;
                                continue 'objects;
                            }
                            if let Some(end) =
                                candidate_prefix_end(&mut reader, start, candidates).await?
                            {
                                cursor = end;
                                continue 'objects;
                            }
                            return Err(error);
                        }
                        Err(error) => return Err(error),
                    };
                    retain_length(&mut pending_lengths, (target, length), limits)?;
                    length
                } else {
                    return Err(reader.malformed(
                        start,
                        Some(reference),
                        "stream Length is neither an integer nor a reference",
                    ));
                };
                let failure = reader.malformed(data_at, Some(reference), "stream extent overflows");
                let after_data = data_at.checked_add(length).ok_or(failure)?;
                let tail = reader.check_stream_tail(after_data, Some(reference)).await;
                let end = if indirect.is_none() && is_malformed(&tail) {
                    let (corrected_length, corrected_end) =
                        match repair_stream_length(&mut reader, after_data, data_at, reference)
                            .await
                        {
                            Ok(repair) => repair,
                            Err(
                                error @ Error::Pdf {
                                    kind: PdfErrorKind::Malformed,
                                    ..
                                },
                            ) => {
                                if let Some(end) =
                                    candidate_prefix_end(&mut reader, start, candidates).await?
                                {
                                    cursor = end;
                                    continue 'objects;
                                }
                                return Err(error);
                            }
                            Err(error) => return Err(error),
                        };
                    let original = value.to_vec();
                    let replacement = corrected_length.to_string().into_bytes();
                    if original.len() != replacement.len() {
                        return Err(reader.problem(
                            after_data,
                            Some(reference),
                            PdfErrorKind::UnsupportedFeature,
                            "stream Length repair changes PDF object width",
                        ));
                    }
                    let failure = reader.malformed(
                        start,
                        Some(reference),
                        "stream dictionary offset is missing",
                    );
                    let dictionary_start = head.dictionary_start.ok_or(failure)?;
                    let patch_offset = body_start
                        .checked_add(start)
                        .and_then(|n| n.checked_add(dictionary_start as u64))
                        .and_then(|n| n.checked_add(entry.value.start as u64))
                        .ok_or(Error::InvalidInput {
                            reason: "stream Length offset overflows",
                        })?;
                    patches.push(LengthPatch {
                        offset: patch_offset,
                        original,
                        replacement,
                    });
                    object_repaired = true;
                    corrected_end
                } else {
                    tail?
                };
                Some(end)
            }
        }
        .ok_or(end_overflow)?;
        // `load_head` reads only within the bounded range, and a stream end
        // follows an `endstream` and `endobj` read by `bytes`, which also
        // stays within it.
        debug_assert!(start < end && end <= range.length);
        let count =
            next_object_count(objects.len()).map_err(reader.locator(start, Some(reference)))?;
        let allocation = (count as u64)
            .saturating_mul(std::mem::size_of::<FragmentObject>() as u64)
            .saturating_add(
                ((lengths.len() + pending_lengths.len()) * std::mem::size_of::<(PdfRef, u64)>())
                    as u64,
            )
            .saturating_add(
                (patches.len() as u64)
                    .saturating_mul(std::mem::size_of::<LengthPatch>() as u64 + 40),
            );
        limits
            .check_allocation(allocation)
            .map_err(reader.locator(start, Some(reference)))?;
        let refused = reader.allocation_limit(
            start,
            Some(reference),
            "PDF fragment object index allocation",
            allocation,
        );
        reserve(&mut objects, 1, refused)?;
        objects.push(FragmentObject {
            reference,
            range: PdfRange {
                offset: body_start + start,
                length: end - start,
            },
        });
        cursor = end;
        final_object_repaired = object_repaired;
        if end >= minimum_relative {
            break end;
        }
    };
    if objects.is_empty() {
        return Err(reader.malformed(0, None, "CAJ PDF fragment has no indirect objects"));
    }
    if final_object_repaired {
        // A repaired final stream can otherwise stop at a fake terminator in
        // its own data once that candidate passes the page-table end hint.
        // The only observed such repair ends immediately before an XML CAJ
        // trailer. Unknown trailing bytes remain an ambiguous repair.
        let mut tail = logical_end;
        reader.skip_space(&mut tail).await?;
        if body_start.saturating_add(tail) < reader.source.size()
            && (tail.saturating_add(5) > range.length
                || reader.bytes(tail, 5).await?.as_slice() != b"<?xml")
        {
            return Err(reader.problem(
                tail,
                None,
                PdfErrorKind::AmbiguousRepair,
                "repaired final stream has an unrecognized CAJ trailer",
            ));
        }
    }
    let actual_length = logical_end.saturating_sub(0);
    limits
        .check_input_size(actual_length)
        .map_err(reader.locator(logical_end, None))?;
    lengths.sort_unstable_by_key(|entry| entry.0);
    if lengths
        .windows(2)
        .any(|pair| pair[0].0 == pair[1].0 && pair[0].1 != pair[1].1)
    {
        return Err(reader.malformed(0, None, "conflicting fragment integer objects"));
    }
    lengths.dedup();
    // A candidate reached through a container anchor may actually be inside a
    // stream. Only the complete forward parse establishes its object boundary.
    for candidate in candidates.iter().filter(|candidate| candidate.used) {
        let object = candidate.object;
        let confirmed = objects
            .binary_search_by_key(&object.range.offset, |actual| actual.range.offset)
            .is_ok_and(|index| {
                objects[index].reference == object.reference && objects[index].range == object.range
            });
        if !confirmed {
            return Err(reader.problem(
                object.range.offset.saturating_sub(body_start),
                Some(object.reference),
                PdfErrorKind::AmbiguousRepair,
                "recovery candidate is not a complete fragment object",
            ));
        }
    }
    // Compact exact object replays in place, then restore source order. Compare
    // streams in bounded chunks; never buffer a complete replayed image.
    objects.sort_unstable_by_key(|object| (object.reference, object.range.offset));
    let mut kept = 0;
    for index in 0..objects.len() {
        let object = objects[index];
        if kept > 0 && objects[kept - 1].reference == object.reference {
            let prior = objects[kept - 1];
            let mut equal = prior.range.length == object.range.length;
            let mut compared = 0;
            while equal && compared < object.range.length {
                let amount = (object.range.length - compared)
                    .min(4096)
                    .min(limits.io_chunk_bytes as u64) as usize;
                let original = reader
                    .bytes(prior.range.offset - body_start + compared, amount)
                    .await?;
                let replay = reader
                    .bytes(object.range.offset - body_start + compared, amount)
                    .await?;
                equal = original == replay;
                compared += amount as u64;
            }
            if !equal {
                return Err(reader.problem(
                    object.range.offset - body_start,
                    Some(object.reference),
                    PdfErrorKind::AmbiguousRepair,
                    "duplicate indirect object differs from original",
                ));
            }
        } else {
            objects[kept] = object;
            kept += 1;
        }
    }
    objects.truncate(kept);
    if verify_document {
        for prefix in pending_prefixes {
            let original = objects
                .binary_search_by_key(&prefix.reference, |object| object.reference)
                .ok()
                .map(|index| objects[index]);
            let failure = reader.problem(
                prefix.range.offset - body_start,
                Some(prefix.reference),
                PdfErrorKind::Malformed,
                "interrupted prefix has no exact complete counterpart",
            );
            let Some(original) = original else {
                return Err(failure);
            };
            if original.range.offset <= prefix.range.offset
                || prefix.range.length >= original.range.length
            {
                return Err(failure);
            }
            let partial = reader
                .bytes(
                    prefix.range.offset - body_start,
                    prefix.range.length as usize,
                )
                .await?;
            let complete = reader
                .bytes(
                    original.range.offset - body_start,
                    prefix.range.length as usize,
                )
                .await?;
            if partial != complete {
                return Err(failure);
            }
        }
    }
    objects.sort_unstable_by_key(|object| object.range.offset);
    let scan = FragmentScan {
        objects,
        patches,
        lengths,
    };
    if verify_document {
        for (target, actual) in pending_lengths {
            if scan.resolve_length(target) != Some(actual) {
                return Err(reader.malformed(
                    0,
                    Some(target),
                    "indirect stream Length does not match its integer object",
                ));
            }
        }
    }
    Ok(scan)
}

/// Defer a short syntax interruption until the complete scan can prove its
/// exact counterpart. The parser supplies the sole boundary; no marker search.
async fn interrupted_syntax_prefix<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    start: u64,
    error: &Error,
) -> Result<Option<(u64, FragmentObject)>> {
    let Error::Pdf {
        offset,
        reason:
            reason @ ("expected PDF name"
            | "invalid PDF value token"
            | "PDF object lacks endobj or stream"
            | "unexpected PDF keyword"),
        ..
    } = error
    else {
        return Ok(None);
    };
    let header_error = *reason == "unexpected PDF keyword";
    let mut end = offset.saturating_sub(reader.range.offset);
    if end <= start || end - start > 256 || (header_error && end - start > 64) {
        return Ok(None);
    }
    // A cut inside `obj` leaves only `o` or `ob` at the parser error.
    // Consume that fixed keyword prefix, never an arbitrary token.
    if header_error && reader.byte(end).await? == Some(b'o') {
        end += 1;
        if reader.byte(end).await? == Some(b'b') {
            end += 1;
        }
        while end - start <= 64
            && reader
                .byte(end)
                .await?
                .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            end += 1;
        }
        if end - start > 64 {
            return Ok(None);
        }
    }
    // A dictionary cut between the two closing brackets reports its first
    // bracket as the invalid name. Retain that byte in the exact prefix proof.
    if reader.byte(end).await? == Some(b'>') {
        end += 1;
        while end - start <= 256
            && reader
                .byte(end)
                .await?
                .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            end += 1;
        }
        if end - start > 256 {
            return Ok(None);
        }
    }
    let bytes = reader.bytes(start, (end - start) as usize).await?;
    let Some((reference, _)) = replay_prefix(&bytes) else {
        return Ok(None);
    };
    let mut boundary = bytes.len();
    // An array/value parser can consume the first one or two numeric fields
    // of the next `number generation obj` header before rejecting `obj`.
    // Try only these adjacent lexical boundaries, never search later bytes.
    for _ in 0..3 {
        if boundary == 0 {
            break;
        }
        let prefix = bytes[..boundary].trim_ascii_end();
        match reader.load_head(start + boundary as u64, None).await {
            Ok(_) => {
                if header_error {
                    let mut fields = prefix
                        .split(u8::is_ascii_whitespace)
                        .filter(|field| !field.is_empty());
                    let number = fields.next().and_then(exact_unsigned);
                    let generation = fields.next();
                    let keyword = fields.next();
                    if number != Some(u64::from(reference.number))
                        || !matches!(generation, None | Some(b"0"))
                        || !matches!(keyword, None | Some(b"o" | b"ob"))
                        || (keyword.is_some() && generation != Some(b"0"))
                        || fields.next().is_some()
                    {
                        return Ok(None);
                    }
                }
                return Ok(Some((
                    start + boundary as u64,
                    FragmentObject {
                        reference,
                        range: PdfRange {
                            offset: reader.range.offset + start,
                            length: prefix.len() as u64,
                        },
                    },
                )));
            }
            Err(Error::Pdf {
                kind: PdfErrorKind::Malformed,
                ..
            }) => {}
            Err(error) => return Err(error),
        }
        boundary = prefix
            .iter()
            .rposition(u8::is_ascii_whitespace)
            .map_or(0, |index| index + 1);
    }
    Ok(None)
}

/// Compare an interrupted object with a uniquely indexed later copy. A
/// mismatch defines the only possible boundary; never search a payload for
/// markers. The caller subsequently proves the copy is reached by the full scan.
async fn candidate_prefix_end<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    start: u64,
    candidates: &mut [FragmentCandidate],
) -> Result<Option<u64>> {
    if candidates.is_empty() {
        return Ok(None);
    }
    let bytes = reader
        .bytes(start, 256.min(reader.range.length - start) as usize)
        .await?;
    let Some((reference, header_end)) = replay_prefix(&bytes) else {
        return Ok(None);
    };
    let mut matches = candidates
        .iter_mut()
        .filter(|candidate| candidate.object.reference == reference);
    let Some(candidate) = matches.next() else {
        return Ok(None);
    };
    if matches.next().is_some() {
        return Ok(None);
    }
    let original = candidate.object;
    let Some(relative) = original.range.offset.checked_sub(reader.range.offset) else {
        return Ok(None);
    };
    if relative <= start || original.range.length > reader.range.length.saturating_sub(relative) {
        return Ok(None);
    }
    let original_bytes = reader
        .bytes(
            relative,
            original.range.length.min(bytes.len() as u64) as usize,
        )
        .await?;
    let shared = bytes
        .iter()
        .zip(&original_bytes)
        .take_while(|(a, b)| a == b)
        .count();
    if shared <= header_end || shared as u64 >= original.range.length {
        return Ok(None);
    }
    let mut boundary = shared;
    while bytes.get(boundary).is_some_and(u8::is_ascii_whitespace) {
        boundary += 1;
    }
    if !bytes.get(boundary).is_some_and(u8::is_ascii_digit) {
        return Ok(None);
    }
    match reader.load_head(start + boundary as u64, None).await {
        Ok(_) => {
            candidate.used = true;
            Ok(Some(start + boundary as u64))
        }
        Err(Error::Pdf {
            kind: PdfErrorKind::Malformed,
            ..
        }) => Ok(None),
        Err(error) => Err(error),
    }
}

/// Admit only an unfinished `number 0` header immediately followed by its
/// complete same-reference object, or exactly repeating an already indexed
/// header. No object body is discarded or searched.
async fn adjacent_header_end<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    start: u64,
    error: &Error,
    objects: &[FragmentObject],
) -> Result<Option<u64>> {
    let Error::Pdf {
        offset,
        reason: "unexpected PDF keyword",
        ..
    } = error
    else {
        return Ok(None);
    };
    let end = offset
        .checked_sub(reader.range.offset)
        .expect("reader errors use absolute source offsets");
    if end <= start || end - start > 64 {
        return Ok(None);
    }
    let bytes = reader.bytes(start, (end - start) as usize).await?;
    let prefix = bytes.trim_ascii_end();
    let mut fields = prefix
        .split(u8::is_ascii_whitespace)
        .filter(|field| !field.is_empty());
    let number = fields.next().and_then(exact_unsigned);
    if fields.next() != Some(b"0".as_slice()) || fields.next().is_some() {
        return Ok(None);
    }
    let head = match reader.load_head(end, None).await {
        Ok(head) => head,
        Err(Error::Pdf {
            kind: PdfErrorKind::Malformed,
            ..
        }) => return Ok(None),
        Err(error) => return Err(error),
    };
    if number == Some(u64::from(head.reference.number))
        && head.reference.generation == 0
        && head.bytes.starts_with(prefix)
    {
        return Ok(Some(end));
    }
    let mut matches = objects.iter().filter(|object| {
        number == Some(u64::from(object.reference.number)) && object.reference.generation == 0
    });
    let Some(original) = matches.next() else {
        return Ok(None);
    };
    if matches.next().is_some() {
        return Ok(None);
    }
    let original_prefix = reader
        .bytes(original.range.offset - reader.range.offset, prefix.len())
        .await?;
    Ok((original_prefix == prefix).then_some(end))
}

/// Syntax errors can identify an interrupted duplicate dictionary or integer.
/// Only discard a bounded exact prefix of one already validated object,
/// including stream dictionaries before their payload. The exact shared prefix
/// and trailing whitespace define the boundary without a marker search. The
/// main loop must then parse a complete next object and validate all links.
async fn known_prefix_end<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    start: u64,
    error: &Error,
    objects: &[FragmentObject],
) -> Result<Option<u64>> {
    let Error::Pdf {
        offset,
        reason:
            "expected PDF name" | "invalid PDF value token" | "PDF object lacks endobj or stream",
        ..
    } = error
    else {
        return Ok(None);
    };
    let end = offset
        .checked_sub(reader.range.offset)
        .expect("reader errors use absolute source offsets");
    if end <= start || end - start > 256 {
        return Ok(None);
    }
    let amount = 256.min(reader.range.length - start) as usize;
    let bytes = reader.bytes(start, amount).await?;
    let Some((reference, _)) = replay_prefix(&bytes) else {
        return Ok(None);
    };
    let mut prior = objects
        .iter()
        .filter(|object| object.reference == reference);
    let Some(original) = prior.next() else {
        return Ok(None);
    };
    if prior.next().is_some() {
        return Ok(None);
    }
    let original_start = original.range.offset - reader.range.offset;
    let original_head = reader.load_head(original_start, Some(reference)).await?;
    let integer = original_head
        .scalar
        .as_ref()
        .and_then(|range| exact_unsigned(&original_head.bytes[range.clone()]));
    if original_head.dictionary.is_none() && integer.is_none() {
        return Ok(None);
    }
    let shared = bytes
        .iter()
        .zip(&original_head.bytes)
        .take_while(|(left, right)| left == right)
        .count();
    let mut boundary = shared;
    while bytes.get(boundary).is_some_and(u8::is_ascii_whitespace) {
        boundary += 1;
    }
    // A cut may occur between the two dictionary-closing '>' bytes. Follow
    // only the exact shared prefix and its trailing whitespace, never search
    // for a later marker. The suffix must parse as a new indirect object;
    // changing a value alone cannot make an arbitrary suffix into an object.
    if shared as u64 >= original.range.length
        || matches!(original_head.tail, ObjectTail::Stream { data_start } if shared >= data_start)
        || !bytes.get(boundary).is_some_and(u8::is_ascii_digit)
    {
        return Ok(None);
    }
    Ok(Some(start + boundary as u64))
}

/// A partial integer-object header may precede its complete copy. Require
/// the same reference and measured length as the most recently framed stream.
async fn orphan_length_end<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    start: u64,
    expected: Option<(PdfRef, u64)>,
) -> Result<Option<u64>> {
    let Some((reference, length)) = expected else {
        return Ok(None);
    };
    let amount = 256.min(reader.range.length - start) as usize;
    let bytes = reader.bytes(start, amount).await?;
    for split in 1..bytes.len() {
        if !bytes[split - 1].is_ascii_whitespace() || !bytes[split].is_ascii_digit() {
            continue;
        }
        let prefix = bytes[..split].trim_ascii_end();
        if prefix
            .split(u8::is_ascii_whitespace)
            .next()
            .and_then(exact_unsigned)
            != Some(u64::from(reference.number))
            || !bytes[split..].starts_with(prefix)
        {
            continue;
        }
        let Ok(head) = super::parse_object_head(bytes[split..].to_vec()) else {
            continue;
        };
        if head.reference == reference
            && matches!(head.tail, ObjectTail::EndObject { .. })
            && head
                .scalar
                .as_ref()
                .and_then(|range| exact_unsigned(&head.bytes[range.clone()]))
                == Some(length)
        {
            return Ok(Some(start + split as u64));
        }
    }
    Ok(None)
}

/// Recognize a partial known object followed by an exact copy of a
/// previously indexed integer. Only inspect the bounded malformed-object
/// boundary; never search inside a successfully framed stream payload.
async fn replay_end<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    start: u64,
    objects: &[FragmentObject],
    lengths: &[(PdfRef, u64)],
) -> Result<Option<u64>> {
    let amount = 256.min(reader.range.length - start) as usize;
    let bytes = reader.bytes(start, amount).await?;
    let Some((reference, header_end)) = replay_prefix(&bytes) else {
        return Ok(None);
    };
    let mut prior = objects
        .iter()
        .filter(|object| object.reference == reference);
    let Some(original) = prior.next() else {
        return Ok(None);
    };
    if prior.next().is_some() {
        return Ok(None);
    }
    let original_bytes = reader
        .bytes(
            original.range.offset - reader.range.offset,
            original.range.length.min(amount as u64) as usize,
        )
        .await?;
    let mut result = None;
    for split in header_end + 1..bytes.len() {
        if !bytes[split - 1].is_ascii_whitespace() || !bytes[split].is_ascii_digit() {
            continue;
        }
        let prefix = bytes[..split].trim_ascii_end();
        if prefix.len() as u64 >= original.range.length || !original_bytes.starts_with(prefix) {
            continue;
        }
        let Ok(head) = super::parse_object_head(bytes[split..].to_vec()) else {
            continue;
        };
        let (ObjectTail::EndObject { end }, Some(value)) = (
            head.tail,
            head.scalar
                .as_ref()
                .and_then(|range| exact_unsigned(&head.bytes[range.clone()])),
        ) else {
            continue;
        };
        if !lengths.contains(&(head.reference, value)) {
            continue;
        }
        let mut scalars = objects
            .iter()
            .filter(|object| object.reference == head.reference);
        let scalar = scalars.next().expect("indexed integer has an object span");
        if scalars.next().is_some() || scalar.range.length != end as u64 {
            continue;
        }
        let scalar_bytes = reader
            .bytes(scalar.range.offset - reader.range.offset, end)
            .await?;
        if bytes[split..split + end] != scalar_bytes {
            continue;
        }
        // The object parser already checked the endobj token boundary.
        if result.replace(start + (split + end) as u64).is_some() {
            return Ok(None);
        }
    }
    Ok(result)
}

fn replay_prefix(bytes: &[u8]) -> Option<(PdfRef, usize)> {
    let end = bytes.iter().position(u8::is_ascii_whitespace)?;
    let number = u32::try_from(exact_unsigned(&bytes[..end])?).ok()?;
    Some((
        PdfRef {
            number,
            generation: 0,
        },
        end,
    ))
}

/// Store only bounded scalar metadata; stream bytes never enter this index.
fn retain_length(
    entries: &mut Vec<(PdfRef, u64)>,
    entry: (PdfRef, u64),
    limits: &Limits,
) -> Result<()> {
    let count = next_object_count(entries.len())?;
    let bytes = (count * std::mem::size_of::<(PdfRef, u64)>()) as u64;
    limits.check_allocation(bytes)?;
    let refused = limits.allocation_refused("fragment Length index", bytes);
    reserve(entries, 1, refused)?;
    entries.push(entry);
    Ok(())
}

/// Zlib framing locates a stream independently of marker-like payload bytes.
/// The referenced integer is checked after all objects have been indexed.
async fn flate_extent<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    data_at: u64,
    reference: PdfRef,
    inflated_bytes: &mut u64,
) -> Result<u64> {
    // Include the inflater's fixed history/state in the allocation allowance.
    reader.limits.check_allocation(64 * 1024)?;
    let mut inflater = Decompress::new(true);
    let mut output = [0_u8; 4096];
    loop {
        if reader.cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let at = data_at + inflater.total_in();
        let amount = reader.range.length.saturating_sub(at).min(4096) as usize;
        let input = reader.bytes(at, amount).await?;
        let before_in = inflater.total_in();
        let before_out = inflater.total_out();
        let status = inflater.decompress(&input, &mut output, FlushDecompress::None);
        *inflated_bytes = inflated_bytes.saturating_add(inflater.total_out() - before_out);
        if *inflated_bytes > reader.limits.max_output_bytes {
            return Err(Error::LimitExceeded {
                resource: "CAJ Flate scan bytes",
                limit: reader.limits.max_output_bytes,
                attempted: *inflated_bytes,
            });
        }
        let status = status.map_err(|_| {
            reader.malformed(
                at,
                Some(reference),
                "invalid Flate stream while resolving Length",
            )
        })?;
        if status == Status::StreamEnd {
            return Ok(inflater.total_in());
        }
        if inflater.total_in() == before_in && inflater.total_out() == before_out {
            return Err(reader.malformed(
                at,
                Some(reference),
                "truncated Flate stream while resolving Length",
            ));
        }
    }
}

/// The object count after indexing one more fragment object.
fn next_object_count(indexed: usize) -> Result<usize> {
    let count = indexed.saturating_add(1);
    if count > MAX_PDF_OBJECTS as usize {
        return Err(Error::LimitExceeded {
            resource: "PDF fragment objects",
            limit: MAX_PDF_OBJECTS as u64,
            attempted: count as u64,
        });
    }
    Ok(count)
}

/// Whether a stream-tail check failed on malformed syntax, which a bounded
/// Length repair may correct, rather than on a source or limit error.
fn is_malformed(result: &Result<u64>) -> bool {
    matches!(
        result,
        Err(Error::Pdf {
            kind: PdfErrorKind::Malformed,
            ..
        })
    )
}

async fn repair_stream_length<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    declared_after: u64,
    data_at: u64,
    reference: PdfRef,
) -> Result<(u64, u64)> {
    let last = declared_after
        .saturating_add(MAX_STREAM_LENGTH_REPAIR)
        .min(reader.range.length.saturating_sub(9));
    let mut found = None;
    for marker in declared_after.saturating_add(1)..=last {
        if reader.bytes(marker, 9).await?.as_slice() != b"endstream" {
            continue;
        }
        let after = if marker >= 2
            && reader.byte(marker - 2).await? == Some(b'\r')
            && reader.byte(marker - 1).await? == Some(b'\n')
        {
            marker - 2
        } else if matches!(reader.byte(marker - 1).await?, Some(b'\r' | b'\n')) {
            marker - 1
        } else {
            marker
        };
        if after <= declared_after || after < data_at {
            continue;
        }
        let tail = reader.check_stream_tail(after, Some(reference)).await;
        if is_malformed(&tail) {
            continue;
        }
        let end = tail?;
        if found.is_some() {
            return Err(reader.problem(
                marker,
                Some(reference),
                PdfErrorKind::AmbiguousRepair,
                "multiple nearby stream terminators match an understated Length",
            ));
        }
        found = Some((after - data_at, end));
    }
    found.ok_or(reader.malformed(
        declared_after,
        Some(reference),
        "stream Length has no unique bounded repair",
    ))
}

#[cfg(test)]
mod tests {
    // Kept inline rather than in `../tests.rs`: several of the scanner's
    // defensive branches are unreachable, and without these lines the file
    // falls below the per-file coverage floor.
    use super::*;
    use crate::native::SeekableSource;
    use crate::read_exact_at;
    use crate::test_support::{NEVER, run};
    use std::io::{self, Cursor};

    /// A source whose bytes from `unreadable_from` onward fail with an I/O
    /// error, as a truncated network range or failing disk sector would.
    struct UnreadableTail {
        bytes: Vec<u8>,
        unreadable_from: u64,
    }

    impl RangedSource for UnreadableTail {
        fn size(&self) -> u64 {
            self.bytes.len() as u64
        }

        async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
            if offset >= self.unreadable_from {
                return Err(Error::Io(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "injected unreadable fragment tail",
                )));
            }
            let start = offset as usize;
            let end = (self.unreadable_from.min(self.bytes.len() as u64) as usize)
                .min(start + destination.len());
            destination[..end - start].copy_from_slice(&self.bytes[start..end]);
            Ok(end - start)
        }
    }

    fn one_byte_reads() -> Limits {
        Limits {
            io_chunk_bytes: 1,
            ..Limits::default()
        }
    }

    fn expect_injected_io(result: Result<FragmentScan>) {
        let error = result
            .err()
            .expect("unreadable fragment bytes were accepted");
        assert!(
            matches!(
                &error,
                Error::Io(inner) if inner.kind() == io::ErrorKind::UnexpectedEof
                    && inner.to_string() == "injected unreadable fragment tail"
            ),
            "{error:?}"
        );
    }

    #[test]
    fn repair_skips_terminators_at_the_declared_end_or_without_endobj() {
        // The first candidate sits exactly at the declared end, the second
        // lacks `endobj`, and only the LF-separated third one is complete.
        let payload = b"0123456789\nendstream junk\nendstream junk2";
        let mut bytes = b"1 0 obj\n<< /Length 10 >>\nstream\n".to_vec();
        let data_at = bytes.len() as u64;
        bytes.extend_from_slice(payload);
        bytes.extend_from_slice(b"\nendstream\nendobj");
        let end = bytes.len() as u64;
        let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
        let scan = run(scan_fragment_objects(
            &mut source,
            0,
            end,
            &Limits::default(),
            &NEVER,
        ))
        .unwrap();
        assert_eq!(scan.objects.len(), 1);
        assert_eq!(scan.objects[0].range.length, end);
        assert_eq!(scan.patches.len(), 1);
        let patch = &scan.patches[0];
        assert_eq!(patch.original, b"10");
        assert_eq!(patch.replacement, payload.len().to_string().into_bytes());
        assert!(patch.offset < data_at);

        let mut patched = PatchedSource::new(&mut source, &scan.patches);
        let mut length = [0_u8; 2];
        run(read_exact_at(
            &mut patched,
            patch.offset,
            &mut length,
            &Limits::default(),
            &NEVER,
        ))
        .unwrap();
        assert_eq!(&length, b"41");
        // An empty read inside a patch overlaps none of its bytes.
        assert_eq!(run(patched.read_at(patch.offset + 1, &mut [])).unwrap(), 0);
    }

    #[test]
    fn the_object_count_is_limited_to_the_pdf_object_limit() {
        let limit = MAX_PDF_OBJECTS as usize;
        assert_eq!(next_object_count(limit - 1).unwrap(), limit);
        assert!(matches!(
            next_object_count(limit),
            Err(Error::LimitExceeded {
                resource: "PDF fragment objects",
                limit: 8_388_607,
                attempted: 8_388_608,
            })
        ));
    }

    #[test]
    fn repair_accepts_a_terminator_without_a_preceding_end_of_line() {
        let mut bytes = b"1 0 obj\n<< /Length 10 >>\nstream\n0123456789ab".to_vec();
        bytes.extend_from_slice(b"endstream\nendobj");
        let end = bytes.len() as u64;
        let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
        let scan = run(scan_fragment_objects(
            &mut source,
            0,
            end,
            &Limits::default(),
            &NEVER,
        ))
        .unwrap();
        assert_eq!(scan.objects[0].range.length, end);
        assert_eq!(scan.patches.len(), 1);
        assert_eq!(scan.patches[0].replacement, b"12");
    }

    #[test]
    fn source_failure_after_the_declared_stream_extent_is_not_repaired() {
        let mut bytes = b"1 0 obj\n<< /Length 2000 >>\nstream\n".to_vec();
        let data_at = bytes.len() as u64;
        bytes.extend(std::iter::repeat_n(b'x', 2000));
        bytes.extend_from_slice(b"\nendstream\nendobj");
        let size = bytes.len() as u64;
        let mut source = UnreadableTail {
            bytes,
            unreadable_from: data_at + 2000,
        };
        expect_injected_io(run(scan_fragment_objects(
            &mut source,
            0,
            size,
            &one_byte_reads(),
            &NEVER,
        )));
    }

    #[test]
    fn source_failure_while_checking_a_repair_candidate_is_propagated() {
        let mut bytes = b"1 0 obj\n<< /Length 1000 >>\nstream\n".to_vec();
        bytes.extend(std::iter::repeat_n(b'x', 1002));
        let marker = bytes.len() as u64 + 1;
        bytes.extend_from_slice(b"\nendstream\nendobj");
        let size = bytes.len() as u64;
        let mut source = UnreadableTail {
            bytes,
            unreadable_from: marker + 9,
        };
        expect_injected_io(run(scan_fragment_objects(
            &mut source,
            0,
            size,
            &one_byte_reads(),
            &NEVER,
        )));

        // The same bytes repair cleanly once the tail is readable.
        source.unreadable_from = u64::MAX;
        let scan = run(scan_fragment_objects(
            &mut source,
            0,
            size,
            &one_byte_reads(),
            &NEVER,
        ))
        .unwrap();
        assert_eq!(scan.patches.len(), 1);
        assert_eq!(scan.patches[0].original, b"1000");
        assert_eq!(scan.patches[0].replacement, b"1002");
    }

    #[test]
    fn an_object_extending_past_the_page_table_end_is_charged_as_input() {
        let bytes = b"1 0 obj\nnull\nendobj\n2 0 obj\nnull\nendobj".to_vec();
        let end = bytes.len() as u64;
        // The page table ends inside the second object, and every single
        // read (bounded by the 30-byte syntax window) stays within the limit.
        let hint = 30;
        let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
        let limits = Limits {
            io_chunk_bytes: 8,
            max_allocation_bytes: hint * 32,
            max_input_bytes: hint,
            ..Limits::default()
        };
        let error = run(scan_fragment_objects(&mut source, 0, hint, &limits, &NEVER))
            .err()
            .expect("an object past the input limit was accepted");
        assert!(
            matches!(
                error,
                Error::PdfLimitExceeded {
                    offset,
                    object: None,
                    resource: "input bytes",
                    limit,
                    attempted,
                } if offset == end && limit == hint && attempted == end
            ),
            "{error:?}"
        );
    }
    fn replay_fixture(prefix: &[u8], scalar: &[u8]) -> Vec<u8> {
        let mut bytes =
            b"1 0 obj\n<<\n/Length 1 >>\nstream\nx\nendstream\nendobj\n2 0 obj\n1\nendobj\n"
                .to_vec();
        bytes.extend_from_slice(prefix);
        bytes.extend_from_slice(scalar);
        bytes.extend_from_slice(b"\n3 0 obj\nnull\nendobj\n");
        bytes
    }

    #[test]
    fn skips_only_a_known_partial_header_and_exact_previous_scalar_replay() {
        let bytes = replay_fixture(b"1 0 obj\n<<\n/Length\n", b"2 0 obj\n1\nendobj");
        for chunk in [1, 4096] {
            let end = bytes.len() as u64;
            let mut source = SeekableSource::new(Cursor::new(bytes.clone())).unwrap();
            let limits = Limits {
                io_chunk_bytes: chunk,
                ..Limits::default()
            };
            let scan = run(scan_fragment_objects(&mut source, 0, end, &limits, &NEVER)).unwrap();
            assert_eq!(
                scan.objects
                    .iter()
                    .map(|o| o.reference.number)
                    .collect::<Vec<_>>(),
                [1, 2, 3]
            );
            assert_eq!(
                scan.resolve_length(PdfRef {
                    number: 2,
                    generation: 0
                }),
                Some(1)
            );
            assert!(scan.patches.is_empty());
        }
    }

    #[test]
    fn refuses_unknown_prefixes_changed_scalars_and_unbounded_replays() {
        for (prefix, scalar) in [
            (
                b"9 0 obj\n<<\n/Length\n".as_slice(),
                b"2 0 obj\n1\nendobj".as_slice(),
            ),
            (b"1 1 obj\n<<\n/Length\n", b"2 0 obj\n1\nendobj"),
            (b"1 0 obj\n<<\n/Other\n", b"2 0 obj\n1\nendobj"),
            (b"1 0 obj\n<<\n/Length\n", b"2 0 obj\n2\nendobj"),
            (b"1 0 obj\n<<\n/Length\n", b"2 0 obj\nnull\nendobj"),
            (b"1 0 obj\n<<\n/Length\n", b"2 0 obj 1\nendobj"),
            (b"1 0 obj\n<<\n/Length\n", b"2 0 obj\n1\nendobjJUNK"),
        ] {
            let bytes = replay_fixture(prefix, scalar);
            let end = bytes.len() as u64;
            let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
            assert!(
                run(scan_fragment_objects(
                    &mut source,
                    0,
                    end,
                    &Limits::default(),
                    &NEVER
                ))
                .is_err()
            );
        }
        let mut prefix = b"1 0 obj\n<<\n/Length".to_vec();
        prefix.extend(std::iter::repeat_n(b' ', 256));
        let bytes = replay_fixture(&prefix, b"2 0 obj\n1\nendobj");
        let end = bytes.len() as u64;
        let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
        assert!(
            run(scan_fragment_objects(
                &mut source,
                0,
                end,
                &Limits::default(),
                &NEVER
            ))
            .is_err()
        );
    }
    #[test]
    fn refuses_unknown_original_or_scalar_and_changed_prefix() {
        let original = replay_fixture(b"1 0 obj\n<<\n/Length\n", b"2 0 obj\n1\nendobj");
        let text = String::from_utf8(original).unwrap();
        let duplicated = text.replacen("2 0 obj", "1 0 obj\nnull\nendobj\n2 0 obj", 1);
        let non_integer = text.replacen("2 0 obj\n1", "2 0 obj\nnull", 1);
        let changed_prefix = text.replacen("1 0 obj\n<<", "1 0 obj <<", 1);
        let long_integer =
            text.replacen("2 0 obj\n1", &format!("2 0 obj\n{}1", " ".repeat(256)), 1);
        let non_stream = text.replacen("<<\n/Length 1 >>\nstream\nx\nendstream", "null", 1);
        let no_history = "1 0 obj\n<<\n/Length\n2 0 obj\n1\nendobj\n".to_owned();
        for text in [
            duplicated,
            non_integer,
            changed_prefix,
            long_integer,
            no_history,
            non_stream,
        ] {
            let bytes = text.into_bytes();
            let end = bytes.len() as u64;
            let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
            assert!(
                run(scan_fragment_objects(
                    &mut source,
                    0,
                    end,
                    &Limits::default(),
                    &NEVER
                ))
                .is_err()
            );
        }
    }
    #[test]
    fn recovers_a_partial_filter_name_without_a_sample_specific_rule() {
        let mut bytes = b"1 0 obj\n<< /Length 1 /Filter /FlateDecode >>\nstream\nx\nendstream\nendobj\n2 0 obj\n1\nendobj\n".to_vec();
        bytes.extend_from_slice(
            b"1 0 obj\n<< /Length 1 /Filter /FlateD\n2 0 obj\n1\nendobj\n3 0 obj\nnull\nendobj\n",
        );
        let end = bytes.len() as u64;
        let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
        let scan = run(scan_fragment_objects(
            &mut source,
            0,
            end,
            &Limits::default(),
            &NEVER,
        ))
        .unwrap();
        assert_eq!(scan.objects.len(), 3);
    }
    fn scan_bytes(bytes: Vec<u8>) -> Result<FragmentScan> {
        let end = bytes.len() as u64;
        let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
        run(scan_fragment_objects(
            &mut source,
            0,
            end,
            &one_byte_reads(),
            &NEVER,
        ))
    }

    #[test]
    fn recovers_partial_length_objects_only_when_measured_value_matches() {
        let jpeg = [0xff, 0xd8, 0xff, 0xda, 0, 2, 7, 0xff, 0xd9];
        for prefix in [
            "2",
            "2 0",
            "2 0 obj",
            "2 0 obj\n9",
            "2 0 obj\n9\nendob",
            "3 0",
            "2 1",
            "2 0 obj\n8",
        ] {
            for value in [9, 8] {
                let mut bytes =
                    b"1 0 obj\n<< /Filter /DCTDecode /Length 2 0 R >>\nstream\n".to_vec();
                bytes.extend_from_slice(&jpeg);
                bytes.extend_from_slice(
                    format!("\nendstream\nendobj\n{prefix}\n2 0 obj\n{value}\nendobj\n").as_bytes(),
                );
                let expected = value == 9 && !["3 0", "2 1", "2 0 obj\n8"].contains(&prefix);
                assert_eq!(scan_bytes(bytes).is_ok(), expected, "{prefix}, {value}");
            }
        }
    }

    #[test]
    fn interrupted_length_prefixes_preserve_intervening_objects() {
        let jpeg = [0xff, 0xd8, 0xff, 0xda, 0, 2, 7, 0xff, 0xd9];
        for (prefix, following, final_scalar, succeeds) in [
            (
                "2 0 obj 9",
                "3 0 obj 15336 endobj",
                "2 0 obj 9 endobj",
                true,
            ),
            (
                "2 0 obj 9",
                "3 0 obj 15336 endobj",
                "2 0 obj 8 endobj",
                false,
            ),
            ("2 0 obj 9", "3 0 obj 15336 endobj", "", false),
            (
                "2 0 obj 8",
                "3 0 obj 15336 endobj",
                "2 0 obj 9 endobj",
                false,
            ),
            (
                "2 1 obj 9",
                "3 0 obj 15336 endobj",
                "2 0 obj 9 endobj",
                false,
            ),
            (
                "2 0 obj 9 extra",
                "3 0 obj 15336 endobj",
                "2 0 obj 9 endobj",
                false,
            ),
            (
                "2 0 obj 9",
                "3 0 obj << /Broken @ >> endobj",
                "2 0 obj 9 endobj",
                false,
            ),
        ] {
            let mut bytes = b"1 0 obj\n<< /Filter /DCTDecode /Length 2 0 R >>\nstream\n".to_vec();
            bytes.extend_from_slice(&jpeg);
            bytes.extend_from_slice(
                format!("\nendstream\nendobj\n{prefix}\n{following}\n{final_scalar}\n").as_bytes(),
            );
            let result = scan_bytes(bytes);
            assert_eq!(
                result.is_ok(),
                succeeds,
                "{prefix}, {following}, {final_scalar}"
            );
            if let Ok(scan) = result {
                assert_eq!(
                    scan.objects
                        .iter()
                        .map(|o| o.reference.number)
                        .collect::<Vec<_>>(),
                    [1, 3, 2]
                );
                assert_eq!(
                    scan.resolve_length(PdfRef {
                        number: 3,
                        generation: 0
                    }),
                    Some(15336)
                );
            }
        }
    }

    #[test]
    fn compacts_identical_complete_objects_and_keeps_source_order() {
        let first = b"3 0 obj\nnull\nendobj\n2 0 obj\n7\nendobj\n";
        let mut bytes = first.to_vec();
        bytes.extend_from_slice(first);
        bytes.extend_from_slice(b"1 0 obj\n<< /Length 5000 >>\nstream\n");
        bytes.extend(std::iter::repeat_n(b'x', 5000));
        bytes.extend_from_slice(b"\nendstream\nendobj\n");
        let stream = bytes[first.len() * 2..].to_vec();
        bytes.extend_from_slice(&stream);
        let scan = scan_bytes(bytes).unwrap();
        assert_eq!(
            scan.objects
                .iter()
                .map(|o| o.reference.number)
                .collect::<Vec<_>>(),
            [3, 2, 1]
        );
        assert_eq!(
            scan.lengths,
            [(
                PdfRef {
                    number: 2,
                    generation: 0
                },
                7
            )]
        );
    }

    #[test]
    fn rejects_conflicting_complete_replays() {
        for bytes in [
            b"1 0 obj\n7\nendobj\n1 0 obj\n8\nendobj\n".as_slice(),
            b"1 0 obj\n7\nendobj\n1 0 obj\n07\nendobj\n",
            b"1 0 obj\n7\nendobj\n1 0 obj 7\nendobj\n",
            b"1 0 obj\n<< /A 1 >>\nendobj\n1 0 obj\n<< /A 2 >>\nendobj\n",
        ] {
            assert!(scan_bytes(bytes.to_vec()).is_err());
        }
    }

    #[test]
    fn recovers_only_adjacent_same_reference_unfinished_headers() {
        for body in [
            b"10 0 obj<< /Value 37 >>endobj\n".as_slice(),
            b"10 0 obj[3 7 19]endobj\n",
            b"10 0 obj<< /Length 3 >>stream\nabc\nendstream\nendobj\n",
        ] {
            let mut bytes = b"10 0 \r\n".to_vec();
            let offset = bytes.len() as u64;
            bytes.extend_from_slice(body);
            let scan = scan_bytes(bytes).unwrap();
            assert_eq!(scan.objects.len(), 1);
            assert_eq!(scan.objects[0].reference.number, 10);
            assert_eq!(scan.objects[0].range.offset, offset);
        }
        for bytes in [
            b"10 0 \r\n11 0 obj<<>>endobj\n".as_slice(),
            b"10 1 \r\n10 1 obj<<>>endobj\n",
            b"10 0 \r\n10 1 obj<<>>endobj\n",
            b"10 0 \r\n10 0 obj<<",
            b"10 0 obj garbage\n10 0 obj<<>>endobj\n",
        ] {
            assert!(scan_bytes(bytes.to_vec()).is_err());
        }
    }

    #[test]
    fn adjacent_header_recovery_has_a_fixed_boundary_budget() {
        for length in [64, 65] {
            let mut bytes = b"10 0".to_vec();
            bytes.resize(length, b' ');
            bytes.extend_from_slice(b"10 0 obj<< /Value 37 >>endobj\n");
            assert_eq!(scan_bytes(bytes).is_ok(), length == 64);
        }
    }

    #[test]
    fn unfinished_headers_require_a_unique_identical_known_header() {
        for (prior, prefix, succeeds) in [
            ("7 0 obj 11 endobj\n", "7 0", true),
            ("7 0 obj << /Original 19 >> endobj\n", "7 0", true),
            ("7 0 obj 11 endobj\n", "7\t0", false),
            ("6 0 obj 11 endobj\n", "7 0", false),
            ("7 0 obj 11 endobj\n7 0 obj 11 endobj\n", "7 0", false),
        ] {
            let bytes = format!("{prior}{prefix}\n8 0 obj << /Next 23 >> endobj\n").into_bytes();
            let result = scan_bytes(bytes);
            assert_eq!(result.is_ok(), succeeds, "{prior:?}, {prefix:?}");
            if let Ok(scan) = result {
                assert_eq!(
                    scan.objects
                        .iter()
                        .map(|o| o.reference.number)
                        .collect::<Vec<_>>(),
                    [7, 8]
                );
            }
        }
    }

    #[test]
    fn recovers_known_dictionary_prefix_at_the_syntax_error_boundary() {
        let original = b"1 0 obj<< /A 7 /B << /C 9 >> >>endobj\n";
        for prefix in [
            b"1 0 obj<<\r\n".as_slice(),
            b"1 0 obj<< /A 7 /B <<\n",
            b"1 0 obj<< /A 7 /B << /C 9 >\r\n",
            b"1 0 obj<< /A 7 /B\r\n",
        ] {
            let mut bytes = original.to_vec();
            bytes.extend_from_slice(prefix);
            bytes.extend_from_slice(b"2 0 obj<< /Different 42 >>endobj\n");
            let scan = scan_bytes(bytes).unwrap();
            assert_eq!(scan.objects.len(), 2);
            assert_eq!(scan.objects[0].reference.number, 1);
            assert_eq!(scan.objects[0].range.length, (original.len() - 1) as u64);
            assert_eq!(scan.objects[1].reference.number, 2);
            assert_eq!(
                scan.objects[1].range.offset,
                (original.len() + prefix.len()) as u64
            );
        }
    }

    #[test]
    fn recovers_known_dictionary_cut_inside_an_array() {
        let original = b"7 0 obj<< /Box [-1 3 20 40] >>endobj\n";
        for prefix in [
            b"7 0 obj<< /Box [\n".as_slice(),
            b"7 0 obj<< /Box [-1 3\r\n",
        ] {
            let mut bytes = original.to_vec();
            bytes.extend_from_slice(prefix);
            bytes.extend_from_slice(b"9 0 obj<< /New 81 >>endobj\n");
            let scan = scan_bytes(bytes).unwrap();
            assert_eq!(scan.objects.len(), 2);
            assert_eq!(
                scan.objects[1].range.offset,
                (original.len() + prefix.len()) as u64
            );
        }
        let mut changed = original.to_vec();
        changed.extend_from_slice(b"7 0 obj<< /Box [99\n9 0 obj<< /New 81 >>endobj\n");
        assert!(scan_bytes(changed).is_err());
    }

    #[test]
    fn dictionary_prefix_recovery_requires_exact_prior_dictionary_bytes() {
        for bytes in [
            b"1 0 obj<< /A 7 >>endobj\n1 0 obj<< /A 8\n2 0 obj<<>>endobj\n".as_slice(),
            b"1 0 obj<< /A 7 >>endobj\n3 0 obj<<\n2 0 obj<<>>endobj\n",
            b"1 0 obj<< /A 7 /Bee 9 >>endobj\n1 0 obj<< /A 7 /Boo 9\n2 0 obj<<>>endobj\n",
            b"4294967296 0 obj<<\n2 0 obj<<>>endobj\n",
            b"1 0 obj<< /A 7 >>endobj\n1 0 obj<<\n2 0 R\n",
            b"1 0 obj<< /A (literal) >>endobj\n1 0 obj<< /A (literal\n2 0 obj<<>>endobj\n",
        ] {
            assert!(scan_bytes(bytes.to_vec()).is_err());
        }
    }

    #[test]
    fn known_integer_prefix_preserves_a_new_following_object() {
        for suffix in ["e", "en", "end", "endo", "endob"] {
            let bytes = format!("7 0 obj 137 endobj\n7 0 obj 137 {suffix}\n8 0 obj 19 endobj\n")
                .into_bytes();
            let result = scan_bytes(bytes).unwrap();
            assert_eq!(
                result
                    .objects
                    .iter()
                    .map(|object| object.reference.number)
                    .collect::<Vec<_>>(),
                [7, 8]
            );
        }
        for bytes in [
            "7 0 obj 137 endobj\n7 0 obj 138 endob\n8 0 obj 19 endobj\n",
            "7 0 obj 137 endobj\n7 0 obj 137 endox\n8 0 obj 19 endobj\n",
            "7 0 obj 137 endobj\n7 0 obj 137 endobj\n7 0 obj 137 endob\n8 0 obj 19 endobj\n",
        ] {
            assert!(scan_bytes(bytes.as_bytes().to_vec()).is_err(), "{bytes}");
        }
    }

    #[test]
    fn a_known_stream_dictionary_prefix_does_not_discard_its_payload() {
        let original = b"1 0 obj<< /Length 1 >>stream\nx\nendstream\nendobj\n";
        let mut bytes = original.to_vec();
        bytes.extend_from_slice(b"1 0 obj<< /Length\n2 0 obj<< /Next 7 >>endobj\n");
        let scan = scan_bytes(bytes).unwrap();
        assert_eq!(scan.objects.len(), 2);
        assert_eq!(scan.objects[0].range.offset, 0);
        assert_eq!(scan.objects[0].range.length, original.len() as u64 - 1);
        assert_eq!(scan.objects[1].reference.number, 2);
        let bytes = replay_fixture(b"1 0 obj\n<<\n/Length\n", b"4 0 obj\n1\nendobj");
        let scan = scan_bytes(bytes).unwrap();
        assert_eq!(
            scan.objects
                .iter()
                .map(|o| o.reference.number)
                .collect::<Vec<_>>(),
            [1, 2, 4, 3]
        );
    }

    #[test]
    fn recovers_a_dictionary_prefix_and_older_integer_copy() {
        let bytes = b"1 0 obj\n<< /Type /Page /A 42 >>\nendobj\n2 0 obj\n7\nendobj\n3 0 obj\nnull\nendobj\n1 0 obj\n<< /Type /Page /A\n2 0 obj\n7\nendobj\n4 0 obj\nnull\nendobj\n";
        let scan = scan_bytes(bytes.to_vec()).unwrap();
        assert_eq!(scan.objects.len(), 4);
    }

    #[test]
    fn recovers_truncated_replayed_stream_but_rejects_changed_prefix() {
        use std::io::Write;
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(b"original stream payload").unwrap();
        let encoded = encoder.finish().unwrap();
        let header = b"1 0 obj\n<< /Length 2 0 R /Filter /FlateDecode >>\nstream\n";
        let scalar = format!("2 0 obj\n{}\nendobj\n", encoded.len());
        let mut original = header.to_vec();
        original.extend_from_slice(&encoded);
        original.extend_from_slice(b"\nendstream\nendobj\n");
        original.extend_from_slice(scalar.as_bytes());
        original.extend_from_slice(b"3 0 obj\nnull\nendobj\n");
        for changed in [false, true] {
            let mut bytes = original.clone();
            bytes.extend_from_slice(header);
            bytes.extend_from_slice(&encoded[..3]);
            if changed {
                *bytes.last_mut().unwrap() ^= 1;
            }
            bytes.extend_from_slice(b"\n");
            bytes.extend_from_slice(scalar.as_bytes());
            bytes.extend_from_slice(b"4 0 obj\nnull\nendobj\n");
            let result = scan_bytes(bytes);
            assert_eq!(result.is_ok(), !changed);
            if let Ok(scan) = result {
                assert_eq!(scan.objects.len(), 4);
            }
        }
    }
    #[test]
    fn refuses_incomplete_candidate_and_ambiguous_embedded_scalar_copies() {
        let mut bytes = b"1 0 obj\n<< /Filter /DCTDecode /Length 2 0 R >>\nstream\n".to_vec();
        bytes.extend_from_slice(&[0xff, 0xd8, 0xff, 0xda, 0, 2, 7, 0xff, 0xd9]);
        bytes.extend_from_slice(b"\nendstream\nendobj\n2 0\n2 0 obj\n9\nendob");
        assert!(scan_bytes(bytes).is_err());

        let scalar = b"2 0 obj\n7\nendobj";
        let mut bytes = scalar.to_vec();
        bytes.push(b'\n');
        let original_at = bytes.len() as u64;
        let payload = b"x\n2 0 obj\n7\nendobj\ny\n2 0 obj\n7\nendobj\nz";
        let header = format!("1 0 obj\n<< /Length {} >>\nstream\n", payload.len());
        bytes.extend_from_slice(header.as_bytes());
        bytes.extend_from_slice(payload);
        bytes.extend_from_slice(b"\nendstream\nendobj\n");
        let original_length = bytes.len() as u64 - original_at - 1;
        let replay_at = bytes.len() as u64;
        bytes.extend_from_slice(header.as_bytes());
        bytes.extend_from_slice(&payload[..payload.len() - 1]);
        let end = bytes.len() as u64;
        let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
        let limits = Limits::default();
        let mut reader = Reader::new(
            &mut source,
            PdfRange {
                offset: 0,
                length: end,
            },
            &limits,
            &NEVER,
        )
        .unwrap();
        let reference = PdfRef {
            number: 2,
            generation: 0,
        };
        let objects = [
            FragmentObject {
                reference,
                range: PdfRange {
                    offset: 0,
                    length: scalar.len() as u64,
                },
            },
            FragmentObject {
                reference: PdfRef {
                    number: 1,
                    generation: 0,
                },
                range: PdfRange {
                    offset: original_at,
                    length: original_length,
                },
            },
        ];
        assert_eq!(
            run(replay_end(
                &mut reader,
                replay_at,
                &objects,
                &[(reference, 7)]
            ))
            .unwrap(),
            None
        );
    }
    #[test]
    fn recovered_stream_prefixes_do_not_refund_decoding_work() {
        use std::io::Write;
        let plain = [b'x'; 120];
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::none());
        encoder.write_all(&plain).unwrap();
        let encoded = encoder.finish().unwrap();
        let header = b"1 0 obj\n<< /Length 2 0 R /Filter /FlateDecode >>\nstream\n";
        let scalar = format!("2 0 obj\n{}\nendobj\n", encoded.len());
        let mut bytes = header.to_vec();
        bytes.extend_from_slice(&encoded);
        bytes.extend_from_slice(b"\nendstream\nendobj\n");
        bytes.extend_from_slice(scalar.as_bytes());
        bytes.extend_from_slice(header);
        bytes.extend_from_slice(&encoded[..50]);
        bytes.push(b'\n');
        bytes.extend_from_slice(scalar.as_bytes());
        bytes.extend_from_slice(b"4 0 obj\n<< /Length 5 0 R /Filter /FlateDecode >>\nstream\n");
        bytes.extend_from_slice(&encoded);
        bytes.extend_from_slice(
            format!("\nendstream\nendobj\n5 0 obj\n{}\nendobj\n", encoded.len()).as_bytes(),
        );
        assert!(
            scan_bytes(bytes.clone()).is_ok(),
            "fixture must recover without the work ceiling"
        );
        let end = bytes.len() as u64;
        let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
        let limits = Limits {
            max_output_bytes: 240,
            ..one_byte_reads()
        };
        let error = run(scan_fragment_objects(&mut source, 0, end, &limits, &NEVER))
            .err()
            .unwrap();
        assert!(
            matches!(
                error,
                Error::LimitExceeded {
                    resource: "CAJ Flate scan bytes",
                    ..
                }
            ),
            "{error}"
        );
    }
}

#[cfg(test)]
mod candidate_tests {
    use super::*;
    use crate::native::SeekableSource;
    use crate::test_support::{NEVER, run};
    use std::io::Cursor;

    const ORIGINAL: &[u8] = b"7 0 obj\n<< /Type /Example /Values [3 9] >>\nendobj\n";
    const PREFIX: &[u8] = b"7 0 obj\n<< /Type /Exa\n";
    const NEXT: &[u8] = b"8 0 obj\n<< /Value 42 >>\nendobj\n";

    fn candidate(offset: usize) -> FragmentCandidate {
        FragmentCandidate {
            object: FragmentObject {
                reference: PdfRef {
                    number: 7,
                    generation: 0,
                },
                range: PdfRange {
                    offset: offset as u64,
                    length: (ORIGINAL.len() - 1) as u64,
                },
            },
            used: false,
        }
    }

    fn scan(bytes: Vec<u8>, candidates: &mut [FragmentCandidate]) -> Result<FragmentScan> {
        let end = bytes.len() as u64;
        let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
        run(scan_fragment_with_candidates(
            &mut source,
            0,
            end,
            &Limits {
                io_chunk_bytes: 1,
                ..Limits::default()
            },
            &NEVER,
            candidates,
            &mut 0,
        ))
    }

    #[test]
    fn patched_rows_do_not_supply_recovery_candidates() {
        let bytes = b"1 0 obj << /Length 3 >> stream\nabcdef\nendstream\nendobj\n".to_vec();
        let end = bytes.len() as u64;
        let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
        let scan = run(scan_fragment_objects(
            &mut source,
            0,
            end,
            &Limits::default(),
            &NEVER,
        ))
        .unwrap();
        assert_eq!(scan.patches.len(), 1);
        assert!(
            run(collect_fragment_candidates(
                &mut source,
                0,
                end,
                &Limits::default(),
                &NEVER,
                &mut 0
            ))
            .unwrap()
            .is_empty()
        );
    }

    #[test]
    fn candidate_stream_lengths_are_still_verified_by_the_complete_scan() {
        let mut bytes = b"8 0 obj << /Length 12 0 R /Filter /DCTDecode >> stream\n".to_vec();
        bytes.extend_from_slice(&[0xff, 0xd8, 0xff, 0xda, 0, 2, 7, 0xff, 0xd9]);
        bytes.extend_from_slice(b"\nendstream\nendobj\n");
        let end = bytes.len() as u64;
        let mut source = SeekableSource::new(Cursor::new(bytes.clone())).unwrap();
        assert_eq!(
            run(collect_fragment_candidates(
                &mut source,
                0,
                end,
                &Limits::default(),
                &NEVER,
                &mut 0
            ))
            .unwrap()
            .len(),
            1
        );
        assert!(scan(bytes.clone(), &mut []).is_err());
        let mut valid = bytes.clone();
        valid.extend_from_slice(b"12 0 obj 9 endobj\n");
        assert_eq!(scan(valid, &mut []).unwrap().objects.len(), 2);
        bytes.extend_from_slice(b"12 0 obj 8 endobj\n");
        assert!(scan(bytes, &mut []).is_err());
    }

    #[test]
    fn anchored_candidates_can_depend_on_a_later_row_prefix_proof() {
        let header = b"8 0 obj << /Length 600 >> stream\n";
        let mut bytes = header.to_vec();
        bytes.extend_from_slice(b"ZZZZZ\n");
        let row_start = bytes.len() as u64;
        bytes.extend_from_slice(b"9 0 obj 42 endobj\n");
        bytes.extend_from_slice(PREFIX);
        bytes.extend_from_slice(b"10 0 obj 13 endobj\n");
        bytes.extend_from_slice(header);
        bytes.extend_from_slice(&[b'Z'; 600]);
        bytes.extend_from_slice(b"\nendstream\nendobj\n");
        let row_end = bytes.len() as u64;
        bytes.extend_from_slice(ORIGINAL);
        let mut source = SeekableSource::new(Cursor::new(bytes.clone())).unwrap();
        // A row alone is not a complete verified document: object 7 is later.
        assert!(
            run(scan_fragment_objects(
                &mut source,
                row_start,
                row_end,
                &Limits::default(),
                &NEVER
            ))
            .is_err()
        );
        let objects = run(collect_fragment_candidates(
            &mut source,
            row_start,
            row_end,
            &Limits::default(),
            &NEVER,
            &mut 0,
        ))
        .unwrap();
        assert_eq!(
            objects
                .iter()
                .map(|o| o.reference.number)
                .collect::<Vec<_>>(),
            [9, 10, 8]
        );
        let mut candidates: Vec<_> = objects
            .into_iter()
            .map(|object| FragmentCandidate {
                object,
                used: false,
            })
            .collect();
        assert_eq!(
            scan(bytes.clone(), &mut candidates).unwrap().objects.len(),
            4
        );
        assert!(candidates.iter().any(|candidate| candidate.used));
        // Collection must never waive proof on the final whole fragment.
        bytes.truncate(row_end as usize);
        assert!(scan(bytes, &mut candidates).is_err());
    }

    #[test]
    fn later_dictionary_must_be_reached_at_its_exact_boundary() {
        let mut bytes = [PREFIX, NEXT].concat();
        let mut candidates = [candidate(bytes.len())];
        bytes.extend_from_slice(ORIGINAL);
        let result = scan(bytes, &mut candidates).unwrap();
        assert!(candidates[0].used);
        assert_eq!(result.objects.len(), 2);
        assert_eq!(result.objects[1].range, candidates[0].object.range);

        // An apparently valid candidate embedded inside a real opaque stream
        // must not justify discarding an earlier interrupted object.
        let mut bytes = PREFIX.to_vec();
        bytes.extend_from_slice(
            format!("8 0 obj\n<< /Length {} >>\nstream\n", ORIGINAL.len()).as_bytes(),
        );
        let mut candidates = [candidate(bytes.len())];
        bytes.extend_from_slice(ORIGINAL);
        bytes.extend_from_slice(b"\nendstream\nendobj\n");
        assert!(matches!(
            scan(bytes, &mut candidates),
            Err(Error::Pdf {
                kind: PdfErrorKind::AmbiguousRepair,
                reason: "recovery candidate is not a complete fragment object",
                ..
            })
        ));
    }

    #[test]
    fn candidate_recovery_rejects_changed_prefixes_and_conflicting_copies() {
        let mut bytes = [PREFIX, NEXT].concat();
        let offset = bytes.len();
        bytes.extend_from_slice(ORIGINAL);
        let mut conflicting = [candidate(offset), candidate(offset + 1)];
        assert!(scan(bytes.clone(), &mut conflicting).is_err());
        assert!(conflicting.iter().all(|c| !c.used));
        for altered in *b"b!\0" {
            let mut changed = bytes.clone();
            changed[PREFIX.len() - 2] = altered;
            let mut candidates = [candidate(offset)];
            assert!(scan(changed, &mut candidates).is_err());
            assert!(!candidates[0].used);
        }
    }
    #[test]
    fn later_flate_copy_replaces_only_the_matching_interruption() {
        use std::io::Write;
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder
            .write_all(b"Original bounded candidate recovery stream.")
            .unwrap();
        let encoded = encoder.finish().unwrap();
        let header = b"7 0 obj\n<< /Length 9 0 R /Filter /FlateDecode >>\nstream\n";
        let mut complete = header.to_vec();
        complete.extend_from_slice(&encoded);
        complete.extend_from_slice(b"\nendstream\nendobj");
        let mut bytes = header.to_vec();
        bytes.extend_from_slice(&encoded[..5]);
        bytes.push(b'\n');
        bytes.extend_from_slice(NEXT);
        let mut candidates = [candidate(bytes.len())];
        candidates[0].object.range.length = complete.len() as u64;
        bytes.extend_from_slice(&complete);
        bytes.extend_from_slice(format!("\n9 0 obj\n{}\nendobj\n", encoded.len()).as_bytes());
        let result = scan(bytes, &mut candidates).unwrap();
        assert!(candidates[0].used);
        assert_eq!(result.objects.len(), 3);
    }

    #[test]
    fn anchored_attempts_share_the_inflate_work_limit() {
        use std::io::Write;
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&[b'X'; 200]).unwrap();
        let encoded = encoder.finish().unwrap();
        let mut bytes = b"1 0 obj\n<< /Length 2 0 R /Filter /FlateDecode >>\nstream\n".to_vec();
        bytes.extend_from_slice(&encoded);
        bytes.extend_from_slice(
            format!("\nendstream\nendobj\n2 0 obj\n{}\nendobj", encoded.len()).as_bytes(),
        );
        let end = bytes.len() as u64;
        let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
        let limits = Limits {
            max_output_bytes: 300,
            ..Limits::default()
        };
        let mut work = 0;
        run(scan_fragment_with_candidates(
            &mut source,
            0,
            end,
            &limits,
            &NEVER,
            &mut [],
            &mut work,
        ))
        .unwrap();
        assert_eq!(work, 200);
        assert!(matches!(
            run(scan_fragment_with_candidates(
                &mut source,
                0,
                end,
                &limits,
                &NEVER,
                &mut [],
                &mut work
            )),
            Err(Error::LimitExceeded {
                resource: "CAJ Flate scan bytes",
                ..
            })
        ));
        assert_eq!(work, 400);
    }
    #[test]
    fn later_direct_length_stream_has_a_bounded_prefix_recovery() {
        let header = b"7 0 obj\n<< /Length 600 >>\nstream\n";
        let mut complete = header.to_vec();
        complete.extend_from_slice(&[b'Z'; 600]);
        complete.extend_from_slice(b"\nendstream\nendobj");
        let mut bytes = header.to_vec();
        bytes.extend_from_slice(b"ZZZZZ\n");
        bytes.extend_from_slice(NEXT);
        let mut candidates = [candidate(bytes.len())];
        candidates[0].object.range.length = complete.len() as u64;
        bytes.extend_from_slice(&complete);
        assert_eq!(scan(bytes, &mut candidates).unwrap().objects.len(), 2);
        assert!(candidates[0].used);
    }

    #[test]
    fn deferred_prefixes_require_real_later_counterparts() {
        let bytes = [PREFIX, NEXT, ORIGINAL].concat();
        assert_eq!(scan(bytes, &mut []).unwrap().objects.len(), 2);
        let bytes = b"7 0 obj << /Box [1 3\n8 0 obj 42 endobj\n7 0 obj << /Box [1 3 9] >> endobj\n"
            .to_vec();
        assert_eq!(scan(bytes, &mut []).unwrap().objects.len(), 2);
        let cut = b"7 0 obj << /Value 3 >\r\n8 0 obj 42 endobj\n7 0 obj << /Value 3 >> endobj\n";
        assert_eq!(scan(cut.to_vec(), &mut []).unwrap().objects.len(), 2);
        let changed =
            b"7 0 obj << /Value 3 >\r\n8 0 obj 42 endobj\n7 0 obj << /Value 4 >> endobj\n";
        assert!(scan(changed.to_vec(), &mut []).is_err());
        let mut padded = b"7 0 obj << /Value 3 >".to_vec();
        padded.extend_from_slice(&[b' '; 257]);
        padded.extend_from_slice(&cut[22..]);
        assert!(scan(padded, &mut []).is_err());
        let mut fake = PREFIX.to_vec();
        fake.extend_from_slice(
            format!("8 0 obj << /Length {} >> stream\n", ORIGINAL.len()).as_bytes(),
        );
        fake.extend_from_slice(ORIGINAL);
        fake.extend_from_slice(b"\nendstream\nendobj\n");
        assert!(matches!(
            scan(fake, &mut []),
            Err(Error::Pdf {
                reason: "interrupted prefix has no exact complete counterpart",
                ..
            })
        ));
        let changed =
            b"7 0 obj << /Box [1 3\n8 0 obj 42 endobj\n7 0 obj << /Box [1 4 9] >> endobj\n"
                .to_vec();
        assert!(scan(changed, &mut []).is_err());
    }

    #[test]
    fn prior_indirect_lengths_frame_opaque_streams_without_searching_payloads() {
        let payload = b"opaque endstream endobj 99 0 obj";
        let mut bytes = format!(
            "2 0 obj {} endobj\n1 0 obj << /Length 2 0 R /Filter /RunLengthDecode >> stream\n",
            payload.len()
        )
        .into_bytes();
        bytes.extend_from_slice(payload);
        bytes.extend_from_slice(b"\nendstream\nendobj\n");
        assert_eq!(scan(bytes.clone(), &mut []).unwrap().objects.len(), 2);
        bytes.extend_from_slice(b"2 0 obj 1 endobj\n");
        assert!(scan(bytes, &mut []).is_err());
        for bytes in [
            b"2 0 obj 1 endobj\n1 0 obj << /Length 2 0 R /Filter /RunLengthDecode >> stream\nlong\nendstream\nendobj\n".as_slice(),
            b"1 0 obj << /Length 2 0 R /Filter /RunLengthDecode >> stream\nx\nendstream\nendobj\n2 0 obj 1 endobj\n",
        ] {
            assert!(scan(bytes.to_vec(), &mut []).is_err());
        }
    }

    #[test]
    fn unfinished_headers_require_a_real_later_object() {
        for prefix in ["7", "7 0", "7 0 o", "7 0 ob"] {
            let bytes = format!("{prefix}\n8 0 obj 42 endobj\n7 0 obj << /Value 19 >> endobj\n")
                .into_bytes();
            let result = scan(bytes, &mut []).unwrap();
            assert_eq!(
                result
                    .objects
                    .iter()
                    .map(|object| object.reference.number)
                    .collect::<Vec<_>>(),
                [8, 7]
            );
        }
        for bytes in [
            "7\n8 0 obj 42 endobj\n",
            "7 1\n8 0 obj 42 endobj\n7 0 obj 19 endobj\n",
            "7 0 nonsense\n8 0 obj 42 endobj\n7 0 obj 19 endobj\n",
            "7 0 ox\n8 0 obj 42 endobj\n7 0 obj 19 endobj\n",
            "7 1 o\n8 0 obj 42 endobj\n7 0 obj 19 endobj\n",
            "7\n8 0 obj 42 endobj\n7 0 obj 19 endobj\n7 0 obj 20 endobj\n",
            "7 0\n8 0 obj 42 endobj\n7 1 obj 19 endobj\n",
        ] {
            assert!(scan(bytes.as_bytes().to_vec(), &mut []).is_err(), "{bytes}");
        }
        let mut padded = b"7 0 ob".to_vec();
        padded.extend_from_slice(&[b' '; 65]);
        padded.extend_from_slice(b"8 0 obj 42 endobj\n7 0 obj 19 endobj\n");
        assert!(scan(padded, &mut []).is_err());
        let payload = b"7 0 obj 19 endobj";
        let mut fake = format!("7\n8 0 obj << /Length {} >> stream\n", payload.len()).into_bytes();
        fake.extend_from_slice(payload);
        fake.extend_from_slice(b"\nendstream\nendobj\n");
        assert!(scan(fake, &mut []).is_err());
    }

    #[test]
    fn unfinished_header_candidate_propagates_syntax_limits() {
        let mut bytes = b"7 0\n8 0 obj << /Long (".to_vec();
        bytes.extend_from_slice(&[b'A'; 2000]);
        bytes.extend_from_slice(b") >> endobj\n7 0 obj 42 endobj\n");
        let size = bytes.len() as u64;
        let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
        let limits = Limits {
            max_allocation_bytes: 4096,
            io_chunk_bytes: 1,
            ..Limits::default()
        };
        assert!(matches!(
            run(scan_fragment_objects(&mut source, 0, size, &limits, &NEVER)),
            Err(Error::PdfLimitExceeded { .. })
        ));
    }

    #[test]
    fn deferred_boundary_probe_preserves_syntax_limits() {
        fn probe(
            bytes: Vec<u8>,
            boundary: u64,
            limit: u64,
        ) -> Result<Option<(u64, FragmentObject)>> {
            let size = bytes.len() as u64;
            let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
            let limits = Limits {
                max_allocation_bytes: limit,
                io_chunk_bytes: 1,
                ..Limits::default()
            };
            let mut reader = Reader::new(
                &mut source,
                PdfRange {
                    offset: 0,
                    length: size,
                },
                &limits,
                &NEVER,
            )
            .unwrap();
            let error = reader.malformed(boundary, None, "expected PDF name");
            run(interrupted_syntax_prefix(&mut reader, 0, &error))
        }
        // A short, malformed header cannot create a pending object or loop
        // when walking back to the first token exhausts the prefix.
        assert!(probe(b"7 ?".to_vec(), 2, 4096).unwrap().is_none());
        let mut bytes = PREFIX.to_vec();
        bytes.extend_from_slice(b"8 0 obj << /Long (");
        bytes.extend_from_slice(&[b'A'; 2000]);
        bytes.extend_from_slice(b") >> endobj");
        assert!(matches!(
            probe(bytes, PREFIX.len() as u64, 4096),
            Err(Error::PdfLimitExceeded { .. })
        ));
    }

    #[test]
    fn candidate_bounds_and_following_syntax_are_checked() {
        fn probe(
            bytes: Vec<u8>,
            start: u64,
            mut item: FragmentCandidate,
            limit: u64,
        ) -> Result<Option<u64>> {
            let size = bytes.len() as u64;
            let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
            let limits = Limits {
                io_chunk_bytes: 1,
                max_allocation_bytes: limit,
                ..Limits::default()
            };
            let mut reader = Reader::new(
                &mut source,
                PdfRange {
                    offset: start,
                    length: size - start,
                },
                &limits,
                &NEVER,
            )
            .unwrap();
            run(candidate_prefix_end(
                &mut reader,
                0,
                std::slice::from_mut(&mut item),
            ))
        }
        let mut bytes = [PREFIX, NEXT].concat();
        let at = bytes.len();
        bytes.extend_from_slice(ORIGINAL);
        let mut unknown = candidate(at);
        unknown.object.reference.number = 99;
        assert_eq!(probe(bytes.clone(), 0, unknown, 4096).unwrap(), None);
        assert_eq!(probe(bytes.clone(), 0, candidate(0), 4096).unwrap(), None);
        let mut shifted = vec![b' '; 10];
        shifted.extend_from_slice(&bytes);
        assert_eq!(probe(shifted, 10, candidate(0), 4096).unwrap(), None);
        let mut no_header = bytes.clone();
        no_header[0] = b'?';
        assert_eq!(probe(no_header, 0, candidate(at), 4096).unwrap(), None);
        let mut header_only = bytes.clone();
        header_only[1] = b'\t';
        assert_eq!(probe(header_only, 0, candidate(at), 4096).unwrap(), None);
        let mut bad_next = bytes.clone();
        bad_next[PREFIX.len() + 4] = b'x';
        assert_eq!(probe(bad_next, 0, candidate(at), 4096).unwrap(), None);
        let mut long = PREFIX.to_vec();
        long.extend_from_slice(b"8 0 obj\n<< /Long (");
        long.extend_from_slice(&[b'A'; 2000]);
        long.extend_from_slice(b") >>\nendobj\n");
        let at = long.len();
        long.extend_from_slice(ORIGINAL);
        assert!(matches!(
            probe(long, 0, candidate(at), 512),
            Err(Error::PdfLimitExceeded { .. })
        ));
    }
}
