// SPDX-License-Identifier: MIT

//! Bounded decoder probe: cargo run -p caj2pdf-core --example native_kdh_probe -- INPUT.caj

use caj2pdf_core::{Limits, NeverCancel, kdh::KdhPdfSource, native::SeekableSource};
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
    let input = args.next().ok_or("usage: native_kdh_probe INPUT.caj")?;
    if args.next().is_some() {
        return Err("usage: native_kdh_probe INPUT.caj".into());
    }
    let mut source = SeekableSource::new(File::open(input)?)?;
    let decoded = poll_native(KdhPdfSource::open(
        &mut source,
        &Limits::default(),
        &NeverCancel,
    ))?;
    println!(
        "PDF bytes: {}, trailing bytes: {}, input bytes read: {}",
        decoded.pdf_len(),
        decoded.trailing_len(),
        decoded.scan_bytes_read()
    );
    Ok(())
}
