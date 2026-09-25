// SPDX-License-Identifier: MIT

//! Bounded page and outline assembly over the forward-only PDF writer.

use super::writer::{MAX_PDF_INTEGER, ObjectId, PdfWriter};
use crate::fallible::{checked_read_count, len_u64, reserve_exact, usize_from_u32};
use crate::{
    Bookmark, BookmarkVisitor, Cancellation, ConversionReport, Error, Limits, RangedSource, Result,
    SequentialSink, read_exact_at,
};

const PAGE_TREE_FANOUT: usize = 256;
const MAX_TREE_PAGES: u64 = (PAGE_TREE_FANOUT as u64).pow(3);
const MAX_PAGE_POINTS: f64 = 14_400.0;
const MIN_PAGE_POINTS: f64 = 0.000_001;
const HEX: &[u8; 16] = b"0123456789ABCDEF";

/// A page's visible size in PDF points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageSpec {
    pub width_points: f64,
    pub height_points: f64,
}

/// The narrow image formats emitted by this writer.
///
/// JPEG bytes are passed through unchanged; callers must supply a valid JPEG
/// with the declared dimensions and color space. Raw bytes are checked against
/// their exact expected length before any image output is written.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageEncoding {
    Gray8,
    Rgb8,
    JpegGray8,
    JpegRgb8,
}

/// One image XObject placed on a page.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImageSpec {
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub encoding: ImageEncoding,
}

#[derive(Debug)]
struct PageNode {
    id: ObjectId,
    parent: ObjectId,
    children: Vec<ObjectId>,
    page_count: u32,
}

#[derive(Default)]
struct ChildLinks {
    first: Option<ObjectId>,
    last: Option<ObjectId>,
}

struct OpenOutline {
    id: ObjectId,
    parent: ObjectId,
    previous: Option<ObjectId>,
    page: ObjectId,
    title: String,
    ordinal: u32,
    children: ChildLinks,
}

struct ClosedOutline {
    id: ObjectId,
    parent: ObjectId,
    previous: Option<ObjectId>,
    page: ObjectId,
    title: String,
    first_child: Option<ObjectId>,
    last_child: Option<ObjectId>,
    descendants: u32,
}

/// A 1 bit-per-component image whose rows the caller streams in order.
///
/// Each supplied row has `row_stride` bytes and is packed most significant
/// bit first. Only its first `ceil(pixel_width / 8)` bytes enter the PDF;
/// the remaining bytes are source row padding and are dropped, so a DIB
/// 32-bit-aligned row can be passed unchanged. Unused low bits of the last
/// kept byte are PDF row padding, which readers ignore. The first supplied
/// row is the top of the image. A set bit is black and a clear bit is white:
/// the XObject is `/DeviceGray` with `/BitsPerComponent 1` and
/// `/Decode [1 0]`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BilevelImageSpec {
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub row_stride: usize,
}

/// A completely written image XObject that can be placed on a page with
/// [`PdfDocument::add_page`]. It is valid only in the document that wrote it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImageObject {
    object: ObjectId,
}

/// Streams one bilevel image's rows into an open PDF image stream.
///
/// Obtain it from [`PdfDocument::begin_bilevel_image`], write exactly
/// `row_stride * pixel_height` bytes through [`SequentialSink::write`], then
/// call [`BilevelImageWriter::finish`]. Writes may split or join rows. Only
/// row-position state is retained; image bytes are passed straight to the
/// document sink. Dropping the writer before `finish` leaves the document's
/// stream open, so every later document operation fails.
pub struct BilevelImageWriter<'d, 'a, W: SequentialSink, C: Cancellation> {
    document: &'d mut PdfDocument<'a, W, C>,
    object: ObjectId,
    visible: usize,
    stride: usize,
    column: usize,
    remaining: u64,
}

impl<W: SequentialSink, C: Cancellation> SequentialSink for BilevelImageWriter<'_, '_, W, C> {
    async fn write(&mut self, bytes: &[u8]) -> Result<usize> {
        if len_u64(bytes.len()) > self.remaining {
            return Err(Error::InvalidInput {
                reason: "bilevel image rows exceed the declared height",
            });
        }
        let mut done = 0;
        while done < bytes.len() {
            let row_left = self.stride - self.column;
            let count = row_left.min(bytes.len() - done);
            let kept = count.min(self.visible.saturating_sub(self.column));
            // Padding-only chunks still pass an empty slice, so a poisoned
            // writer or cancellation is reported for them too.
            self.document
                .writer
                .write_stream_bytes(&bytes[done..done + kept])
                .await?;
            // Account for the bytes only after the document accepted them.
            self.column = (self.column + count) % self.stride;
            self.remaining -= len_u64(count);
            done += count;
        }
        Ok(bytes.len())
    }

