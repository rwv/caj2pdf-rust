// SPDX-License-Identifier: MIT

//! Experimental HN/C8 CLI routing with standard states and optional overrides.

use crate::signals::ProcessCancellation;
use crate::{
    CliError,
    args::{ConvertOptions, Endpoint},
    files::{Input, anonymous_file, open_input},
};
use caj2pdf_core::{
    ConversionReport, Error, Limits, RangedSource, SequentialSink,
    hnc8::{
        ComposeOptions, ComposePage, ComposeType3Workspaces, ComposeVisitor, ComposeWorkspaces,
        Type3PdfOptions, convert_source_pages_pdf,
    },
    jbig2::mq::{MqState, MqTable},
    jbig2::text::TextHeaderPolicy,
    native::FileScratch,
    qm::{QmState, QmTable},
};
use std::{io::Read, path::Path};

const MAX_STATE_BYTES: u64 = 16 * 1024;

#[derive(Default)]
pub struct Resources {
    pub qm: Option<QmTable>,
    pub mq: Option<MqTable>,
    // Retain opened input identities so --force cannot overwrite a state file.
    pub inputs: Vec<Input>,
    font_start: usize,
    font_roles: Option<caj2pdf_core::hnc8::C8PageFonts>,
}

impl Resources {
    pub fn has_fonts(&self) -> bool {
        self.font_roles.is_some()
    }
    pub fn load(options: &ConvertOptions, limits: &Limits) -> Result<Self, CliError> {
        let mut resources = Self::default();
        if let Some(path) = &options.qm_states {
            let rows = resources.read(path, 113)?;
            resources.qm = Some(
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
        } else {
            resources.qm = Some(QmTable::standard());
        }
        if let Some(path) = &options.mq_states {
            let rows = resources.read(path, 47)?;
            resources.mq = Some(
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
        } else {
            resources.mq = Some(MqTable::standard());
        }
        resources.font_start = resources.inputs.len();
        if options.fonts.iter().any(Option::is_some) {
            let mut paths = Vec::new();
            let mut indices = [0; 4];
            for (role, path) in options.fonts.iter().enumerate() {
                if let Some(path) = path {
                    indices[role] = if let Some(index) = paths.iter().position(|p| *p == path) {
                        index
                    } else {
                        let index = paths.len();
                        resources.inputs.push(open_input(
                            &Endpoint::Path(path.clone()),
                            limits.max_input_bytes,
                        )?);
                        paths.push(path);
                        index
                    };
                }
            }
            resources.font_roles = Some(caj2pdf_core::hnc8::C8PageFonts {
                cjk: indices[0],
                latin: indices[1],
                alternate_latin: indices[2],
                decoration: options.fonts[3]
                    .as_ref()
                    .map(|_| (indices[3], options.decoration_char.unwrap_or('►'))),
            });
        }
        Ok(resources)
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
    resources: &mut Resources,
    include_bookmarks: bool,
    limits: &Limits,
) -> Result<ConversionReport, String> {
    let options = ComposeOptions {
        // The HN/C8 profile explicitly admits the measured unused-template
        // anomaly; general JBIG2 APIs and all other malformed flags stay strict.
        type3: Type3PdfOptions {
            text_header_policy: TextHeaderPolicy::HnC8UnusedRefinementTemplate,
            ..Default::default()
        },
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
    let type3 = resources.mq.as_ref().map(|table| ComposeType3Workspaces {
        table,
        first: &mut first,
        second: &mut second,
        refined: &mut refined,
    });
    if let Some(roles) = resources.font_roles {
        let mut fonts = resources.inputs[resources.font_start..]
            .iter_mut()
            .map(|input| caj2pdf_core::native::SeekableSource::new(&mut input.file))
            .collect::<caj2pdf_core::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        return caj2pdf_core::hnc8::convert_c8_native_pdf(
            source,
            sink,
            caj2pdf_core::hnc8::C8FontSources {
                sources: &mut fonts,
                roles,
            },
            resources.qm.as_ref(),
            ComposeWorkspaces {
                rows: &mut rows,
                type3,
            },
            options,
            limits,
            &ProcessCancellation,
        )
        .await
        .map(|report| report.conversion)
        .map_err(|e| e.to_string());
    }
    convert_source_pages_pdf(
        source,
        sink,
        resources.qm.as_ref(),
        ComposeWorkspaces {
            rows: &mut rows,
            type3,
        },
        &mut CompletePages,
        options,
        limits,
        &ProcessCancellation,
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
    let mut reader = Hnc8Reader::open(source, limits, &ProcessCancellation, Budget::default())
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
    fn default_states_are_available_without_allocating_tables() {
        let options = ConvertOptions::default();
        let resources = Resources::load(&options, &Limits::default()).unwrap();
        assert!(resources.qm.is_some());
        assert!(resources.mq.is_some());
        assert!(resources.inputs.is_empty());
        let limited = Limits {
            max_allocation_bytes: 0,
            ..Limits::default()
        };
        assert!(Resources::load(&options, &limited).is_ok());
    }

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
