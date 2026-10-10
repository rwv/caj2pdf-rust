// SPDX-License-Identifier: MIT

//! Explicit glyph/text pairs with stable, bounded PDF CIDs.

use super::has_code;
use crate::fallible::reserve_exact;
use crate::{Limits, Result};
use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct GlyphCharacters {
    pub glyph: char,
    pub text: char,
}

/// First-use order fixes the CID; hashing only accelerates pair lookup.
/// Zero is reserved for .notdef and for empty hash-table slots. The table
/// stays at most half full and stores CIDs, not another copy of each pair.
pub(crate) struct CharacterMap {
    pub entries: Vec<GlyphCharacters>,
    slots: Vec<u16>,
    hash: RandomState,
    maximum: u16,
}

impl CharacterMap {
    pub fn new(maximum: u16) -> Self {
        Self {
            entries: Vec::new(),
            slots: Vec::new(),
            hash: RandomState::new(),
            maximum,
        }
    }

    fn slot(&self, pair: GlyphCharacters) -> usize {
        let mut slot = self.hash.hash_one(pair) as usize & (self.slots.len() - 1);
        loop {
            let cid = self.slots[slot];
            if cid == 0 || self.entries[usize::from(cid) - 1] == pair {
                return slot;
            }
            slot = (slot + 1) & (self.slots.len() - 1);
        }
    }

    pub fn code(&mut self, pair: GlyphCharacters, limits: &Limits) -> Result<u16> {
        if !self.slots.is_empty() {
            let cid = self.slots[self.slot(pair)];
            if cid != 0 {
                return Ok(cid);
            }
        }
        let next = self.entries.len() + 1;
        if next > usize::from(self.maximum) {
            return Err(crate::Error::limit(
                "PDF mapped font CIDs",
                u64::from(self.maximum),
                next as u64,
            ));
        }
        let cid = next as u16;
        let entries = next.next_power_of_two().max(4);
        let slots = (2 * next).next_power_of_two().max(8);
        // Include the old buffers while either allocation is being replaced.
        let entries_bytes = (entries.max(self.entries.capacity())
            + if entries > self.entries.capacity() {
                self.entries.capacity()
            } else {
                0
            })
            * size_of::<GlyphCharacters>();
        let slots_bytes = (slots.max(self.slots.capacity())
            + if slots > self.slots.capacity() {
                self.slots.capacity()
            } else {
                0
            })
            * size_of::<u16>();
        let bytes = entries_bytes + slots_bytes;
        limits.check_allocation(bytes as u64)?;
        let refused = || limits.allocation_refused("PDF mapped font characters", bytes as u64);
        if self.entries.capacity() < entries {
            let additional = entries - self.entries.len();
            reserve_exact(&mut self.entries, additional, refused())?;
        }
        if self.slots.len() < slots {
            let additional = slots - self.slots.len();
            reserve_exact(&mut self.slots, additional, refused())?;
            self.slots.resize(slots, 0);
            self.slots.fill(0);
            for (index, pair) in self.entries.iter().copied().enumerate() {
                let slot = self.slot(pair);
                self.slots[slot] = (index + 1) as u16;
            }
        }
        let slot = self.slot(pair);
        self.entries.push(pair);
        self.slots[slot] = cid;
        Ok(cid)
    }
}

/// The source character selected by each used PDF CID. Unicode encoding
/// retains the existing sparse BMP bitmap; explicit encoding starts at CID 1.
#[derive(Clone, Copy)]
pub(crate) enum Characters<'a> {
    Unicode(&'a [u8]),
    Mapped(&'a [GlyphCharacters]),
}

impl Characters<'_> {
    pub fn contains(self, code: usize) -> bool {
        match self {
            Self::Unicode(used) => code < used.len() * 8 && has_code(used, code),
            Self::Mapped(entries) => code > 0 && code <= entries.len(),
        }
    }

    pub fn glyph(self, code: usize) -> Option<char> {
        match self {
            Self::Unicode(_) => self
                .contains(code)
                .then(|| char::from_u32(code as u32))
                .flatten(),
            Self::Mapped(entries) => code
                .checked_sub(1)
                .and_then(|index| entries.get(index))
                .map(|p| p.glyph),
        }
    }

    pub fn codes(self) -> impl DoubleEndedIterator<Item = u16> {
        let end = match self {
            Self::Unicode(used) => used.len() * 8,
            Self::Mapped(entries) => entries.len() + 1,
        };
        (0..end)
            .filter(move |code| self.contains(*code))
            .map(|code| code as u16)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_pairs_keep_cids_through_growth_and_distinct_text() {
        let mut map = CharacterMap::new(u16::MAX);
        let limits = Limits::default();
        for value in 0..4096 {
            let pair = GlyphCharacters {
                glyph: 'A',
                text: char::from_u32(0x10000 + value).unwrap(),
            };
            assert_eq!(map.code(pair, &limits).unwrap(), (value + 1) as u16);
        }
        for value in (0..4096).rev() {
            let pair = GlyphCharacters {
                glyph: 'A',
                text: char::from_u32(0x10000 + value).unwrap(),
            };
            assert_eq!(map.code(pair, &limits).unwrap(), (value + 1) as u16);
        }
        assert_eq!(
            map.code(
                GlyphCharacters {
                    glyph: 'B',
                    text: '\u{10000}'
                },
                &limits
            )
            .unwrap(),
            4097
        );
    }

    #[test]
    fn cid_and_allocation_limits_refuse_growth_without_losing_entries() {
        let mut map = CharacterMap::new(u16::MAX);
        let limits = Limits::default();
        for value in 0..65535 {
            map.code(
                GlyphCharacters {
                    glyph: 'A',
                    text: char::from_u32(0x10000 + value).unwrap(),
                },
                &limits,
            )
            .unwrap();
        }
        assert!(
            map.code(
                GlyphCharacters {
                    glyph: 'B',
                    text: 'B'
                },
                &limits
            )
            .is_err()
        );
        assert_eq!(
            map.code(
                GlyphCharacters {
                    glyph: 'A',
                    text: '\u{10000}'
                },
                &limits
            )
            .unwrap(),
            1
        );
        let mut empty = CharacterMap::new(u16::MAX);
        let low = Limits {
            max_allocation_bytes: 1,
            ..limits
        };
        assert!(
            empty
                .code(
                    GlyphCharacters {
                        glyph: 'A',
                        text: 'A'
                    },
                    &low
                )
                .is_err()
        );
        assert!(empty.entries.is_empty());
    }

    #[test]
    fn available_capacity_does_not_count_a_nonexistent_reallocation() {
        let mut map = CharacterMap::new(u16::MAX);
        let limits = Limits {
            max_allocation_bytes: 144,
            ..Limits::default()
        };
        for text in 'A'..='H' {
            map.code(GlyphCharacters { glyph: 'A', text }, &limits)
                .unwrap();
        }
        assert_eq!(map.entries.len(), 8);
        assert!(
            map.code(
                GlyphCharacters {
                    glyph: 'A',
                    text: 'I'
                },
                &limits
            )
            .is_err()
        );
        assert_eq!(map.entries.len(), 8);
    }
}