    /// Rows are flushed with the whole PDF by [`PdfDocument::finish`].
    async fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

impl<W: SequentialSink, C: Cancellation> BilevelImageWriter<'_, '_, W, C> {
    /// Close the image stream after exactly the declared rows were written.
    pub async fn finish(self) -> Result<ImageObject> {
        if self.remaining != 0 {
            return Err(Error::InvalidInput {
                reason: "bilevel image ended before its declared height",
            });
        }
        self.document.writer.end_stream().await?;
        Ok(ImageObject {
            object: self.object,
        })
    }
}

const OVERREAD: &str = "image source reported more bytes than requested";

struct CountingSource<'a, R> {
    inner: &'a mut R,
    total: &'a mut u64,
}

impl<R: RangedSource> RangedSource for CountingSource<'_, R> {
    fn size(&self) -> u64 {
        self.inner.size()
    }

    async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
        let read = self.inner.read_at(offset, destination).await?;
        let read = checked_read_count(read, destination.len(), OVERREAD)?;
        let read = len_u64(read);
        *self.total = self.total.checked_add(read).ok_or(Error::InvalidInput {
            reason: "image input byte count overflows",
        })?;
        Ok(read as usize)
    }
}

/// Build PDF pages and outline items without retaining image or PDF payloads.
///
/// Pages are emitted as they arrive. The writer retains one page object ID per
/// page for outline destinations, at most one active group at each page-tree
/// level, and one open outline item per active outline depth. A caller may
/// add a bookmark only after its destination page has been emitted. A document
/// with no pages is rejected by `finish`.
pub struct PdfDocument<'a, W: SequentialSink, C: Cancellation> {
    writer: PdfWriter<'a, W, C>,
    limits: &'a Limits,
    cancellation: &'a C,
    catalog_id: ObjectId,
    pages_root_id: ObjectId,
    root_children: Vec<ObjectId>,
    middle: Option<PageNode>,
    leaf: Option<PageNode>,
    page_ids: Vec<ObjectId>,
    pages_written: u32,
    outline_root_id: Option<ObjectId>,
    outline_root_children: ChildLinks,
    open_outlines: Vec<OpenOutline>,
    bookmarks_written: u32,
    retained_outlines: u32,
    retained_title_bytes: u64,
    input_bytes_read: u64,
    image_buffer: Vec<u8>,
}

impl<'a, W: SequentialSink, C: Cancellation> PdfDocument<'a, W, C> {
    /// Write the PDF header and reserve the catalog and Pages root.
    pub async fn new(sink: &'a mut W, limits: &'a Limits, cancellation: &'a C) -> Result<Self> {
        limits.validate()?;
        let mut writer = PdfWriter::new(sink, limits, cancellation).await?;
        let catalog_id = writer.reserve_object()?;
        let pages_root_id = writer.reserve_object()?;
        Ok(Self {
            writer,
            limits,
            cancellation,
            catalog_id,
            pages_root_id,
            root_children: Vec::new(),
            middle: None,
            leaf: None,
            page_ids: Vec::new(),
            pages_written: 0,
            outline_root_id: None,
            outline_root_children: ChildLinks::default(),
            open_outlines: Vec::new(),
            bookmarks_written: 0,
            retained_outlines: 0,
            retained_title_bytes: 0,
            input_bytes_read: 0,
            image_buffer: Vec::new(),
        })
    }

    /// Add one image that fills a new page, returning its zero-based page index.
    ///
    /// The input is read by checked ranges and never collected into a complete
    /// image buffer. More image placements can reuse the private page emission
    /// path in a later format handler.
    pub async fn add_image_page<R: RangedSource>(
        &mut self,
        source: &mut R,
        offset: u64,
        length: u64,
        page: PageSpec,
        image: ImageSpec,
    ) -> Result<u32> {
        let width = pdf_page_number(page.width_points)?;
        let height = pdf_page_number(page.height_points)?;
        image.validate(length)?;
        validate_image_range(
            self.limits,
            self.input_bytes_read,
            source.size(),
            offset,
            length,
        )?;
        self.check_next_page()?;
        self.reserve_page_index_slot()?;
        self.ensure_leaf().await?;

        let image_id = self
            .emit_image_xobject(source, offset, length, image)
            .await?;
        self.push_page(&width, &height, &[ImageObject { object: image_id }])
            .await
    }

