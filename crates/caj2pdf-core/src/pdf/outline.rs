// SPDX-License-Identifier: MIT

//! The one outline serializer and streaming outline builder of every PDF
//! emitter.
//!
//! The builder keeps one open item per active depth and writes each item as
//! soon as its last link is known, so only O(depth) titles stay in memory.

use super::document::reserve_bounded;
use super::types::PdfRef;
use crate::fallible::len_u64;
use crate::{Error, Limits, Result};
use std::fmt::Write as _;
use std::mem::size_of;

/// An explicit depth cap keeps per-bookmark stack work bounded.
pub(super) const MAX_OUTLINE_DEPTH: usize = 256;
const HEX: &[u8; 16] = b"0123456789ABCDEF";
const OUTLINE_STACK: &str = "bookmark stack allocation";

/// View applied when following a bookmark.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BookmarkView {
    /// Fit the entire destination page (the existing default).
    Fit,
    /// Keep the viewer's current position and zoom: `/XYZ null null null`.
    Xyz,
}

/// Writes whole indirect objects. Each PDF emitter implements it, so shared
/// objects such as outline items have one serializer.
pub(super) trait ObjectSink {
    type Ref: Copy + Into<PdfRef>;

    /// Start the object, writing its `number generation obj` header.
    fn begin_object(&mut self, reference: Self::Ref) -> Result<()>;
    fn write(&mut self, bytes: &[u8]) -> Result<()>;
    /// End the object with `endobj`.
    fn end_object(&mut self) -> Result<()>;
}

/// An [`ObjectSink`] that also numbers new objects.
pub(super) trait ObjectAllocator: ObjectSink {
    fn reserve(&mut self) -> Result<Self::Ref>;
}

/// One outline item's links, in any reference type.
#[derive(Clone, Copy)]
pub(super) struct OutlineItem<R> {
    pub(super) reference: R,
    pub(super) parent: R,
    pub(super) page: R,
    pub(super) view: BookmarkView,
    pub(super) previous: Option<R>,
    pub(super) next: Option<R>,
    pub(super) first_child: Option<R>,
    pub(super) last_child: Option<R>,
    /// Open descendants, written as `/Count` when the item has children.
    pub(super) descendants: u32,
}

impl<R: Copy> OutlineItem<R> {
    /// A new item without children or a next sibling.
    pub(super) fn new(
        reference: R,
        parent: R,
        previous: Option<R>,
        page: R,
        view: BookmarkView,
    ) -> Self {
        Self {
            reference,
            parent,
            page,
            view,
            previous,
            next: None,
            first_child: None,
            last_child: None,
            descendants: 0,
        }
    }
}

fn pdf_ref(reference: impl Into<PdfRef>) -> String {
    let reference = reference.into();
    format!("{} {} R", reference.number, reference.generation)
}

/// Write `text` as UTF-16BE hexadecimal digits in bounded chunks.
pub(super) fn write_utf16_hex<O: ObjectSink>(out: &mut O, text: &str) -> Result<()> {
    let mut hex = [0_u8; 4096];
    let mut used = 0;
    for unit in text.encode_utf16() {
        if used == hex.len() {
            out.write(&hex)?;
            used = 0;
        }
        for byte in unit.to_be_bytes() {
            hex[used] = HEX[(byte >> 4) as usize];
            hex[used + 1] = HEX[(byte & 0x0f) as usize];
            used += 2;
        }
    }
    if used > 0 {
        out.write(&hex[..used])?;
    }
    Ok(())
}

/// Write one outline item dictionary as a whole object.
pub(super) fn write_item<O: ObjectSink>(
    out: &mut O,
    item: &OutlineItem<O::Ref>,
    title: &str,
) -> Result<()> {
    out.begin_object(item.reference)?;
    out.write(b"<< /Title <FEFF")?;
    write_utf16_hex(out, title)?;
    let view = match item.view {
        BookmarkView::Fit => "/Fit",
        BookmarkView::Xyz => "/XYZ null null null",
    };
    let mut text = format!(
        "> /Parent {} /Dest [{} {view}]",
        pdf_ref(item.parent),
        pdf_ref(item.page)
    );
    // Formatting into a `String` cannot fail.
    if let Some(previous) = item.previous {
        let _ = write!(text, " /Prev {}", pdf_ref(previous));
    }
    if let Some(next) = item.next {
        let _ = write!(text, " /Next {}", pdf_ref(next));
    }
    if let (Some(first), Some(last)) = (item.first_child, item.last_child) {
        let _ = write!(
            text,
            " /First {} /Last {} /Count {}",
            pdf_ref(first),
            pdf_ref(last),
            item.descendants
        );
    }
    text.push_str(" >>");
    out.write(text.as_bytes())?;
    out.end_object()
}

