// SPDX-License-Identifier: MIT

use std::{fmt, io};

/// The category of a located PDF input or repair failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdfErrorKind {
    Malformed,
    Encrypted,
    UnsupportedFeature,
    AmbiguousRepair,
}

impl fmt::Display for PdfErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Malformed => "malformed",
            Self::Encrypted => "encrypted",
            Self::UnsupportedFeature => "unsupported feature",
            Self::AmbiguousRepair => "ambiguous repair",
        })
    }
}

/// A failure visible to native and JavaScript callers.
#[derive(Debug)]
pub enum Error {
    /// The input's format or a format feature is not supported.
    UnsupportedFormat,
    /// A malformed range, count, or field was supplied.
    InvalidInput { reason: &'static str },
    /// The source ended before a required range was complete.
    TruncatedInput {
        offset: u64,
        expected: u64,
        available: u64,
    },
    /// A configured resource bound would be exceeded.
    LimitExceeded {
        resource: &'static str,
        limit: u64,
        attempted: u64,
    },
    /// A source or sink failed.
    Io(io::Error),
    /// The caller cancelled the operation.
    Cancelled,
    /// A forward-only source was supplied without a seekable spool.
    RandomAccessRequired,
    /// A located problem in a CAJ container field or record.
    Caj {
        /// Absolute byte offset in the input source.
        offset: u64,
        /// One-based TOC or page-table record number, when applicable.
        record: Option<u32>,
        reason: &'static str,
    },
    /// A resource bound hit while parsing a located CAJ structure.
    CajLimitExceeded {
        offset: u64,
        record: Option<u32>,
        resource: &'static str,
        limit: u64,
        attempted: u64,
    },
    /// A located problem in a KDH wrapper or its encoded PDF boundary.
    Kdh {
        /// Absolute byte offset in the KDH input.
        offset: u64,
        reason: &'static str,
    },
    /// A located problem in an embedded or standalone PDF.
    Pdf {
        /// Absolute byte offset in the input source.
        offset: u64,
        /// The related indirect object number and generation, if known.
        object: Option<(u32, u16)>,
        kind: PdfErrorKind,
        reason: &'static str,
    },
    /// A resource bound hit while parsing a located PDF structure.
    PdfLimitExceeded {
        /// Absolute byte offset in the input source.
        offset: u64,
        /// The related indirect object number and generation, if known.
        object: Option<(u32, u16)>,
        resource: &'static str,
        limit: u64,
        attempted: u64,
    },
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedFormat => f.write_str("unsupported input format"),
            Self::InvalidInput { reason } => write!(f, "invalid input: {reason}"),
            Self::TruncatedInput {
                offset,
                expected,
                available,
            } => write!(
                f,
                "truncated input at offset {offset}: needed {expected} bytes, got {available}"
            ),
            Self::LimitExceeded {
                resource,
                limit,
                attempted,
            } => write!(
                f,
                "{resource} limit exceeded: maximum {limit}, attempted {attempted}"
            ),
            Self::Io(error) => write!(f, "I/O error: {error}"),
            Self::Cancelled => f.write_str("operation cancelled"),
            Self::RandomAccessRequired => f.write_str("random-access input required"),
            Self::Caj {
                offset,
                record,
                reason,
            } => {
                write!(f, "malformed CAJ at byte {offset}")?;
                if let Some(record) = record {
                    write!(f, ", record {record}")?;
                }
                write!(f, ": {reason}")
            }
            Self::CajLimitExceeded {
                offset,
                record,
                resource,
                limit,
                attempted,
            } => {
                write!(f, "CAJ {resource} limit exceeded at byte {offset}")?;
                if let Some(record) = record {
                    write!(f, ", record {record}")?;
                }
                write!(f, ": maximum {limit}, attempted {attempted}")
            }
            Self::Kdh { offset, reason } => {
                write!(f, "malformed KDH at byte {offset}: {reason}")
            }
            Self::Pdf {
                offset,
                object,
                kind,
                reason,
            } => {
                write!(f, "{kind} PDF at byte {offset}")?;
                if let Some((number, generation)) = object {
                    write!(f, ", object {number} {generation}")?;
                }
                write!(f, ": {reason}")
            }
            Self::PdfLimitExceeded {
                offset,
                object,
                resource,
                limit,
                attempted,
            } => {
                write!(f, "PDF {resource} limit exceeded at byte {offset}")?;
                if let Some((number, generation)) = object {
                    write!(f, ", object {number} {generation}")?;
                }
                write!(f, ": maximum {limit}, attempted {attempted}")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl Error {
    /// Attach a CAJ location to an unlocated resource-limit error. Every other
    /// error is returned unchanged.
    pub(crate) fn locate_caj_limit(self, offset: u64, record: Option<u32>) -> Self {
        match self {
            Self::LimitExceeded {
                resource,
                limit,
                attempted,
            } => Self::CajLimitExceeded {
                offset,
                record,
                resource,
                limit,
                attempted,
            },
            other => other,
        }
    }

    /// Attach a PDF location to an unlocated resource-limit error. Every other
    /// error is returned unchanged.
    pub(crate) fn locate_pdf_limit(self, offset: u64, object: Option<(u32, u16)>) -> Self {
        match self {
            Self::LimitExceeded {
                resource,
                limit,
                attempted,
            } => Self::PdfLimitExceeded {
                offset,
                object,
                resource,
                limit,
                attempted,
            },
            other => other,
        }
    }
}

#[cfg(test)]
mod tests;