    /// Start a 1 bpp image XObject whose rows the caller streams.
    ///
    /// The dimensions, row stride, and PDF stream length are checked before
    /// any output. No page is added; place the finished image with
    /// [`PdfDocument::add_page`]. The image must be finished before any other
    /// document operation.
    pub async fn begin_bilevel_image(
        &mut self,
        image: BilevelImageSpec,
    ) -> Result<BilevelImageWriter<'_, 'a, W, C>> {
        let (visible, remaining) = image.validate()?;
        let object = self.writer.reserve_object()?;
        let length_id = self.writer.reserve_object()?;
        let dictionary = format!(
            "/Type /XObject\n/Subtype /Image\n/Width {}\n/Height {}\n/ColorSpace /DeviceGray\n/BitsPerComponent 1\n/Decode [1 0]\n",
            image.pixel_width, image.pixel_height
        );
        self.writer
            .begin_stream(object, length_id, dictionary.as_bytes())
            .await?;
        Ok(BilevelImageWriter {
            document: self,
            object,
            visible,
            stride: image.row_stride,
            column: 0,
            remaining,
        })
    }

    /// Add a page showing previously finished images and return its zero-based
    /// page index. Each image is scaled to fill the whole page, drawn in
    /// slice order, so a later image paints over an earlier one.
    pub async fn add_page(&mut self, page: PageSpec, images: &[ImageObject]) -> Result<u32> {
        if images.is_empty() {
            return Err(Error::InvalidInput {
                reason: "PDF page requires at least one image",
            });
        }
        let width = pdf_page_number(page.width_points)?;
        let height = pdf_page_number(page.height_points)?;
        self.check_next_page()?;
        self.reserve_page_index_slot()?;
        self.ensure_leaf().await?;
        self.push_page(&width, &height, images).await
    }

    async fn push_page(
        &mut self,
        width: &str,
        height: &str,
        images: &[ImageObject],
    ) -> Result<u32> {
        let parent = self
            .leaf
            .as_ref()
            .ok_or(Error::InvalidInput {
                reason: "PDF page-tree leaf is missing",
            })?
            .id;
        let page_id = self.emit_page(parent, width, height, images).await?;

        let leaf = self.leaf.as_mut().ok_or(Error::InvalidInput {
            reason: "PDF page-tree leaf is missing",
        })?;
        reserve_bounded(
            &mut leaf.children,
            PAGE_TREE_FANOUT as u64,
            self.limits,
            "PDF page-tree leaf children",
        )?;
        leaf.children.push(page_id);
        leaf.page_count = leaf.page_count.checked_add(1).ok_or(Error::InvalidInput {
            reason: "PDF leaf page count overflows",
        })?;
        let middle = self.middle.as_mut().ok_or(Error::InvalidInput {
            reason: "PDF page-tree middle node is missing",
        })?;
        middle.page_count = middle
            .page_count
            .checked_add(1)
            .ok_or(Error::InvalidInput {
                reason: "PDF middle page count overflows",
            })?;
        let page_index = self.pages_written;
        self.pages_written = self
            .pages_written
            .checked_add(1)
            .ok_or(Error::InvalidInput {
                reason: "PDF page count overflows",
            })?;
        self.page_ids.push(page_id);
        Ok(page_index)
    }

