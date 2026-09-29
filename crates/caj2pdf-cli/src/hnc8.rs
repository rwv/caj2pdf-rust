// SPDX-License-Identifier: MIT

//! Experimental HN/C8 CLI routing. State files are caller inputs, never bundled.

use crate::{
    CliError,
    args::{ConvertOptions, Endpoint},
    files::{Input, anonymous_file, open_input},
};
use caj2pdf_core::{
    ConversionReport, Error, Limits, NeverCancel, RangedSource, SequentialSink,
    hnc8::{
        ComposeOptions, ComposePage, ComposeType3Workspaces, ComposeVisitor, ComposeWorkspaces,
        convert_source_pages_pdf,
    },
    jbig2::mq::{MqState, MqTable},
    native::FileScratch,
    qm::{QmState, QmTable},
};
use std::{io::Read, path::Path};

const MAX_STATE_BYTES: u64 = 16 * 1024;

#[derive(Default)]
pub struct Tables {
    pub qm: Option<QmTable>,
    pub mq: Option<MqTable>,
    // Retain opened input identities so --force cannot overwrite a state file.
    pub inputs: Vec<Input>,
}

impl Tables {
    pub fn load(options: &ConvertOptions, limits: &Limits) -> Result<Self, CliError> {
        let mut tables = Self::default();
        if let Some(path) = &options.qm_states {
            let rows = tables.read(path, 113)?;
            tables.qm = Some(
                QmTable::new(
                    rows.into_iter()
                        .map(|[qe, lps, mps, switch]| QmState {
                            qe,
                            next_lps: lps as u8,
                            next_mps: mps as u8,
                            switch_mps: switch != 0,
                        })
                        .collect(),
                )
                .map_err(|e| {
                    CliError::runtime(format!("invalid codec states '{}': {e}", path.display()))
                })?,
            );
        }
        if let Some(path) = &options.mq_states {
            let rows = tables.read(path, 47)?;
            tables.mq = Some(
                MqTable::new(
                    rows.into_iter()
                        .map(|[qe, lps, mps, switch]| MqState {
                            qe,
                            next_lps: lps as u8,
                            next_mps: mps as u8,
                            switch_mps: switch != 0,
                        })
                        .collect(),
                    limits,
                )
                .map_err(|e| {
                    CliError::runtime(format!("invalid codec states '{}': {e}", path.display()))
                })?,
            );
        }
        Ok(tables)
    }

    fn read(&mut self, path: &Path, count: usize) -> Result<Vec<[u16; 4]>, CliError> {
        let mut input = open_input(&Endpoint::Path(path.to_owned()), MAX_STATE_BYTES)?;
        let mut text = String::new();
        (&mut input.file)
            .take(MAX_STATE_BYTES + 1)
            .read_to_string(&mut text)
            .map_err(|e| {
                CliError::runtime(format!(
                    "cannot read codec states '{}': {e}",
                    path.display()
                ))
            })?;
        let rows = parse_states(&text, count).map_err(|e| {
            CliError::runtime(format!("invalid codec states '{}': {e}", path.display()))
        })?;
        self.inputs.push(input);
        Ok(rows)
    }
}

fn parse_states(text: &str, count: usize) -> Result<Vec<[u16; 4]>, &'static str> {
    if text.len() as u64 > MAX_STATE_BYTES {
        return Err("file exceeds 16 KiB");
    }
    let mut rows = Vec::with_capacity(count);
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let mut row = [0; 4];
        for value in &mut row {
            *value = fields
                .next()
                .ok_or("expected four decimal integers per row")?
                .parse::<u16>()
                .map_err(|_| "expected unsigned decimal integers")?;
        }
        if fields.next().is_some() || rows.len() == count {
            return Err("unexpected extra field or row");
        }
        let [_, lps, mps, switch] = row;
        if lps > u8::MAX as u16 || mps > u8::MAX as u16 || switch > 1 {
            return Err("state value out of range");
        }
        rows.push(row);
    }
    if rows.len() != count {
        return Err("expected exactly 113 QM or 47 MQ rows");
    }
    Ok(rows)
}

struct CompletePages;
impl ComposeVisitor for CompletePages {
    async fn page(&mut self, page: ComposePage<'_>) -> caj2pdf_core::Result<()> {
        if page.output_page.is_none() {
            return Err(Error::InvalidInput {
                reason: "HN/C8 conversion cannot omit source pages without image content",
            });
        }
        Ok(())
    }
}

