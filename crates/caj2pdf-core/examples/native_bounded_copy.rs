// SPDX-License-Identifier: MIT

//! Run with `cargo run -p caj2pdf-core --example native_bounded_copy`.
//! This demonstrates the I/O contract, not CAJ format conversion.

use caj2pdf_core::{
    ConversionReport, Limits, NeverCancel, Result, copy_range, native::SeekableSource,
    native::WriteSink,
};
use std::{
    future::Future,
    io::{Cursor, Read, Seek, Write},
    pin::pin,
    task::{Context, Poll, Waker},
};

// Native Read/Seek/Write adapters complete without a runtime. This small
// bridge is intentionally limited to futures backed only by those adapters.
fn poll_native<F: Future>(future: F) -> F::Output {
    let mut context = Context::from_waker(Waker::noop());
    let mut future = pin!(future);
    match future.as_mut().poll(&mut context) {
        Poll::Ready(value) => value,
        Poll::Pending => unreachable!("native adapters never suspend"),
    }
}

/// Copy a range from a caller-owned seekable input to a caller-owned writer.
/// The writer does not need `Seek`.
fn copy_into<R: Read + Seek, W: Write>(input: &mut R, output: &mut W) -> Result<ConversionReport> {
    let mut source = SeekableSource::new(input)?;
    let size = caj2pdf_core::RangedSource::size(&source);
    let mut sink = WriteSink::new(output);
    poll_native(copy_range(
        &mut source,
        &mut sink,
        0,
        size,
        &Limits::default(),
        &NeverCancel,
    ))
}

fn main() -> Result<()> {
    let mut input = Cursor::new(b"bounded I/O example".to_vec());
    let mut output = Vec::new();
    let report = copy_into(&mut input, &mut output)?;
    assert_eq!(output, b"bounded I/O example");
    assert_eq!(report.output_bytes_written, output.len() as u64);
    println!("copied {} bytes", report.output_bytes_written);
    Ok(())
}
