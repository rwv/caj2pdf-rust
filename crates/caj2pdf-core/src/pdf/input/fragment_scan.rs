// SPDX-License-Identifier: MIT

//! Bounded object scanning for headerless CAJ PDF fragments.
//!
//! Each indirect object is parsed and inspected once. A stream ends at its
//! declared `/Length` when `endstream` follows it. Otherwise, and for an
//! indirect length that is not yet known, the scanner searches forward for
//! `endstream` and keeps the first candidate that the later-resolved
//! `/Length` confirms. Damaged-input rules live in `recovery`.

use super::recovery::{self, LengthPatch, Recovery, find_endstream};
use super::{
    FragmentInspection, ObjectTail, Reader, exact_reference, exact_unsigned, inspect_head,
};
use crate::caj::CajPageRow;
use crate::fallible::reserve;
use crate::pdf::writer::MAX_PDF_OBJECTS;
use crate::pdf::{FragmentObject, PdfRange, PdfRef};
use crate::{Cancellation, Error, ErrorKind, Limits, RangedSource, Result};
use std::collections::BTreeMap;

/// Bytes past the page table's body end that may finish the final object.
const MAX_FRAGMENT_TAIL_EXTENSION: u64 = 64 * 1024;
/// Re-scans that may move a searched stream end before the first error stands.
const MAX_EXTENT_RETRIES: usize = 16;

/// One indexed object and its single structural inspection. An inspection
/// error is kept, not raised, so callers report it in object order.
pub(crate) struct ScannedObject {
    pub object: FragmentObject,
    pub inspection: Result<FragmentInspection>,
}

/// Validated object boundaries within a headerless PDF fragment. The CAJ
/// page table supplies the minimum body end and page order, never object spans.
pub(crate) struct FragmentScan {
    pub objects: Vec<ScannedObject>,
    pub patches: Vec<LengthPatch>,
    pub damaged: Vec<(Option<PdfRef>, u64)>,
}

/// Independently framed object; `used` requires confirmation by the full scan.
#[derive(Clone, Copy)]
pub(crate) struct FragmentCandidate {
    pub object: FragmentObject,
    pub used: bool,
}

/// Collect locally framed candidates from an anchored row. Deferred prefixes
/// and indirect Length references are proved by the final whole-fragment scan,
/// never by this index.
pub(crate) fn collect_fragment_candidates<S: RangedSource, C: Cancellation>(
    source: &mut S,
    start: u64,
    end: u64,
    limits: &Limits,
    cancellation: &C,
) -> Result<Vec<FragmentObject>> {
    let scan = scan_fragment(source, start, end, limits, cancellation, Mode::Candidates)?;
    Ok(if scan.patches.is_empty() {
        scan.objects.iter().map(|scanned| scanned.object).collect()
    } else {
        Vec::new()
    })
}

/// Scan a whole fragment, letting `candidates` from later page-table rows
/// bound interrupted objects. A used candidate must be a scanned object.
pub(crate) fn scan_fragment_with_candidates<S: RangedSource, C: Cancellation>(
    source: &mut S,
    body_start: u64,
    minimum_end: u64,
    limits: &Limits,
    cancellation: &C,
    candidates: &mut [FragmentCandidate],
) -> Result<FragmentScan> {
    let mode = Mode::Complete(candidates);
    scan_fragment(source, body_start, minimum_end, limits, cancellation, mode)
}

/// Resume only at container page anchors after malformed object syntax.
/// Objects reachable only through an unvalidated byte search are never admitted.
pub(crate) fn scan_damaged_fragment<S: RangedSource, C: Cancellation>(
    source: &mut S,
    rows: &[CajPageRow],
    end: u64,
    limits: &Limits,
    cancellation: &C,
    candidates: &mut [FragmentCandidate],
) -> Result<FragmentScan> {
    let mode = Mode::Damaged(rows, candidates);
    scan_fragment(source, rows[0].offset, end, limits, cancellation, mode)
}

enum Mode<'a> {
    Complete(&'a mut [FragmentCandidate]),
    Candidates,
    Damaged(&'a [CajPageRow], &'a mut [FragmentCandidate]),
}

/// Why an object at a known start could not be indexed.
pub(super) enum Failure {
    /// The object header or value does not parse.
    Syntax(Error),
    /// A parsed stream's extent is not confirmed.
    Stream(Error, Box<StreamFailure>),
    /// Any other malformed object.
    Other(Error),
}

