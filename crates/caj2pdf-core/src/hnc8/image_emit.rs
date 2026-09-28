// SPDX-License-Identifier: MIT

//! One padded, bottom-first type-0 XObject using caller-owned row storage.

use super::ImageRecord;
use super::convert::{Type0DecodeSettings, Type0PdfError, decode_type0_rows};
use crate::fallible::{len_u64, reserve_exact};
use crate::jbig1::Type0Info;
use crate::jbig2::text_composer::RandomAccessScratch;
use crate::pdf::{BilevelImageSpec, ImageObject, PdfDocument};
use crate::{Cancellation, Error, MAX_BUDGET_COUNT, RangedSource, SequentialSink, write_all};
use std::{error, fmt};

#[derive(Clone, Copy, Debug)]
pub(super) struct Type0ScratchBudget {
    pub max_bytes: u64,
    /// Charge each requested read/write length, including short-I/O retries.
    pub max_work_bytes: u64,
}

impl Default for Type0ScratchBudget {
    fn default() -> Self {
        Self {
            max_bytes: 64 * 1024 * 1024,
            max_work_bytes: 1024 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct Type0ScratchReport {
    pub peak_scratch_bytes: u64,
    pub scratch_read_bytes: u64,
    pub scratch_write_bytes: u64,
    pub scratch_work_bytes: u64,
    pub max_request_bytes: usize,
    pub read_calls: u64,
    pub write_calls: u64,
    /// Allocated only after the decoder's three row buffers have been dropped.
    pub copy_buffer_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Type0ScratchStage {
    Prepare,
    Decode,
    Emit,
    Cleanup,
}

#[derive(Debug)]
pub(super) enum Type0ScratchErrorKind {
    Decode(Type0PdfError),
    Store(Error),
    Pdf(Error),
}

#[derive(Debug)]
pub(super) struct Type0ScratchError {
    pub stage: Type0ScratchStage,
    pub kind: Type0ScratchErrorKind,
    /// The conversion failure remains primary if resetting the store also fails.
    pub cleanup_error: Option<Error>,
    pub report: Type0ScratchReport,
}

impl fmt::Display for Type0ScratchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "padded type-0 image {:?}: ", self.stage)?;
        match &self.kind {
            Type0ScratchErrorKind::Decode(error) => write!(f, "{error}"),
            Type0ScratchErrorKind::Store(error) => write!(f, "row storage: {error}"),
            Type0ScratchErrorKind::Pdf(error) => write!(f, "PDF output: {error}"),
        }?;
        if let Some(error) = &self.cleanup_error {
            write!(f, "; row-storage cleanup also failed: {error}")?;
        }
        Ok(())
    }
}

impl error::Error for Type0ScratchError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        Some(match &self.kind {
            Type0ScratchErrorKind::Decode(error) => error,
            Type0ScratchErrorKind::Store(error) | Type0ScratchErrorKind::Pdf(error) => error,
        })
    }
}

fn failed(
    stage: Type0ScratchStage,
    kind: Type0ScratchErrorKind,
    report: Type0ScratchReport,
) -> Box<Type0ScratchError> {
    Box::new(Type0ScratchError {
        stage,
        kind,
        cleanup_error: None,
        report,
    })
}

fn check_size<T: RandomAccessScratch>(scratch: &T, expected: u64) -> crate::Result<()> {
    if scratch.size()? != expected {
        return Err(Error::InvalidInput {
            reason: "type-0 row-storage size changed",
        });
    }
    Ok(())
}

fn charge_request(
    report: &mut Type0ScratchReport,
    budget: Type0ScratchBudget,
    count: usize,
) -> crate::Result<()> {
    let attempted = report.scratch_work_bytes + len_u64(count);
    if attempted > budget.max_work_bytes {
        return Err(Error::LimitExceeded {
            resource: "type-0 row-storage work bytes",
            limit: budget.max_work_bytes,
            attempted,
        });
    }
    report.scratch_work_bytes = attempted;
    report.max_request_bytes = report.max_request_bytes.max(count);
    Ok(())
}

/// Check the requested and actual copy-buffer allocation independently of the
/// caller's chunk calculation. Capacity overflow is a typed allocator refusal.
fn copy_buffer(
    bytes: usize,
    limits: &crate::Limits,
    report: Type0ScratchReport,
) -> Result<Vec<u8>, Box<Type0ScratchError>> {
    let refused = |error| {
        failed(
            Type0ScratchStage::Emit,
            Type0ScratchErrorKind::Store(error),
            report,
        )
    };
    limits.check_allocation(len_u64(bytes)).map_err(refused)?;
    let mut buffer = Vec::new();
    reserve_exact(
        &mut buffer,
        bytes,
        limits.allocation_refused("type-0 row-storage copy bytes", len_u64(bytes)),
    )
    .map_err(refused)?;
    buffer.resize(bytes, 0);
    limits
        .check_allocation(len_u64(buffer.capacity()))
        .map_err(refused)?;
    Ok(buffer)
}