    /// Add an outline item in preorder after its destination page is emitted.
    ///
    /// Depth starts at zero. Each new item may be a sibling, ancestor sibling,
    /// or direct child; a jump over a missing parent is rejected. Completed
    /// siblings and ancestors are written immediately, keeping only O(depth)
    /// titles and links in memory.
    pub async fn add_bookmark(&mut self, bookmark: Bookmark) -> Result<()> {
        let destination = self
            .page_ids
            .get(bookmark.page_index as usize)
            .copied()
            .ok_or(Error::InvalidInput {
                reason: "bookmark destination page has not been emitted",
            })?;
        self.limits
            .check_bookmarks(self.bookmarks_written.checked_add(1).ok_or(
                Error::InvalidInput {
                    reason: "bookmark count overflows",
                },
            )?)?;
        let depth = usize_from_u32(bookmark.depth);
        if depth > self.open_outlines.len() {
            return Err(Error::InvalidInput {
                reason: "bookmark depth skips a parent",
            });
        }
        let title_capacity = len_u64(bookmark.title.capacity());
        // A rejected title must leave all reserved objects and outline links
        // intact, so account for siblings that this insertion would emit first.
        let (next_titles, next_nodes) = self.preflight_outline_memory(depth, title_capacity)?;
        // Reserve before closing anything, so a refused reservation leaves
        // every open item and link untouched.
        let outline_root = match self.outline_root_id {
            Some(id) => id,
            None => {
                let id = self.writer.reserve_object()?;
                self.outline_root_id = Some(id);
                id
            }
        };
        let id = self.writer.reserve_object()?;
        let previous_item = self.close_outlines_to(depth).await?;
        let (parent, previous) = {
            let (parent, links) = match self.open_outlines.last_mut() {
                Some(active) => (active.id, &mut active.children),
                None => (outline_root, &mut self.outline_root_children),
            };
            let previous = links.last;
            if links.first.is_none() {
                links.first = Some(id);
            }
            links.last = Some(id);
            (parent, previous)
        };
        if let Some(previous_item) = previous_item {
            self.emit_outline_item(previous_item, Some(id)).await?;
        }
        self.open_outlines.push(OpenOutline {
            id,
            parent,
            previous,
            page: destination,
            title: bookmark.title,
            ordinal: self.bookmarks_written,
            children: ChildLinks::default(),
        });
        debug_assert_eq!(
            self.retained_title_bytes.checked_add(title_capacity),
            Some(next_titles)
        );
        debug_assert_eq!(self.retained_outlines.checked_add(1), Some(next_nodes));
        self.retained_title_bytes = next_titles;
        self.retained_outlines = next_nodes;
        self.bookmarks_written += 1;
        Ok(())
    }

    /// Write the remaining page tree, outlines, catalog, xref, and trailer.
    pub async fn finish(mut self) -> Result<ConversionReport> {
        if self.pages_written == 0 {
            return Err(Error::InvalidInput {
                reason: "PDF document requires at least one page",
            });
        }
        self.close_page_tree().await?;
        self.close_outlines().await?;

        self.writer.begin_object(self.catalog_id).await?;
        self.writer
            .write_bytes(
                format!(
                    "<< /Type /Catalog /Pages {} 0 R",
                    self.pages_root_id.number()
                )
                .as_bytes(),
            )
            .await?;
        if let Some(outline_root) = self.outline_root_id {
            self.writer
                .write_bytes(
                    format!(
                        " /Outlines {} 0 R /PageMode /UseOutlines",
                        outline_root.number()
                    )
                    .as_bytes(),
                )
                .await?;
        }
        self.writer.write_bytes(b" >>").await?;
        self.writer.end_object().await?;

        let output_bytes_written = self.writer.finish(self.catalog_id).await?;
        Ok(ConversionReport {
            input_bytes_read: self.input_bytes_read,
            output_bytes_written,
            pages_converted: self.pages_written,
            bookmarks_written: self.bookmarks_written,
        })
    }

    fn check_next_page(&self) -> Result<()> {
        let next = self
            .pages_written
            .checked_add(1)
            .ok_or(Error::InvalidInput {
                reason: "PDF page count overflows",
            })?;
        self.limits.check_pages(next)?;
        if u64::from(next) > MAX_TREE_PAGES {
            return Err(Error::LimitExceeded {
                resource: "PDF page-tree capacity",
                limit: MAX_TREE_PAGES,
                attempted: u64::from(next),
            });
        }
        Ok(())
    }

    fn reserve_page_index_slot(&mut self) -> Result<()> {
        reserve_bounded(
            &mut self.page_ids,
            u64::from(self.limits.max_pages).min(MAX_TREE_PAGES),
            self.limits,
            "PDF page index allocation",
        )
    }

    async fn ensure_leaf(&mut self) -> Result<()> {
        if self
            .leaf
            .as_ref()
            .is_some_and(|leaf| leaf.children.len() < PAGE_TREE_FANOUT)
        {
            return Ok(());
        }
        if let Some(leaf) = self.leaf.take() {
            self.emit_page_node(leaf).await?;
        }
        if self
            .middle
            .as_ref()
            .is_none_or(|middle| middle.children.len() == PAGE_TREE_FANOUT)
        {
            if let Some(middle) = self.middle.take() {
                let id = middle.id;
                self.emit_page_node(middle).await?;
                reserve_bounded(
                    &mut self.root_children,
                    PAGE_TREE_FANOUT as u64,
                    self.limits,
                    "PDF page-tree root children",
                )?;
                self.root_children.push(id);
            }
            let id = self.writer.reserve_object()?;
            self.middle = Some(PageNode {
                id,
                parent: self.pages_root_id,
                children: Vec::new(),
                page_count: 0,
            });
        }
        let middle = self.middle.as_mut().ok_or(Error::InvalidInput {
            reason: "PDF page-tree middle node is missing",
        })?;
        let id = self.writer.reserve_object()?;
        reserve_bounded(
            &mut middle.children,
            PAGE_TREE_FANOUT as u64,
            self.limits,
            "PDF page-tree middle children",
        )?;
        middle.children.push(id);
        self.leaf = Some(PageNode {
            id,
            parent: middle.id,
            children: Vec::new(),
            page_count: 0,
        });
        Ok(())
    }

