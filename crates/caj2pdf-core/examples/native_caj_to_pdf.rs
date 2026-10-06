// SPDX-License-Identifier: MIT

//! Native integration probe: cargo run -p caj2pdf-core --example native_caj_to_pdf -- INPUT.caj OUTPUT.pdf

use caj2pdf_core::{
    ConversionOptions, Limits, NeverCancel, caj::convert_caj, native::SeekableSource,
};
use std::{env, fs::File};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args_os().skip(1);
    let input = args
        .next()
        .ok_or("usage: native_caj_to_pdf INPUT.caj OUTPUT.pdf")?;
    let output = args
        .next()
        .ok_or("usage: native_caj_to_pdf INPUT.caj OUTPUT.pdf")?;
    if args.next().is_some() {
        return Err("usage: native_caj_to_pdf INPUT.caj OUTPUT.pdf".into());
    }
    let mut source = SeekableSource::new(File::open(input)?)?;
    let mut sink = File::create(output)?;
    let report = convert_caj(
        &mut source,
        &mut sink,
        &ConversionOptions::default(),
        &Limits::default(),
        &NeverCancel,
    )?;
    eprintln!(
        "converted {} pages and {} bookmarks; read {} bytes; wrote {} bytes",
        report.pages_converted,
        report.bookmarks_written,
        report.input_bytes_read,
        report.output_bytes_written
    );
    Ok(())
}
