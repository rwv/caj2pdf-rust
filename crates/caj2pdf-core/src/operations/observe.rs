// SPDX-License-Identifier: MIT

//! A caller's [`Progress`] as the format engines see it: a [`Cancellation`],
//! and a document source that reports its furthest byte read.

use super::{Detection, InputFormat, detect_source};
use crate::{Cancellation, ErrorKind, Limits, NeverCancel, RangedSource, Result};
use std::cell::RefCell;

/// What a facade operation tells its caller while it runs, and how the
/// caller stops it. Every method has a default; [`NeverCancel`] uses them
/// all.
pub trait Progress {
    /// The input family, given in the options or detected from the leading
    /// signature; `None` when the input is empty or unrecognized. Called once,
    /// before the operation refuses the family or reads past the signature.
    fn format(&mut self, _format: Option<InputFormat>) {}

    /// The furthest document byte read so far advanced to `done` of the
    /// document's `total` bytes. Format engines read their indexes first and
    /// then page payloads in order, so this advances with the operation.
    fn input_read(&mut self, _done: u64, _total: u64) {}

    /// Whether the caller asked to stop; checked between rows, pages and I/O
    /// chunks. A cancelled operation fails with [`ErrorKind::Cancelled`] and
    /// does not undo bytes already written.
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// Reports nothing and never cancels.
impl Progress for NeverCancel {}

/// The refusal of an empty, unrecognized or unaccepted input: an
/// [`ErrorKind::UnsupportedFormat`] error without offset, context or reason.
/// [`Progress::format`] has named the family, if any.
pub(super) fn refused() -> crate::Error {
    ErrorKind::UnsupportedFormat.into()
}

/// One operation's view of a [`Progress`].
pub(super) struct Observer<'p> {
    progress: RefCell<&'p mut dyn Progress>,
}

impl<'p> Observer<'p> {
    pub(super) fn new(progress: &'p mut dyn Progress) -> Self {
        Self {
            progress: RefCell::new(progress),
        }
    }

    /// `source` reporting its furthest byte read to the caller.
    pub(super) fn track<'a, S: RangedSource>(&'a self, source: &'a mut S) -> Tracked<'a, 'p, S> {
        Tracked {
            source,
            observer: self,
            furthest: 0,
        }
    }

    /// Use `format`, or detect it, and report it; an empty or unrecognized
    /// input is then refused. An explicit format skips detection, so an
    /// explicit PDF must start with its `%PDF-` header.
    pub(super) fn resolve<S: RangedSource>(
        &self,
        source: &mut S,
        format: Option<InputFormat>,
        limits: &Limits,
    ) -> Result<Detection> {
        let detection = match format {
            Some(format) => Some(Detection {
                format,
                header_offset: 0,
                bytes_read: 0,
            }),
            None => detect_source(source, limits, self)?,
        };
        self.progress
            .borrow_mut()
            .format(detection.map(|detection| detection.format));
        detection.ok_or_else(refused)
    }
}

impl Cancellation for Observer<'_> {
    fn is_cancelled(&self) -> bool {
        self.progress.borrow().is_cancelled()
    }
}

/// The document source of one operation.
pub(super) struct Tracked<'a, 'p, S> {
    source: &'a mut S,
    observer: &'a Observer<'p>,
    furthest: u64,
}

impl<S: RangedSource> RangedSource for Tracked<'_, '_, S> {
    fn size(&self) -> u64 {
        self.source.size()
    }

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
        let count = self.source.read_at(offset, destination)?;
        let end = offset.saturating_add(count as u64);
        if end > self.furthest {
            self.furthest = end;
            let total = self.source.size();
            self.observer.progress.borrow_mut().input_read(end, total);
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Recorded {
        formats: Vec<Option<InputFormat>>,
        reads: Vec<(u64, u64)>,
    }

    impl Progress for Recorded {
        fn format(&mut self, format: Option<InputFormat>) {
            self.formats.push(format);
        }

        fn input_read(&mut self, done: u64, total: u64) {
            self.reads.push((done, total));
        }
    }

    #[test]
    fn the_furthest_byte_read_is_reported_once_per_advance() {
        let mut recorded = Recorded::default();
        {
            let observer = Observer::new(&mut recorded);
            let bytes = [7; 200];
            let mut slice = &bytes[..];
            let mut source = observer.track(&mut slice);
            let mut buffer = [0; 100];
            assert_eq!(source.read_at(0, &mut buffer).unwrap(), 100);
            source.read_at(0, &mut buffer[..10]).unwrap();
            source.read_at(100, &mut buffer[..1]).unwrap();
            source.read_at(150, &mut buffer).unwrap();
            assert!(source.read_at(201, &mut buffer).is_err());
            assert_eq!(source.size(), 200);
            assert!(!observer.is_cancelled());
        }
        assert_eq!(recorded.reads, [(100, 200), (101, 200), (200, 200)]);
    }

    #[test]
    fn the_format_is_reported_before_an_unrecognized_input_is_refused() {
        let mut recorded = Recorded::default();
        {
            let observer = Observer::new(&mut recorded);
            let limits = Limits::default();
            let error = observer
                .resolve(&mut &b"unknown"[..], None, &limits)
                .unwrap_err();
            assert!(matches!(error.kind, ErrorKind::UnsupportedFormat));
            assert_eq!(error.to_string(), "unsupported input format");
            let explicit = observer
                .resolve(&mut &b""[..], Some(InputFormat::Teb), &limits)
                .unwrap();
            assert_eq!(
                (explicit.format, explicit.bytes_read),
                (InputFormat::Teb, 0)
            );
        }
        assert_eq!(recorded.formats, [None, Some(InputFormat::Teb)]);
    }
}
