// SPDX-License-Identifier: MIT

//! The bounded page-information body for the observed HN/C8 page profile.
//!
//! The field layout follows ITU-T T.88 (02/2000) §7.4.8. This reads and
//! bounds the 19-byte body only; [`super::page_profile`] checks the segment,
//! its flags, and its striping against the observed one-page, unstriped OR
//! profile. It is not a general JBIG2 page-information decoder.

use super::{SegmentHeader, SegmentSpan};
use crate::{Cancellation, Context, Error, ErrorKind, Limits, RangedSource, Result};

const PAGE_INFORMATION_BYTES: u64 = 19;
const BODY: &str = "page information body";

/// Validated page geometry and the retained original page flags.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PageInfo {
    /// Exact page-information payload read from the caller's source.
    pub data: SegmentSpan,
    pub width: u32,
    pub height: u32,
    /// Pixels per metre; zero means unknown, per T.88 §7.4.8.3.
    pub x_resolution: u32,
    /// Pixels per metre; zero means unknown, per T.88 §7.4.8.4.
    pub y_resolution: u32,
    pub flags_raw: u8,
    pub striping_raw: u16,
    pub row_stride: usize,
    pub packed_bytes: u64,
}

/// Locate an error at `offset` in the page-information segment.
fn at(header: &SegmentHeader, offset: u64, error: Error) -> Error {
    error.or_at(
        offset,
        Context::Jbig2 {
            segment: Some(header.number),
        },
    )
}

fn limit(
    header: &SegmentHeader,
    offset: u64,
    resource: &'static str,
    maximum: u64,
    attempted: u64,
) -> Error {
    at(header, offset, Error::limit(resource, maximum, attempted))
}

fn validate_span(
    header: &SegmentHeader,
    source_size: u64,
    limits: &Limits,
    cancellation: &dyn Cancellation,
) -> Result<()> {
    let offset = header.data.offset;
    if cancellation.is_cancelled() {
        return Err(at(header, offset, Error::cancelled()));
    }
    if header.data.length < PAGE_INFORMATION_BYTES {
        return Err(at(
            header,
            offset,
            Error::truncated(offset, PAGE_INFORMATION_BYTES, header.data.length).because(BODY),
        ));
    }
    if header.data.length > PAGE_INFORMATION_BYTES {
        return Err(at(
            header,
            offset,
            Error::invalid("extra page information bytes"),
        ));
    }
    if PAGE_INFORMATION_BYTES > limits.max_input_bytes {
        return Err(limit(
            header,
            offset,
            "page information input bytes",
            limits.max_input_bytes,
            PAGE_INFORMATION_BYTES,
        ));
    }
    let end = offset.checked_add(PAGE_INFORMATION_BYTES).ok_or_else(|| {
        at(
            header,
            offset,
            Error::invalid("page information end overflows"),
        )
    })?;
    if end > source_size {
        return Err(at(
            header,
            offset,
            Error::truncated(
                offset,
                PAGE_INFORMATION_BYTES,
                source_size.saturating_sub(offset),
            )
            .because("page information source span"),
        ));
    }
    Ok(())
}

fn checked_info(
    header: &SegmentHeader,
    bytes: [u8; PAGE_INFORMATION_BYTES as usize],
    limits: &Limits,
) -> Result<PageInfo> {
    let offset = header.data.offset;
    let width = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let height = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let x_resolution = u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    let y_resolution = u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
    let flags = bytes[16];
    let striping = u16::from_be_bytes([bytes[17], bytes[18]]);
    // Both dimensions are u32, so their product is strictly below u64::MAX.
    let pixels = u64::from(width) * u64::from(height);
    if pixels > limits.max_image_pixels {
        return Err(limit(
            header,
            offset,
            "page pixels",
            limits.max_image_pixels,
            pixels,
        ));
    }
    // ceil(u32::MAX / 8) fits a 32-bit usize; multiplying by height fits
    // u64 (the maximum is below 2^61). All supported native/WASM targets
    // have at least a 32-bit usize.
    let row_stride = width.div_ceil(8) as usize;
    let packed_bytes = row_stride as u64 * u64::from(height);
    if packed_bytes > limits.max_allocation_bytes {
        return Err(limit(
            header,
            offset,
            "packed page bytes",
            limits.max_allocation_bytes,
            packed_bytes,
        ));
    }
    if packed_bytes > limits.max_output_bytes {
        return Err(limit(
            header,
            offset,
            "output bytes",
            limits.max_output_bytes,
            packed_bytes,
        ));
    }
    Ok(PageInfo {
        data: header.data,
        width,
        height,
        x_resolution,
        y_resolution,
        flags_raw: flags,
        striping_raw: striping,
        row_stride,
        packed_bytes,
    })
}

/// Read and bound the caller-delimited page-information body of `header`.
///
/// The caller owns the range source. Reads are positioned, at most 19 bytes
/// and one I/O chunk, and may complete through several short reads. No heap
/// allocation occurs.
pub fn read_page_info<S: RangedSource, C: Cancellation>(
    source: &mut S,
    header: &SegmentHeader,
    limits: &Limits,
    cancellation: &C,
) -> Result<PageInfo> {
    validate_span(header, source.size(), limits, cancellation)?;
    let mut bytes = [0_u8; PAGE_INFORMATION_BYTES as usize];
    let mut done = 0_usize;
    let request_bound = limits.io_chunk_bytes.max(1);
    while done < bytes.len() {
        let offset = header.data.offset + done as u64;
        let failed = |error| at(header, offset, error);
        if cancellation.is_cancelled() {
            return Err(failed(Error::cancelled()));
        }
        let request = (bytes.len() - done).min(request_bound);
        let count = source
            .read_at(offset, &mut bytes[done..done + request])
            .map_err(|error| match error.kind {
                ErrorKind::Truncated { .. } => failed(error.because(BODY)),
                _ => failed(error),
            })?;
        if count > request {
            return Err(failed(Error::invalid(
                "source reported more bytes than requested",
            )));
        }
        done += count;
        if cancellation.is_cancelled() {
            return Err(at(
                header,
                header.data.offset + done as u64,
                Error::cancelled(),
            ));
        }
        if count == 0 {
            let remaining = (bytes.len() - done) as u64;
            return Err(failed(Error::truncated(offset, remaining, 0).because(BODY)));
        }
    }
    checked_info(header, bytes, limits)
}