    async fn close_page_tree(&mut self) -> Result<()> {
        if let Some(leaf) = self.leaf.take() {
            self.emit_page_node(leaf).await?;
        }
        if let Some(middle) = self.middle.take() {
            let id = middle.id;
            self.emit_page_node(middle).await?;
            reserve_bounded(
                &mut self.root_children,
                PAGE_TREE_FANOUT as u64,
                self.limits,
                "PDF page-tree root children",
            )?;
            self.root_children.push(id);
        }
        self.writer.begin_object(self.pages_root_id).await?;
        self.writer
            .write_bytes(
                format!("<< /Type /Pages /Count {} /Kids [", self.pages_written).as_bytes(),
            )
            .await?;
        for child in &self.root_children {
            self.writer
                .write_bytes(format!(" {} 0 R", child.number()).as_bytes())
                .await?;
        }
        self.writer.write_bytes(b" ] >>").await?;
        self.writer.end_object().await
    }

    async fn emit_page_node(&mut self, node: PageNode) -> Result<()> {
        self.writer.begin_object(node.id).await?;
        self.writer
            .write_bytes(
                format!(
                    "<< /Type /Pages /Parent {} 0 R /Count {} /Kids [",
                    node.parent.number(),
                    node.page_count
                )
                .as_bytes(),
            )
            .await?;
        for child in node.children {
            self.writer
                .write_bytes(format!(" {} 0 R", child.number()).as_bytes())
                .await?;
        }
        self.writer.write_bytes(b" ] >>").await?;
        self.writer.end_object().await
    }

    async fn emit_image_xobject<R: RangedSource>(
        &mut self,
        source: &mut R,
        offset: u64,
        length: u64,
        image: ImageSpec,
    ) -> Result<ObjectId> {
        let image_id = self.writer.reserve_object()?;
        let length_id = self.writer.reserve_object()?;
        let dictionary = image.dictionary();
        self.writer
            .begin_stream(image_id, length_id, dictionary.as_bytes())
            .await?;
        let chunk_size = length.min(self.limits.io_chunk_bytes as u64) as usize;
        if self.image_buffer.len() < chunk_size {
            let refused = self
                .limits
                .allocation_refused("PDF image I/O buffer", chunk_size as u64);
            let additional = chunk_size - self.image_buffer.len();
            reserve_exact(&mut self.image_buffer, additional, refused)?;
            self.image_buffer.resize(chunk_size, 0);
        }
        let mut source = CountingSource {
            inner: source,
            total: &mut self.input_bytes_read,
        };
        let mut done = 0_u64;
        while done < length {
            let chunk = (length - done).min(self.image_buffer.len() as u64) as usize;
            let position = offset.checked_add(done).ok_or(Error::InvalidInput {
                reason: "image range offset overflows",
            })?;
            read_exact_at(
                &mut source,
                position,
                &mut self.image_buffer[..chunk],
                self.limits,
                self.cancellation,
            )
            .await?;
            self.writer
                .write_stream_bytes(&self.image_buffer[..chunk])
                .await?;
            done = done.checked_add(chunk as u64).ok_or(Error::InvalidInput {
                reason: "image input byte count overflows",
            })?;
        }
        self.writer.end_stream().await?;
        Ok(image_id)
    }

    async fn emit_page(
        &mut self,
        parent: ObjectId,
        width: &str,
        height: &str,
        images: &[ImageObject],
    ) -> Result<ObjectId> {
        let content_id = self.writer.reserve_object()?;
        let content_length_id = self.writer.reserve_object()?;
        let page_id = self.writer.reserve_object()?;

        self.writer
            .begin_stream(content_id, content_length_id, b"")
            .await?;
        for index in 0..images.len() {
            self.writer
                .write_stream_bytes(
                    format!("q\n{width} 0 0 {height} 0 0 cm\n/Im{index} Do\nQ\n").as_bytes(),
                )
                .await?;
        }
        self.writer.end_stream().await?;

        self.writer.begin_object(page_id).await?;
        self.writer.write_bytes(format!("<< /Type /Page /Parent {} 0 R /MediaBox [0 0 {width} {height}] /Resources << /XObject <<", parent.number()).as_bytes()).await?;
        for (index, image) in images.iter().enumerate() {
            self.writer
                .write_bytes(format!(" /Im{index} {} 0 R", image.object.number()).as_bytes())
                .await?;
        }
        self.writer
            .write_bytes(format!(" >> >> /Contents {} 0 R >>", content_id.number()).as_bytes())
            .await?;
        self.writer.end_object().await?;
        Ok(page_id)
    }

