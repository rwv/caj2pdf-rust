// SPDX-License-Identifier: MIT

//! Damaged-input recovery for the headerless CAJ fragment scanner.
//!
//! The scanner calls [`try_recover`] at one point, after an object fails to
//! parse or its stream extent is not confirmed. Each lossless rule either
//! resumes at an independently derived boundary or defers interrupted bytes
//! until the complete scan proves them an exact prefix of a complete copy.
//! Explicit partial conversion (`--allow-damaged`) then discards what no
//! rule recovers and blanks the pages that depend on it.

use super::fragment_scan::{Failure, FragmentCandidate, Pass, ScannedObject, StreamFailure};
use super::{ObjectTail, Reader, exact_name, exact_reference, exact_unsigned, media_box};
use crate::caj::{CajMetadata, CajPageRow};
use crate::fallible::{checked_read_count, reserve};
use crate::pdf::{FragmentObject, PdfRange, PdfRef};
use crate::{Cancellation, Error, Limits, OmittedPage, PdfErrorKind, RangedSource, Result};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// The latest start, after an interrupted stream, of its complete replay.
const MAX_REPLAY_DISTANCE: u64 = 64 * 1024;
/// How far past an understated direct `/Length` a terminator may be found.
const MAX_STREAM_LENGTH_REPAIR: u64 = 64;

/// What the scanner does after a recovered failure.
pub(super) enum Recovery {
    /// Continue scanning at this offset within the fragment.
    Resume(u64),
    /// Continue at `resume` once the complete scan proves `prefix` an exact
    /// proper prefix of the indexed object with the same number.
    Defer { resume: u64, prefix: FragmentObject },
    /// The stream ends at `end` after a same-width `/Length` correction.
    Repaired { end: u64, patch: LengthPatch },
    /// Partial mode recorded damage; continue at this offset, or stop.
    Discarded(Option<u64>),
}

/// Recover from `failure` at `start`, or return `None` to report its error.
/// An error raised while recovering is reported instead.
pub(super) fn try_recover<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    pass: &mut Pass<'_>,
    start: u64,
    failure: &Failure,
) -> Result<Option<Recovery>> {
    let attempt = match failure {
        Failure::Syntax(error) => syntax_recovery(reader, pass, start, error),
        Failure::Stream(_, stream) => stream_recovery(reader, pass, start, stream),
        Failure::Other(_) => Ok(None),
    };
    let error = match attempt {
        Ok(Some(recovery)) => return Ok(Some(recovery)),
        Ok(None) => None,
        Err(error) => Some(error),
    };
    let Some(rows) = pass.rows else {
        return error.map_or(Ok(None), Err);
    };
    let reported = error.as_ref().unwrap_or(failure.error());
    let Error::Pdf {
        offset,
        object,
        kind: PdfErrorKind::Malformed,
        ..
    } = *reported
    else {
        return error.map_or(Ok(None), Err);
    };
    salvage(reader, pass, rows, start, (object, offset)).map(Some)
}

/// Lossless rules for an object whose header or value does not parse.
fn syntax_recovery<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    pass: &mut Pass<'_>,
    start: u64,
    error: &Error,
) -> Result<Option<Recovery>> {
    if let Some(end) = replay_end(reader, start, &pass.objects, &pass.lengths)? {
        return Ok(Some(Recovery::Resume(end)));
    }
    let expected = pass
        .pending_lengths
        .last()
        .map(|pending| (pending.target, pending.length));
    if let Some(end) = orphan_length_end(reader, start, expected)? {
        return Ok(Some(Recovery::Resume(end)));
    }
    if let Some(end) = known_prefix_end(reader, start, error, &pass.objects)? {
        return Ok(Some(Recovery::Resume(end)));
    }
    if let Some(end) = adjacent_header_end(reader, start, error, &pass.objects)? {
        return Ok(Some(Recovery::Resume(end)));
    }
    if let Some(end) = candidate_prefix_end(reader, start, pass.candidates)? {
        return Ok(Some(Recovery::Resume(end)));
    }
    let Some((resume, prefix)) = interrupted_syntax_prefix(reader, start, error)? else {
        return Ok(None);
    };
    // A prefix of an object indexed or offered twice has no unique original.
    let named = |reference: PdfRef| reference == prefix.reference;
    let candidates = pass.candidates.iter().map(|c| c.object.reference);
    let objects = pass.objects.iter().map(|o| o.object.reference);
    if candidates.filter(|r| named(*r)).count() > 1 || objects.filter(|r| named(*r)).count() > 1 {
        return Ok(None);
    }
    Ok(Some(Recovery::Defer { resume, prefix }))
}

