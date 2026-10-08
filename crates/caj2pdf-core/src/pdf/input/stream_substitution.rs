// SPDX-License-Identifier: MIT

//! A measured four-to-two-byte expansion inside damaged CAJ Flate streams.
//! This is never an unrestricted text replacement. Original lengths,
//! checksums and every original page-table anchor must corroborate it.

use super::fragment_scan::{FragmentScan, StreamFailure};
use super::{FragmentKind, Reader};
use crate::caj::CajMetadata;
use crate::fallible::{checked_read_count, reserve};
use crate::pdf::{FragmentObject, PdfRange};
use crate::{Cancellation, Error, ErrorKind, Limits, RangedSource, Result, read_exact_at};
use flate2::{Decompress, FlushDecompress, Status};
use sha2::{Digest, Sha256};

const EXPANDED: [u8; 4] = [0xca, 0xa7, 0xc2, 0xe4];
const ORIGINAL: [u8; 2] = [0xb5, 0xf4];
const WINDOW: usize = 4096;
const MAX_ENCODED: u64 = 256 * 1024;
const MAX_DECODED: u64 = 4 * 1024 * 1024;
pub(super) const MAX_CANDIDATES: usize = 64;

pub(crate) struct Candidate {
    object: FragmentObject,
    encoded: PdfRange,
    digest: [u8; 32],
    positions: Vec<u64>,
}

impl Candidate {
    pub(super) fn retained_bytes(&self) -> usize {
        size_of::<Self>() + self.positions.len() * size_of::<u64>()
    }
}

/// Probe only a uniquely framed, understated direct Length with one simple
/// Flate filter. All other framing and all valid codec streams stay opaque.
pub(super) fn candidate<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    start: u64,
    stream: &StreamFailure,
    corrected: u64,
    end: u64,
) -> Result<Option<Candidate>> {
    let declared = stream.length.expect("understated direct Length");
    let extra = corrected - declared;
    if !stream.simple_flate
        || stream.reference.generation != 0
        || stream.inspection.is_err()
        || !extra.is_multiple_of(2)
        || declared < 7
        || !(8..=MAX_ENCODED).contains(&corrected)
    {
        return Ok(None);
    }
    let data_at = start + stream.data_start;
    if reader.byte(data_at + corrected - 1)? != Some(b'\n') {
        return Ok(None);
    }
    let encoded = PdfRange {
        offset: reader.absolute(data_at),
        length: corrected - 1,
    };
    let mut positions = Vec::new();
    let mut hash = Sha256::new();
    let mut rolling = [0; 4];
    let mut consumed = 0_u64;
    while consumed < encoded.length {
        let amount = (encoded.length - consumed).min(WINDOW as u64) as usize;
        let bytes = reader.bytes(data_at + consumed, amount)?;
        hash.update(&bytes);
        for byte in bytes {
            rolling.rotate_left(1);
            rolling[3] = byte;
            consumed += 1;
            if consumed >= 6 && consumed <= encoded.length - 4 && rolling == EXPANDED {
                if positions.len() as u64 == extra / 2 {
                    return Ok(None);
                }
                reader
                    .limits
                    .check_allocation(((positions.len() + 1) * size_of::<u64>()) as u64)?;
                reserve(
                    &mut positions,
                    1,
                    reader
                        .limits
                        .allocation_refused("CAJ stream substitutions", extra * 4),
                )?;
                positions.push(encoded.offset + consumed - 4);
            }
        }
    }
    if positions.is_empty() || positions.len() as u64 * 2 != extra {
        return Ok(None);
    }
    let digest = hash.finalize().into();
    // Check the decoder's bounded state allocation before constructing it.
    reader.limits.check_allocation(64 * 1024)?;
    let original = inflate(reader.source, encoded, reader.limits, reader.cancellation)?;
    if !matches!(original, Codec::Invalid) {
        return Ok(None);
    }
    let sites = sites(&positions, reader.limits)?;
    let mut source =
        SubstitutedSource::new(reader.source, &sites, reader.limits, reader.cancellation)?;
    let repaired = PdfRange {
        offset: encoded.offset,
        length: declared - 1,
    };
    let decoded = inflate(&mut source, repaired, reader.limits, reader.cancellation)
        .map_err(|error| source.locate(error))?;
    if !matches!(decoded, Codec::Valid(n) if n == repaired.length) {
        return Ok(None);
    }
    let candidate = Candidate {
        object: FragmentObject {
            reference: stream.reference,
            range: PdfRange {
                offset: reader.absolute(start),
                length: end - start,
            },
        },
        encoded,
        digest,
        positions,
    };
    candidate.verify(reader.source, reader.limits, reader.cancellation)?;
    Ok(Some(candidate))
}

enum Codec {
    /// A checksum-valid prefix also prevents rewriting an otherwise valid stream.
    Valid(u64),
    Invalid,
    OutsideProfile,
}