impl Failure {
    pub fn error(&self) -> &Error {
        match self {
            Self::Syntax(error) | Self::Stream(error, _) | Self::Other(error) => error,
        }
    }

    fn into_error(self) -> Error {
        match self {
            Self::Syntax(error) | Self::Stream(error, _) | Self::Other(error) => error,
        }
    }
}

/// A parsed stream whose `/Length` does not reach `endstream`.
pub(super) struct StreamFailure {
    pub reference: PdfRef,
    /// The payload offset from the object start.
    pub data_start: u64,
    /// The declared or resolved length; `None` when no `endstream` follows.
    pub length: Option<u64>,
    /// For a direct `/Length`: the absolute offset and bytes of its digits.
    pub direct: Option<(u64, Vec<u8>)>,
    pub inspection: Result<FragmentInspection>,
}

/// An indirect stream `/Length`, checked against its integer object once the
/// pass is complete.
#[derive(Clone, Copy)]
pub(super) struct PendingLength {
    pub target: PdfRef,
    pub length: u64,
    start: u64,
    data_at: u64,
    end: u64,
    /// The searched `endstream`, while the length was still unknown.
    marker: Option<u64>,
}

/// A stream extent fixed by an earlier pass of the same scan.
#[derive(Clone, Copy)]
enum Extent {
    /// The integer object the earlier pass resolved.
    Length(u64),
    /// Search for `endstream` from here: the earlier candidate failed.
    After(u64),
}

/// The indexes of one forward pass. Recovery reads and extends them.
pub(super) struct Pass<'p> {
    pub objects: Vec<ScannedObject>,
    /// Integer objects by reference: the first value of each.
    pub lengths: BTreeMap<PdfRef, u64>,
    /// Whether two integer objects with one reference differ.
    conflicting_lengths: bool,
    pub pending_lengths: Vec<PendingLength>,
    pub pending_prefixes: Vec<FragmentObject>,
    pub patches: Vec<LengthPatch>,
    pub damaged: Vec<(Option<PdfRef>, u64)>,
    pub candidates: &'p mut [FragmentCandidate],
    pub rows: Option<&'p [CajPageRow]>,
    /// Whether a failure may repeat the pass with another stream end; only
    /// a complete scan does, so row candidates and partial scans stay linear.
    retries: bool,
}

impl Pass<'_> {
    pub fn push_damaged(&mut self, reference: Option<PdfRef>, offset: u64) -> Result<()> {
        push_counted(
            &mut self.damaged,
            (reference, offset),
            "damaged PDF objects",
        )
    }

    fn length_of(&self, target: PdfRef) -> Option<u64> {
        self.lengths.get(&target).copied()
    }
}

/// An object framed at its start.
struct Framed {
    object: ScannedObject,
    end: u64,
    integer: Option<u64>,
    pending: Option<PendingLength>,
}

/// A failed pass to repeat with the stream at `.0` framed by `.1`. The
/// error stands if no repeated pass succeeds.
struct Retry(u64, Extent, Error);

/// How one pass ended.
enum Outcome {
    /// Every object up to this offset was indexed or recovered.
    Done {
        end: u64,
        final_repaired: bool,
    },
    Retry(Retry),
}

fn scan_fragment<S: RangedSource, C: Cancellation>(
    source: &mut S,
    body_start: u64,
    minimum_end: u64,
    limits: &Limits,
    cancellation: &C,
    mode: Mode<'_>,
) -> Result<FragmentScan> {
    limits.validate()?;
    if body_start >= minimum_end || minimum_end > source.size() {
        return Err(
            Error::malformed(body_start, "CAJ PDF fragment body range is invalid").in_caj(None),
        );
    }
    let minimum_relative = minimum_end - body_start;
    limits
        .check_input_size(minimum_relative)
        .map_err(|error| error.at(body_start).in_caj(None))?;
    let scan_end = minimum_end
        .saturating_add(MAX_FRAGMENT_TAIL_EXTENSION)
        .min(source.size());
    let range = PdfRange {
        offset: body_start,
        length: scan_end - body_start,
    };
    let mut reader = Reader::new(source, range, limits, cancellation)?;
    let (candidates, rows, verify) = match mode {
        Mode::Complete(candidates) => (candidates, None, true),
        Mode::Candidates => (&mut [][..], None, false),
        Mode::Damaged(rows, candidates) => (candidates, Some(rows), true),
    };
    let mut hints: Vec<(u64, Extent)> = Vec::new();
    let mut first_error = None;
    for _ in 0..=MAX_EXTENT_RETRIES {
        for candidate in candidates.iter_mut() {
            candidate.used = false;
        }
        let mut pass = Pass {
            objects: Vec::new(),
            lengths: BTreeMap::new(),
            conflicting_lengths: false,
            pending_lengths: Vec::new(),
            pending_prefixes: Vec::new(),
            patches: Vec::new(),
            damaged: Vec::new(),
            candidates: &mut *candidates,
            rows,
            retries: verify && rows.is_none(),
        };
        let Retry(start, extent, error) =
            match run_pass(&mut reader, &mut pass, minimum_relative, &hints)? {
                Outcome::Done {
                    end,
                    final_repaired,
                } => match finish(&mut reader, pass, end, final_repaired, verify)? {
                    Ok(scan) => return Ok(scan),
                    Err(retry) => retry,
                },
                Outcome::Retry(retry) => retry,
            };
        first_error.get_or_insert(error);
        match hints.iter_mut().find(|(at, _)| *at == start) {
            Some(hint) => hint.1 = extent,
            None => hints.push((start, extent)),
        }
    }
    Err(first_error.expect("every repeated pass failed"))
}