/// Lossless rules for a parsed stream whose extent is not confirmed.
fn stream_recovery<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    pass: &mut Pass<'_>,
    start: u64,
    stream: &StreamFailure,
) -> Result<Option<Recovery>> {
    let Some((patch_offset, original)) = &stream.direct else {
        if let Some(end) = replay_end(reader, start, &pass.objects, &pass.lengths)? {
            return Ok(Some(Recovery::Resume(end)));
        }
        if let Some(end) = candidate_prefix_end(reader, start, pass.candidates)? {
            return Ok(Some(Recovery::Resume(end)));
        }
        return Ok(match stream_replay(reader, start, stream)? {
            Replay::Found(recovery) => Some(recovery),
            Replay::Absent | Replay::Unproven => None,
        });
    };
    match stream_replay(reader, start, stream)? {
        Replay::Found(recovery) => return Ok(Some(recovery)),
        // A replayed header is no understated Length: never repair it.
        Replay::Unproven => {
            let end = candidate_prefix_end(reader, start, pass.candidates)?;
            return Ok(end.map(Recovery::Resume));
        }
        Replay::Absent => {}
    }
    let length = stream.length.expect("a direct Length is always known");
    let data_at = start + stream.data_start;
    let (corrected, end) =
        match repair_stream_length(reader, data_at + length, data_at, stream.reference) {
            Ok(repair) => repair,
            Err(
                error @ Error::Pdf {
                    kind: PdfErrorKind::Malformed,
                    ..
                },
            ) => {
                if let Some(end) = candidate_prefix_end(reader, start, pass.candidates)? {
                    return Ok(Some(Recovery::Resume(end)));
                }
                return Err(error);
            }
            Err(error) => return Err(error),
        };
    let replacement = corrected.to_string().into_bytes();
    if original.len() != replacement.len() {
        return Err(reader.problem(
            data_at + length,
            Some(stream.reference),
            PdfErrorKind::UnsupportedFeature,
            "stream Length repair changes PDF object width",
        ));
    }
    Ok(Some(Recovery::Repaired {
        end,
        patch: LengthPatch {
            offset: *patch_offset,
            original: original.clone(),
            replacement,
        },
    }))
}

/// An interrupted stream followed, possibly after other complete objects, by
/// its complete replay. A later `endstream`, less at most two end-of-line
/// bytes and the declared or resolved `/Length`, fixes where the replay
/// starts; it must repeat this stream's header. The replay is framed when the
/// scan reaches it, and the interrupted bytes are deferred until the complete
/// scan proves them an exact prefix of the indexed replay. No codec is decoded.
fn stream_replay<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    start: u64,
    stream: &StreamFailure,
) -> Result<Replay> {
    let Some(length) = stream.length else {
        return Ok(Replay::Absent);
    };
    let header = stream.data_start;
    let Some(first) = start
        .checked_add(1)
        .and_then(|value| value.checked_add(header))
        .and_then(|value| value.checked_add(length))
    else {
        return Ok(Replay::Absent);
    };
    let last = first.saturating_add(MAX_REPLAY_DISTANCE).saturating_add(2);
    let original = reader.bytes(start, header as usize)?;
    let mut from = first;
    while let Some(marker) = find_endstream(reader, from, last.saturating_add(9))? {
        from = marker + 1;
        for eol in [2, 1, 0] {
            let Some(copy) = marker
                .checked_sub(eol)
                .and_then(|data_end| data_end.checked_sub(length))
                .and_then(|data_at| data_at.checked_sub(header))
                .filter(|copy| *copy > start && *copy - start <= MAX_REPLAY_DISTANCE)
            else {
                continue;
            };
            if reader.bytes(copy, header as usize)? == original {
                return Ok(match deferred_replay(reader, start, copy, stream)? {
                    Some(recovery) => Replay::Found(recovery),
                    None => Replay::Unproven,
                });
            }
        }
    }
    Ok(Replay::Absent)
}

/// Whether a stream's complete replay follows it.
enum Replay {
    Absent,
    /// A replay follows, but the interrupted bytes are not its exact prefix
    /// with at least one payload byte.
    Unproven,
    Found(Recovery),
}

