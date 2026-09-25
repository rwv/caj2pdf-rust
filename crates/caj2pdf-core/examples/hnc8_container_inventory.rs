// SPDX-License-Identifier: MIT

//! Metadata-only development tool for the opt-in, hash-pinned HN/C8 corpus.
//! No source bytes or image payloads are written to stdout.

use caj2pdf_core::{
    Limits, NeverCancel,
    hnc8::{Budget, Hnc8Error, Hnc8Reader, ImageRecord},
    native::SeekableSource,
};
use std::{
    env,
    fs::File,
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};

fn ready<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    match future.as_mut().poll(&mut context) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("native file adapter unexpectedly yielded"),
    }
}

fn emit_image(record: ImageRecord, variant: &str) {
    println!(
        "I\t{variant}\t{}\t{}\t{}\t{}\t{}\t{}",
        record.page_number,
        record.image_number,
        record.descriptor_offset,
        record.record_type,
        record.payload.offset,
        record.payload.length
    );
}

fn emit_error(error: &Hnc8Error) {
    println!(
        "E\t{}\t{}\t{}\t{}\t{}\t{}",
        error.variant.map(|v| v.as_str()).unwrap_or("-"),
        error.page.map_or("-".to_owned(), |n| n.to_string()),
        error.image.map_or("-".to_owned(), |n| n.to_string()),
        error.offset,
        error.kind.field(),
        error.kind.as_str()
    );
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let path = args
        .next()
        .ok_or("usage: hnc8_container_inventory SOURCE_PATH [--diagnostic]")?;
    let diagnostic = match args.next().as_deref() {
        None => false,
        Some("--diagnostic") => true,
        _ => return Err("usage: hnc8_container_inventory SOURCE_PATH [--diagnostic]".into()),
    };
    if args.next().is_some() {
        return Err("usage: hnc8_container_inventory SOURCE_PATH [--diagnostic]".into());
    }
    let limits = Limits::default();
    let budget = Budget::default();
    let mut source = SeekableSource::new(File::open(path)?)?;
    let header = match ready(Hnc8Reader::open(&mut source, &limits, &NeverCancel, budget)) {
        Ok(reader) => reader.header(),
        Err(error) => {
            emit_error(&error);
            return Err(Box::new(error));
        }
    };
    if diagnostic {
        for page_number in 1..=header.page_count {
            let mut reader = match ready(Hnc8Reader::probe_at_page(
                &mut source,
                &limits,
                &NeverCancel,
                budget,
                page_number,
            )) {
                Ok(reader) => reader,
                Err(error) => {
                    emit_error(&error);
                    return Err(Box::new(error));
                }
            };
            if let Err(error) = ready(reader.next_page()) {
                emit_error(&error);
                continue;
            }
            loop {
                match ready(reader.next_image()) {
                    Ok(Some(image)) => emit_image(image, header.variant.as_str()),
                    Ok(None) => break,
                    Err(error) => {
                        emit_error(&error);
                        break;
                    }
                }
            }
        }
    } else {
        let mut reader = ready(Hnc8Reader::open(&mut source, &limits, &NeverCancel, budget))?;
        while ready(reader.next_page()).inspect_err(emit_error)?.is_some() {
            loop {
                match ready(reader.next_image()) {
                    Ok(Some(image)) => emit_image(image, header.variant.as_str()),
                    Ok(None) => break,
                    Err(error) => {
                        emit_error(&error);
                        return Err(Box::new(error));
                    }
                }
            }
        }
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