/// Index objects forward from the fragment start until the page table's body
/// end is reached, recovering what `recovery` can.
fn run_pass<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    pass: &mut Pass<'_>,
    minimum_relative: u64,
    hints: &[(u64, Extent)],
) -> Result<Outcome> {
    let mut cursor = 0_u64;
    let mut final_repaired = false;
    // `minimum_end <= source.size()` was checked, so the bounded range always
    // reaches the page table's body end; `load_head` rejects syntax past it.
    debug_assert!(minimum_relative <= reader.range.length);
    loop {
        reader.skip_space(&mut cursor)?;
        if cursor >= minimum_relative {
            return Ok(Outcome::Done {
                end: cursor,
                final_repaired,
            });
        }
        let start = cursor;
        let failure = match frame_object(reader, pass, start, hints)? {
            Ok(framed) => {
                cursor = index_object(pass, framed)?;
                final_repaired = false;
                if cursor >= minimum_relative {
                    return Ok(Outcome::Done {
                        end: cursor,
                        final_repaired,
                    });
                }
                continue;
            }
            Err(failure) => failure,
        };
        let recovery = match recovery::try_recover(reader, pass, start, &failure) {
            Ok(Some(recovery)) => recovery,
            Ok(None) => {
                return retry(reader, pass, failure.into_error()).map(Outcome::Retry);
            }
            Err(error) => return retry(reader, pass, error).map(Outcome::Retry),
        };
        match recovery {
            Recovery::Resume(resume) => cursor = resume,
            Recovery::Defer { resume, prefix } => {
                push_counted(
                    &mut pass.pending_prefixes,
                    prefix,
                    "CAJ interrupted prefixes",
                )?;
                cursor = resume;
            }
            Recovery::Repaired { end, patch } => {
                let Failure::Stream(_, stream) = failure else {
                    unreachable!("only a stream extent is repaired");
                };
                push_counted(&mut pass.patches, patch, "CAJ stream Length repairs")?;
                let object = ScannedObject {
                    object: object_at(reader, stream.reference, start, end),
                    inspection: stream.inspection,
                };
                let framed = Framed {
                    object,
                    end,
                    integer: None,
                    pending: None,
                };
                cursor = index_object(pass, framed)?;
                final_repaired = true;
                if cursor >= minimum_relative {
                    return Ok(Outcome::Done {
                        end: cursor,
                        final_repaired,
                    });
                }
            }
            Recovery::Discarded(resume) => {
                final_repaired = false;
                match resume {
                    Some(resume) => cursor = resume,
                    None => {
                        return Ok(Outcome::Done {
                            end: minimum_relative,
                            final_repaired,
                        });
                    }
                }
            }
        }
    }
}

/// Add one framed object to the pass indexes; return the object's end.
fn index_object(pass: &mut Pass<'_>, framed: Framed) -> Result<u64> {
    let reference = framed.object.object.reference;
    if let Some(value) = framed.integer {
        let count = pass.lengths.len() as u64;
        if count >= u64::from(MAX_PDF_OBJECTS) {
            return Err(Error::limit(
                "fragment Length index",
                u64::from(MAX_PDF_OBJECTS),
                count + 1,
            ));
        }
        let first = *pass.lengths.entry(reference).or_insert(value);
        pass.conflicting_lengths |= first != value;
    }
    if let Some(pending) = framed.pending {
        push_counted(&mut pass.pending_lengths, pending, "fragment Length index")?;
    }
    push_counted(&mut pass.objects, framed.object, "PDF fragment objects")?;
    Ok(framed.end)
}

