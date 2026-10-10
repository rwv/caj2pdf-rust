// SPDX-License-Identifier: MIT

//! Raw WASM imports and exports over the platform-neutral [`Session`].
//!
//! JavaScript registers any C8 fonts, then calls [`caj2pdf_convert`] or
//! [`caj2pdf_inspect`], which runs to completion. Rust reads and writes
//! through the imported `caj2pdf` functions: a read fills a Rust-owned
//! destination in this memory, and a write passes a pointer to Rust-owned
//! bytes that JavaScript copies before returning. Accessors then describe
//! the result until [`caj2pdf_reset`].

use crate::engine::{Host, Operation, Outcome, Session, error_code, format_code, format_from_code};
use caj2pdf_core::{ConversionOptions, Limits};
use std::{cell::RefCell, io};

#[link(wasm_import_module = "caj2pdf")]
unsafe extern "C" {
    /// Copy at most `len` bytes of `resource` at `offset` to `ptr`; returns
    /// the count, or a negative value after a host failure.
    fn caj2pdf_read(resource: u32, offset: f64, ptr: *mut u8, len: u32) -> i32;
    /// Take the `len` bytes at `ptr`; returns the count accepted, or a
    /// negative value after a host failure.
    fn caj2pdf_write(ptr: *const u8, len: u32) -> i32;
    /// The output barrier after the last write; 0, or negative on failure.
    fn caj2pdf_flush() -> i32;
    /// `done` of `total` parts of the document have been read.
    fn caj2pdf_progress(done: u32, total: u32);
    /// Nonzero once the caller asked to stop.
    fn caj2pdf_cancelled() -> i32;
}

/// Offsets travel as JavaScript Numbers, exact below 2^53.
const MAX_EXACT_OFFSET: u64 = 1 << 53;

/// The host behind the imported functions.
struct Imports;

impl Host for Imports {
    fn read(&mut self, resource: u32, offset: u64, destination: &mut [u8]) -> io::Result<usize> {
        if offset > MAX_EXACT_OFFSET {
            return Err(io::Error::other("offset exceeds the JavaScript safe range"));
        }
        // A request never exceeds the configured I/O chunk, which fits u32.
        let length = u32::try_from(destination.len()).map_err(io::Error::other)?;
        // SAFETY: `destination` is a live, exclusively borrowed slice of this
        // module's memory; the host writes at most `length` bytes into it
        // before returning and keeps no reference to it.
        let count =
            unsafe { caj2pdf_read(resource, offset as f64, destination.as_mut_ptr(), length) };
        usize::try_from(count).map_err(|_| io::Error::other("host read failed"))
    }

    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let length = u32::try_from(bytes.len()).map_err(io::Error::other)?;
        // SAFETY: `bytes` stays borrowed for the call; the host copies them
        // before returning and keeps no reference to this memory.
        let count = unsafe { caj2pdf_write(bytes.as_ptr(), length) };
        usize::try_from(count).map_err(|_| io::Error::other("host write failed"))
    }

    fn flush(&mut self) -> io::Result<()> {
        // SAFETY: the import takes no pointers.
        match unsafe { caj2pdf_flush() } {
            0.. => Ok(()),
            _ => Err(io::Error::other("host flush failed")),
        }
    }

    fn progress(&mut self, done: u32, total: u32) {
        // SAFETY: the import takes no pointers.
        unsafe { caj2pdf_progress(done, total) }
    }

    fn cancelled(&mut self) -> bool {
        // SAFETY: the import takes no pointers.
        unsafe { caj2pdf_cancelled() != 0 }
    }
}

const STATUS_BUSY: u32 = 3;
const STATUS_INVALID: u32 = 2;

thread_local! {
    static SESSION: RefCell<Session> = RefCell::new(Session::default());
}

/// Run `access` on the session, or return `default` while an operation
/// holds it (an import called back into an export).
fn with_session<T>(default: T, access: impl FnOnce(&mut Session) -> T) -> T {
    SESSION.with(|cell| match cell.try_borrow_mut() {
        Ok(mut session) => access(&mut session),
        Err(_) => default,
    })
}

fn with_outcome<T>(default: T, access: impl FnOnce(&Outcome) -> T) -> T {
    SESSION.with(|cell| match cell.try_borrow() {
        Ok(session) => session.outcome().map_or(default, access),
        Err(_) => default,
    })
}

