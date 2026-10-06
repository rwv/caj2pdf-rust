// SPDX-License-Identifier: MIT

//! Native probe: cargo run -p caj2pdf-core --example native_kdh_to_pdf -- INPUT.caj OUTPUT.pdf

use caj2pdf_core::{Limits, NeverCancel, kdh::convert_kdh, native::SeekableSource};
use std::{env, fs::File};

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
    let mut sink = File::create(output)?;
    let report = convert_kdh(&mut source, &mut sink, &Limits::default(), &NeverCancel)?;
    eprintln!(
        "converted {} pages; read {} bytes; wrote {} bytes",
        report.pages_converted, report.input_bytes_read, report.output_bytes_written
    );
    Ok(())
}
