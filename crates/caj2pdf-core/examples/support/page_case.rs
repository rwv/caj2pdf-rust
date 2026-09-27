// SPDX-License-Identifier: MIT

//! Private full-page plan: the text plan prefix plus three encoded-span pins.

use crate::text_case::{Case, EXPECTED_CASES, MAX_PLAN_BYTES, parse_case_fields};
use caj2pdf_core::jbig2::SegmentSpan;
use std::{
    error::Error as StdError,
    fs::{self, File},
    io::Read,
    path::Path,
};

pub(crate) struct EncodedPin {
    pub(crate) span: SegmentSpan,
    pub(crate) sha256: String,
}

pub(crate) struct PageCase {
    pub(crate) text: Case,
    pub(crate) record: EncodedPin,
    pub(crate) page: EncodedPin,
    pub(crate) generic: EncodedPin,
}

fn pin(fields: &[&str], label: &'static str) -> Result<EncodedPin, Box<dyn StdError>> {
    if fields.len() != 3
        || fields[2].len() != 64
        || !fields[2].bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(format!("{label} encoded pin is invalid").into());
    }
    let span = SegmentSpan {
        offset: fields[0].parse()?,
        length: fields[1].parse()?,
    };
    if span.length == 0
        || span.length > 64 * 1024 * 1024
        || span.offset.checked_add(span.length).is_none()
    {
        return Err(format!("{label} encoded span is invalid").into());
    }
    Ok(EncodedPin {
        span,
        sha256: fields[2].to_ascii_lowercase(),
    })
}

pub(crate) fn parse_page_plan(path: &Path) -> Result<Vec<PageCase>, Box<dyn StdError>> {
    if fs::metadata(path)?.len() > MAX_PLAN_BYTES {
        return Err("full-page plan exceeds 2 MiB".into());
    }
    let mut text = String::new();
    File::open(path)?
        .take(MAX_PLAN_BYTES + 1)
        .read_to_string(&mut text)?;
    if text.len() as u64 > MAX_PLAN_BYTES {
        return Err("full-page plan grew beyond 2 MiB".into());
    }
    let mut cases = Vec::new();
    cases.try_reserve_exact(EXPECTED_CASES)?;
    for line in text.lines() {
        let fields: Vec<_> = line.split('\t').collect();
        if fields.len() != 28 {
            return Err("full-page plan line must contain 28 tab fields".into());
        }
        let text = parse_case_fields(&fields[..19])?;
        let record = pin(&fields[19..22], "record")?;
        let page = pin(&fields[22..25], "page")?;
        let generic = pin(&fields[25..28], "generic")?;
        let record_end = record.span.offset + record.span.length;
        if record.span.length <= 48
            || page.span.offset != record.span.offset + 48
            || page
                .span
                .offset
                .checked_add(page.span.length)
                .is_none_or(|end| end > record_end)
            || generic.span.offset.checked_add(generic.span.length) != Some(record_end)
            || !(page.span.offset < text.first.offset
                && text.first.offset < text.second.offset
                && text.second.offset < text.text.offset
                && text.text.offset < generic.span.offset)
        {
            return Err("full-page plan segment spans are not ordered within record".into());
        }
        cases.push(PageCase {
            text,
            record,
            page,
            generic,
        });
        if cases.len() > EXPECTED_CASES {
            return Err("full-page plan has too many cases".into());
        }
    }
    if cases.len() != EXPECTED_CASES {
        return Err("full-page plan does not contain 546 cases".into());
    }
    Ok(cases)
}