/// Defer the bytes that `start` shares with the replay at `copy`, resuming at
/// the next object after them. They must include at least one payload byte.
fn deferred_replay<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    start: u64,
    copy: u64,
    stream: &StreamFailure,
) -> Result<Option<Recovery>> {
    let span = copy - start;
    let interrupted = reader.bytes(start, span as usize)?;
    let available = span.min(reader.range.length - copy);
    let replay = reader.bytes(copy, available as usize)?;
    let shared = interrupted
        .iter()
        .zip(&replay)
        .take_while(|(left, right)| left == right)
        .count();
    let prefix = interrupted[..shared].trim_ascii_end().len();
    let mut resume = shared;
    while interrupted.get(resume).is_some_and(u8::is_ascii_whitespace) {
        resume += 1;
    }
    if prefix as u64 <= stream.data_start
        || !(resume == interrupted.len() || interrupted[resume].is_ascii_digit())
    {
        return Ok(None);
    }
    Ok(Some(Recovery::Defer {
        resume: start + resume as u64,
        prefix: FragmentObject {
            reference: stream.reference,
            range: PdfRange {
                offset: reader.range.offset + start,
                length: prefix as u64,
            },
        },
    }))
}

/// The next `endstream` keyword starting at or after `from` and ending by
/// `limit`, both within the reader's range.
pub(super) fn find_endstream<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    from: u64,
    limit: u64,
) -> Result<Option<u64>> {
    const KEYWORD: &[u8] = b"endstream";
    let limit = limit.min(reader.range.length);
    let mut at = from;
    while at.saturating_add(KEYWORD.len() as u64) <= limit {
        // Search the reader's window in place; only a keyword that crosses
        // the window end is compared byte by byte.
        reader.byte(at)?;
        let first = (at - reader.window_offset) as usize;
        let window = &reader.window[first..reader.window_len];
        if window.len() >= KEYWORD.len() {
            if let Some(index) = window
                .windows(KEYWORD.len())
                .position(|bytes| bytes == KEYWORD)
            {
                let found = at + index as u64;
                return Ok((found + KEYWORD.len() as u64 <= limit).then_some(found));
            }
            at += (window.len() - (KEYWORD.len() - 1)) as u64;
            continue;
        }
        let mut matched = true;
        for (index, &expected) in KEYWORD.iter().enumerate() {
            if reader.byte(at + index as u64)? != Some(expected) {
                matched = false;
                break;
            }
        }
        if matched {
            return Ok(Some(at));
        }
        at += 1;
    }
    Ok(None)
}

/// The data end that a `endstream` at `marker` implies, excluding one EOL.
pub(super) fn data_end_before<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    marker: u64,
) -> Result<u64> {
    Ok(
        if marker >= 2
            && reader.byte(marker - 2)? == Some(b'\r')
            && reader.byte(marker - 1)? == Some(b'\n')
        {
            marker - 2
        } else if marker >= 1 && matches!(reader.byte(marker - 1)?, Some(b'\r' | b'\n')) {
            marker - 1
        } else {
            marker
        },
    )
}