/// Write the outline root dictionary as a whole object.
pub(super) fn write_root<O: ObjectSink>(
    out: &mut O,
    root: O::Ref,
    first: O::Ref,
    last: O::Ref,
    count: u32,
) -> Result<()> {
    out.begin_object(root)?;
    out.write(
        format!(
            "<< /Type /Outlines /First {} /Last {} /Count {count} >>",
            pdf_ref(first),
            pdf_ref(last)
        )
        .as_bytes(),
    )?;
    out.end_object()
}

struct OpenItem<R> {
    item: OutlineItem<R>,
    title: String,
    /// The number of items added before this one.
    ordinal: u32,
}

/// A closed item awaiting its `/Next` link.
struct ClosedItem<R> {
    item: OutlineItem<R>,
    title: String,
}

/// Builds an outline from items in depth-first order while writing it.
///
/// Completed siblings and ancestors are written immediately. A failure once
/// an insertion starts closing items can drop an item that its siblings or
/// parent already link to, so the outline then refuses every later call.
pub(super) struct OutlineBuilder<R> {
    root: Option<R>,
    first: Option<R>,
    last: Option<R>,
    open: Vec<OpenItem<R>>,
    written: u32,
    retained_titles: u64,
    retained_nodes: u32,
    failed: bool,
}

impl<R: Copy + Into<PdfRef>> OutlineBuilder<R> {
    pub(super) const fn new() -> Self {
        Self {
            root: None,
            first: None,
            last: None,
            open: Vec::new(),
            written: 0,
            retained_titles: 0,
            retained_nodes: 0,
            failed: false,
        }
    }

    /// The number of items added.
    pub(super) const fn written(&self) -> u32 {
        self.written
    }

    /// Check that no earlier insertion failed after it began closing items.
    pub(super) fn ensure_intact(&self) -> Result<()> {
        if self.failed {
            Err(Error::invalid(
                "PDF outline cannot continue after a failed bookmark operation",
            ))
        } else {
            Ok(())
        }
    }

    /// Check that an item at `depth` would not skip a missing parent.
    pub(super) fn check_depth(&self, depth: usize) -> Result<()> {
        if depth > self.open.len() {
            return Err(Error::invalid("bookmark depth skips a parent"));
        }
        Ok(())
    }

    /// Add one item at zero-based `depth`, as a sibling, an ancestor's
    /// sibling, or a direct child of the previous item.
    ///
    /// The depth, retained memory and new object numbers are checked before
    /// anything is closed, so such a refusal leaves the outline usable.
    pub(super) fn add<O: ObjectAllocator<Ref = R>>(
        &mut self,
        out: &mut O,
        limits: &Limits,
        depth: usize,
        page: R,
        view: BookmarkView,
        title: String,
    ) -> Result<()> {
        self.check_depth(depth)?;
        let title_capacity = len_u64(title.capacity());
        // A rejected title must leave all reserved objects and outline links
        // intact, so account for siblings that this insertion would emit first.
        let (next_titles, next_nodes) = self.preflight_memory(depth, title_capacity, limits)?;
        let root = match self.root {
            Some(root) => root,
            None => {
                let root = out.reserve()?;
                self.root = Some(root);
                root
            }
        };
        let reference = out.reserve()?;
        // Closing pops items before writing them, so a failure from here on
        // can lose an item that the outline already links to.
        self.failed = true;
        let previous_item = self.close_to(out, depth)?;
        let (parent, links) = match self.open.last_mut() {
            Some(active) => (
                active.item.reference,
                (&mut active.item.first_child, &mut active.item.last_child),
            ),
            None => (root, (&mut self.first, &mut self.last)),
        };
        let previous = *links.1;
        links.0.get_or_insert(reference);
        *links.1 = Some(reference);
        if let Some(previous_item) = previous_item {
            self.emit(out, previous_item, Some(reference))?;
        }
        self.open.push(OpenItem {
            item: OutlineItem::new(reference, parent, previous, page, view),
            title,
            ordinal: self.written,
        });
        debug_assert_eq!(
            self.retained_titles.checked_add(title_capacity),
            Some(next_titles)
        );
        debug_assert_eq!(self.retained_nodes.checked_add(1), Some(next_nodes));
        self.retained_titles = next_titles;
        self.retained_nodes = next_nodes;
        self.written += 1;
        self.failed = false;
        Ok(())
    }