fn inflate<S: RangedSource, C: Cancellation>(
    source: &mut S,
    range: PdfRange,
    limits: &Limits,
    cancellation: &C,
) -> Result<Codec> {
    let mut decoder = Decompress::new(true);
    let mut input = [0; WINDOW];
    let mut output = [0; WINDOW];
    let mut fetched = 0_u64;
    let mut available = 0;
    let mut used = 0;
    loop {
        if cancellation.is_cancelled() {
            return Err(Error::cancelled());
        }
        if used == available && fetched < range.length {
            available = (range.length - fetched)
                .min(WINDOW as u64)
                .min(limits.io_chunk_bytes as u64) as usize;
            read_exact_at(
                source,
                range.offset + fetched,
                &mut input[..available],
                limits,
                cancellation,
            )?;
            fetched += available as u64;
            used = 0;
        }
        let before_in = decoder.total_in();
        let before_out = decoder.total_out();
        let status =
            match decoder.decompress(&input[used..available], &mut output, FlushDecompress::None) {
                Ok(status) => status,
                Err(_) => return Ok(Codec::Invalid),
            };
        used += (decoder.total_in() - before_in) as usize;
        limits.check_allocation(decoder.total_out())?;
        if decoder.total_out() > MAX_DECODED {
            return Ok(Codec::OutsideProfile);
        }
        if status == Status::StreamEnd {
            return Ok(Codec::Valid(decoder.total_in()));
        }
        if decoder.total_in() == before_in && decoder.total_out() == before_out {
            return Ok(Codec::Invalid);
        }
    }
}

impl Candidate {
    fn verify<S: RangedSource, C: Cancellation>(
        &self,
        source: &mut S,
        limits: &Limits,
        cancellation: &C,
    ) -> Result<()> {
        let mut hash = Sha256::new();
        let mut buffer = [0; WINDOW];
        let mut at = 0;
        while at < self.encoded.length {
            let count = (self.encoded.length - at)
                .min(WINDOW as u64)
                .min(limits.io_chunk_bytes as u64) as usize;
            read_exact_at(
                source,
                self.encoded.offset + at,
                &mut buffer[..count],
                limits,
                cancellation,
            )?;
            hash.update(&buffer[..count]);
            at += count as u64;
        }
        if <[u8; 32]>::from(hash.finalize()) != self.digest {
            return Err(Error::pdf(
                ErrorKind::Malformed,
                self.encoded.offset,
                Some((self.object.reference.number, 0)),
                "source stream changed after substitution validation",
            ));
        }
        Ok(())
    }
}

pub(crate) struct Site {
    source: u64,
    logical: u64,
}

fn sites(positions: &[u64], limits: &Limits) -> Result<Vec<Site>> {
    let mut result = Vec::new();
    limits.check_allocation((positions.len() * size_of::<Site>()) as u64)?;
    reserve(
        &mut result,
        positions.len(),
        limits.allocation_refused(
            "CAJ substitution map",
            (positions.len() * size_of::<Site>()) as u64,
        ),
    )?;
    for (i, &source) in positions.iter().enumerate() {
        result.push(Site {
            source,
            logical: source - 2 * i as u64,
        });
    }
    Ok(result)
}

pub(crate) struct Plan {
    sites: Vec<Site>,
    candidates: Vec<Candidate>,
}

impl Plan {
    /// Independent container evidence is mandatory: every mapped Page must
    /// start at its original table offset, and at least one offset must move.
    pub(crate) fn from_scan(
        scan: &mut FragmentScan,
        metadata: &CajMetadata,
        limits: &Limits,
    ) -> Result<Option<Self>> {
        let candidates = std::mem::take(&mut scan.substitutions);
        if candidates.is_empty()
            || !scan.damaged.is_empty()
            || scan.objects.iter().any(|o| o.inspection.is_err())
        {
            return Ok(None);
        }
        let mut positions = Vec::new();
        for candidate in &candidates {
            if !scan.objects.iter().any(|o| o.object == candidate.object) {
                return Ok(None);
            }
            let bytes = ((positions.len() + candidate.positions.len()) * size_of::<u64>()) as u64;
            limits.check_allocation(bytes)?;
            reserve(
                &mut positions,
                candidate.positions.len(),
                limits.allocation_refused("CAJ substitution positions", bytes),
            )?;
            positions.extend_from_slice(&candidate.positions);
        }
        positions.sort_unstable();
        if positions.windows(2).any(|p| p[0] + 4 > p[1]) {
            return Ok(None);
        }
        let sites = sites(&positions, limits)?;
        let mut pages = Vec::new();
        let bytes = metadata.page_rows.len() as u64 * size_of::<(u32, u64)>() as u64;
        limits.check_allocation(bytes)?;
        reserve(
            &mut pages,
            metadata.page_rows.len(),
            limits.allocation_refused("CAJ substitution page anchors", bytes),
        )?;
        for object in &scan.objects {
            if matches!(
                object.inspection.as_ref().unwrap().kind,
                FragmentKind::Page { .. }
            ) {
                if object.object.reference.generation != 0
                    || pages.len() == metadata.page_rows.len()
                {
                    return Ok(None);
                }
                pages.push((object.object.reference.number, object.object.range.offset));
            }
        }
        if pages.len() != metadata.page_rows.len() {
            return Ok(None);
        }
        pages.sort_unstable();
        let mut moved = false;
        for row in &metadata.page_rows {
            let Ok(index) = pages.binary_search_by_key(&row.page_object_id, |p| p.0) else {
                return Ok(None);
            };
            let physical = pages[index].1;
            let count = sites.partition_point(|site| site.source + 4 <= physical);
            if physical - count as u64 * 2 != row.offset {
                return Ok(None);
            }
            moved |= physical != row.offset;
        }
        Ok(moved.then_some(Self { sites, candidates }))
    }

