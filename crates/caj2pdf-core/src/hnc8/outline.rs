// SPDX-License-Identifier: MIT

//! Independently observed HN-A outline records; see docs/hnc8-outline-fields.md.

use super::{ErrorKind, Hnc8Reader, Location, Result, Variant, read_fixed};
use crate::{Bookmark, BookmarkVisitor, Cancellation, Error, RangedSource, gb18030};
use std::fmt;

/// Defects whose location [`OutlineReport`] retains; later ones are only counted.
pub const MAX_RECORDED_OUTLINE_DEFECTS: usize = 16;

/// How a defective HN-A outline entry was handled.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum OutlineRepair {
    /// The entry was not written. Its descendants are re-parented to the
    /// nearest written ancestor instead of being dropped.
    #[default]
    Skipped,
    /// The entry was written at a shallower level.
    Clamped,
}

/// One defective HN-A outline entry, located by an absolute source offset.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OutlineDefect {
    /// Absolute byte offset of the defective field in the source.
    pub offset: u64,
    pub reason: &'static str,
    pub repair: OutlineRepair,
}

impl fmt::Display for OutlineDefect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let action = match self.repair {
            OutlineRepair::Skipped => "skipped",
            OutlineRepair::Clamped => "re-parented",
        };
        write!(
            f,
            "{action} HN-A bookmark at byte {}: {}",
            self.offset, self.reason
        )
    }
}

/// Bounded result of an HN-A outline traversal.
///
/// A malformed entry is a bookmark defect, not a document defect: it is
/// skipped or clamped and counted here. Errors that make the whole outline
/// table unreadable still fail the traversal.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OutlineReport {
    /// Records declared by the outline table.
    pub declared: u32,
    /// Entries passed to the visitor.
    pub written: u32,
    /// Defective entries, including those beyond the recorded locations.
    pub defects: u32,
    recorded: [OutlineDefect; MAX_RECORDED_OUTLINE_DEFECTS],
}

impl OutlineReport {
    /// The first [`MAX_RECORDED_OUTLINE_DEFECTS`] defects in source order.
    pub fn recorded_defects(&self) -> &[OutlineDefect] {
        &self.recorded[..(self.defects as usize).min(MAX_RECORDED_OUTLINE_DEFECTS)]
    }

    fn record(&mut self, defect: OutlineDefect) {
        if let Some(slot) = self.recorded.get_mut(self.defects as usize) {
            *slot = defect;
        }
        // Bounded by the declared record count, itself at most `u32::MAX`.
        self.defects += 1;
    }
}