/// The JavaScript limits; the image-pixel and symbol limits keep their
/// defaults.
fn limits(
    chunk_size: u32,
    max_input_bytes: u64,
    max_output_bytes: u64,
    max_allocation_bytes: u64,
    max_pages: u32,
    max_bookmarks: u32,
) -> Limits {
    Limits {
        io_chunk_bytes: chunk_size as usize,
        max_input_bytes,
        max_output_bytes,
        max_allocation_bytes,
        max_pages,
        max_bookmarks,
        ..Limits::default()
    }
}

fn run(source_size: u64, limits: Limits, operation: Operation) -> u32 {
    with_session(STATUS_BUSY, |session| {
        session.run(&mut Imports, source_size, limits, operation) as u32
    })
}

/// Convert a document of `source_size` bytes. `format` 0 detects it from its
/// leading signature; `flags` bit 0 writes bookmarks, bit 1 allows damaged
/// CAJ pages. Returns 0 (done), 1 (failed), 2 (invalid configuration) or 3
/// (busy: reset first).
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn caj2pdf_convert(
    source_size: u64,
    chunk_size: u32,
    format: u32,
    flags: u32,
    max_input_bytes: u64,
    max_output_bytes: u64,
    max_allocation_bytes: u64,
    max_pages: u32,
    max_bookmarks: u32,
) -> u32 {
    let Some(format) = format_from_code(format) else {
        return STATUS_INVALID;
    };
    let operation = Operation::Convert {
        options: ConversionOptions {
            format,
            include_bookmarks: flags & 1 != 0,
            allow_damaged: flags & 2 != 0,
            ..ConversionOptions::default()
        },
    };
    let limits = limits(
        chunk_size,
        max_input_bytes,
        max_output_bytes,
        max_allocation_bytes,
        max_pages,
        max_bookmarks,
    );
    run(source_size, limits, operation)
}

/// Inspect a document without writing; status codes as for
/// [`caj2pdf_convert`].
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn caj2pdf_inspect(
    source_size: u64,
    chunk_size: u32,
    format: u32,
    max_input_bytes: u64,
    max_output_bytes: u64,
    max_allocation_bytes: u64,
    max_pages: u32,
    max_bookmarks: u32,
) -> u32 {
    let Some(format) = format_from_code(format) else {
        return STATUS_INVALID;
    };
    let limits = limits(
        chunk_size,
        max_input_bytes,
        max_output_bytes,
        max_allocation_bytes,
        max_pages,
        max_bookmarks,
    );
    run(source_size, limits, Operation::Inspect { format })
}

/// Register a font resource of `size` bytes and its collection face (0 for a
/// standalone font) before converting; returns its read resource 1..=8, or 0.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_c8_add_font(size: u64, face: u32) -> u32 {
    with_session(0, |session| session.add_font_source(size, face))
}

/// Assign zero-based font indices and the optional decoration alias.
/// `u32::MAX` marks an absent alternate Latin or decoration role; core then
/// applies the documented CJK/Latin fallback. Returns 1 when accepted.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_c8_set_fonts(
    cjk: u32,
    latin: u32,
    alternate: u32,
    decoration: u32,
    alias: u32,
) -> u32 {
    with_session(0, |session| {
        session.set_c8_fonts(cjk, latin, alternate, decoration, alias, u32::MAX) as u32
    })
}

/// As [`caj2pdf_c8_set_fonts`], with an optional semantic symbol font
/// (`u32::MAX` if absent).
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_c8_set_fonts_with_symbols(
    cjk: u32,
    latin: u32,
    alternate: u32,
    decoration: u32,
    alias: u32,
    symbols: u32,
) -> u32 {
    with_session(0, |session| {
        session.set_c8_fonts(cjk, latin, alternate, decoration, alias, symbols) as u32
    })
}

/// Assign a verified Latin state (3, 28 or 31) after the base roles.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_c8_set_latin_state(state: u32, index: u32) -> u32 {
    with_session(0, |session| session.set_c8_latin_state(state, index) as u32)
}

/// Map an HN-B mode-0 symbol code to a BMP glyph of the symbols role, after
/// the roles. Rejects other codes, duplicates and a missing symbols role.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_hnb_add_symbol_glyph(code: u32, glyph: u32) -> u32 {
    with_session(0, |session| {
        session.add_hnb_symbol_glyph(code, glyph) as u32
    })
}

/// Numeric error category of a failed operation (1..=16), else 0.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_error_kind() -> u32 {
    with_session(0, |session| match session.result() {
        Some(Err(error)) => error_code(error),
        _ => 0,
    })
}

