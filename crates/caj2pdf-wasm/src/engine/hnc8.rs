// SPDX-License-Identifier: MIT

use super::*;
use caj2pdf_core::{
    hnc8::{
        ComposeOptions, ComposePage, ComposeType3Workspaces, ComposeVisitor, ComposeWorkspaces,
        Type3PdfOptions, convert_source_pages_pdf,
    },
    jbig2::mq::{MQ_STATE_COUNT, MqState, MqTable},
    jbig2::text::TextHeaderPolicy,
    qm::{QM_STATE_COUNT, QmState, QmTable},
};

#[derive(Default)]
pub(super) struct Fonts {
    sizes: [u64; 8],
    count: usize,
    roles: Option<caj2pdf_core::hnc8::C8PageFonts>,
}
impl Fonts {
    pub(super) fn count(&self) -> usize {
        self.count
    }
    pub(super) fn add(&mut self, size: u64) -> u32 {
        if self.count == self.sizes.len() || self.roles.is_some() {
            return 0;
        }
        self.sizes[self.count] = size;
        self.count += 1;
        self.count as u32
    }
    pub(super) fn set_latin_state(&mut self, state: u32, index: u32) -> bool {
        let Some(roles) = &mut self.roles else {
            return false;
        };
        let role = match state {
            3 => &mut roles.latin_state3,
            28 => &mut roles.latin_state28,
            31 => &mut roles.latin_state31,
            _ => return false,
        };
        if index as usize >= self.count || role.is_some() {
            return false;
        }
        *role = Some(index as usize);
        true
    }

    pub(super) fn set(
        &mut self,
        cjk: u32,
        latin: u32,
        alternate: u32,
        decoration: u32,
        alias: u32,
        symbols: u32,
    ) -> bool {
        // `u32::MAX` marks an absent optional role; core applies its fallback.
        if self.roles.is_some()
            || [cjk, latin]
                .into_iter()
                .any(|index| index as usize >= self.count)
            || (alternate != u32::MAX && alternate as usize >= self.count)
        {
            return false;
        }
        let alternate_latin = (alternate != u32::MAX).then_some(alternate as usize);
        let decoration = if decoration == u32::MAX {
            None
        } else {
            let Some(character) = char::from_u32(alias) else {
                return false;
            };
            if decoration as usize >= self.count || alias > 0xffff {
                return false;
            }
            Some((decoration as usize, character))
        };
        let symbols = if symbols == u32::MAX {
            None
        } else if symbols as usize >= self.count {
            return false;
        } else {
            Some(symbols as usize)
        };
        self.roles = Some(caj2pdf_core::hnc8::C8PageFonts {
            cjk: cjk as usize,
            latin: latin as usize,
            alternate_latin,
            decoration,
            symbols,
            latin_state3: None,
            latin_state28: None,
            latin_state31: None,
        });
        true
    }
}

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
) -> Result<(ConversionReport, OutlineReport)> {
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
        // The HN/C8 profile explicitly admits the measured unused-template
        // anomaly; general JBIG2 APIs and all other malformed flags stay strict.
        type3: Type3PdfOptions {
            text_header_policy: TextHeaderPolicy::HnC8UnusedRefinementTemplate,
            ..Default::default()
        },
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
    let fonts = std::mem::take(&mut source.shared.borrow_mut().fonts);
    if fonts.count != 0 {
        let roles = fonts.roles.ok_or(Error::InvalidInput {
            reason: "C8 font resources require explicit roles",
        })?;
        let mut sources = std::array::from_fn::<_, 8, _>(|index| BridgeSource {
            resource: index as u32 + 1,
            shared: Rc::clone(&source.shared),
            size: fonts.sizes[index],
        });
        return caj2pdf_core::hnc8::convert_c8_native_pdf(
            source,
            sink,
            caj2pdf_core::hnc8::C8FontSources {
                sources: &mut sources[..fonts.count],
                roles,
            },
            Some(&qm),
            workspaces,
            options,
            limits,
            cancellation,
        )
        .await
        .map(|report| (report.conversion, report.outline))
        .map_err(|error| Error::Hnc8(Box::new(error)));
    }
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
    .map(|report| (report.conversion, report.outline))
    .map_err(|error| Error::Hnc8(Box::new(error)))
}

struct IgnoreBookmarks;
impl caj2pdf_core::BookmarkVisitor for IgnoreBookmarks {
    async fn visit(&mut self, _: caj2pdf_core::Bookmark) -> Result<()> {
        Ok(())
    }
}

/// Page count, written bookmark count (`None` when unknown) and the number of
/// skipped or clamped HN-A outline entries.
pub(super) async fn inspect<S: RangedSource, C: Cancellation>(
    source: &mut S,
    limits: &Limits,
    cancellation: &C,
) -> Result<(u32, Option<u32>, u32)> {
    use caj2pdf_core::hnc8::{Budget, Hnc8Reader};
    let result: caj2pdf_core::hnc8::Result<_> = async {
        let mut reader = Hnc8Reader::open(source, limits, cancellation, Budget::default()).await?;
        let pages = reader.header().page_count;
        if reader.declared_bookmark_count().is_none() {
            return Ok((pages, None, 0));
        }
        let outline = reader
            .visit_bookmarks(64, pages, |page| Some(page - 1), &mut IgnoreBookmarks)
            .await?;
        Ok((pages, Some(outline.written), outline.defects))
    }
    .await;
    result.map_err(|error| Error::Hnc8Metadata(Box::new(error)))
}