/// Retry the latest stream whose searched end is not confirmed, or fail.
fn retry<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    pass: &Pass<'_>,
    error: Error,
) -> Result<Retry> {
    if !pass.retries || !is_malformed(&error) {
        return Err(error);
    }
    for pending in pass.pending_lengths.iter().rev() {
        let Some(marker) = pending.marker else {
            continue;
        };
        if confirms(reader, pending, pass.length_of(pending.target))? != Some(true) {
            return Ok(Retry(pending.start, Extent::After(marker + 1), error));
        }
    }
    Err(error)
}

/// Whether the resolved `length`, if any, frames this stream at the same end.
fn confirms<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    pending: &PendingLength,
    length: Option<u64>,
) -> Result<Option<bool>> {
    let Some(length) = length else {
        return Ok(None);
    };
    if length == pending.length {
        return Ok(Some(true));
    }
    let Some(after) = pending.data_at.checked_add(length) else {
        return Ok(Some(false));
    };
    match reader.check_stream_tail(after, None) {
        Ok(end) => Ok(Some(end == pending.end)),
        Err(error) if is_malformed(&error) => Ok(Some(false)),
        Err(error) => Err(error),
    }
}

fn object_at<S, C>(
    reader: &Reader<'_, S, C>,
    reference: PdfRef,
    start: u64,
    end: u64,
) -> FragmentObject {
    FragmentObject {
        reference,
        range: PdfRange {
            offset: reader.range.offset + start,
            length: end - start,
        },
    }
}

/// Parse and frame the object at `start`. A malformed object is returned as
/// a `Failure` for recovery; any other error ends the scan.
fn frame_object<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    pass: &Pass<'_>,
    start: u64,
    hints: &[(u64, Extent)],
) -> Result<std::result::Result<Framed, Failure>> {
    let head = match reader.load_head(start, None) {
        Ok(head) => head,
        Err(error) if is_malformed(&error) => return Ok(Err(Failure::Syntax(error))),
        Err(error) => return Err(error),
    };
    let reference = head.reference;
    if reference.generation != 0 {
        return Err(reader.problem(
            start,
            Some(reference),
            ErrorKind::UnsupportedFormat,
            "CAJ PDF fragment has a nonzero object generation",
        ));
    }
    let inspection = inspect_head(&head, reader.range, start, reader.limits);
    let other = |reader: &Reader<'_, S, C>, at, reason| {
        Ok(Err(Failure::Other(reader.malformed(
            at,
            Some(reference),
            reason,
        ))))
    };
    let data_start = match head.tail {
        // `load_head` reads only within the bounded range.
        ObjectTail::EndObject { end } => {
            let end = start + end as u64;
            let integer = head
                .scalar
                .as_ref()
                .and_then(|span| exact_unsigned(&head.bytes[span.clone()]));
            return Ok(Ok(Framed {
                object: ScannedObject {
                    object: object_at(reader, reference, start, end),
                    inspection,
                },
                end,
                integer,
                pending: None,
            }));
        }
        ObjectTail::Stream { data_start } => data_start as u64,
    };
    let Some(dictionary) = head.dictionary.as_ref() else {
        return other(reader, start, "stream has no dictionary");
    };
    let Some(entry) = dictionary.entry(b"Length") else {
        return other(reader, start, "stream lacks Length");
    };
    let value = entry.value(&dictionary.bytes);
    let data_at = start + data_start;
    let hint = hints
        .iter()
        .find_map(|&(at, extent)| (at == start).then_some(extent));
    let (length, target, direct) = if let Some(length) = exact_unsigned(value) {
        let Some(dictionary_start) = head.dictionary_start else {
            return other(reader, start, "stream dictionary offset is missing");
        };
        let offset = reader.range.offset + start + (dictionary_start + entry.value.start) as u64;
        (Some(length), None, Some((offset, value.to_vec())))
    } else if let Some(target) = exact_reference(value) {
        let length = match hint {
            Some(Extent::Length(length)) => Some(length),
            Some(Extent::After(_)) => None,
            None => pass.length_of(target),
        };
        (length, Some(target), None)
    } else {
        return other(
            reader,
            start,
            "stream Length is neither an integer nor a reference",
        );
    };
    let stream_failure = |error, length, inspection| {
        Ok(Err(Failure::Stream(
            error,
            Box::new(StreamFailure {
                reference,
                data_start,
                length,
                direct,
                inspection,
            }),
        )))
    };
    let (length, end, marker) = if let Some(length) = length {
        let Some(after) = data_at.checked_add(length) else {
            return other(reader, data_at, "stream extent overflows");
        };
        match reader.check_stream_tail(after, Some(reference)) {
            Ok(end) => (length, end, None),
            Err(error) if is_malformed(&error) => {
                return stream_failure(error, Some(length), inspection);
            }
            Err(error) => return Err(error),
        }
    } else {
        let from = match hint {
            Some(Extent::After(at)) => at,
            _ => data_at,
        };
        match next_stream_end(reader, data_at, from, reference)? {
            Some((data_end, end, marker)) => (data_end - data_at, end, Some(marker)),
            None => {
                let error = reader.malformed(data_at, Some(reference), "stream has no endstream");
                return stream_failure(error, None, inspection);
            }
        }
    };
    let pending = target.map(|target| PendingLength {
        target,
        length,
        start,
        data_at,
        end,
        marker,
    });
    Ok(Ok(Framed {
        object: ScannedObject {
            object: object_at(reader, reference, start, end),
            inspection,
        },
        end,
        integer: None,
        pending,
    }))
}