/// Find the one complete terminator shortly after an understated direct
/// `/Length`. Two candidates are an ambiguous repair.
fn repair_stream_length<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    declared_after: u64,
    data_at: u64,
    reference: PdfRef,
) -> Result<(u64, u64)> {
    let last = declared_after
        .saturating_add(MAX_STREAM_LENGTH_REPAIR)
        .min(reader.range.length.saturating_sub(9));
    let mut found = None;
    let mut from = declared_after.saturating_add(1);
    while let Some(marker) = find_endstream(reader, from, last.saturating_add(9))? {
        from = marker + 1;
        let after = data_end_before(reader, marker)?;
        if after <= declared_after || after < data_at {
            continue;
        }
        let tail = reader.check_stream_tail(after, Some(reference));
        if matches!(
            tail,
            Err(Error::Pdf {
                kind: PdfErrorKind::Malformed,
                ..
            })
        ) {
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

/// Partial mode: record the damage, then resume after an independently
/// framed stream or at the page table's next page dictionary.
fn salvage<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    pass: &mut Pass<'_>,
    rows: &[CajPageRow],
    start: u64,
    (object, offset): (Option<(u32, u16)>, u64),
) -> Result<Recovery> {
    let reference = object.map(|(number, generation)| PdfRef { number, generation });
    pass.push_damaged(reference, offset)?;
    if let Some(end) = damaged_stream_end(reader, start, &pass.lengths)? {
        return Ok(Recovery::Discarded(Some(end)));
    }
    let body_start = reader.range.offset;
    let next = rows
        .iter()
        .find(|row| row.offset > body_start + start && row.length != 0);
    Ok(Recovery::Discarded(match next {
        Some(row) => Some(damaged_page_anchor(reader, row)?),
        None => None,
    }))
}

/// A codec failure need not discard later objects: a parsed Length and exact
/// terminator can still establish where the discarded stream ends.
pub(super) fn damaged_stream_end<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    start: u64,
    lengths: &BTreeMap<PdfRef, u64>,
) -> Result<Option<u64>> {
    let attempt = (|| {
        let head = reader.load_head(start, None)?;
        let ObjectTail::Stream { data_start } = head.tail else {
            return Ok(None);
        };
        let Some(dict) = head.dictionary else {
            return Ok(None);
        };
        let Some(value) = dict.value(b"Length") else {
            return Ok(None);
        };
        let length = exact_unsigned(value)
            .or_else(|| exact_reference(value).and_then(|target| lengths.get(&target).copied()));
        let Some(length) = length else {
            return Ok(None);
        };
        let Some(end) = start
            .checked_add(data_start as u64)
            .and_then(|at| at.checked_add(length))
        else {
            return Ok(None);
        };
        match reader.check_stream_tail(end, Some(head.reference)) {
            Ok(end) => Ok(Some(end)),
            Err(Error::Pdf {
                kind: PdfErrorKind::Malformed,
                ..
            }) => repair_stream_length(reader, end, start + data_start as u64, head.reference)
                .map(|(_, end)| Some(end)),
            Err(error) => Err(error),
        }
    })();
    match attempt {
        Err(Error::Pdf {
            kind: PdfErrorKind::Malformed,
            ..
        }) => Ok(None),
        result => result,
    }
}

/// Page table boundaries may precede the page dictionary by a short tail of
/// the previous object. Admit only the table's exact page ID, with one complete
/// Page dictionary in this bounded prefix. This is used solely after explicitly
/// dropping damaged content, never as evidence for lossless repair.
pub(super) fn damaged_page_anchor<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    row: &CajPageRow,
) -> Result<u64> {
    let relative = row.offset - reader.range.offset;
    let header = format!("{} 0 obj", row.page_object_id);
    let bytes = reader.bytes(relative, row.length.min(64 + header.len() as u64) as usize)?;
    let mut found = None;
    for (index, window) in bytes.windows(header.len()).enumerate() {
        if window != header.as_bytes()
            || index > 64
            || (index != 0 && !matches!(bytes[index - 1], 0 | b'\t' | b'\n' | 12 | b'\r' | b' '))
        {
            continue;
        }
        let expected = PdfRef {
            number: row.page_object_id,
            generation: 0,
        };
        let head = match reader.load_head(relative + index as u64, Some(expected)) {
            Ok(head) => head,
            Err(Error::Pdf {
                kind: PdfErrorKind::Malformed,
                ..
            }) => continue,
            Err(error) => return Err(error),
        };
        if matches!(head.tail, ObjectTail::EndObject { .. })
            && head
                .dictionary
                .as_ref()
                .and_then(|dict| dict.value(b"Type"))
                .and_then(exact_name)
                .as_deref()
                == Some(b"Page")
            && found.replace(relative + index as u64).is_some()
        {
            return Err(reader.malformed(
                relative,
                Some(expected),
                "ambiguous damaged page boundary",
            ));
        }
    }
    found.ok_or_else(|| reader.malformed(relative, None, "damaged page boundary is unavailable"))
}

