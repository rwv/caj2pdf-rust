// SPDX-License-Identifier: MIT

//! Cooperative cancellation. A repeated termination signal retains its normal
//! action, so users can still stop a process blocked inside an operating-system
//! read or write. Only normal unwinding can clean up staged output paths.

use caj2pdf_core::Cancellation;
use signal_hook::{
    consts::signal::{SIGINT, SIGTERM},
    flag,
};
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, Ordering},
};

static CANCELLED: OnceLock<Arc<AtomicBool>> = OnceLock::new();

pub fn install() -> std::io::Result<()> {
    let cancelled = CANCELLED.get_or_init(|| Arc::new(AtomicBool::new(false)));
    for signal in [SIGINT, SIGTERM] {
        flag::register_conditional_default(signal, Arc::clone(cancelled))?;
        flag::register(signal, Arc::clone(cancelled))?;
    }
    Ok(())
}

pub struct ProcessCancellation;

impl Cancellation for ProcessCancellation {
    fn is_cancelled(&self) -> bool {
        CANCELLED
            .get()
            .is_some_and(|flag| flag.load(Ordering::Relaxed))
    }
}
