// SPDX-License-Identifier: MIT

//! Original, measured TTKN server-response PDF decoding. No network access.

mod crypto;
mod source;
pub(crate) use source::convert;
mod wrapper;

use crate::{Error, Result};
use zeroize::Zeroizing;

/// An explicitly supplied response for the measured TTKN server-auth profile.
///
/// The 32 hexadecimal ASCII bytes are preserved exactly, including case. They
/// are not a conventional PDF password. This type intentionally has no Debug
/// implementation, and clears its owned bytes when dropped.
pub struct TtknResponse(Zeroizing<[u8; 32]>);

impl TtknResponse {
    pub fn new(ascii: &[u8]) -> Result<Self> {
        if ascii.len() != 32 || !ascii.iter().all(u8::is_ascii_hexdigit) {
            return Err(Error::invalid(
                "TTKN response must contain exactly 32 hexadecimal ASCII bytes",
            ));
        }
        Ok(Self(Zeroizing::new(
            ascii.try_into().expect("checked response length"),
        )))
    }
}