/// The decoder may split or join rows. Accept only a prefix ending at the
/// current row boundary, and position it in the opposite row without changing
/// byte/bit order inside that row. No additional row buffer is needed.
struct ReversedRows<'a, T, C> {
    scratch: &'a mut T,
    report: &'a mut Type0ScratchReport,
    budget: Type0ScratchBudget,
    cancellation: &'a C,
    stride: u64,
    length: u64,
    position: u64,
}

impl<T: RandomAccessScratch, C: Cancellation> SequentialSink for ReversedRows<'_, T, C> {
    async fn write(&mut self, bytes: &[u8]) -> crate::Result<usize> {
        if self.cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        check_size(self.scratch, self.length)?;
        if bytes.is_empty() {
            return Ok(0);
        }
        if len_u64(bytes.len()) > self.length - self.position {
            return Err(Error::InvalidInput {
                reason: "type-0 rows exceed row-storage length",
            });
        }
        let column = self.position % self.stride;
        let count = bytes.len().min((self.stride - column) as usize);
        let offset = self.length - self.stride - self.position / self.stride * self.stride + column;
        charge_request(self.report, self.budget, count)?;
        self.report.write_calls += 1;
        let written = self.scratch.write_at(offset, &bytes[..count]).await?;
        if written > count {
            return Err(Error::InvalidInput {
                reason: "type-0 row storage overreported a write",
            });
        }
        self.position += len_u64(written);
        self.report.scratch_write_bytes += len_u64(written);
        check_size(self.scratch, self.length)?;
        // The decoder's write_all checks cancellation after accounting for
        // accepted bytes; returning the count preserves its progress report.
        Ok(written)
    }

    async fn flush(&mut self) -> crate::Result<()> {
        check_size(self.scratch, self.length)?;
        self.scratch.flush().await?;
        check_size(self.scratch, self.length)
    }
}

#[allow(clippy::too_many_arguments)]
async fn emit_inner<S, W, T, C>(
    source: &mut S,
    document: &mut PdfDocument<'_, W, C>,
    image: ImageRecord,
    checked: Type0Info,
    contexts: &mut crate::qm::ContextBank,
    scratch: &mut T,
    budget: Type0ScratchBudget,
    settings: &Type0DecodeSettings<'_, C>,
    report: &mut Type0ScratchReport,
) -> Result<ImageObject, Box<Type0ScratchError>>
where
    S: RangedSource,
    W: SequentialSink,
    T: RandomAccessScratch,
    C: Cancellation,
{
    let prepare = |error| {
        failed(
            Type0ScratchStage::Prepare,
            Type0ScratchErrorKind::Store(error),
            *report,
        )
    };
    settings.limits.validate().map_err(prepare)?;
    if budget.max_bytes > MAX_BUDGET_COUNT || budget.max_work_bytes > MAX_BUDGET_COUNT {
        return Err(prepare(Error::InvalidInput {
            reason: "type-0 row-storage budgets exceed MAX_BUDGET_COUNT",
        }));
    }
    let stride = len_u64(checked.dib_stride);
    if checked.width == 0
        || checked.height == 0
        || stride != u64::from(checked.width).div_ceil(32) * 4
        || len_u64(checked.visible_bytes) != u64::from(checked.width).div_ceil(8)
    {
        return Err(prepare(Error::InvalidInput {
            reason: "type-0 row-storage geometry differs from checked DIB dimensions",
        }));
    }
    let padded_width = stride * 8;
    let length = stride * u64::from(checked.height);
    for (resource, attempted, maximum) in [
        ("PDF image width", padded_width, i32::MAX as u64),
        ("PDF image stream bytes", length, i32::MAX as u64),
        ("type-0 row-storage bytes", length, budget.max_bytes),
        (
            "type-0 row-storage output bytes",
            length,
            settings.limits.max_output_bytes,
        ),
    ] {
        if attempted > maximum {
            return Err(prepare(Error::LimitExceeded {
                resource,
                limit: maximum,
                attempted,
            }));
        }
    }
    if settings.cancellation.is_cancelled() {
        return Err(prepare(Error::Cancelled));
    }
    scratch.set_len(0).await.map_err(prepare)?;
    check_size(scratch, 0).map_err(prepare)?;
    scratch.set_len(length).await.map_err(prepare)?;
    check_size(scratch, length).map_err(prepare)?;
    report.peak_scratch_bytes = length;
    {
        let mut rows = ReversedRows {
            scratch,
            report,
            budget,
            cancellation: settings.cancellation,
            stride,
            length,
            position: 0,
        };
        decode_type0_rows(source, image, checked, contexts, &mut rows, settings)
            .await
            .map_err(|error| {
                failed(
                    Type0ScratchStage::Decode,
                    Type0ScratchErrorKind::Decode(error),
                    *rows.report,
                )
            })?;
    }
    let chunk = length.min(len_u64(settings.limits.io_chunk_bytes)) as usize;
    let mut buffer = copy_buffer(chunk, settings.limits, *report)?;
    report.copy_buffer_bytes = buffer.capacity();
    let mut rows = document
        .begin_bilevel_image(BilevelImageSpec {
            pixel_width: padded_width as u32,
            pixel_height: checked.height,
            row_stride: checked.dib_stride,
        })
        .await
        .map_err(|error| {
            failed(
                Type0ScratchStage::Emit,
                Type0ScratchErrorKind::Pdf(error),
                *report,
            )
        })?;
    let mut position = 0_u64;
    let mut written = 0;
    while position < length {
        if settings.cancellation.is_cancelled() {
            return Err(failed(
                Type0ScratchStage::Emit,
                Type0ScratchErrorKind::Store(Error::Cancelled),
                *report,
            ));
        }
        check_size(scratch, length).map_err(|error| {
            failed(
                Type0ScratchStage::Emit,
                Type0ScratchErrorKind::Store(error),
                *report,
            )
        })?;
        let count = (length - position).min(len_u64(buffer.len())) as usize;
        charge_request(report, budget, count).map_err(|error| {
            failed(
                Type0ScratchStage::Emit,
                Type0ScratchErrorKind::Store(error),
                *report,
            )
        })?;
        report.read_calls += 1;
        let read = scratch
            .read_at(position, &mut buffer[..count])
            .await
            .map_err(|error| {
                failed(
                    Type0ScratchStage::Emit,
                    Type0ScratchErrorKind::Store(error),
                    *report,
                )
            })?;
        if read > count {
            return Err(failed(
                Type0ScratchStage::Emit,
                Type0ScratchErrorKind::Store(Error::InvalidInput {
                    reason: "type-0 row storage overreported a read",
                }),
                *report,
            ));
        }
        if read == 0 {
            return Err(failed(
                Type0ScratchStage::Emit,
                Type0ScratchErrorKind::Store(Error::TruncatedInput {
                    offset: position,
                    expected: len_u64(count),
                    available: 0,
                }),
                *report,
            ));
        }
        report.scratch_read_bytes += len_u64(read);
        check_size(scratch, length).map_err(|error| {
            failed(
                Type0ScratchStage::Emit,
                Type0ScratchErrorKind::Store(error),
                *report,
            )
        })?;
        write_all(
            &mut rows,
            &buffer[..read],
            &mut written,
            settings.limits,
            settings.cancellation,
        )
        .await
        .map_err(|error| {
            failed(
                Type0ScratchStage::Emit,
                Type0ScratchErrorKind::Pdf(error),
                *report,
            )
        })?;
        position += len_u64(read);
    }
    rows.finish().await.map_err(|error| {
        failed(
            Type0ScratchStage::Emit,
            Type0ScratchErrorKind::Pdf(error),
            *report,
        )
    })
}

