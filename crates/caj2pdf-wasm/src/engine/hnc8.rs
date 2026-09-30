// SPDX-License-Identifier: MIT

use super::*;
use caj2pdf_core::{
    hnc8::{
        ComposeOptions, ComposePage, ComposeType3Workspaces, ComposeVisitor, ComposeWorkspaces,
        convert_source_pages_pdf,
    },
    jbig2::mq::{MQ_STATE_COUNT, MqState, MqTable},
    qm::{QM_STATE_COUNT, QmState, QmTable},
};

#[derive(Default)]
pub(super) struct Tables {
    qm: Vec<QmState>,
    mq: Vec<MqState>,
}

impl Tables {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn push(
        &mut self,
        table: u32,
        qe: u32,
        next_lps: u32,
        next_mps: u32,
        switch: u32,
        limits: &Limits,
    ) -> bool {
        let count = match table {
            0 => QM_STATE_COUNT,
            1 => MQ_STATE_COUNT,
            _ => return false,
        };
        if qe == 0
            || qe >= 0x8000
            || next_lps >= count as u32
            || next_mps >= count as u32
            || switch > 1
        {
            return false;
        }
        if table == 0 {
            if self.qm.len() == count || !reserve(&mut self.qm, count, limits) {
                return false;
            }
            self.qm.push(QmState {
                qe: qe as u16,
                next_lps: next_lps as u8,
                next_mps: next_mps as u8,
                switch_mps: switch != 0,
            });
        } else {
            if self.mq.len() == count || !reserve(&mut self.mq, count, limits) {
                return false;
            }
            self.mq.push(MqState {
                qe: qe as u16,
                next_lps: next_lps as u8,
                next_mps: next_mps as u8,
                switch_mps: switch != 0,
            });
        }
        true
    }
}

fn reserve<T>(states: &mut Vec<T>, count: usize, limits: &Limits) -> bool {
    !states.is_empty()
        || (limits
            .check_allocation((count * std::mem::size_of::<T>()) as u64)
            .is_ok()
            && states.try_reserve_exact(count).is_ok())
}

struct CompletePages;
impl ComposeVisitor for CompletePages {
    async fn page(&mut self, page: ComposePage<'_>) -> Result<()> {
        if page.output_page.is_none() {
            return Err(Error::InvalidInput {
                reason: "HN/C8 conversion cannot omit source pages without image content",
            });
        }
        Ok(())
    }
}

pub(super) async fn convert(
    source: &mut BridgeSource,
    sink: &mut BridgeSink,
    options: ConversionOptions,
    limits: &Limits,
    cancellation: &BridgeCancellation,
) -> Result<ConversionReport> {
    let tables = std::mem::take(&mut source.shared.borrow_mut().tables);
    let qm = if tables.qm.is_empty() {
        QmTable::standard()
    } else {
        QmTable::new(tables.qm).map_err(|_| Error::InvalidInput {
            reason: "incomplete caller QM state table",
        })?
    };
    let mq = if tables.mq.is_empty() {
        MqTable::standard()
    } else {
        MqTable::new(tables.mq, limits).map_err(|error| match error.kind {
            caj2pdf_core::jbig2::mq::MqErrorKind::Source(error) => error,
            _ => Error::InvalidInput {
                reason: "invalid or incomplete MQ state table",
            },
        })?
    };
    let options = ComposeOptions {
        include_bookmarks: options.include_bookmarks,
        ..Default::default()
    };
    let mut stores = std::array::from_fn::<_, 4, _>(|index| {
        scratch::Scratch::new(
            Rc::clone(&source.shared),
            index as u32 + 1,
            options.budget.max_row_store_bytes,
        )
    });
    let [rows, first, second, refined] = &mut stores;
    let workspaces = ComposeWorkspaces {
        rows,
        type3: Some(ComposeType3Workspaces {
            table: &mq,
            first,
            second,
            refined,
        }),
    };
    convert_source_pages_pdf(
        source,
        sink,
        Some(&qm),
        workspaces,
        &mut CompletePages,
        options,
        limits,
        cancellation,
    )
    .await
    .map(|report| report.conversion)
    .map_err(|error| Error::Hnc8(Box::new(error)))
}

struct IgnoreBookmarks;
impl caj2pdf_core::BookmarkVisitor for IgnoreBookmarks {
    async fn visit(&mut self, _: caj2pdf_core::Bookmark) -> Result<()> {
        Ok(())
    }
}

pub(super) async fn inspect<S: RangedSource, C: Cancellation>(
    source: &mut S,
    limits: &Limits,
    cancellation: &C,
) -> Result<(u32, Option<u32>)> {
    use caj2pdf_core::hnc8::{Budget, Hnc8Reader};
    let result: caj2pdf_core::hnc8::Result<_> = async {
        let mut reader = Hnc8Reader::open(source, limits, cancellation, Budget::default()).await?;
        let pages = reader.header().page_count;
        let count = if reader.declared_bookmark_count().is_some() {
            Some(
                reader
                    .visit_bookmarks(64, pages, |page| Some(page - 1), &mut IgnoreBookmarks)
                    .await?,
            )
        } else {
            None
        };
        Ok((pages, count))
    }
    .await;
    result.map_err(|error| Error::Hnc8Metadata(Box::new(error)))
}
