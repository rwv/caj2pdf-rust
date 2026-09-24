// SPDX-License-Identifier: MIT

use crate::{Cancellation, Limits, RangedSource, Result, SequentialSink};

/// Recognized input families. Recognition does not imply conversion support.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputFormat {
    Pdf,
    Caj,
    Kdh,
    Nh,
    Hn,
    C8,
    Teb,
}

/// Bounded summary returned by inspection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DocumentInfo {
    pub format: InputFormat,
    pub page_count: u32,
    /// `None` if counting bookmarks requires a separate pass.
    pub bookmark_count: Option<u32>,
}

/// One outline entry in document order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Bookmark {
    /// Zero means a root entry; each child increments the depth by one.
    pub depth: u32,
    pub title: String,
    /// Zero-based page index.
    pub page_index: u32,
}

/// A backpressure-aware recipient for streamed bookmark entries.
#[allow(async_fn_in_trait)]
pub trait BookmarkVisitor {
    async fn visit(&mut self, bookmark: Bookmark) -> Result<()>;
}

/// Options shared by platform adapters and format implementations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConversionOptions {
    pub include_bookmarks: bool,
}

impl Default for ConversionOptions {
    fn default() -> Self {
        Self {
            include_bookmarks: true,
        }
    }
}

/// Counters from a completed conversion or bounded I/O proof.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ConversionReport {
    pub input_bytes_read: u64,
    pub output_bytes_written: u64,
    pub pages_converted: u32,
    pub bookmarks_written: u32,
}

/// Operation signatures for format engines built on the same I/O contract.
///
/// This issue defines the interface only; format engines arrive in later
/// issues. Bookmark enumeration uses a visitor to avoid a whole-outline
/// allocation. An implementation must enforce `Limits` and cancellation.
#[allow(async_fn_in_trait)]
pub trait DocumentOperations {
    async fn inspect<S: RangedSource, C: Cancellation>(
        &self,
        source: &mut S,
        limits: &Limits,
        cancellation: &C,
    ) -> Result<DocumentInfo>;

    async fn visit_bookmarks<S: RangedSource, V: BookmarkVisitor, C: Cancellation>(
        &self,
        source: &mut S,
        visitor: &mut V,
        limits: &Limits,
        cancellation: &C,
    ) -> Result<u32>;

    async fn convert<S: RangedSource, W: SequentialSink, C: Cancellation>(
        &self,
        source: &mut S,
        sink: &mut W,
        options: ConversionOptions,
        limits: &Limits,
        cancellation: &C,
    ) -> Result<ConversionReport>;

    async fn import_bookmarks<
        Outline: RangedSource,
        Pdf: RangedSource,
        W: SequentialSink,
        C: Cancellation,
    >(
        &self,
        outline_source: &mut Outline,
        pdf_source: &mut Pdf,
        sink: &mut W,
        limits: &Limits,
        cancellation: &C,
    ) -> Result<ConversionReport>;
}
