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

/// Leading bytes that [`detect_format`] needs to recognize every signature.
pub const SIGNATURE_BYTES: usize = 5;

/// Recognize an input family from its leading bytes, never from a file name:
/// some observed `.caj` files are plain PDFs.
///
/// `prefix` should hold the first `min(SIGNATURE_BYTES, size)` bytes; extra
/// bytes are ignored. The signatures are those recorded in
/// `tests/fixtures/README.md`, `docs/caj-format.md`, `docs/kdh-format.md`,
/// and `docs/hnc8-container.md`. Recognition does not imply support; the
/// selected reader validates the complete header.
pub fn detect_format(prefix: &[u8]) -> Option<InputFormat> {
    const SIGNATURES: [(&[u8], InputFormat); 6] = [
        (b"%PDF-", InputFormat::Pdf),
        (b"CAJ", InputFormat::Caj),
        (b"KDH", InputFormat::Kdh),
        (b"HN", InputFormat::Hn),
        (b"\xc8\0\0\0", InputFormat::C8),
        (b"TEB", InputFormat::Teb),
    ];
    SIGNATURES
        .iter()
        .find(|(signature, _)| prefix.starts_with(signature))
        .map(|&(_, format)| format)
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

#[cfg(test)]
mod tests {
    use super::{InputFormat, SIGNATURE_BYTES, detect_format};

    #[test]
    fn detects_observed_signatures_only_at_the_start() {
        let cases: [(&[u8], Option<InputFormat>); 12] = [
            (b"%PDF-1.7", Some(InputFormat::Pdf)),
            (b"CAJ\0", Some(InputFormat::Caj)),
            (b"KDH 2.00", Some(InputFormat::Kdh)),
            (b"HN\0\0", Some(InputFormat::Hn)),
            (b"\xc8\0\0\0\x01", Some(InputFormat::C8)),
            (b"TEB", Some(InputFormat::Teb)),
            (b"%PDF", None),
            (b"\xc8\0\0", None),
            (b"NH\0\0", None),
            (b" %PDF-", None),
            (b"caj", None),
            (b"", None),
        ];
        for (prefix, expected) in cases {
            assert_eq!(detect_format(prefix), expected, "{prefix:?}");
        }
    }

    #[test]
    fn signature_bytes_cover_the_longest_signature() {
        assert_eq!(
            detect_format(&b"%PDF-1.7"[..SIGNATURE_BYTES]),
            Some(InputFormat::Pdf)
        );
        assert_eq!(detect_format(&b"%PDF-"[..SIGNATURE_BYTES - 1]), None);
    }
}
