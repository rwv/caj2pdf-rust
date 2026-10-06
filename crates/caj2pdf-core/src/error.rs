// SPDX-License-Identifier: MIT

use crate::hnc8::Variant;
use std::{fmt, io};

/// A failure visible to native and JavaScript callers: what went wrong, the
/// absolute input offset and the structure it was found in, when known, and
/// a fixed description.
#[derive(Debug)]
pub struct Error {
    pub kind: ErrorKind,
    /// Absolute byte offset in the input when known. For a [`Context::Pdf`]
    /// error it is in the whole input, not in an embedded PDF range.
    pub offset: Option<u64>,
    pub context: Context,
    /// What was wrong or which field; empty when the kind says it all.
    pub reason: &'static str,
}

/// What went wrong.
#[derive(Debug)]
pub enum ErrorKind {
    /// The input's format or a feature of it is not supported.
    UnsupportedFormat,
    /// A structure or field the format does not allow. Without a context it
    /// is an invalid caller-supplied range, count or option.
    Malformed,
    Encrypted,
    /// The source ended before a required range was complete.
    Truncated {
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
}

/// The structure a failure was located in.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Context {
    #[default]
    None,
    /// A CAJ container field, or a one-based TOC or page-table record.
    Caj { record: Option<u32> },
    /// A KDH wrapper or its encoded PDF boundary.
    Kdh,
    /// An embedded or standalone PDF. `repair` marks a damaged structure
    /// whose repair would be ambiguous.
    Pdf {
        object: Option<(u32, u16)>,
        repair: bool,
    },
    /// An HN/C8 container, page or image, with one-based numbers, the JBIG2
    /// segment of a type-3 image, and the conversion stage when converting.
    Hnc8 {
        variant: Option<Variant>,
        page: Option<u32>,
        image: Option<u32>,
        segment: Option<u32>,
        stage: Option<Hnc8Stage>,
    },
    /// A JBIG2 segment, by its segment number.
    Jbig2 { segment: Option<u32> },
}

/// The HN/C8 conversion stage a failure arose in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Hnc8Stage {
    Preflight,
    Container,
    Text,
    Headers,
    Geometry,
    Decode,
    Pdf,
    Visitor,
    /// A stage of type-3 JBIG2 image decoding.
    Type3(Type3Stage),
}

/// The type-3 image decoding stage a failure arose in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Type3Stage {
    Directory,
    PageInfo,
    TextHeader,
    GenericHeader,
    Profile,
    FirstDictionary,
    SecondDictionary,
    TextInstances,
    TextCompose,
    GenericRegion,
    PageCompose,
    Contexts,
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<ErrorKind> for Error {
    fn from(kind: ErrorKind) -> Self {
        Self {
            kind,
            offset: None,
            context: Context::None,
            reason: "",
        }
    }
}

impl Error {
    /// An invalid caller-supplied range, count or option.
    pub fn invalid(reason: &'static str) -> Self {
        Self::from(ErrorKind::Malformed).because(reason)
    }

    /// A structure or field at `offset` the format does not allow.
    pub fn malformed(offset: u64, reason: &'static str) -> Self {
        Self::invalid(reason).at(offset)
    }

    /// A structure or field at `offset` outside the supported profile.
    pub fn unsupported(offset: u64, reason: &'static str) -> Self {
        Self::from(ErrorKind::UnsupportedFormat)
            .at(offset)
            .because(reason)
    }

    pub fn cancelled() -> Self {
        ErrorKind::Cancelled.into()
    }

    /// The source ended at `offset` with `available` of `expected` bytes.
    pub fn truncated(offset: u64, expected: u64, available: u64) -> Self {
        Self::from(ErrorKind::Truncated {
            expected,
            available,
        })
        .at(offset)
    }

    /// A configured resource bound would be exceeded.
    pub fn limit(resource: &'static str, limit: u64, attempted: u64) -> Self {
        Self::from(ErrorKind::LimitExceeded {
            resource,
            limit,
            attempted,
        })
    }

    /// Set the absolute input offset.
    #[must_use]
    pub fn at(self, offset: u64) -> Self {
        Self {
            offset: Some(offset),
            ..self
        }
    }

    /// Set the description.
    #[must_use]
    pub fn because(self, reason: &'static str) -> Self {
        Self { reason, ..self }
    }

    /// Set the structure the failure is located in.
    #[must_use]
    pub fn within(self, context: Context) -> Self {
        Self { context, ..self }
    }

    #[must_use]
    pub fn in_caj(self, record: Option<u32>) -> Self {
        self.within(Context::Caj { record })
    }

    #[must_use]
    pub fn in_pdf(self, object: Option<(u32, u16)>) -> Self {
        self.within(Context::Pdf {
            object,
            repair: false,
        })
    }

    #[must_use]
    pub fn in_jbig2(self, segment: Option<u32>) -> Self {
        self.within(Context::Jbig2 { segment })
    }