impl<S: RangedSource, C: Cancellation> Hnc8Reader<'_, S, C> {
    /// Declared outline record count for the validated HN-A container layout.
    /// Titles, levels and destinations still need `visit_bookmarks` validation.
    /// C8/HN-B return `None`: their outline absence must not be inferred.
    pub fn declared_bookmark_count(&self) -> Option<u32> {
        (self.header.variant == Variant::HnA)
            .then(|| ((self.header.page_index.offset - 0x15c) / 308) as u32)
    }

    /// Visit HN-A bookmarks one record at a time, awaiting every visitor call.
    ///
    /// `map_page` maps a one-based physical source page to a zero-based emitted
    /// page. `None` marks an omitted destination; no neighboring-page fallback
    /// is applied. Returned indexes must be below `output_pages`. The caller
    /// owns any mapping storage. This does not infer C8/HN-B outline absence.
    ///
    /// Depth is one-based in the source and zero-based in `Bookmark`; use 64
    /// for the initial validated profile. An entry with an invalid title,
    /// page, destination or zero level is skipped; a level deeper than its
    /// written parent allows, or than `max_depth`, is clamped. Both are
    /// counted in the returned report. Read errors, limits and cancellation
    /// still fail. Visitor failure or a dropped future poisons the reader,
    /// preventing an accidental retry of partial output.
    pub async fn visit_bookmarks<V: BookmarkVisitor, F: FnMut(u32) -> Option<u32>>(
        &mut self,
        max_depth: u32,
        output_pages: u32,
        map_page: F,
        visitor: &mut V,
    ) -> Result<OutlineReport> {
        let loc = Location {
            variant: Some(self.header.variant),
            offset: 0x158,
            page: None,
            image: None,
        };
        if self.poisoned {
            return Err(loc.error(ErrorKind::Poisoned));
        }
        self.poisoned = true;
        let result = self
            .bookmarks(max_depth, output_pages, map_page, visitor, loc)
            .await;
        if result.is_ok() {
            self.poisoned = false;
        }
        result
    }

    async fn bookmarks<V: BookmarkVisitor, F: FnMut(u32) -> Option<u32>>(
        &mut self,
        max_depth: u32,
        output_pages: u32,
        mut map_page: F,
        visitor: &mut V,
        loc: Location,
    ) -> Result<OutlineReport> {
        if self.cancellation.is_cancelled() {
            return Err(loc.error(ErrorKind::Cancelled));
        }
        let Some(count) = self.declared_bookmark_count() else {
            return Err(loc.error(ErrorKind::Unsupported {
                field: "outline variant",
                value: 0,
            }));
        };
        if max_depth == 0 {
            return Err(loc.malformed("outline depth", "depth limit must be positive"));
        }
        if output_pages > self.limits.max_pages {
            return Err(loc.limit(
                "outline output pages",
                u64::from(self.limits.max_pages),
                u64::from(output_pages),
            ));
        }
        // HN-A open already validated the record interval and format-specific count.
        if count > self.limits.max_bookmarks {
            return Err(loc.limit(
                "bookmarks",
                u64::from(self.limits.max_bookmarks),
                u64::from(count),
            ));
        }
        let mut report = OutlineReport {
            declared: count,
            ..OutlineReport::default()
        };
        // The last written level bounds the next one. The previous source
        // level tells a source level skip (a defect) from a child re-parented
        // below a skipped or clamped ancestor (not a further defect). A zero
        // level hides its children's intended parent, so it becomes unknown.
        let mut written_level = 0_u32;
        let mut source_level = Some(0_u64);
        for ordinal in 0..count {
            let offset = 0x15c + u64::from(ordinal) * 308;
            let at = loc.at(offset);
            let mut record = [0_u8; 308];
            read_fixed(
                self.source,
                self.limits,
                self.cancellation,
                offset,
                &mut record,
                at,
                "outline record",
            )
            .await?;
            let level = u32::from_le_bytes(record[304..308].try_into().expect("fixed field width"));
            let entry = 'entry: {
                let Some(title_end) = record[..256].iter().position(|&byte| byte == 0) else {
                    break 'entry Err((offset, "title has no NUL terminator"));
                };
                // A conservative UTF-8 capacity bound for this fixed-width title.
                let allocation = title_end as u64 * 4;
                if allocation > self.limits.max_allocation_bytes {
                    return Err(at.limit(
                        "outline title bytes",
                        self.limits.max_allocation_bytes,
                        allocation,
                    ));
                }
                let title = match gb18030::decode(&record[..title_end]) {
                    Ok(title) => title,
                    Err(error) => {
                        break 'entry Err((
                            offset + error.offset as u64,
                            "title is not valid GB18030",
                        ));
                    }
                };
                let page_at = offset + 280;
                let field = &record[280..292];
                let Some(end) = field.iter().position(|&byte| byte == 0) else {
                    break 'entry Err((page_at, "page has no NUL terminator"));
                };
                if end == 0 || !field[..end].iter().all(u8::is_ascii_digit) {
                    break 'entry Err((page_at, "page is not a decimal number"));
                }
                // At most eleven digits fit before NUL, so accumulation fits u64.
                let page = field[..end]
                    .iter()
                    .fold(0_u64, |value, byte| value * 10 + u64::from(byte - b'0'));
                if page == 0 || page > u64::from(self.header.page_count) {
                    break 'entry Err((page_at, "destination is outside source pages"));
                }
                let Some(page_index) = map_page(page as u32).filter(|&index| index < output_pages)
                else {
                    break 'entry Err((page_at, "destination page was not emitted"));
                };
                if level == 0 {
                    break 'entry Err((offset + 304, "level is zero"));
                }
                Ok((title, page_index))
            };
            let source_parent =
                std::mem::replace(&mut source_level, (level != 0).then_some(u64::from(level)));
            let (title, page_index) = match entry {
                Ok(entry) => entry,
                Err((offset, reason)) => {
                    report.record(OutlineDefect {
                        offset,
                        reason,
                        repair: OutlineRepair::Skipped,
                    });
                    continue;
                }
            };
            let written = level.min(max_depth).min(written_level.saturating_add(1));
            let reason = if level > max_depth {
                Some("level exceeds the configured depth")
            } else if source_parent.is_some_and(|parent| u64::from(level) > parent + 1) {
                Some("level skips a parent")
            } else {
                None
            };
            if let Some(reason) = reason.filter(|_| written < level) {
                report.record(OutlineDefect {
                    offset: offset + 304,
                    reason,
                    repair: OutlineRepair::Clamped,
                });
            }
            visitor
                .visit(Bookmark {
                    title,
                    depth: written - 1,
                    page_index,
                })
                .await
                .map_err(|source| match source {
                    Error::Cancelled => at.error(ErrorKind::Cancelled),
                    source => at.error(ErrorKind::Source {
                        field: "outline visitor",
                        source,
                    }),
                })?;
            report.written += 1;
            written_level = written;
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests;
