// SPDX-License-Identifier: MIT

//! Accessors for located errors, shared by unit and integration tests. The
//! including module imports `Context`, `Error`, `ErrorKind`, `Hnc8Stage` and
//! `Variant`.

use super::{Context, Error, ErrorKind, Hnc8Stage, Variant};

/// The field an error names: a limit's resource, or the reason up to its
/// first `": "`.
pub fn field_of(error: &Error) -> &'static str {
    match error.kind {
        ErrorKind::LimitExceeded { resource, .. } => resource,
        _ => error
            .reason
            .split_once(": ")
            .map_or(error.reason, |(field, _)| field),
    }
}

/// A short name of the error kind.
pub fn kind_name(error: &Error) -> &'static str {
    match error.kind {
        ErrorKind::UnsupportedFormat => "unsupported",
        ErrorKind::Malformed => "malformed",
        ErrorKind::Encrypted => "encrypted",
        ErrorKind::Truncated { .. } => "truncated",
        ErrorKind::LimitExceeded { .. } => "limit",
        ErrorKind::Io(_) => "io",
        ErrorKind::Cancelled => "cancelled",
    }
}

/// The class of a located PDF problem: `"malformed"`, `"ambiguous"`,
/// `"encrypted"` or `"unsupported"`.
pub fn pdf_class(error: &Error) -> Option<&'static str> {
    match (&error.kind, error.context) {
        (ErrorKind::Malformed, Context::Pdf { repair: true, .. }) => Some("ambiguous"),
        (ErrorKind::Malformed, Context::Pdf { .. }) => Some("malformed"),
        (ErrorKind::Encrypted, Context::Pdf { .. }) => Some("encrypted"),
        (ErrorKind::UnsupportedFormat, Context::Pdf { .. }) => Some("unsupported"),
        _ => None,
    }
}

/// The HN/C8 container variant of an error.
pub fn variant_of(error: &Error) -> Option<Variant> {
    match error.context {
        Context::Hnc8 { variant, .. } => variant,
        _ => None,
    }
}

/// The one-based HN/C8 page and image of an error.
pub fn page_image(error: &Error) -> (Option<u32>, Option<u32>) {
    match error.context {
        Context::Hnc8 { page, image, .. } => (page, image),
        _ => (None, None),
    }
}

/// The HN/C8 conversion stage of an error.
pub fn stage_of(error: &Error) -> Option<Hnc8Stage> {
    match error.context {
        Context::Hnc8 { stage, .. } => stage,
        _ => None,
    }
}

/// The JBIG2 segment of an error.
pub fn segment(error: &Error) -> Option<u32> {
    match error.context {
        Context::Jbig2 { segment } | Context::Hnc8 { segment, .. } => segment,
        _ => None,
    }
}