/// The first `endstream` at or after `from` with a complete tail: the data
/// end it implies, the object end, and the keyword offset.
fn next_stream_end<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    data_at: u64,
    from: u64,
    reference: PdfRef,
) -> Result<Option<(u64, u64, u64)>> {
    let mut from = from;
    while let Some(marker) = find_endstream(reader, from, reader.range.length)? {
        from = marker + 1;
        let data_end = recovery::data_end_before(reader, marker)?;
        if data_end < data_at {
            continue;
        }
        match reader.check_stream_tail(data_end, Some(reference)) {
            Ok(end) => return Ok(Some((data_end, end, marker))),
            Err(error) if is_malformed(&error) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(None)
}

/// Check the complete pass: candidates, replays, deferred prefixes and
/// indirect lengths. Returns a scan, or a retry with a resolved length.
fn finish<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    mut pass: Pass<'_>,
    logical_end: u64,
    final_repaired: bool,
    verify: bool,
) -> Result<std::result::Result<FragmentScan, Retry>> {
    let body_start = reader.range.offset;
    if pass.objects.is_empty() {
        return Err(reader.malformed(0, None, "CAJ PDF fragment has no indirect objects"));
    }
    if final_repaired {
        // A repaired final stream could otherwise stop at a fake terminator
        // in its own data. Accept it only as the sole candidate in the scan.
        let mut after = logical_end;
        reader.skip_space(&mut after)?;
        if let Some(marker) = find_endstream(reader, after, reader.range.length)?
            && reader.check_stream_tail(marker, None).is_ok()
        {
            return Err(reader
                .problem(
                    marker,
                    None,
                    ErrorKind::Malformed,
                    "repaired final stream has a later stream terminator",
                )
                .ambiguous_repair());
        }
    }
    reader
        .limits
        .check_input_size(logical_end)
        .map_err(reader.locator(logical_end, None))?;
    if pass.conflicting_lengths {
        let error = reader.malformed(0, None, "conflicting fragment integer objects");
        return retry(reader, &pass, error).map(Err);
    }
    // A candidate reached through a container anchor may actually be inside a
    // stream. Only the complete forward parse establishes its object boundary.
    for candidate in pass.candidates.iter().filter(|candidate| candidate.used) {
        let object = candidate.object;
        let confirmed = pass
            .objects
            .binary_search_by_key(&object.range.offset, |actual| actual.object.range.offset)
            .is_ok_and(|index| pass.objects[index].object == object);
        if !confirmed {
            return Err(reader
                .problem(
                    object.range.offset.saturating_sub(body_start),
                    Some(object.reference),
                    ErrorKind::Malformed,
                    "recovery candidate is not a complete fragment object",
                )
                .ambiguous_repair());
        }
    }
    compact_replays(reader, &mut pass.objects)?;
    if verify {
        for prefix in std::mem::take(&mut pass.pending_prefixes) {
            match proves_prefix(reader, &pass.objects, prefix)? {
                Some(true) => {}
                None if pass.rows.is_some() => {
                    pass.push_damaged(Some(prefix.reference), prefix.range.offset)?;
                }
                _ => {
                    return Err(reader.malformed(
                        prefix.range.offset - body_start,
                        Some(prefix.reference),
                        "interrupted prefix has no exact complete counterpart",
                    ));
                }
            }
        }
    }
    pass.objects
        .sort_unstable_by_key(|scanned| scanned.object.range.offset);
    if verify {
        for pending in std::mem::take(&mut pass.pending_lengths) {
            let resolved = pass.length_of(pending.target);
            if confirms(reader, &pending, resolved)? == Some(true) {
                continue;
            }
            if pass.rows.is_some() {
                let offset = pass
                    .objects
                    .iter()
                    .find(|scanned| scanned.object.reference == pending.target)
                    .map_or(body_start, |scanned| scanned.object.range.offset);
                pass.push_damaged(Some(pending.target), offset)?;
                pass.objects
                    .retain(|scanned| scanned.object.reference != pending.target);
                continue;
            }
            let error = reader.malformed(
                0,
                Some(pending.target),
                "indirect stream Length does not match its integer object",
            );
            if let (Some(length), Some(_)) = (resolved, pending.marker) {
                return Ok(Err(Retry(pending.start, Extent::Length(length), error)));
            }
            return Err(error);
        }
    }
    Ok(Ok(FragmentScan {
        objects: pass.objects,
        patches: pass.patches,
        damaged: pass.damaged,
    }))
}

/// Whether `prefix` is an exact proper prefix of the indexed object with the
/// same number, or `None` when there is no such object. Exact replays were
/// already compacted.
fn proves_prefix<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    objects: &[ScannedObject],
    prefix: FragmentObject,
) -> Result<Option<bool>> {
    let body_start = reader.range.offset;
    let Ok(index) =
        objects.binary_search_by_key(&prefix.reference, |scanned| scanned.object.reference)
    else {
        return Ok(None);
    };
    let original = objects[index].object;
    if prefix.range.length >= original.range.length {
        return Ok(Some(false));
    }
    let length = prefix.range.length as usize;
    let partial = reader.bytes(prefix.range.offset - body_start, length)?;
    let complete = reader.bytes(original.range.offset - body_start, length)?;
    Ok(Some(partial == complete))
}

