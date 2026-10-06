// SPDX-License-Identifier: MIT

//! Font registration: host font resources and the roles JavaScript assigns
//! them.

use super::{Host, HostSource};
use caj2pdf_core::{
    Fonts as FontResources, RangedSource,
    hnc8::{C8FontSource, C8PageFonts},
};
use std::cell::RefCell;

/// Explicitly registered font resources and their roles.
#[derive(Default)]
pub(super) struct Fonts {
    sizes: [u64; 8],
    faces: [u32; 8],
    count: usize,
    roles: Option<C8PageFonts>,
}

impl Fonts {
    pub(super) fn count(&self) -> usize {
        self.count
    }

    pub(super) fn add(&mut self, size: u64, face: u32) -> u32 {
        if self.count == self.sizes.len() || self.roles.is_some() {
            return 0;
        }
        self.sizes[self.count] = size;
        self.faces[self.count] = face;
        self.count += 1;
        self.count as u32
    }

    pub(super) fn set_latin_state(&mut self, state: u32, index: u32) -> bool {
        let Some(roles) = &mut self.roles else {
            return false;
        };
        let role = match state {
            3 => &mut roles.latin_state3,
            28 => &mut roles.latin_state28,
            31 => &mut roles.latin_state31,
            _ => return false,
        };
        if index as usize >= self.count || role.is_some() {
            return false;
        }
        *role = Some(index as usize);
        true
    }

    pub(super) fn set(
        &mut self,
        cjk: u32,
        latin: u32,
        alternate: u32,
        decoration: u32,
        alias: u32,
        symbols: u32,
    ) -> bool {
        // `u32::MAX` marks an absent optional role; core applies its fallback.
        if self.roles.is_some()
            || [cjk, latin]
                .into_iter()
                .any(|index| index as usize >= self.count)
            || (alternate != u32::MAX && alternate as usize >= self.count)
        {
            return false;
        }
        let alternate_latin = (alternate != u32::MAX).then_some(alternate as usize);
        let decoration = if decoration == u32::MAX {
            None
        } else {
            let Some(character) = char::from_u32(alias) else {
                return false;
            };
            if decoration as usize >= self.count || alias > 0xffff {
                return false;
            }
            Some((decoration as usize, character))
        };
        let symbols = if symbols == u32::MAX {
            None
        } else if symbols as usize >= self.count {
            return false;
        } else {
            Some(symbols as usize)
        };
        self.roles = Some(C8PageFonts {
            cjk: cjk as usize,
            latin: latin as usize,
            alternate_latin,
            decoration,
            symbols,
            latin_state3: None,
            latin_state28: None,
            latin_state31: None,
        });
        true
    }
}

impl Fonts {
    /// The registered fonts as host resources 1..=8 for one conversion;
    /// `None` when none are registered.
    pub(super) fn resources<'h, H: Host + 'h>(
        &self,
        host: &'h RefCell<&'h mut H>,
    ) -> Option<FontResources<'h>> {
        (self.count != 0).then(|| FontResources {
            sources: (0..self.count)
                .map(|index| {
                    let source: Box<dyn RangedSource + 'h> =
                        Box::new(HostSource::new(host, index as u32 + 1, self.sizes[index]));
                    C8FontSource {
                        source,
                        face: self.faces[index],
                    }
                })
                .collect(),
            roles: self.roles,
        })
    }
}