/// Pointer to the UTF-8 message of a failed operation.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_message_ptr() -> u32 {
    with_session(0, |session| session.message().as_ptr() as u32)
}

/// Byte length of the error message (at most 1 KiB).
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_message_len() -> u32 {
    with_session(0, |session| session.message().len() as u32)
}

/// Selected or detected format code once known (0 when unknown).
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_format() -> u32 {
    with_session(0, |session| format_code(session.format()))
}

/// Bytes read by a successful operation; zero when unavailable.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_input_bytes_read() -> u64 {
    with_outcome(0, |outcome| outcome.report.input_bytes_read)
}

/// Bytes written by a successful operation; zero when unavailable.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_output_bytes_written() -> u64 {
    with_outcome(0, |outcome| outcome.report.output_bytes_written)
}

/// Pages converted by a successful operation; zero when unavailable.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_pages_converted() -> u32 {
    with_outcome(0, |outcome| outcome.report.pages_converted)
}

/// Private-use glyphs drawn using an explicitly reported visual substitute.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_substituted_glyphs() -> u64 {
    with_outcome(0, |outcome| outcome.report.substituted_glyphs)
}

/// Bookmarks written by a successful operation; zero when unavailable.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_bookmarks_written() -> u32 {
    with_outcome(0, |outcome| outcome.report.bookmarks_written)
}

/// HN-A outline entries skipped or clamped by a successful operation.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_outline_warnings() -> u32 {
    with_outcome(0, |outcome| outcome.outline_warnings)
}

/// 1 when requested C8/HN-B bookmarks were not written because their layout
/// is unverified; 0 otherwise.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_outline_omitted() -> u32 {
    with_outcome(0, |outcome| u32::from(outcome.outline_omitted))
}

/// Number of explicitly blanked pages in the completed conversion.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_omitted_pages_count() -> u32 {
    with_outcome(0, |outcome| outcome.report.omitted_pages.len() as u32)
}

/// Zero-based source page index; callers must check the count first.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_omitted_page_index(index: u32) -> u32 {
    with_outcome(0, |outcome| {
        outcome
            .report
            .omitted_pages
            .get(index as usize)
            .map_or(0, |page| page.page_index)
    })
}

/// Absolute source offset explaining one blank substitution.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_omitted_page_offset(index: u32) -> u64 {
    with_outcome(0, |outcome| {
        outcome
            .report
            .omitted_pages
            .get(index as usize)
            .map_or(0, |page| page.offset)
    })
}

/// Page count from a successful inspection; zero when unavailable.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_info_page_count() -> u32 {
    with_outcome(0, |outcome| {
        outcome
            .info
            .as_ref()
            .and_then(|info| info.page_count)
            .unwrap_or(0)
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

/// Annotation count of the C8 application-info package read by a successful
/// inspection, or -1 when there is none (absent, defective or not C8).
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_info_note_count() -> i64 {
    with_outcome(-1, |outcome| {
        outcome
            .application_info
            .as_ref()
            .map_or(-1, |info| i64::from(info.note_count))
    })
}

/// The package's DOI (`field` 0) or URL (`field` 1); empty when absent.
fn application_text(outcome: &Outcome, field: u32) -> &str {
    let info = outcome.application_info.as_ref();
    let text = match field {
        0 => info.and_then(|info| info.doi.as_deref()),
        1 => info.and_then(|info| info.url.as_deref()),
        _ => None,
    };
    text.unwrap_or("")
}

/// Pointer to the UTF-8 application-info text selected by `field`.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_info_text_ptr(field: u32) -> u32 {
    with_outcome(0, |outcome| {
        application_text(outcome, field).as_ptr() as u32
    })
}

/// Byte length of the application-info text selected by `field`; 0 when absent.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_info_text_len(field: u32) -> u32 {
    with_outcome(0, |outcome| application_text(outcome, field).len() as u32)
}

/// Drop the registered fonts and the last result, ready for a new operation.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_reset() {
    with_session((), |session| *session = Session::default());
}

/// Set 32 case-sensitive response bytes, packed as four little-endian words.
/// No pointer to caller-owned memory is retained.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_set_ttkn_response(a: u64, b: u64, c: u64, d: u64) -> u32 {
    let mut bytes = [0_u8; 32];
    for (chunk, word) in bytes.as_chunks_mut::<8>().0.iter_mut().zip([a, b, c, d]) {
        chunk.copy_from_slice(&word.to_le_bytes());
    }
    with_session(0, |session| session.set_ttkn_response(&bytes) as u32)
}
