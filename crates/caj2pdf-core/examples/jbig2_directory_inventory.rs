// SPDX-License-Identifier: MIT

//! Rust half of the optional SHA-pinned HN/C8 metadata inventory.
//!
//! The repository-owned Python driver verifies source hashes and supplies
//! bounded image record spans. This example uses the core directory API for
//! every compressed payload and emits one JSON line per image.

use caj2pdf_core::{
    Limits, NeverCancel,
    jbig2::{DirectoryLimits, HeaderLimits, SegmentSpan, read_embedded_directory},
    native::SeekableSource,
};
use std::{
    env,
    error::Error,
    fmt::Write as _,
    fs,
    fs::File,
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};

const WRAPPER_BYTES: u64 = 48;
const MAX_MANIFEST_BYTES: u64 = 2 * 1024 * 1024;
const MAX_IMAGES: usize = 546;

fn run<F: Future>(future: F) -> F::Output {
    let mut context = Context::from_waker(Waker::noop());
    let mut future = pin!(future);
    match future.as_mut().poll(&mut context) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("seekable file source unexpectedly yielded"),
    }
}

fn inventory() -> Result<(), Box<dyn Error>> {
    let mut args = env::args_os();
    let _program = args.next();
    let path = args
        .next()
        .ok_or("usage: jbig2_directory_inventory MANIFEST")?;
    if args.next().is_some() {
        return Err("usage: jbig2_directory_inventory MANIFEST".into());
    }
    let manifest_size = fs::metadata(&path)?.len();
    if manifest_size > MAX_MANIFEST_BYTES {
        return Err("inventory manifest exceeds 2 MiB".into());
    }
    let manifest = fs::read_to_string(path)?;
    if manifest.len() as u64 != manifest_size {
        return Err("inventory manifest changed while reading".into());
    }
    let lines = manifest.lines().collect::<Vec<_>>();
    if lines.len() != MAX_IMAGES {
        return Err(format!("expected {MAX_IMAGES} image spans, got {}", lines.len()).into());
    }
    for (index, line) in lines.into_iter().enumerate() {
        let mut fields = line.split('\t');
        let source_path = fields.next().ok_or("missing source path")?;
        let record_offset: u64 = fields.next().ok_or("missing record offset")?.parse()?;
        let record_length: u64 = fields.next().ok_or("missing record length")?.parse()?;
        if fields.next().is_some() || source_path.is_empty() {
            return Err(format!("image {index}: invalid inventory manifest row").into());
        }
        let payload_offset = record_offset
            .checked_add(WRAPPER_BYTES)
            .ok_or("payload offset overflows")?;
        let payload_length = record_length
            .checked_sub(WRAPPER_BYTES)
            .ok_or("image record is shorter than its wrapper")?;
        let file = File::open(source_path)?;
        let mut source = SeekableSource::new(file)?;
        let directory = run(read_embedded_directory(
            &mut source,
            SegmentSpan {
                offset: payload_offset,
                length: payload_length,
            },
            &Limits::default(),
            HeaderLimits::default(),
            DirectoryLimits::default(),
            &NeverCancel,
        ))
        .map_err(|error| format!("image {index}: {error}"))?;
        let expected_types = [48, 0, 0, 6, 38];
        let expected_refs: [&[u32]; 5] = [&[], &[], &[1], &[2], &[]];
        if directory.segments.len() != expected_types.len() {
            return Err(format!("image {index}: expected five embedded segments").into());
        }
        let mut result = format!("{{\"index\":{index},\"segments\":[");
        for (position, segment) in directory.segments.iter().enumerate() {
            if segment.number != position as u32
                || segment.segment_type != expected_types[position]
                || segment.page_association != 1
                || segment.referred_to != expected_refs[position]
            {
                return Err(format!("image {index}: embedded header profile differs").into());
            }
            if position != 0 {
                result.push(',');
            }
            write!(
                result,
                "{{\"number\":{},\"type\":{},\"page_association\":{},\"refs\":[",
                segment.number, segment.segment_type, segment.page_association
            )?;
            for (reference_position, reference) in segment.referred_to.iter().enumerate() {
                if reference_position != 0 {
                    result.push(',');
                }
                write!(result, "{reference}")?;
            }
            write!(
                result,
                "],\"data_offset\":{},\"data_length\":{},\"header_length\":{}}}",
                segment.data.offset, segment.data.length, segment.header_length
            )?;
        }
        result.push_str("]}");
        println!("{result}");
    }
    Ok(())
}

fn main() {
    if let Err(error) = inventory() {
        eprintln!("JBIG2 directory inventory FAIL: {error}");
        std::process::exit(1);
    }
}
