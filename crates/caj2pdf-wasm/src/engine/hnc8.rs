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
        None
    } else {
        Some(QmTable::new(tables.qm).map_err(|_| Error::InvalidInput {
            reason: "incomplete caller QM state table",
        })?)
    };
    let mq = if tables.mq.is_empty() {
        None
    } else {
        Some(
            MqTable::new(tables.mq, limits).map_err(|_| Error::InvalidInput {
                reason: "invalid or incomplete caller MQ state table",
            })?,
        )
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
        type3: mq.as_ref().map(|table| ComposeType3Workspaces {
            table,
            first,
            second,
            refined,
        }),
    };
    convert_source_pages_pdf(
        source,
        sink,
        qm.as_ref(),
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