/// Keep the first of identical object replays, comparing them in bounded
/// chunks; differing replays are ambiguous. Leaves objects sorted by number.
fn compact_replays<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    objects: &mut Vec<ScannedObject>,
) -> Result<()> {
    let body_start = reader.range.offset;
    objects.sort_unstable_by_key(|scanned| (scanned.object.reference, scanned.object.range.offset));
    let mut kept = 0;
    for index in 0..objects.len() {
        let object = objects[index].object;
        if kept > 0 && objects[kept - 1].object.reference == object.reference {
            let prior = objects[kept - 1].object;
            let mut equal = prior.range.length == object.range.length;
            let mut compared = 0;
            while equal && compared < object.range.length {
                let amount = (object.range.length - compared)
                    .min(4096)
                    .min(reader.limits.io_chunk_bytes as u64) as usize;
                let original = reader.bytes(prior.range.offset - body_start + compared, amount)?;
                let replay = reader.bytes(object.range.offset - body_start + compared, amount)?;
                equal = original == replay;
                compared += amount as u64;
            }
            if !equal {
                return Err(reader
                    .problem(
                        object.range.offset - body_start,
                        Some(object.reference),
                        ErrorKind::Malformed,
                        "duplicate indirect object differs from original",
                    )
                    .ambiguous_repair());
            }
        } else {
            objects.swap(kept, index);
            kept += 1;
        }
    }
    objects.truncate(kept);
    Ok(())
}

/// Push one entry of an index bounded by the PDF object-number limit.
fn push_counted<T>(items: &mut Vec<T>, item: T, resource: &'static str) -> Result<()> {
    push_capped(items, item, u64::from(MAX_PDF_OBJECTS), resource)
}

/// Push one entry of an index of at most `cap` entries.
pub(super) fn push_capped<T>(
    items: &mut Vec<T>,
    item: T,
    cap: u64,
    resource: &'static str,
) -> Result<()> {
    let attempted = items.len() as u64 + 1;
    let limit = Error::limit(resource, cap, attempted);
    if attempted > cap {
        return Err(limit);
    }
    reserve(items, 1, limit)?;
    items.push(item);
    Ok(())
}

fn is_malformed(error: &Error) -> bool {
    error.is_malformed_pdf()
}

#[cfg(test)]
mod damaged_tests;
#[cfg(test)]
mod tests;