    /// Write every open item and the outline root, returning the root when
    /// any item was added.
    pub(super) fn finish<O: ObjectSink<Ref = R>>(&mut self, out: &mut O) -> Result<Option<R>> {
        if let Some(last_root) = self.close_to(out, 0)? {
            self.emit(out, last_root, None)?;
        }
        if let Some(root) = self.root {
            let first = self
                .first
                .ok_or(Error::invalid("outline root has no first child"))?;
            let last = self
                .last
                .ok_or(Error::invalid("outline root has no last child"))?;
            write_root(out, root, first, last, self.written)?;
        }
        Ok(self.root)
    }

    fn preflight_memory(
        &mut self,
        depth: usize,
        title_capacity: u64,
        limits: &Limits,
    ) -> Result<(u64, u32)> {
        let mut released_titles = 0_u64;
        let mut released_nodes = 0_u32;
        // Every item that inserting at `depth` closes is written before the
        // new item is retained; no other closed item is ever held unwritten.
        for item in self.open.iter().skip(depth) {
            released_titles = released_titles
                .checked_add(len_u64(item.title.capacity()))
                .ok_or(Error::invalid("released bookmark title bytes overflow"))?;
            released_nodes = released_nodes
                .checked_add(1)
                .ok_or(Error::invalid("released bookmark count overflows"))?;
        }
        let next_titles = self
            .retained_titles
            .checked_sub(released_titles)
            .and_then(|retained| retained.checked_add(title_capacity))
            .ok_or(Error::invalid("retained bookmark title bytes overflow"))?;
        let next_nodes = self
            .retained_nodes
            .checked_sub(released_nodes)
            .and_then(|retained| retained.checked_add(1))
            .ok_or(Error::invalid("retained bookmark count overflows"))?;
        let attempted = u64::from(next_nodes)
            .checked_mul(size_of::<OpenItem<R>>() as u64)
            .and_then(|node_bytes| node_bytes.checked_add(next_titles))
            .ok_or(Error::invalid("retained bookmark allocation overflows"))?;
        limits.check_allocation(attempted)?;
        if depth == self.open.len() {
            // The caller checked the bookmark count, and `attempted` covers
            // every retained item including this one, so only an allocator
            // refusal fails this reservation.
            let maximum = u64::from(limits.max_bookmarks);
            reserve_bounded(&mut self.open, maximum, limits, OUTLINE_STACK)?;
        }
        Ok((next_titles, next_nodes))
    }

    /// Close every open item deeper than `depth` and return the last one
    /// closed, which is the item at `depth` itself when one was open.
    ///
    /// Each closed item is the last child of the next item closed, which
    /// writes it; the caller writes the returned item once its `/Next` link
    /// is known. Because the only unwritten closed item is carried here
    /// rather than stored on its parent's links, a sibling can never be
    /// left unwritten.
    fn close_to<O: ObjectSink<Ref = R>>(
        &mut self,
        out: &mut O,
        depth: usize,
    ) -> Result<Option<ClosedItem<R>>> {
        let mut closed = None;
        while self.open.len() > depth {
            let Some(open) = self.open.pop() else {
                break;
            };
            if let Some(last_child) = closed.take() {
                self.emit(out, last_child, None)?;
            }
            let mut item = open.item;
            item.descendants = self
                .written
                .checked_sub(open.ordinal + 1)
                .ok_or(Error::invalid("bookmark descendant count underflows"))?;
            closed = Some(ClosedItem {
                item,
                title: open.title,
            });
        }
        Ok(closed)
    }

    fn emit<O: ObjectSink<Ref = R>>(
        &mut self,
        out: &mut O,
        closed: ClosedItem<R>,
        next: Option<R>,
    ) -> Result<()> {
        let mut item = closed.item;
        item.next = next;
        write_item(out, &item, &closed.title)?;
        self.retained_titles = self
            .retained_titles
            .checked_sub(len_u64(closed.title.capacity()))
            .ok_or(Error::invalid("retained bookmark title bytes underflow"))?;
        self.retained_nodes = self
            .retained_nodes
            .checked_sub(1)
            .ok_or(Error::invalid("retained bookmark count underflows"))?;
        Ok(())
    }
}
