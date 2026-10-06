// SPDX-License-Identifier: MIT

use super::*;
use caj2pdf_core::{
    hnc8::{
        ApplicationInfo, C8FontSource, C8FontSources, C8PageFonts, ComposeOptions, ComposePage,
        ComposeVisitor, convert_document_pdf,
    },
    jbig2::text::TextHeaderPolicy,
    qm::QmTable,
};

/// Explicitly registered font resources and their roles.
#[derive(Default)]
pub(super) struct Fonts {
    sizes: [u64; 8],
    faces: [u32; 8],
    count: usize,
    roles: Option<C8PageFonts>,
}

impl Fonts {
    pub(super) fn count(&self) -> usize {
        self.count
    }

    pub(super) fn add(&mut self, size: u64, face: u32) -> u32 {
        if self.count == self.sizes.len() || self.roles.is_some() {
            return 0;
        }
        self.sizes[self.count] = size;
        self.faces[self.count] = face;
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
        self.roles = Some(C8PageFonts {
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

struct CompletePages;

impl ComposeVisitor for CompletePages {
    fn page(&mut self, page: ComposePage<'_>) -> Result<()> {
        if page.output_page.is_none() {
            return Err(Error::InvalidInput {
                reason: "HN/C8 conversion cannot omit source pages without image content",
            });
        }
        Ok(())
    }
}

pub(super) fn convert<'h, H: Host>(
    host: &'h RefCell<&'h mut H>,
    source: &mut HostSource<'h, H>,
    sink: &mut HostSink<'h, H>,
    fonts: &Fonts,
    options: ConversionOptions,
    limits: &Limits,
    cancellation: &HostCancellation<'h, H>,
) -> Result<(ConversionReport, OutlineReport)> {
    let options = ComposeOptions {
        // The HN/C8 profile explicitly admits the measured unused-template
        // anomaly; general JBIG2 APIs and all other malformed flags stay strict.
        text_header_policy: TextHeaderPolicy::HnC8UnusedRefinementTemplate,
        include_bookmarks: options.include_bookmarks,
    };
    let roles = match fonts.count {
        0 => None,
        _ => Some(fonts.roles.ok_or(Error::InvalidInput {
            reason: "C8 font resources require explicit roles",
        })?),
    };
    for &size in &fonts.sizes[..fonts.count] {
        limits.check_input_size(size)?;
    }
    let mut sources = std::array::from_fn::<_, 8, _>(|index| C8FontSource {
        source: HostSource::new(host, index as u32 + 1, fonts.sizes[index]),
        face: fonts.faces[index],
    });
    // The core routes by text framing: image documents ignore the fonts.
    convert_document_pdf(
        source,
        sink,
        roles.map(|roles| C8FontSources {
            sources: &mut sources[..fonts.count],
            roles,
        }),
        Some(&QmTable::standard()),
        &mut CompletePages,
        options,
        limits,
        cancellation,
    )
    .map(|report| (report.conversion, report.outline))
    .map_err(|error| Error::Hnc8(Box::new(error)))
}

struct IgnoreBookmarks;

impl caj2pdf_core::BookmarkVisitor for IgnoreBookmarks {
    fn visit(&mut self, _: caj2pdf_core::Bookmark) -> Result<()> {
        Ok(())
    }
}

/// What an HN/C8 inspection found without decoding any page.
pub(super) struct Inspected {
    pub pages: u32,
    /// Written bookmark count; `None` when unknown.
    pub bookmarks: Option<u32>,
    /// Skipped or clamped HN-A outline entries.
    pub outline_warnings: u32,
    /// The C8 application-info package; `None` when absent or defective,
    /// as in conversion.
    pub application_info: Option<ApplicationInfo>,
}

fn read_metadata<S: RangedSource, C: Cancellation>(
    source: &mut S,
    limits: &Limits,
    cancellation: &C,
) -> caj2pdf_core::hnc8::Result<Inspected> {
    use caj2pdf_core::hnc8::Hnc8Reader;
    let mut reader = Hnc8Reader::open(source, limits, cancellation)?;
    let pages = reader.header().page_count;
    let application_info = reader.application_info_report()?.info;
    let mut inspected = Inspected {
        pages,
        bookmarks: None,
        outline_warnings: 0,
        application_info,
    };
    if reader.declared_bookmark_count().is_some() {
        let outline =
            reader.visit_bookmarks(64, pages, |page| Some(page - 1), &mut IgnoreBookmarks)?;
        inspected.bookmarks = Some(outline.written);
        inspected.outline_warnings = outline.defects;
    }
    Ok(inspected)
}

pub(super) fn inspect<S: RangedSource, C: Cancellation>(
    source: &mut S,
    limits: &Limits,
    cancellation: &C,
) -> Result<Inspected> {
    read_metadata(source, limits, cancellation)
        .map_err(|error| Error::Hnc8Metadata(Box::new(error)))
}
