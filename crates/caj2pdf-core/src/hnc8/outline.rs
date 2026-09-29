// SPDX-License-Identifier: MIT

//! Independently observed HN-A outline records; see docs/hnc8-outline-fields.md.

use super::{ErrorKind, Hnc8Reader, Location, Result, Variant, read_fixed};
use crate::{Bookmark, BookmarkVisitor, Cancellation, Error, RangedSource, gb18030};

impl<S: RangedSource, C: Cancellation> Hnc8Reader<'_, S, C> {
    /// Visit HN-A bookmarks one record at a time, awaiting every visitor call.
    ///
    /// `map_page` maps a one-based physical source page to a zero-based emitted
    /// page. `None` rejects an omitted destination; no neighboring-page fallback
    /// is applied. Returned indexes must be below `output_pages`. The caller
    /// owns any mapping storage. This does not infer C8/HN-B outline absence.
    ///
    /// Depth is one-based in the source and zero-based in `Bookmark`; use 64
    /// for the initial validated profile. Visitor failure or a dropped future
    /// poisons the reader, preventing an accidental retry of partial output.
    pub async fn visit_bookmarks<V: BookmarkVisitor, F: FnMut(u32) -> Option<u32>>(
        &mut self,
        max_depth: u32,
        output_pages: u32,
        map_page: F,
        visitor: &mut V,
    ) -> Result<u32> {
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
    ) -> Result<u32> {
        if self.cancellation.is_cancelled() {
            return Err(loc.error(ErrorKind::Cancelled));
        }
        if self.header.variant != Variant::HnA {
            return Err(loc.error(ErrorKind::Unsupported {
                field: "outline variant",
                value: 0,
            }));
        }
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
        let count = ((self.header.page_index.offset - 0x15c) / 308) as u32;
        if count > self.limits.max_bookmarks {
            return Err(loc.limit(
                "bookmarks",
                u64::from(self.limits.max_bookmarks),
                u64::from(count),
            ));
        }
        let mut previous_level = 0_u32;
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
            let title_end = record[..256]
                .iter()
                .position(|&byte| byte == 0)
                .ok_or(at.malformed("outline title", "missing NUL terminator"))?;
            // A conservative UTF-8 capacity bound for this fixed-width title.
            let allocation = title_end as u64 * 4;
            if allocation > self.limits.max_allocation_bytes {
                return Err(at.limit(
                    "outline title bytes",
                    self.limits.max_allocation_bytes,
                    allocation,
                ));
            }
            let title = gb18030::decode(&record[..title_end]).map_err(|error| {
                loc.at(offset + error.offset as u64)
                    .malformed("outline title", "invalid GB18030 sequence")
            })?;
            let page_at = loc.at(offset + 280);
            let field = &record[280..292];
            let end = field
                .iter()
                .position(|&byte| byte == 0)
                .ok_or(page_at.malformed("outline page", "missing NUL terminator"))?;
            if end == 0 || !field[..end].iter().all(u8::is_ascii_digit) {
                return Err(page_at.malformed("outline page", "expected decimal digits"));
            }
            // At most eleven digits fit before NUL, so accumulation fits u64.
            let page = field[..end]
                .iter()
                .fold(0_u64, |value, byte| value * 10 + u64::from(byte - b'0'));
            if page == 0 || page > u64::from(self.header.page_count) {
                return Err(
                    page_at.malformed("outline page", "destination is outside source pages")
                );
            }
            let page_index = map_page(page as u32)
                .filter(|&index| index < output_pages)
                .ok_or(page_at.malformed("outline page map", "destination page was not emitted"))?;
            let level = u32::from_le_bytes(record[304..308].try_into().expect("fixed field width"));
            let level_at = loc.at(offset + 304);
            if level == 0 || level > max_depth {
                return Err(
                    level_at.malformed("outline level", "level is outside the configured depth")
                );
            }
            if u64::from(level) > u64::from(previous_level) + 1 {
                return Err(level_at.malformed("outline level", "level skips a parent"));
            }
            visitor
                .visit(Bookmark {
                    title,
                    depth: level - 1,
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
            previous_level = level;
        }
        Ok(count)
    }
}