    pub(crate) fn source<'a, S: RangedSource, C: Cancellation>(
        &'a self,
        source: &'a mut S,
        limits: &'a Limits,
        cancellation: &'a C,
    ) -> Result<SubstitutedSource<'a, S, C>> {
        SubstitutedSource::new(source, &self.sites, limits, cancellation)
    }

    pub(crate) fn verify<S: RangedSource, C: Cancellation>(
        &self,
        source: &mut S,
        limits: &Limits,
        cancellation: &C,
    ) -> Result<()> {
        for candidate in &self.candidates {
            candidate.verify(source, limits, cancellation)?;
        }
        Ok(())
    }
}

/// A sparse ranged view. It stores positions, never stream or document data.
pub(crate) struct SubstitutedSource<'a, S, C> {
    source: &'a mut S,
    sites: &'a [Site],
    size: u64,
    original_size: u64,
    limits: &'a Limits,
    cancellation: &'a C,
}

impl<'a, S: RangedSource, C: Cancellation> SubstitutedSource<'a, S, C> {
    fn new(
        source: &'a mut S,
        sites: &'a [Site],
        limits: &'a Limits,
        cancellation: &'a C,
    ) -> Result<Self> {
        let original_size = source.size();
        let size = original_size
            .checked_sub(sites.len() as u64 * 2)
            .ok_or_else(|| Error::invalid("CAJ substitution size underflows"))?;
        Ok(Self {
            source,
            sites,
            size,
            original_size,
            limits,
            cancellation,
        })
    }

    pub(crate) fn locate(&self, mut error: Error) -> Error {
        // Source failures already carry physical offsets. Keep them opaque
        // while parsing the logical view, then restore the original error.
        if let ErrorKind::Io(inner) = error.kind {
            error.kind = match inner.downcast::<SourceFailure>() {
                Ok(SourceFailure(original)) => return original,
                Err(inner) => ErrorKind::Io(inner),
            };
        }
        if let Some(at) = error.offset {
            let index = self.sites.partition_point(|site| site.logical + 2 <= at);
            error.offset = Some(at.saturating_add(2 * index as u64));
        }
        error
    }
}

#[derive(Debug)]
struct SourceFailure(Error);

impl std::fmt::Display for SourceFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for SourceFailure {}

fn source_failure(error: Error) -> Error {
    ErrorKind::Io(std::io::Error::other(SourceFailure(error))).into()
}

impl<S: RangedSource, C: Cancellation> RangedSource for SubstitutedSource<'_, S, C> {
    fn size(&self) -> u64 {
        self.size
    }

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
        if self.cancellation.is_cancelled() {
            return Err(Error::cancelled());
        }
        if self.source.size() != self.original_size {
            return Err(Error::invalid(
                "source size changed after substitution validation",
            ));
        }
        if offset >= self.size || destination.is_empty() {
            return Ok(0);
        }
        let index = self
            .sites
            .partition_point(|site| site.logical + 2 <= offset);
        let site = self.sites.get(index);
        if let Some(site) = site.filter(|site| offset >= site.logical) {
            let mut actual = [0; 4];
            for (i, chunk) in actual.chunks_mut(self.limits.io_chunk_bytes).enumerate() {
                read_exact_at(
                    self.source,
                    site.source + (i * self.limits.io_chunk_bytes) as u64,
                    chunk,
                    self.limits,
                    self.cancellation,
                )
                .map_err(source_failure)?;
            }
            if actual != EXPANDED {
                return Err(Error::pdf(
                    ErrorKind::Malformed,
                    site.logical,
                    None,
                    "source bytes changed after substitution validation",
                ));
            }
            let start = (offset - site.logical) as usize;
            let count = destination
                .len()
                .min(2 - start)
                .min(self.limits.io_chunk_bytes);
            destination[..count].copy_from_slice(&ORIGINAL[start..start + count]);
            return Ok(count);
        }
        let end = site.map_or(self.size, |site| site.logical);
        let count = (end - offset)
            .min(destination.len() as u64)
            .min(self.limits.io_chunk_bytes as u64) as usize;
        let read = self
            .source
            .read_at(offset + 2 * index as u64, &mut destination[..count])
            .map_err(source_failure)?;
        checked_read_count(read, count, "CAJ substitution source over-read")
    }
}

#[cfg(test)]
mod tests;