pub async fn convert<S: RangedSource, W: SequentialSink>(
    source: &mut S,
    sink: &mut W,
    tables: &Tables,
    include_bookmarks: bool,
    limits: &Limits,
) -> Result<ConversionReport, String> {
    let options = ComposeOptions {
        include_bookmarks,
        ..Default::default()
    };
    let directory = std::env::temp_dir();
    let scratch = || {
        let file = anonymous_file(&directory).map_err(|e| {
            format!(
                "cannot create HN/C8 scratch in '{}': {e}",
                directory.display()
            )
        })?;
        FileScratch::new(file, options.budget.max_row_store_bytes).map_err(|e| e.to_string())
    };
    let mut rows = scratch()?;
    let mut first = scratch()?;
    let mut second = scratch()?;
    let mut refined = scratch()?;
    let type3 = tables.mq.as_ref().map(|table| ComposeType3Workspaces {
        table,
        first: &mut first,
        second: &mut second,
        refined: &mut refined,
    });
    convert_source_pages_pdf(
        source,
        sink,
        tables.qm.as_ref(),
        ComposeWorkspaces {
            rows: &mut rows,
            type3,
        },
        &mut CompletePages,
        options,
        limits,
        &NeverCancel,
    )
    .await
    .map(|report| report.conversion)
    .map_err(|e| e.to_string())
}

/// Keep only bounded outline metadata; image payloads are never read here.
pub async fn inspect<S: RangedSource>(
    source: &mut S,
    limits: &Limits,
) -> Result<
    (
        caj2pdf_core::hnc8::Header,
        Option<Vec<caj2pdf_core::Bookmark>>,
    ),
    String,
> {
    use caj2pdf_core::hnc8::{Budget, Hnc8Reader};
    let mut reader = Hnc8Reader::open(source, limits, &NeverCancel, Budget::default())
        .await
        .map_err(|e| e.to_string())?;
    let header = reader.header();
    let Some(count) = reader.declared_bookmark_count() else {
        return Ok((header, None));
    };
    if count > limits.max_bookmarks {
        return Err("HN-A bookmark count exceeds the configured limit".into());
    }
    let bytes = u64::from(count) * std::mem::size_of::<caj2pdf_core::Bookmark>() as u64;
    limits.check_allocation(bytes).map_err(|e| e.to_string())?;
    let mut collected = CollectedBookmarks {
        items: Vec::new(),
        limits: *limits,
        bytes,
    };
    collected
        .items
        .try_reserve_exact(count as usize)
        .map_err(|_| "cannot allocate HN-A outline metadata")?;
    reader
        .visit_bookmarks(64, header.page_count, |page| Some(page - 1), &mut collected)
        .await
        .map_err(|e| e.to_string())?;
    Ok((header, Some(collected.items)))
}

struct CollectedBookmarks {
    items: Vec<caj2pdf_core::Bookmark>,
    limits: Limits,
    bytes: u64,
}

impl caj2pdf_core::BookmarkVisitor for CollectedBookmarks {
    async fn visit(&mut self, bookmark: caj2pdf_core::Bookmark) -> caj2pdf_core::Result<()> {
        self.bytes += bookmark.title.capacity() as u64;
        self.limits.check_allocation(self.bytes)?;
        self.items.push(bookmark);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_text_is_small_strict_and_uses_explicit_column_order() {
        assert_eq!(
            parse_states("16384 1 0 1\r\n16384 0 1 0\n", 2).unwrap(),
            [[16384, 1, 0, 1], [16384, 0, 1, 0]]
        );
        for text in [
            "",
            "1 0 0",
            "x 0 0 0",
            "1 0 0 0 0",
            "1 0 0 0\n1 0 0 0",
            "1 256 0 0",
            "1 0 256 0",
            "1 0 0 2",
            "65536 0 0 0",
        ] {
            assert!(parse_states(text, 1).is_err(), "{text:?}");
        }
        assert!(parse_states(&" ".repeat(MAX_STATE_BYTES as usize + 1), 1).is_err());
    }

    #[test]
    fn outline_collection_accounts_for_records_and_retained_titles() {
        use crate::document::block_on;
        use caj2pdf_core::native::SeekableSource;
        let mut bytes = vec![0; 0x15c + 308 + 20];
        bytes[..8].copy_from_slice(&[72, 78, 0, 0, 0x90, 1, 0, 0]);
        bytes[0x90] = 1;
        bytes[0x158] = 1;
        bytes[0x15c..0x160].copy_from_slice(b"Root");
        bytes[0x15c + 280] = b'1';
        bytes[0x15c + 304] = 1;
        for (max_bookmarks, max_allocation_bytes) in [
            (0, 4096),
            (1, 1),
            (1, std::mem::size_of::<caj2pdf_core::Bookmark>() as u64),
        ] {
            let mut source = SeekableSource::new(std::io::Cursor::new(&bytes)).unwrap();
            let limits = Limits {
                max_bookmarks,
                max_allocation_bytes,
                io_chunk_bytes: 1,
                ..Limits::default()
            };
            let error = block_on(inspect(&mut source, &limits)).unwrap_err();
            assert!(error.contains("limit"), "{error}");
        }
    }
}