    fn preflight_outline_memory(
        &mut self,
        depth: usize,
        title_capacity: u64,
    ) -> Result<(u64, u32)> {
        let mut released_titles = 0_u64;
        let mut released_nodes = 0_u32;
        let mut release = |capacity: usize| -> Result<()> {
            let capacity = len_u64(capacity);
            released_titles = released_titles
                .checked_add(capacity)
                .ok_or(Error::InvalidInput {
                    reason: "released bookmark title bytes overflow",
                })?;
            released_nodes = released_nodes.checked_add(1).ok_or(Error::InvalidInput {
                reason: "released bookmark count overflows",
            })?;
            Ok(())
        };
        // Every item that inserting at `depth` closes is written before the
        // new item is retained; no other closed item is ever held unwritten.
        for item in self.open_outlines.iter().skip(depth) {
            release(item.title.capacity())?;
        }
        let next_titles = self
            .retained_title_bytes
            .checked_sub(released_titles)
            .and_then(|retained| retained.checked_add(title_capacity))
            .ok_or(Error::InvalidInput {
                reason: "retained bookmark title bytes overflow",
            })?;
        let next_nodes = self
            .retained_outlines
            .checked_sub(released_nodes)
            .and_then(|retained| retained.checked_add(1))
            .ok_or(Error::InvalidInput {
                reason: "retained bookmark count overflows",
            })?;
        let node_bytes = u64::from(next_nodes)
            .checked_mul(size_of::<OpenOutline>() as u64)
            .ok_or(Error::InvalidInput {
                reason: "retained bookmark metadata bytes overflow",
            })?;
        let attempted = node_bytes
            .checked_add(next_titles)
            .ok_or(Error::InvalidInput {
                reason: "retained bookmark allocation overflows",
            })?;
        self.limits.check_allocation(attempted)?;
        if depth == self.open_outlines.len() {
            reserve_bounded(
                &mut self.open_outlines,
                u64::from(self.limits.max_bookmarks),
                self.limits,
                "bookmark stack allocation",
            )?;
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
    async fn close_outlines_to(&mut self, depth: usize) -> Result<Option<ClosedOutline>> {
        let mut closed = None;
        while let Some(item) = super::pop_deeper_than(&mut self.open_outlines, depth) {
            closed = Some(self.close_outline(item, closed).await?);
        }
        Ok(closed)
    }

    async fn close_outline(
        &mut self,
        item: OpenOutline,
        last_child: Option<ClosedOutline>,
    ) -> Result<ClosedOutline> {
        if let Some(last_child) = last_child {
            self.emit_outline_item(last_child, None).await?;
        }
        let descendants =
            self.bookmarks_written
                .checked_sub(item.ordinal + 1)
                .ok_or(Error::InvalidInput {
                    reason: "bookmark descendant count underflows",
                })?;
        Ok(ClosedOutline {
            id: item.id,
            parent: item.parent,
            previous: item.previous,
            page: item.page,
            title: item.title,
            first_child: item.children.first,
            last_child: item.children.last,
            descendants,
        })
    }

    async fn close_outlines(&mut self) -> Result<()> {
        if let Some(last_root) = self.close_outlines_to(0).await? {
            self.emit_outline_item(last_root, None).await?;
        }
        if let Some(root) = self.outline_root_id {
            let first = self
                .outline_root_children
                .first
                .ok_or(Error::InvalidInput {
                    reason: "outline root has no first child",
                })?;
            let last = self.outline_root_children.last.ok_or(Error::InvalidInput {
                reason: "outline root has no last child",
            })?;
            self.writer
                .write_object(
                    root,
                    format!(
                        "<< /Type /Outlines /First {} 0 R /Last {} 0 R /Count {} >>",
                        first.number(),
                        last.number(),
                        self.bookmarks_written
                    )
                    .as_bytes(),
                )
                .await?;
        }
        Ok(())
    }

    async fn emit_outline_item(
        &mut self,
        item: ClosedOutline,
        next: Option<ObjectId>,
    ) -> Result<()> {
        self.writer.begin_object(item.id).await?;
        self.writer.write_bytes(b"<< /Title <FEFF").await?;
        let mut hex = [0_u8; 4096];
        let mut used = 0;
        for unit in item.title.encode_utf16() {
            if used == hex.len() {
                self.writer.write_bytes(&hex).await?;
                used = 0;
            }
            let [high, low] = unit.to_be_bytes();
            for byte in [high, low] {
                hex[used] = HEX[(byte >> 4) as usize];
                hex[used + 1] = HEX[(byte & 0x0f) as usize];
                used += 2;
            }
        }
        if used > 0 {
            self.writer.write_bytes(&hex[..used]).await?;
        }
        self.writer
            .write_bytes(
                format!(
                    "> /Parent {} 0 R /Dest [{} 0 R /Fit]",
                    item.parent.number(),
                    item.page.number()
                )
                .as_bytes(),
            )
            .await?;
        if let Some(previous) = item.previous {
            self.writer
                .write_bytes(format!(" /Prev {} 0 R", previous.number()).as_bytes())
                .await?;
        }
        if let Some(next) = next {
            self.writer
                .write_bytes(format!(" /Next {} 0 R", next.number()).as_bytes())
                .await?;
        }
        if let Some(first) = item.first_child {
            let last = item.last_child.ok_or(Error::InvalidInput {
                reason: "bookmark has a first child but no last child",
            })?;
            self.writer
                .write_bytes(
                    format!(
                        " /First {} 0 R /Last {} 0 R /Count {}",
                        first.number(),
                        last.number(),
                        item.descendants
                    )
                    .as_bytes(),
                )
                .await?;
        }
        self.writer.write_bytes(b" >>").await?;
        self.writer.end_object().await?;
        self.retained_title_bytes = self
            .retained_title_bytes
            .checked_sub(item.title.capacity() as u64)
            .ok_or(Error::InvalidInput {
                reason: "retained bookmark title bytes underflow",
            })?;
        self.retained_outlines =
            self.retained_outlines
                .checked_sub(1)
                .ok_or(Error::InvalidInput {
                    reason: "retained bookmark count underflows",
                })?;
        Ok(())
    }
}

/// Check one image payload range. Only the source size is needed, so every
/// sink, cancellation, and source type shares this check.
fn validate_image_range(
    limits: &Limits,
    input_bytes_read: u64,
    source_size: u64,
    offset: u64,
    length: u64,
) -> Result<()> {
    limits.check_input_size(source_size)?;
    if length > MAX_PDF_INTEGER {
        return Err(Error::LimitExceeded {
            resource: "PDF image stream bytes",
            limit: MAX_PDF_INTEGER,
            attempted: length,
        });
    }
    if offset > source_size {
        return Err(Error::InvalidInput {
            reason: "image range starts beyond source size",
        });
    }
    if length > source_size - offset {
        return Err(Error::TruncatedInput {
            offset,
            expected: length,
            available: source_size - offset,
        });
    }
    input_bytes_read
        .checked_add(length)
        .ok_or(Error::InvalidInput {
            reason: "image input byte count overflows",
        })?;
    Ok(())
}

impl<W: SequentialSink, C: Cancellation> BookmarkVisitor for PdfDocument<'_, W, C> {
    async fn visit(&mut self, bookmark: Bookmark) -> Result<()> {
        self.add_bookmark(bookmark).await
    }
}

impl ImageSpec {
    fn validate(self, length: u64) -> Result<()> {
        check_image_dimensions(self.pixel_width, self.pixel_height)?;
        let channels = match self.encoding {
            ImageEncoding::Gray8 | ImageEncoding::JpegGray8 => 1_u64,
            ImageEncoding::Rgb8 | ImageEncoding::JpegRgb8 => 3_u64,
        };
        if matches!(self.encoding, ImageEncoding::Gray8 | ImageEncoding::Rgb8) {
            let expected = u64::from(self.pixel_width)
                .checked_mul(u64::from(self.pixel_height))
                .and_then(|pixels| pixels.checked_mul(channels))
                .ok_or(Error::InvalidInput {
                    reason: "raw image byte count overflows",
                })?;
            if length != expected {
                return Err(Error::InvalidInput {
                    reason: "raw image byte count does not match dimensions",
                });
            }
        } else if length == 0 {
            return Err(Error::InvalidInput {
                reason: "JPEG image stream must not be empty",
            });
        }
        Ok(())
    }