/// Owns the supplied scratch for this call, truncating previous contents and
/// attempting a verified reset after every completed success/error. If this
/// future is dropped, the caller must dispose/reset its exclusive workspace;
/// async cleanup cannot run from Drop. The caller also discards partial PDF
/// output on any error, including a cleanup-only failure.
#[allow(clippy::too_many_arguments)]
pub(super) async fn emit_padded_type0_xobject<S, W, T, C>(
    source: &mut S,
    document: &mut PdfDocument<'_, W, C>,
    image: ImageRecord,
    checked: Type0Info,
    contexts: &mut crate::qm::ContextBank,
    scratch: &mut T,
    budget: Type0ScratchBudget,
    settings: &Type0DecodeSettings<'_, C>,
) -> Result<(ImageObject, Type0ScratchReport), Box<Type0ScratchError>>
where
    S: RangedSource,
    W: SequentialSink,
    T: RandomAccessScratch,
    C: Cancellation,
{
    let mut report = Type0ScratchReport::default();
    let result = emit_inner(
        source,
        document,
        image,
        checked,
        contexts,
        scratch,
        budget,
        settings,
        &mut report,
    )
    .await;
    // Cleanup intentionally ignores the cancellation flag and work ceiling.
    let cleanup = match scratch.set_len(0).await {
        Ok(()) => check_size(scratch, 0),
        Err(error) => Err(error),
    };
    match (result, cleanup) {
        (Ok(image), Ok(())) => Ok((image, report)),
        (Ok(_), Err(error)) => Err(failed(
            Type0ScratchStage::Cleanup,
            Type0ScratchErrorKind::Store(error),
            report,
        )),
        (Err(mut error), cleanup) => {
            error.cleanup_error = cleanup.err();
            error.report = report;
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Limits;
    use crate::hnc8::Span;
    use crate::jbig1::{Type0Budget, read_type0_info};
    use crate::pdf::PageSpec;
    use crate::qm::{ArithmeticBudget, ContextBank, QM_STATE_COUNT, QmState, QmTable};
    use crate::test_support::{CancelAfter, NEVER, ready};
    use std::{cell::Cell, error::Error as _, io, rc::Rc};

    struct Source(Vec<u8>);

    impl RangedSource for Source {
        fn size(&self) -> u64 {
            self.0.len() as u64
        }

        async fn read_at(&mut self, offset: u64, bytes: &mut [u8]) -> crate::Result<usize> {
            let count = bytes
                .len()
                .min(self.0.len().saturating_sub(offset as usize))
                .min(3);
            bytes[..count].copy_from_slice(&self.0[offset as usize..offset as usize + count]);
            Ok(count)
        }
    }

    #[derive(Default)]
    struct Sink {
        bytes: Vec<u8>,
        fail: Rc<Cell<bool>>,
        fail_endstream: bool,
    }

    impl SequentialSink for Sink {
        async fn write(&mut self, bytes: &[u8]) -> crate::Result<usize> {
            if self.fail.get() {
                return Err(injected());
            }
            if self.fail_endstream && bytes.starts_with(b"\nen") {
                return Err(injected());
            }
            let count = bytes.len().min(3);
            self.bytes.extend_from_slice(&bytes[..count]);
            Ok(count)
        }

        async fn flush(&mut self) -> crate::Result<()> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct Scratch {
        bytes: Vec<u8>,
        snapshot: Vec<u8>,
        short: usize,
        len_calls: usize,
        fail_len_at: Option<usize>,
        wrong_len_at: Option<usize>,
        fail_size: bool,
        size_calls: Cell<usize>,
        fail_size_at: Option<usize>,
        read_fault: u8,
        write_fault: u8,
        fail_flush: bool,
        cancel_on_write: Option<Rc<Cell<bool>>>,
    }

    fn injected() -> Error {
        Error::Io(io::Error::other("original synthetic storage failure"))
    }

    impl RandomAccessScratch for Scratch {
        fn size(&self) -> crate::Result<u64> {
            self.size_calls.set(self.size_calls.get() + 1);
            if self.fail_size || self.fail_size_at == Some(self.size_calls.get()) {
                return Err(injected());
            }
            Ok(self.bytes.len() as u64)
        }

        async fn set_len(&mut self, length: u64) -> crate::Result<()> {
            self.len_calls += 1;
            if self.fail_len_at == Some(self.len_calls) {
                return Err(injected());
            }
            if length == 0 && !self.bytes.is_empty() {
                self.snapshot.clone_from(&self.bytes);
            }
            let length = length as usize + usize::from(self.wrong_len_at == Some(self.len_calls));
            self.bytes.resize(length, 0);
            Ok(())
        }

        async fn read_at(&mut self, offset: u64, output: &mut [u8]) -> crate::Result<usize> {
            match self.read_fault {
                1 => return Ok(0),
                2 => return Ok(output.len() + 1),
                3 => return Err(injected()),
                _ => {}
            }
            let count = output.len().min(self.short.max(1));
            output[..count].copy_from_slice(&self.bytes[offset as usize..offset as usize + count]);
            if self.read_fault == 4 {
                self.bytes.push(0);
            }
            Ok(count)
        }

        async fn write_at(&mut self, offset: u64, bytes: &[u8]) -> crate::Result<usize> {
            if self.write_fault == 5 {
                std::future::pending::<()>().await;
            }
            match self.write_fault {
                1 => return Ok(0),
                2 => return Ok(bytes.len() + 1),
                3 => return Err(injected()),
                _ => {}
            }
            let count = bytes.len().min(self.short.max(1));
            self.bytes[offset as usize..offset as usize + count].copy_from_slice(&bytes[..count]);
            if self.write_fault == 4 {
                self.bytes.push(0);
            }
            if let Some(flag) = &self.cancel_on_write {
                flag.set(true);
            }
            Ok(count)
        }

        async fn flush(&mut self) -> crate::Result<()> {
            if self.fail_flush {
                return Err(injected());
            }
            Ok(())
        }
    }

    struct Flag(Rc<Cell<bool>>);
    impl Cancellation for Flag {
        fn is_cancelled(&self) -> bool {
            self.0.get()
        }
    }

    fn image() -> (Source, ImageRecord) {
        let mut bytes = vec![0; 48];
        bytes[..4].copy_from_slice(&40_u32.to_le_bytes());
        bytes[4..8].copy_from_slice(&1_u32.to_le_bytes());
        bytes[8..12].copy_from_slice(&2_u32.to_le_bytes());
        bytes[12..14].copy_from_slice(&1_u16.to_le_bytes());
        bytes[14..16].copy_from_slice(&1_u16.to_le_bytes());
        bytes[40..43].fill(0xff);
        // Invented stationary Qe=0x4000: control0/pixel1 then control0/pixel0.
        // This gives independently hand-derived top rows [0x80,0,0,0]/[0;4].
        bytes.extend_from_slice(&[0x80, 0, 0]);
        let record = ImageRecord {
            page_number: 7,
            image_number: 3,
            descriptor_offset: 123,
            record_type: 0,
            payload: Span {
                offset: 0,
                length: bytes.len() as u64,
            },
        };
        (Source(bytes), record)
    }

    fn table() -> QmTable {
        QmTable::new(vec![
            QmState {
                qe: 0x4000,
                next_lps: 0,
                next_mps: 0,
                switch_mps: false
            };
            QM_STATE_COUNT
        ])
        .unwrap()
    }

    fn settings<'a, C>(
        table: &'a QmTable,
        limits: &'a Limits,
        cancellation: &'a C,
    ) -> Type0DecodeSettings<'a, C> {
        Type0DecodeSettings {
            table,
            arithmetic: ArithmeticBudget {
                max_symbols: 100,
                max_work: 3000,
            },
            image: Type0Budget::default(),
            limits,
            cancellation,
        }
    }

    fn info<S: RangedSource, C: Cancellation>(
        source: &mut S,
        record: ImageRecord,
        settings: &Type0DecodeSettings<'_, C>,
    ) -> Type0Info {
        ready(read_type0_info(
            source,
            record.type0_span().unwrap(),
            settings.limits,
            settings.cancellation,
            settings.arithmetic,
            settings.image,
        ))
        .unwrap()
    }

    fn run(
        scratch: &mut Scratch,
        budget: Type0ScratchBudget,
        alter: impl FnOnce(&mut Type0Info, &mut Source),
        fail_pdf: bool,
    ) -> Result<Type0ScratchReport, Box<Type0ScratchError>> {
        let (mut source, record) = image();
        let limits = Limits {
            io_chunk_bytes: 3,
            ..Limits::default()
        };
        let table = table();
        let settings = settings(&table, &limits, &NEVER);
        let mut checked = info(&mut source, record, &settings);
        alter(&mut checked, &mut source);
        let mut contexts = ContextBank::new(1024, &limits).unwrap();
        let mut sink = Sink::default();
        let sink_failure = Rc::clone(&sink.fail);
        let mut document = ready(PdfDocument::new(&mut sink, &limits, &NEVER)).unwrap();
        sink_failure.set(fail_pdf);
        ready(emit_padded_type0_xobject(
            &mut source,
            &mut document,
            record,
            checked,
            &mut contexts,
            scratch,
            budget,
            &settings,
        ))
        .map(|(_, report)| report)
    }

    #[test]
    fn preparation_checks_geometry_pdf_ranges_and_independent_store_ceilings() {
        for alter in 0..6 {
            let mut scratch = Scratch::default();
            let error = run(
                &mut scratch,
                Type0ScratchBudget::default(),
                |info, _| match alter {
                    0 => info.width = 0,
                    1 => info.height = 0,
                    2 => info.dib_stride = 3,
                    3 => info.visible_bytes = 0,
                    4 => {
                        info.width = u32::MAX;
                        info.dib_stride = u32::MAX as usize / 32 * 4 + 4;
                        info.visible_bytes = u32::MAX as usize / 8 + 1;
                    }
                    _ => info.height = u32::MAX,
                },
                false,
            )
            .unwrap_err();
            assert_eq!(error.stage, Type0ScratchStage::Prepare);
            assert_eq!(error.report.peak_scratch_bytes, 0);
            assert_eq!(
                scratch.len_calls, 1,
                "only cleanup may resize an invalid request"
            );
            assert!(scratch.bytes.is_empty());
        }
        for budget in [
            Type0ScratchBudget {
                max_bytes: 7,
                ..Type0ScratchBudget::default()
            },
            Type0ScratchBudget {
                max_bytes: MAX_BUDGET_COUNT + 1,
                ..Type0ScratchBudget::default()
            },
            Type0ScratchBudget {
                max_work_bytes: MAX_BUDGET_COUNT + 1,
                ..Type0ScratchBudget::default()
            },
        ] {
            let mut scratch = Scratch {
                bytes: vec![0x5a; 4],
                ..Scratch::default()
            };
            let error = run(&mut scratch, budget, |_, _| {}, false).unwrap_err();
            assert_eq!(error.stage, Type0ScratchStage::Prepare);
            assert!(scratch.bytes.is_empty());
        }
    }

    #[test]
    fn storage_work_refusals_count_requested_retries_in_both_phases() {
        for (max_work_bytes, stage) in [
            (1, Type0ScratchStage::Decode),
            (18, Type0ScratchStage::Emit),
        ] {
            let mut scratch = Scratch::default();
            let error = run(
                &mut scratch,
                Type0ScratchBudget {
                    max_work_bytes,
                    ..Type0ScratchBudget::default()
                },
                |_, _| {},
                false,
            )
            .unwrap_err();
            assert_eq!(error.stage, stage);
            assert!(error.report.scratch_work_bytes <= max_work_bytes);
            assert_eq!(error.report.peak_scratch_bytes, 8);
            if stage == Type0ScratchStage::Emit {
                assert_eq!(error.report.scratch_write_bytes, 8);
            }
            assert!(scratch.bytes.is_empty());
        }
    }

    #[test]
    fn malformed_or_failed_extent_operations_refuse_and_attempt_cleanup() {
        for call in [1, 2, 3] {
            for wrong in [false, true] {
                let mut scratch = Scratch::default();
                if wrong {
                    scratch.wrong_len_at = Some(call);
                } else {
                    scratch.fail_len_at = Some(call);
                }
                let error = run(
                    &mut scratch,
                    Type0ScratchBudget::default(),
                    |_, _| {},
                    false,
                )
                .unwrap_err();
                assert_eq!(
                    error.stage,
                    if call == 3 {
                        Type0ScratchStage::Cleanup
                    } else {
                        Type0ScratchStage::Prepare
                    }
                );
                assert!(error.source().is_some());
                assert!(error.to_string().contains("row storage"));
                if call < 3 {
                    assert!(scratch.bytes.is_empty());
                }
            }
        }
        let mut scratch = Scratch {
            fail_size: true,
            ..Scratch::default()
        };
        let error = run(
            &mut scratch,
            Type0ScratchBudget::default(),
            |_, _| {},
            false,
        )
        .unwrap_err();
        assert_eq!(error.stage, Type0ScratchStage::Prepare);
        assert!(error.cleanup_error.is_some());
        assert!(error.to_string().contains("cleanup also failed"));
    }

    #[test]
    fn scratch_decode_failures_preserve_location_and_secondary_cleanup_failure() {
        for write_fault in [1, 2, 3, 4] {
            let mut scratch = Scratch {
                write_fault,
                ..Scratch::default()
            };
            let error = run(
                &mut scratch,
                Type0ScratchBudget::default(),
                |_, _| {},
                false,
            )
            .unwrap_err();
            assert_eq!(error.stage, Type0ScratchStage::Decode);
            let inner = error
                .source()
                .unwrap()
                .downcast_ref::<Type0PdfError>()
                .expect("typed decoder error remains the source");
            assert_eq!((inner.page, inner.image), (Some(7), Some(3)));
            assert!(inner.offset.is_some());
            assert!(error.to_string().contains("page 7, image 3"));
            assert!(scratch.bytes.is_empty());
        }
        let mut scratch = Scratch {
            write_fault: 3,
            fail_len_at: Some(3),
            ..Scratch::default()
        };
        let error = run(
            &mut scratch,
            Type0ScratchBudget::default(),
            |_, _| {},
            false,
        )
        .unwrap_err();
        assert_eq!(error.stage, Type0ScratchStage::Decode);
        assert!(error.cleanup_error.is_some());
        assert_eq!(
            scratch.bytes.len(),
            8,
            "caller disposes storage after cleanup refusal"
        );
        let mut scratch = Scratch {
            fail_flush: true,
            ..Scratch::default()
        };
        let error = run(
            &mut scratch,
            Type0ScratchBudget::default(),
            |_, _| {},
            false,
        )
        .unwrap_err();
        assert_eq!(error.stage, Type0ScratchStage::Decode);
        assert_eq!(error.report.scratch_write_bytes, 8);
        assert!(scratch.bytes.is_empty());
    }

    #[test]
    fn changed_wrapper_is_rejected_before_any_row_or_image_bytes() {
        let mut scratch = Scratch::default();
        let error = run(
            &mut scratch,
            Type0ScratchBudget::default(),
            |_, source| source.0[4..8].copy_from_slice(&2_u32.to_le_bytes()),
            false,
        )
        .unwrap_err();
        assert_eq!(error.stage, Type0ScratchStage::Decode);
        let inner = error
            .source()
            .unwrap()
            .downcast_ref::<Type0PdfError>()
            .expect("typed wrapper error remains the source");
        assert_eq!(inner.offset, Some(0));
        assert!(
            inner
                .to_string()
                .contains("DIB wrapper that changed between reads")
        );
        assert_eq!(error.report.scratch_write_bytes, 0);
        assert!(scratch.bytes.is_empty());
    }

    #[test]
    fn readback_short_zero_overreported_io_and_size_change_errors_are_typed() {
        for read_fault in [1, 2, 3, 4] {
            let mut scratch = Scratch {
                read_fault,
                ..Scratch::default()
            };
            let error = run(
                &mut scratch,
                Type0ScratchBudget::default(),
                |_, _| {},
                false,
            )
            .unwrap_err();
            assert_eq!(error.stage, Type0ScratchStage::Emit);
            assert!(matches!(error.kind, Type0ScratchErrorKind::Store(_)));
            assert_eq!(error.report.scratch_write_bytes, 8);
            assert!(scratch.bytes.is_empty());
        }
        let mut scratch = Scratch::default();
        let error = run(&mut scratch, Type0ScratchBudget::default(), |_, _| {}, true).unwrap_err();
        assert_eq!(error.stage, Type0ScratchStage::Emit);
        assert!(matches!(error.kind, Type0ScratchErrorKind::Pdf(_)));
        assert!(error.to_string().contains("PDF output"));
        assert!(scratch.bytes.is_empty());
    }

    #[test]
    fn cancellation_before_decode_or_after_accepted_row_still_resets_storage() {
        for during_write in [false, true] {
            let (mut source, record) = image();
            let limits = Limits::default();
            let table = table();
            let flag = Rc::new(Cell::new(false));
            let cancel = Flag(Rc::clone(&flag));
            let settings = settings(&table, &limits, &cancel);
            let checked = info(&mut source, record, &settings);
            let mut contexts = ContextBank::new(1024, &limits).unwrap();
            let mut sink = Sink::default();
            let mut document = ready(PdfDocument::new(&mut sink, &limits, &cancel)).unwrap();
            let mut scratch = Scratch::default();
            if during_write {
                scratch.cancel_on_write = Some(Rc::clone(&flag));
            } else {
                flag.set(true);
            }
            let error = ready(emit_padded_type0_xobject(
                &mut source,
                &mut document,
                record,
                checked,
                &mut contexts,
                &mut scratch,
                Type0ScratchBudget::default(),
                &settings,
            ))
            .unwrap_err();
            assert_eq!(
                error.stage,
                if during_write {
                    Type0ScratchStage::Decode
                } else {
                    Type0ScratchStage::Prepare
                }
            );
            assert!(error.cleanup_error.is_none());
            assert!(scratch.bytes.is_empty());
            if during_write {
                assert_eq!(error.report.scratch_write_bytes, 1);
            }
        }
    }

    #[test]
    fn dropped_future_leaves_storage_for_the_callers_raii_or_explicit_reset() {
        use std::{
            future::Future,
            pin::pin,
            task::{Context, Poll, Waker},
        };
        let (mut source, record) = image();
        let limits = Limits::default();
        let table = table();
        let settings = settings(&table, &limits, &NEVER);
        let checked = info(&mut source, record, &settings);
        let mut contexts = ContextBank::new(1024, &limits).unwrap();
        let mut sink = Sink::default();
        let mut document = ready(PdfDocument::new(&mut sink, &limits, &NEVER)).unwrap();
        let mut scratch = Scratch {
            write_fault: 5,
            ..Scratch::default()
        };
        {
            let mut future = pin!(emit_padded_type0_xobject(
                &mut source,
                &mut document,
                record,
                checked,
                &mut contexts,
                &mut scratch,
                Type0ScratchBudget::default(),
                &settings
            ));
            assert!(matches!(
                future
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop())),
                Poll::Pending
            ));
        }
        assert_eq!((scratch.bytes.len(), scratch.len_calls), (8, 2));
        ready(scratch.set_len(0)).unwrap();
        assert!(scratch.bytes.is_empty());
    }

    #[test]
    fn checked_copy_buffer_refuses_real_capacity_overflow_without_allocating() {
        let report = Type0ScratchReport::default();
        let limits = Limits {
            max_allocation_bytes: u64::MAX,
            ..Limits::default()
        };
        let error = copy_buffer(usize::MAX, &limits, report).unwrap_err();
        assert_eq!(error.stage, Type0ScratchStage::Emit);
        assert!(
            matches!(error.kind, Type0ScratchErrorKind::Store(Error::LimitExceeded { resource: "type-0 row-storage copy bytes", attempted, .. }) if attempted == usize::MAX as u64)
        );
        let limits = Limits {
            max_allocation_bytes: 2,
            ..Limits::default()
        };
        assert!(copy_buffer(3, &limits, report).is_err());
        assert_eq!(copy_buffer(2, &limits, report).unwrap(), [0, 0]);
    }

    #[test]
    fn every_cancellation_checkpoint_refuses_and_completed_calls_reset_storage() {
        fn attempt(cancel: &CancelAfter) -> Result<Type0ScratchReport, Box<Type0ScratchError>> {
            let (mut source, record) = image();
            let limits = Limits {
                io_chunk_bytes: 3,
                ..Limits::default()
            };
            let table = table();
            let checked = info(&mut source, record, &settings(&table, &limits, &NEVER));
            let settings = settings(&table, &limits, cancel);
            let mut contexts = ContextBank::new(1024, &limits).unwrap();
            let mut sink = Sink::default();
            let mut scratch = Scratch::default();
            // Constructor cancellation precedes ownership of the workspace.
            let mut document =
                ready(PdfDocument::new(&mut sink, &limits, cancel)).map_err(|error| {
                    failed(
                        Type0ScratchStage::Prepare,
                        Type0ScratchErrorKind::Pdf(error),
                        Type0ScratchReport::default(),
                    )
                })?;
            let result = ready(emit_padded_type0_xobject(
                &mut source,
                &mut document,
                record,
                checked,
                &mut contexts,
                &mut scratch,
                Type0ScratchBudget::default(),
                &settings,
            ));
            assert!(scratch.bytes.is_empty());
            result.map(|(_, report)| report)
        }
        let counter = CancelAfter::never();
        attempt(&counter).unwrap();
        let mut readback_cancelled = false;
        for allowed in 0..counter.queries() {
            let error = attempt(&CancelAfter::new(allowed)).unwrap_err();
            readback_cancelled |= error.stage == Type0ScratchStage::Emit
                && matches!(error.kind, Type0ScratchErrorKind::Store(Error::Cancelled));
        }
        assert!(
            readback_cancelled,
            "cancellation immediately before readback is tested"
        );
    }

    #[test]
    fn metadata_read_errors_before_readback_are_typed_and_reset_storage() {
        let mut baseline = Scratch::default();
        run(
            &mut baseline,
            Type0ScratchBudget::default(),
            |_, _| {},
            false,
        )
        .unwrap();
        let mut reached_readback = false;
        for at in 1..=baseline.size_calls.get() {
            let mut scratch = Scratch {
                fail_size_at: Some(at),
                ..Scratch::default()
            };
            let error = run(
                &mut scratch,
                Type0ScratchBudget::default(),
                |_, _| {},
                false,
            )
            .unwrap_err();
            reached_readback |= error.stage == Type0ScratchStage::Emit
                && error.report.scratch_write_bytes == 8
                && error.report.scratch_read_bytes == 0;
            assert!(scratch.bytes.is_empty());
        }
        assert!(reached_readback);
    }

    #[test]
    fn pdf_stream_closure_failure_is_primary_and_store_still_resets() {
        let (mut source, record) = image();
        let limits = Limits {
            io_chunk_bytes: 3,
            ..Limits::default()
        };
        let table = table();
        let settings = settings(&table, &limits, &NEVER);
        let checked = info(&mut source, record, &settings);
        let mut contexts = ContextBank::new(1024, &limits).unwrap();
        let mut scratch = Scratch::default();
        let mut sink = Sink {
            fail_endstream: true,
            ..Sink::default()
        };
        let mut document = ready(PdfDocument::new(&mut sink, &limits, &NEVER)).unwrap();
        let error = ready(emit_padded_type0_xobject(
            &mut source,
            &mut document,
            record,
            checked,
            &mut contexts,
            &mut scratch,
            Type0ScratchBudget::default(),
            &settings,
        ))
        .unwrap_err();
        assert_eq!(error.stage, Type0ScratchStage::Emit);
        assert!(matches!(
            error.kind,
            Type0ScratchErrorKind::Pdf(Error::Io(_))
        ));
        assert_eq!(
            (
                error.report.scratch_read_bytes,
                error.report.scratch_write_bytes
            ),
            (8, 8)
        );
        assert!(scratch.bytes.is_empty());
        assert!(error.source().unwrap().downcast_ref::<Error>().is_some());
    }

    #[test]
    fn diagnostic_formatting_propagates_a_refused_output_write() {
        #[derive(Default)]
        struct FormatSink {
            accepted_calls: usize,
            refuse_after: Option<usize>,
        }
        impl fmt::Write for FormatSink {
            fn write_str(&mut self, _: &str) -> fmt::Result {
                if self.refuse_after == Some(self.accepted_calls) {
                    return Err(fmt::Error);
                }
                self.accepted_calls += 1;
                Ok(())
            }
        }
        let error = failed(
            Type0ScratchStage::Prepare,
            Type0ScratchErrorKind::Store(Error::UnsupportedFormat),
            Type0ScratchReport::default(),
        );
        let mut count = FormatSink::default();
        fmt::write(&mut count, format_args!("{error}")).unwrap();
        for refuse_after in 0..count.accepted_calls {
            let mut sink = FormatSink {
                refuse_after: Some(refuse_after),
                ..FormatSink::default()
            };
            assert!(fmt::write(&mut sink, format_args!("{error}")).is_err());
        }
    }

    #[test]
    fn padded_rows_keep_every_bit_and_reverse_asymmetric_display_order() {
        let (mut source, record) = image();
        let limits = Limits {
            io_chunk_bytes: 3,
            ..Limits::default()
        };
        let table = table();
        let settings = settings(&table, &limits, &NEVER);
        let checked = info(&mut source, record, &settings);
        let mut contexts = ContextBank::new(1024, &limits).unwrap();
        let mut scratch = Scratch {
            short: 2,
            ..Scratch::default()
        };
        let mut sink = Sink::default();
        let mut document = ready(PdfDocument::new(&mut sink, &limits, &NEVER)).unwrap();
        let (object, report) = ready(emit_padded_type0_xobject(
            &mut source,
            &mut document,
            record,
            checked,
            &mut contexts,
            &mut scratch,
            Type0ScratchBudget::default(),
            &settings,
        ))
        .unwrap();
        ready(document.add_page(
            PageSpec {
                width_points: 32.0,
                height_points: 2.0,
            },
            &[object],
        ))
        .unwrap();
        ready(document.finish()).unwrap();
        assert!(scratch.bytes.is_empty());
        assert_eq!(scratch.snapshot, [0, 0, 0, 0, 0x80, 0, 0, 0]);
        assert_eq!(report.peak_scratch_bytes, 8);
        assert_eq!(
            (report.scratch_read_bytes, report.scratch_write_bytes),
            (8, 8)
        );
        assert!(report.max_request_bytes <= 3);
        assert!(report.copy_buffer_bytes <= 3);
        assert!(sink.bytes.windows(9).any(|bytes| bytes == b"/Width 32"));
        let start = sink
            .bytes
            .windows(7)
            .position(|bytes| bytes == b"stream\n")
            .unwrap()
            + 7;
        assert_eq!(&sink.bytes[start..start + 8], scratch.snapshot);
    }

    #[test]
    fn reversal_handles_joined_rows_short_writes_and_keeps_padding_bytes() {
        let mut scratch = Scratch {
            bytes: vec![0; 12],
            short: 2,
            ..Scratch::default()
        };
        let mut report = Type0ScratchReport::default();
        let bytes = [
            0x80, 0x37, 0x44, 0x55, 0x40, 0x66, 0x77, 0x88, 0x60, 0x99, 0xaa, 0xbb,
        ];
        let mut rows = ReversedRows {
            scratch: &mut scratch,
            report: &mut report,
            budget: Type0ScratchBudget::default(),
            cancellation: &NEVER,
            stride: 4,
            length: 12,
            position: 0,
        };
        assert_eq!(ready(rows.write(&[])).unwrap(), 0);
        ready(write_all(
            &mut rows,
            &bytes,
            &mut 0,
            &Limits {
                io_chunk_bytes: 7,
                ..Limits::default()
            },
            &NEVER,
        ))
        .unwrap();
        assert!(ready(rows.write(&[1])).is_err());
        ready(rows.flush()).unwrap();
        assert_eq!(
            scratch.bytes,
            [
                0x60, 0x99, 0xaa, 0xbb, 0x40, 0x66, 0x77, 0x88, 0x80, 0x37, 0x44, 0x55
            ]
        );
    }
}
