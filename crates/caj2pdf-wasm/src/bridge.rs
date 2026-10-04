// SPDX-License-Identifier: MIT

//! Raw WASM exports over the platform-neutral [`Engine`].
//!
//! JavaScript starts one operation, polls it until it requests a range read,
//! write, or flush, awaits that request, completes it, and polls again. The
//! staging pointer is valid until reset. Each export is a thin wrapper; the
//! state machine and its tests live in `engine.rs`.

use crate::engine::{
    Engine, Operation, Outcome, Request, error_code, format_code, format_from_code,
};
use caj2pdf_core::{ConversionOptions, Limits};
use std::cell::RefCell;

const START_OK: u32 = 0;
const START_BUSY: u32 = 1;
const START_INVALID: u32 = 2;

const OPERATION_CONVERT: u32 = 1;
const OPERATION_INSPECT: u32 = 2;

thread_local! {
    static ENGINE: RefCell<Option<Engine>> = const { RefCell::new(None) };
}

fn install(source_size: u64, limits: Limits, operation: Operation) -> u32 {
    ENGINE.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_some() {
            return START_BUSY;
        }
        match Engine::start(source_size, limits, operation) {
            Ok(engine) => {
                *slot = Some(engine);
                START_OK
            }
            Err(_) => START_INVALID,
        }
    })
}

fn with_engine<T: Copy>(default: T, access: impl FnOnce(&mut Engine) -> T) -> T {
    ENGINE.with(|cell| cell.borrow_mut().as_mut().map_or(default, access))
}

fn with_outcome<T: Copy>(default: T, access: impl FnOnce(&Outcome) -> T) -> T {
    with_engine(default, |engine| match engine.result() {
        Some(Ok(outcome)) => access(outcome),
        _ => default,
    })
}

/// Start a bounded range copy (an I/O contract diagnostic, not conversion).
/// Returns 0 on success, 1 when busy, or 2 for an invalid configuration.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_io_start(
    source_size: u64,
    offset: u64,
    length: u64,
    chunk_size: u32,
) -> u32 {
    let limits = Limits {
        io_chunk_bytes: chunk_size as usize,
        ..Limits::default()
    };
    install(source_size, limits, Operation::Copy { offset, length })
}

/// Start a conversion (`operation` 1) or inspection (`operation` 2).
///
/// `format` 0 detects the input from its leading signature. Limits are
/// validated before any allocation. Returns 0, 1 (busy), or 2 (invalid).
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn caj2pdf_start(
    operation: u32,
    source_size: u64,
    chunk_size: u32,
    format: u32,
    include_bookmarks: u32,
    max_input_bytes: u64,
    max_output_bytes: u64,
    max_allocation_bytes: u64,
    max_pages: u32,
    max_bookmarks: u32,
) -> u32 {
    let Some(format) = format_from_code(format) else {
        return START_INVALID;
    };
    let operation = match operation {
        OPERATION_CONVERT => Operation::Convert {
            format,
            options: ConversionOptions {
                include_bookmarks: include_bookmarks != 0,
            },
        },
        OPERATION_INSPECT => Operation::Inspect { format },
        _ => return START_INVALID,
    };
    let limits = Limits {
        io_chunk_bytes: chunk_size as usize,
        max_input_bytes,
        max_output_bytes,
        max_allocation_bytes,
        max_pages,
        max_bookmarks,
    };
    install(source_size, limits, operation)
}

/// Poll the operation. 0=idle, 1=read, 2=write, 3=flush, 4=done, 5=error.
/// 6=scratch read, 7=scratch write, 8=resize, 9=scratch flush.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_io_poll() -> u32 {
    with_engine(0, |engine| engine.poll() as u32)
}

/// Byte offset for a read/scratch write, or new scratch extent for a resize.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_io_request_offset() -> u64 {
    with_engine(0, |engine| match engine.request() {
        Some(
            Request::Read { offset, .. }
            | Request::ScratchRead { offset, .. }
            | Request::ScratchWrite { offset, .. },
        ) => offset,
        Some(Request::ScratchResize { bytes, .. }) => bytes,
        _ => 0,
    })
}

/// Byte count for the outstanding read or write request.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_io_request_length() -> u32 {
    with_engine(0, |engine| match engine.request() {
        Some(
            Request::Read { length, .. }
            | Request::Write { length }
            | Request::ScratchRead { length, .. }
            | Request::ScratchWrite { length, .. },
        ) => length as u32,
        _ => 0,
    })
}

/// Fixed scratch workspace identifier (1..=4), or 0 for ordinary I/O.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_io_request_store() -> u32 {
    with_engine(0, |engine| match engine.request() {
        Some(
            Request::ScratchRead { store, .. }
            | Request::ScratchWrite { store, .. }
            | Request::ScratchResize { store, .. }
            | Request::ScratchFlush { store },
        ) => store,
        _ => 0,
    })
}

/// Read resource identifier: 0 for the document, 1..=6 for registered fonts.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_io_request_resource() -> u32 {
    with_engine(0, |engine| match engine.request() {
        Some(Request::Read { resource, .. }) => resource,
        _ => 0,
    })
}