    fn dictionary(self) -> String {
        let color = match self.encoding {
            ImageEncoding::Gray8 | ImageEncoding::JpegGray8 => "DeviceGray",
            ImageEncoding::Rgb8 | ImageEncoding::JpegRgb8 => "DeviceRGB",
        };
        let filter = match self.encoding {
            ImageEncoding::Gray8 | ImageEncoding::Rgb8 => "",
            ImageEncoding::JpegGray8 | ImageEncoding::JpegRgb8 => "/Filter /DCTDecode\n",
        };
        format!(
            "/Type /XObject\n/Subtype /Image\n/Width {}\n/Height {}\n/ColorSpace /{color}\n/BitsPerComponent 8\n{filter}",
            self.pixel_width, self.pixel_height
        )
    }
}

impl BilevelImageSpec {
    /// Return the kept bytes per row and the total input bytes after
    /// checking every PDF size.
    fn validate(self) -> Result<(usize, u64)> {
        check_image_dimensions(self.pixel_width, self.pixel_height)?;
        let visible = self.pixel_width.div_ceil(8);
        if len_u64(self.row_stride) < u64::from(visible) {
            return Err(Error::InvalidInput {
                reason: "bilevel row stride is shorter than the packed row",
            });
        }
        let stream = u64::from(visible) * u64::from(self.pixel_height);
        if stream > MAX_PDF_INTEGER {
            return Err(Error::LimitExceeded {
                resource: "PDF image stream bytes",
                limit: MAX_PDF_INTEGER,
                attempted: stream,
            });
        }
        let input = len_u64(self.row_stride)
            .checked_mul(u64::from(self.pixel_height))
            .ok_or(Error::InvalidInput {
                reason: "bilevel input byte count overflows",
            })?;
        Ok((visible as usize, input))
    }
}

fn check_image_dimensions(width: u32, height: u32) -> Result<()> {
    if width == 0 || height == 0 {
        return Err(Error::InvalidInput {
            reason: "image width and height must be nonzero",
        });
    }
    for (resource, value) in [("PDF image width", width), ("PDF image height", height)] {
        if u64::from(value) > MAX_PDF_INTEGER {
            return Err(Error::LimitExceeded {
                resource,
                limit: MAX_PDF_INTEGER,
                attempted: u64::from(value),
            });
        }
    }
    Ok(())
}

fn pdf_page_number(value: f64) -> Result<String> {
    if !value.is_finite() || !(MIN_PAGE_POINTS..=MAX_PAGE_POINTS).contains(&value) {
        return Err(Error::InvalidInput {
            reason: "page dimension must be finite and within 0.000001..=14400 points",
        });
    }
    Ok(format!("{value:.6}"))
}

fn reserve_bounded<T>(
    values: &mut Vec<T>,
    maximum_items: u64,
    limits: &Limits,
    resource: &'static str,
) -> Result<()> {
    let needed = values.len().checked_add(1).ok_or(Error::InvalidInput {
        reason: "PDF index count overflows address space",
    })?;
    let needed_u64 = len_u64(needed);
    if needed_u64 > maximum_items {
        return Err(Error::LimitExceeded {
            resource,
            limit: maximum_items,
            attempted: needed_u64,
        });
    }
    let element_bytes = len_u64(size_of::<T>().max(1));
    let needed_bytes = needed_u64
        .checked_mul(element_bytes)
        .ok_or(Error::InvalidInput {
            reason: "PDF index allocation overflows 64 bits",
        })?;
    limits.check_allocation(needed_bytes)?;
    if needed <= values.capacity() {
        return Ok(());
    }
    let doubled = values.capacity().max(1).saturating_mul(2);
    let proposed = doubled.max(needed);
    let maximum = usize::try_from(maximum_items).unwrap_or(usize::MAX);
    let by_bytes =
        usize::try_from(limits.max_allocation_bytes / element_bytes).unwrap_or(usize::MAX);
    let target = proposed.min(maximum).min(by_bytes);
    let requested_bytes = u64::try_from(target)
        .ok()
        .and_then(|count| count.checked_mul(element_bytes))
        .ok_or(Error::InvalidInput {
            reason: "PDF index allocation overflows 64 bits",
        })?;
    limits.check_allocation(requested_bytes)?;
    let refused = limits.allocation_refused(resource, requested_bytes);
    let additional = target - values.len();
    reserve_exact(values, additional, refused)?;
    // The limit covers the capacity requested here. Vec may receive extra
    // capacity from its allocator, which is outside the handler's request.
    Ok(())
}

#[cfg(test)]
mod tests;
