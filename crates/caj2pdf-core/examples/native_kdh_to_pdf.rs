// SPDX-License-Identifier: MIT

//! Native probe: cargo run -p caj2pdf-core --example native_kdh_to_pdf -- INPUT.caj OUTPUT.pdf

use caj2pdf_core::{
    Limits, NeverCancel,
    kdh::convert_kdh,
    native::{SeekableSource, WriteSink},
};
use std::{
    env,
    fs::File,
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};

fn poll_native<F: Future>(future: F) -> F::Output {
    let mut context = Context::from_waker(Waker::noop());
    let mut future = pin!(future);
    match future.as_mut().poll(&mut context) {
        Poll::Ready(value) => value,
        Poll::Pending => unreachable!("native adapters never suspend"),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args_os().skip(1);
    let input = args
        .next()
        .ok_or("usage: native_kdh_to_pdf INPUT.caj OUTPUT.pdf")?;
    let output = args
        .next()
        .ok_or("usage: native_kdh_to_pdf INPUT.caj OUTPUT.pdf")?;
    if args.next().is_some() {
        return Err("usage: native_kdh_to_pdf INPUT.caj OUTPUT.pdf".into());
    }
    let mut source = SeekableSource::new(File::open(input)?)?;
    let mut sink = WriteSink::new(File::create(output)?);
    let report = poll_native(convert_kdh(
        &mut source,
        &mut sink,
        &Limits::default(),
        &NeverCancel,
    ))?;
    eprintln!(
        "converted {} pages; read {} bytes; wrote {} bytes",
        report.pages_converted, report.input_bytes_read, report.output_bytes_written
    );
    Ok(())
}