/// Defer a short syntax interruption until the complete scan can prove its
/// exact counterpart. The parser supplies the sole boundary; no marker search.
pub(super) fn interrupted_syntax_prefix<S: RangedSource, C: Cancellation>(
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
    if header_error && reader.byte(end)? == Some(b'o') {
        end += 1;
        if reader.byte(end)? == Some(b'b') {
            end += 1;
        }
        if !skip_bounded_space(reader, start, &mut end, 64)? {
            return Ok(None);
        }
    }
    // After a complete value, accept only a proper prefix of the two legal
    // tail keywords. The later full scan must still prove the entire prefix.
    if *reason == "PDF object lacks endobj or stream" {
        let keyword: &[u8] = match reader.byte(end)? {
            Some(b's') => b"stream",
            Some(b'e') => b"endobj",
            _ => b"",
        };
        if !keyword.is_empty() {
            let begin = end;
            for &byte in keyword {
                if reader.byte(end)? != Some(byte) {
                    break;
                }
                end += 1;
            }
            if end - begin == keyword.len() as u64
                || !reader.byte(end)?.is_some_and(|b| b.is_ascii_whitespace())
                || !skip_bounded_space(reader, start, &mut end, 256)?
            {
                return Ok(None);
            }
        }
    }
    // A cut before the R in an indirect reference leaves generation zero
    // where the dictionary parser expects its next key. The complete-copy
    // proof below must confirm this byte as part of the original value.
    if *reason == "expected PDF name" && reader.byte(end)? == Some(b'0') {
        end += 1;
        if !reader
            .byte(end)?
            .is_some_and(|byte| byte.is_ascii_whitespace())
            || !skip_bounded_space(reader, start, &mut end, 256)?
        {
            return Ok(None);
        }
    }
    // A dictionary cut between the two closing brackets reports its first
    // bracket as the invalid name. Retain that byte in the exact prefix proof.
    if reader.byte(end)? == Some(b'>') {
        end += 1;
        if !skip_bounded_space(reader, start, &mut end, 256)? {
            return Ok(None);
        }
    }
    let bytes = reader.bytes(start, (end - start) as usize)?;
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
        match reader.load_head(start + boundary as u64, None) {
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

/// Skip whitespace at `end`, failing once it passes `budget` bytes after
/// `start`.
fn skip_bounded_space<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    start: u64,
    end: &mut u64,
    budget: u64,
) -> Result<bool> {
    while *end - start <= budget
        && reader
            .byte(*end)?
            .is_some_and(|byte| byte.is_ascii_whitespace())
    {
        *end += 1;
    }
    Ok(*end - start <= budget)
}

/// Compare an interrupted object with a uniquely indexed later copy. A
/// mismatch defines the only possible boundary; never search a payload for
/// markers. The caller subsequently proves the copy is reached by the full scan.
pub(super) fn candidate_prefix_end<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    start: u64,
    candidates: &mut [FragmentCandidate],
) -> Result<Option<u64>> {
    if candidates.is_empty() {
        return Ok(None);
    }
    let bytes = reader.bytes(start, 256.min(reader.range.length - start) as usize)?;
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
    let original_bytes = reader.bytes(
        relative,
        original.range.length.min(bytes.len() as u64) as usize,
    )?;
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
    match reader.load_head(start + boundary as u64, None) {
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

/// The one indexed object numbered `reference`, if it is unique.
fn unique(objects: &[ScannedObject], matches: impl Fn(PdfRef) -> bool) -> Option<FragmentObject> {
    let mut found = objects
        .iter()
        .map(|scanned| scanned.object)
        .filter(|object| matches(object.reference));
    let first = found.next()?;
    found.next().is_none().then_some(first)
}

/// Admit only an unfinished `number 0` header immediately followed by its
/// complete same-reference object, or exactly repeating an already indexed
/// header. No object body is discarded or searched.
fn adjacent_header_end<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    start: u64,
    error: &Error,
    objects: &[ScannedObject],
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
    let bytes = reader.bytes(start, (end - start) as usize)?;
    let prefix = bytes.trim_ascii_end();
    let mut fields = prefix
        .split(u8::is_ascii_whitespace)
        .filter(|field| !field.is_empty());
    let number = fields.next().and_then(exact_unsigned);
    if fields.next() != Some(b"0".as_slice()) || fields.next().is_some() {
        return Ok(None);
    }
    let head = match reader.load_head(end, None) {
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
    let Some(original) = unique(objects, |reference| {
        number == Some(u64::from(reference.number)) && reference.generation == 0
    }) else {
        return Ok(None);
    };
    let original_prefix =
        reader.bytes(original.range.offset - reader.range.offset, prefix.len())?;
    Ok((original_prefix == prefix).then_some(end))
}

/// Syntax errors can identify an interrupted duplicate dictionary or integer.
/// Only discard a bounded exact prefix of one already validated object,
/// including stream dictionaries before their payload. The exact shared prefix
/// and trailing whitespace define the boundary without a marker search. The
/// main loop must then parse a complete next object and validate all links.
fn known_prefix_end<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    start: u64,
    error: &Error,
    objects: &[ScannedObject],
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
    let bytes = reader.bytes(start, amount)?;
    let Some((reference, _)) = replay_prefix(&bytes) else {
        return Ok(None);
    };
    let Some(original) = unique(objects, |item| item == reference) else {
        return Ok(None);
    };
    let original_start = original.range.offset - reader.range.offset;
    let original_head = reader.load_head(original_start, Some(reference))?;
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
fn orphan_length_end<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    start: u64,
    expected: Option<(PdfRef, u64)>,
) -> Result<Option<u64>> {
    let Some((reference, length)) = expected else {
        return Ok(None);
    };
    let amount = 256.min(reader.range.length - start) as usize;
    let bytes = reader.bytes(start, amount)?;
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
pub(super) fn replay_end<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    start: u64,
    objects: &[ScannedObject],
    lengths: &BTreeMap<PdfRef, u64>,
) -> Result<Option<u64>> {
    let amount = 256.min(reader.range.length - start) as usize;
    let bytes = reader.bytes(start, amount)?;
    let Some((reference, header_end)) = replay_prefix(&bytes) else {
        return Ok(None);
    };
    let Some(original) = unique(objects, |item| item == reference) else {
        return Ok(None);
    };
    let original_bytes = reader.bytes(
        original.range.offset - reader.range.offset,
        original.range.length.min(amount as u64) as usize,
    )?;
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
        if lengths.get(&head.reference) != Some(&value) {
            continue;
        }
        let Some(scalar) = unique(objects, |item| item == head.reference) else {
            continue;
        };
        if scalar.range.length != end as u64 {
            continue;
        }
        let scalar_bytes = reader.bytes(scalar.range.offset - reader.range.offset, end)?;
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

/// The generation-zero reference named by a leading object number.
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

/// An equal-width correction to an observed, understated direct `/Length`.
/// The PDF bytes remain at their original offsets when the patch is applied.
#[derive(Clone, Debug)]
pub(crate) struct LengthPatch {
    pub(super) offset: u64,
    pub(super) original: Vec<u8>,
    pub(super) replacement: Vec<u8>,
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

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
        let count = self.source.read_at(offset, destination)?;
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

/// Preserve page geometry and parentage while removing all rendering
/// dependencies. Only valid page boxes, rotation and user unit are kept.
pub(super) fn blank_fragment_page<S: RangedSource, C: Cancellation>(
    source: &mut S,
    object: FragmentObject,
    limits: &Limits,
    cancellation: &C,
) -> Result<Vec<u8>> {
    let mut reader = Reader::new(source, object.range, limits, cancellation)?;
    let head = reader.load_head(0, Some(object.reference))?;
    let failure = || {
        reader.malformed(
            0,
            Some(object.reference),
            "damaged page geometry is unavailable",
        )
    };
    let dict = head.dictionary.as_ref().ok_or_else(failure)?;
    if dict.value(b"Type").and_then(exact_name).as_deref() != Some(b"Page") {
        return Err(failure());
    }
    let parent = dict
        .value(b"Parent")
        .and_then(exact_reference)
        .ok_or_else(failure)?;
    let mut body = format!(
        "{} 0 obj\n<< /Type /Page /Parent {} {} R /Resources << >>",
        object.reference.number, parent.number, parent.generation
    )
    .into_bytes();
    for key in [
        b"MediaBox".as_slice(),
        b"CropBox",
        b"BleedBox",
        b"TrimBox",
        b"ArtBox",
        b"Rotate",
        b"UserUnit",
    ] {
        if let Some(value) = dict.value(key) {
            let valid = if key.ends_with(b"Box") {
                media_box(value).is_some()
            } else {
                std::str::from_utf8(value)
                    .ok()
                    .and_then(|v| v.parse::<f64>().ok())
                    .is_some_and(f64::is_finite)
            };
            if !valid {
                return Err(failure());
            }
            let size = body
                .len()
                .saturating_add(key.len())
                .saturating_add(value.len())
                .saturating_add(32);
            limits.check_allocation(size as u64)?;
            let refused = limits.allocation_refused("blank page dictionary", size as u64);
            let additional = size - body.len();
            reserve(&mut body, additional, refused)?;
            body.extend_from_slice(b" /");
            body.extend_from_slice(key);
            body.push(b' ');
            body.extend_from_slice(value);
        }
    }
    body.extend_from_slice(b" >>\nendobj\n");
    Ok(body)
}

/// Explicit partial output: blank every page whose objects or transitive
/// dependencies were damaged or are missing, keeping its geometry, and drop
/// the damaged objects. Returns the appended page bodies, which start at
/// `source.size()`, and the omitted pages in table order.
pub(crate) fn substitute_damaged_pages<S: RangedSource, C: Cancellation>(
    source: &mut S,
    metadata: &CajMetadata,
    scan: &mut super::FragmentScan,
    limits: &Limits,
    cancellation: &C,
) -> Result<(Vec<u8>, Vec<OmittedPage>)> {
    let pages: BTreeSet<_> = metadata
        .page_rows
        .iter()
        .map(|row| PdfRef {
            number: row.page_object_id,
            generation: 0,
        })
        .collect();
    let present: BTreeSet<_> = scan
        .objects
        .iter()
        .map(|scanned| scanned.object.reference)
        .collect();
    let mut failed = BTreeMap::<PdfRef, u64>::new();
    let mut dependents = BTreeMap::<PdfRef, Vec<PdfRef>>::new();
    for index in 0..scan.objects.len() {
        if cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let object = scan.objects[index].object;
        let inspection = match &scan.objects[index].inspection {
            Ok(inspection) => inspection,
            Err(Error::Pdf {
                offset,
                kind: PdfErrorKind::Malformed,
                ..
            }) => {
                failed.insert(object.reference, *offset);
                continue;
            }
            Err(_) => {
                let scanned = scan.objects.swap_remove(index);
                return Err(scanned.inspection.expect_err("an inspection error"));
            }
        };
        let parent = inspection.page_parent();
        for &dependency in &inspection.references {
            // Page-tree parent links and references to retained (possibly blank)
            // pages are structural, not rendering dependencies.
            if Some(dependency) == parent || pages.contains(&dependency) {
                continue;
            }
            if !present.contains(&dependency) {
                failed.insert(object.reference, object.range.offset);
            }
            dependents
                .entry(dependency)
                .or_default()
                .push(object.reference);
        }
    }
    for &(reference, offset) in &scan.damaged {
        if let Some(reference) = reference
            && !present.contains(&reference)
        {
            failed.insert(reference, offset);
        }
        if let Some(row) = metadata
            .page_rows
            .iter()
            .rev()
            .find(|row| row.offset <= offset && row.length != 0)
        {
            failed.insert(
                PdfRef {
                    number: row.page_object_id,
                    generation: 0,
                },
                offset,
            );
        }
    }
    let mut queue: VecDeque<_> = failed
        .iter()
        .map(|(&reference, &offset)| (reference, offset))
        .collect();
    while let Some((reference, offset)) = queue.pop_front() {
        if cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        // A blank page still satisfies incoming bookmark/link references.
        if pages.contains(&reference) {
            continue;
        }
        if let Some(owners) = dependents.get(&reference) {
            for &owner in owners {
                if let std::collections::btree_map::Entry::Vacant(entry) = failed.entry(owner) {
                    entry.insert(offset);
                    queue.push_back((owner, offset));
                }
            }
        }
    }
    let base = source.size();
    let mut suffix = Vec::new();
    let mut omitted = Vec::new();
    let mut patched = PatchedSource::new(source, &scan.patches);
    for (index, row) in metadata.page_rows.iter().enumerate() {
        let reference = PdfRef {
            number: row.page_object_id,
            generation: 0,
        };
        let scanned = scan
            .objects
            .iter_mut()
            .find(|scanned| scanned.object.reference == reference)
            .ok_or(Error::Caj {
                offset: row.offset,
                record: Some(index as u32 + 1),
                reason: "damaged page has no validated geometry",
            })?;
        let Some(&offset) = failed.get(&reference) else {
            continue;
        };
        let replacement = blank_fragment_page(&mut patched, scanned.object, limits, cancellation)?;
        scanned.object.range = crate::pdf::append_replacement(&mut suffix, base, &replacement)?;
        scanned.inspection = super::inspect_generated_object(&replacement, limits);
        omitted.push(OmittedPage {
            page_index: index as u32,
            offset,
        });
    }
    scan.objects.retain(|scanned| {
        pages.contains(&scanned.object.reference) || !failed.contains_key(&scanned.object.reference)
    });
    Ok((suffix, omitted))
}

#[cfg(test)]
mod tests;
