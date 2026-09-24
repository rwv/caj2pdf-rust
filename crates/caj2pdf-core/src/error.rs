// SPDX-License-Identifier: MIT

use std::{fmt, io};

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