/// Register a stable ranged font source before polling; returns 1..=6 or 0.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_c8_add_font(size: u64) -> u32 {
    with_engine(0, |engine| engine.add_font_source(size))
}

/// Assign zero-based font indices and the optional decoration alias.
/// `u32::MAX` marks an absent alternate Latin or decoration role; core then
/// applies the documented CJK/Latin fallback.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_c8_set_fonts(
    cjk: u32,
    latin: u32,
    alternate: u32,
    decoration: u32,
    alias: u32,
) -> u32 {
    with_engine(0, |engine| {
        engine.set_c8_fonts(cjk, latin, alternate, decoration, alias, u32::MAX) as u32
    })
}

/// Assign font roles including optional semantic symbols (`u32::MAX` if absent).
/// The original five-argument export continues to select no symbol resource.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_c8_set_fonts_with_symbols(
    cjk: u32,
    latin: u32,
    alternate: u32,
    decoration: u32,
    alias: u32,
    symbols: u32,
) -> u32 {
    with_engine(0, |engine| {
        engine.set_c8_fonts(cjk, latin, alternate, decoration, alias, symbols) as u32
    })
}

/// Assign an optional verified Latin state (3, 28 or 31) after the base roles.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_c8_set_latin_state(state: u32, index: u32) -> u32 {
    with_engine(0, |engine| engine.set_c8_latin_state(state, index) as u32)
}

#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_c8_set_latin_state3(index: u32) -> u32 {
    with_engine(0, |engine| engine.set_c8_latin_state3(index) as u32)
}

/// Append a validated caller-owned codec state before the first poll.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_hnc8_add_state(
    table: u32,
    qe: u32,
    next_lps: u32,
    next_mps: u32,
    switch: u32,
) -> u32 {
    with_engine(0, |engine| {
        engine.add_hnc8_state(table, qe, next_lps, next_mps, switch) as u32
    })
}

/// Complete an awaited scratch resize (the requested extent is a u64).
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_io_complete_resize() -> u32 {
    with_engine(0, |engine| engine.complete_resize() as u32)
}

/// Pointer into exported WASM memory for one bounded staging chunk.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_io_buffer_ptr() -> u32 {
    with_engine(0, |engine| {
        engine.with_staging(|staging| staging.as_mut_ptr() as u32)
    })
}

/// Complete the pending read after JavaScript copies bytes into staging.
/// Returns 1 only for a valid response; a zero-length read is allowed.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_io_complete_read(length: u32) -> u32 {
    with_engine(0, |engine| engine.complete_read(length as usize) as u32)
}

/// Complete a pending write after the sink accepts a prefix of its chunk.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_io_complete_write(length: u32) -> u32 {
    with_engine(0, |engine| engine.complete_write(length as usize) as u32)
}

/// Complete an awaited sink flush.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_io_complete_flush() -> u32 {
    with_engine(0, |engine| engine.complete_flush() as u32)
}

/// Cancel the active operation; the next poll resolves with a typed error.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_io_cancel() {
    with_engine((), |engine| engine.cancel());
}

/// Numeric error category for a failed operation (1..=16), else 0.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_io_error_kind() -> u32 {
    with_engine(0, |engine| match engine.result() {
        Some(Err(error)) => error_code(error),
        _ => 0,
    })
}

/// Pointer to the UTF-8 message of a failed operation.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_io_message_ptr() -> u32 {
    with_engine(0, |engine| engine.message().as_ptr() as u32)
}

/// Byte length of the error message (at most 1 KiB).
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_io_message_len() -> u32 {
    with_engine(0, |engine| engine.message().len() as u32)
}

/// Selected or detected format code once known (0 when unknown).
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_io_format() -> u32 {
    with_engine(0, |engine| format_code(engine.format()))
}

/// Bytes read by a successful operation; zero when unavailable.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_io_input_bytes_read() -> u64 {
    with_outcome(0, |outcome| outcome.report.input_bytes_read)
}

/// Bytes written by a successful operation; zero when unavailable.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_io_output_bytes_written() -> u64 {
    with_outcome(0, |outcome| outcome.report.output_bytes_written)
}

/// Pages converted by a successful operation; zero when unavailable.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_io_pages_converted() -> u32 {
    with_outcome(0, |outcome| outcome.report.pages_converted)
}

/// Bookmarks written by a successful operation; zero when unavailable.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_io_bookmarks_written() -> u32 {
    with_outcome(0, |outcome| outcome.report.bookmarks_written)
}

/// HN-A outline entries skipped or clamped by a successful operation.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_io_outline_warnings() -> u32 {
    with_outcome(0, |outcome| outcome.outline_warnings)
}

/// Page count from a successful inspection; zero when unavailable.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_info_page_count() -> u32 {
    with_outcome(0, |outcome| {
        outcome.info.as_ref().map_or(0, |info| info.page_count)
    })
}

/// Bookmark count from a successful inspection, or -1 when not counted.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_info_bookmark_count() -> i64 {
    with_outcome(-1, |outcome| {
        outcome
            .info
            .as_ref()
            .and_then(|info| info.bookmark_count)
            .map_or(-1, i64::from)
    })
}

/// Release the operation and its bounded staging allocation.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_io_reset() {
    ENGINE.with(|cell| *cell.borrow_mut() = None);
}