    /// Give an unlocated error this location; a located one keeps its own.
    #[must_use]
    pub fn or_at(self, offset: u64, context: Context) -> Self {
        if self.context != Context::None {
            return self;
        }
        Self {
            offset: self.offset.or(Some(offset)),
            context,
            ..self
        }
    }
}

impl Context {
    /// An HN/C8 structure not yet located in a page or stage.
    pub(crate) const HNC8: Self = Self::Hnc8 {
        variant: None,
        page: None,
        image: None,
        segment: None,
        stage: None,
    };

    fn write_name(self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => Ok(()),
            Self::Caj { .. } => f.write_str("CAJ"),
            Self::Kdh => f.write_str("KDH"),
            Self::Pdf { .. } => f.write_str("PDF"),
            Self::Hnc8 { variant, .. } => {
                f.write_str("HN/C8")?;
                match variant {
                    Some(variant) => write!(f, " {}", variant.as_str()),
                    None => Ok(()),
                }
            }
            Self::Jbig2 { .. } => f.write_str("JBIG2"),
        }
    }

    fn write_location(self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut item = |name: &str, value: Option<u32>| match value {
            Some(value) => write!(f, ", {name} {value}"),
            None => Ok(()),
        };
        match self {
            Self::None | Self::Kdh => Ok(()),
            Self::Caj { record } => item("record", record),
            Self::Pdf { object, .. } => match object {
                Some((number, generation)) => write!(f, ", object {number} {generation}"),
                None => Ok(()),
            },
            Self::Hnc8 {
                page,
                image,
                segment,
                ..
            } => {
                item("page", page)?;
                item("image", image)?;
                item("segment", segment)
            }
            Self::Jbig2 { segment } => item("segment", segment),
        }
    }
}

struct Name(Context);

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.write_name(f)
    }
}

/// `"{kind} {context} at byte {offset}, {location}: {reason}: {details}"`
/// with the pieces present.
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = Name(self.context);
        let named = self.context != Context::None;
        let input = |f: &mut fmt::Formatter<'_>, adjective: &str, fallback: &str| {
            if named {
                write!(f, "{adjective} {name}")
            } else {
                write!(f, "{adjective} {fallback}")
            }
        };
        let event = |f: &mut fmt::Formatter<'_>, event: &str| {
            f.write_str(event)?;
            if named {
                write!(f, " in {name}")?;
            }
            Ok(())
        };
        match &self.kind {
            ErrorKind::Malformed if matches!(self.context, Context::Pdf { repair: true, .. }) => {
                f.write_str("ambiguous repair PDF")?
            }
            ErrorKind::UnsupportedFormat if matches!(self.context, Context::Pdf { .. }) => {
                f.write_str("unsupported feature PDF")?
            }
            ErrorKind::Malformed if !named => f.write_str("invalid input")?,
            ErrorKind::Malformed => input(f, "malformed", "")?,
            ErrorKind::UnsupportedFormat => input(f, "unsupported", "input format")?,
            ErrorKind::Encrypted => input(f, "encrypted", "input")?,
            ErrorKind::Truncated { .. } => input(f, "truncated", "input")?,
            ErrorKind::LimitExceeded { resource, .. } if named => {
                write!(f, "{name} {resource} limit exceeded")?
            }
            ErrorKind::LimitExceeded { resource, .. } => write!(f, "{resource} limit exceeded")?,
            ErrorKind::Io(_) => event(f, "I/O error")?,
            ErrorKind::Cancelled => event(f, "operation cancelled")?,
        }
        if let Some(offset) = self.offset {
            write!(f, " at byte {offset}")?;
        }
        self.context.write_location(f)?;
        if !self.reason.is_empty() {
            write!(f, ": {}", self.reason)?;
        }
        match &self.kind {
            ErrorKind::Truncated {
                expected,
                available,
            } => write!(f, ": expected {expected} bytes, available {available}"),
            ErrorKind::LimitExceeded {
                limit, attempted, ..
            } => write!(f, ": maximum {limit}, attempted {attempted}"),
            ErrorKind::Io(error) => write!(f, ": {error}"),
            _ => Ok(()),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.kind {
            ErrorKind::Io(error) => Some(error),
            _ => None,
        }
    }
}

/// An I/O error that carries a core error, as an [`io::Write`] adapter in
/// this crate reports one, converts back to that error.
impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        match error.downcast::<Self>() {
            Ok(error) => error,
            Err(error) => ErrorKind::Io(error).into(),
        }
    }
}

/// Carry a core error through an [`io::Write`] adapter; an unlocated I/O
/// error is passed on as it is, and a located one keeps its I/O kind.
impl From<Error> for io::Error {
    fn from(error: Error) -> Self {
        let kind = match &error.kind {
            ErrorKind::Io(inner) => inner.kind(),
            _ => io::ErrorKind::Other,
        };
        match error {
            Error {
                kind: ErrorKind::Io(error),
                offset: None,
                context: Context::None,
                reason: "",
            } => error,
            located => io::Error::new(kind, located),
        }
    }
}

#[cfg(test)]
mod tests;
