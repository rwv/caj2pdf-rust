// SPDX-License-Identifier: MIT

//! Complete TrueType programs through bounded ranged reads. Standalone fonts
//! are copied exactly. A collection face is repacked with all rendering tables,
//! original glyph IDs and a fresh standalone checksum; its DSIG is omitted.

use super::subset::{Measure, SubsetOutput, too_long};
use super::{CHANGED, MAX_TABLES, METADATA_TAGS, OpenTypeFont, invalid, span};
use crate::fallible::reserve_exact;
use crate::{Cancellation, Limits, RangedSource, Result};
use sha2::{Digest, Sha256};

struct FullTable {
    entry: [u8; 16],
    checksum: u32,
    digest: [u8; 32],
}

enum ProgramSource {
    Standalone([u8; 32]),
    Collection(Vec<FullTable>),
}

pub(crate) struct FullFont {
    length: u64,
    source: ProgramSource,
}

impl FullFont {
    pub(super) fn length(&self) -> u64 {
        self.length
    }
}

fn same_overlap(at: u64, bytes: &[u8], expected_at: u64, expected: &[u8]) -> Result<()> {
    let start = at.max(expected_at);
    let end = (at + bytes.len() as u64).min(expected_at + expected.len() as u64);
    if start < end
        && bytes[(start - at) as usize..(end - at) as usize]
            != expected[(start - expected_at) as usize..(end - expected_at) as usize]
    {
        return Err(invalid(CHANGED));
    }
    Ok(())
}

impl<S: RangedSource> OpenTypeFont<'_, S> {
    fn full_snapshot(&self, at: u64, bytes: &[u8]) -> Result<()> {
        same_overlap(at, bytes, self.face_base, &self.header)?;
        for (index, entry) in self.directory[..self.table_count].iter().enumerate() {
            same_overlap(at, bytes, self.face_base + 12 + index as u64 * 16, entry)?;
            if let Some(slot) = METADATA_TAGS.iter().position(|tag| *tag == entry[..4]) {
                same_overlap(at, bytes, span(entry).0, &self.tables[slot])?;
            }
        }
        Ok(())
    }

    /// Hash the exact bytes read, then optionally patch a collection head's
    /// checksum adjustment in the emitted bytes. No table is read whole.
    #[allow(clippy::too_many_arguments)]
    fn full_copy<O: SubsetOutput, C: Cancellation>(
        &mut self,
        offset: u64,
        length: u64,
        head_adjustment: Option<u32>,
        output: &mut O,
        scratch: &mut Vec<u8>,
        limits: &Limits,
        cancellation: &C,
    ) -> Result<[u8; 32]> {
        let capacity = length.min(limits.io_chunk_bytes as u64) as usize;
        if scratch.len() < capacity {
            limits.check_allocation(capacity as u64)?;
            reserve_exact(
                scratch,
                capacity - scratch.len(),
                limits.allocation_refused("complete font buffer", capacity as u64),
            )?;
            scratch.resize(capacity, 0);
        }
        let mut hash = Sha256::new();
        let mut done = 0;
        while done < length {
            let count = (length - done).min(scratch.len() as u64) as usize;
            let bytes = &mut scratch[..count];
            self.fetch(offset + done, bytes, limits, cancellation)?;
            self.full_snapshot(offset + done, bytes)?;
            hash.update(&*bytes);
            if let Some(adjustment) = head_adjustment {
                let field = adjustment.to_be_bytes();
                for (i, byte) in field.iter().enumerate() {
                    let at = 8 + i as u64;
                    if done <= at && at < done + count as u64 {
                        bytes[(at - done) as usize] = *byte;
                    }
                }
            }
            output.put(bytes)?;
            done += count as u64;
        }
        Ok(hash.finalize().into())
    }

    pub(crate) fn plan_full<C: Cancellation>(
        &mut self,
        max_length: u64,
        limits: &Limits,
        cancellation: &C,
    ) -> Result<FullFont> {
        if self.is_cff() {
            return Err(invalid(
                "complete font embedding requires TrueType outlines",
            ));
        }
        let mut scratch = Vec::new();
        if !self.collection {
            let length = self.source.size();
            too_long(length, max_length)?;
            let digest = self.full_copy(
                0,
                length,
                None,
                &mut Measure::default(),
                &mut scratch,
                limits,
                cancellation,
            )?;
            return Ok(FullFont {
                length,
                source: ProgramSource::Standalone(digest),
            });
        }
        let directory = self.directory;
        let entries = directory[..self.table_count]
            .iter()
            .filter(|entry| entry[..4] != *b"DSIG")
            .copied();
        let count = entries.clone().count();
        let length = entries
            .clone()
            .fold(12 + 16 * count as u64, |total, entry| {
                total + span(&entry).1.next_multiple_of(4)
            });
        too_long(length, max_length.min(u64::from(u32::MAX)))?;
        let bytes = count as u64 * size_of::<FullTable>() as u64;
        limits.check_allocation(bytes)?;
        let mut tables = Vec::new();
        reserve_exact(
            &mut tables,
            count,
            limits.allocation_refused("complete font tables", bytes),
        )?;
        for entry in entries {
            let (offset, length) = span(&entry);
            let mut measure = Measure::default();
            let digest = self.full_copy(
                offset,
                length,
                (entry[..4] == *b"head").then_some(0),
                &mut measure,
                &mut scratch,
                limits,
                cancellation,
            )?;
            tables.push(FullTable {
                entry,
                checksum: measure.sum,
                digest,
            });
        }
        Ok(FullFont {
            length,
            source: ProgramSource::Collection(tables),
        })
    }

    pub(super) fn write_full<O: SubsetOutput, C: Cancellation>(
        &mut self,
        plan: &FullFont,
        output: &mut O,
        limits: &Limits,
        cancellation: &C,
    ) -> Result<()> {
        let mut scratch = Vec::new();
        let tables = match &plan.source {
            ProgramSource::Standalone(expected) => {
                let digest = self.full_copy(
                    0,
                    plan.length,
                    None,
                    output,
                    &mut scratch,
                    limits,
                    cancellation,
                )?;
                if &digest != expected {
                    return Err(invalid(CHANGED));
                }
                return Ok(());
            }
            ProgramSource::Collection(tables) => tables,
        };
        let count = tables.len() as u16;
        let selector = count.ilog2() as u16;
        let mut header = [0; 12 + 16 * MAX_TABLES];
        header[..4].copy_from_slice(&0x0001_0000_u32.to_be_bytes());
        header[4..6].copy_from_slice(&count.to_be_bytes());
        header[6..8].copy_from_slice(&(16_u16 << selector).to_be_bytes());
        header[8..10].copy_from_slice(&selector.to_be_bytes());
        header[10..12].copy_from_slice(&(16 * count - (16_u16 << selector)).to_be_bytes());
        let header_length = 12 + 16 * usize::from(count);
        let mut offset = header_length as u64;
        let mut sum = 0_u32;
        for (index, table) in tables.iter().enumerate() {
            let at = 12 + index * 16;
            let length = span(&table.entry).1;
            header[at..at + 4].copy_from_slice(&table.entry[..4]);
            header[at + 4..at + 8].copy_from_slice(&table.checksum.to_be_bytes());
            header[at + 8..at + 12].copy_from_slice(&(offset as u32).to_be_bytes());
            header[at + 12..at + 16].copy_from_slice(&(length as u32).to_be_bytes());
            offset += length.next_multiple_of(4);
            sum = sum.wrapping_add(table.checksum);
        }
        let mut directory = Measure::default();
        directory.put(&header[..header_length])?;
        let adjustment = 0xb1b0_afba_u32.wrapping_sub(sum.wrapping_add(directory.sum));
        output.put(&header[..header_length])?;
        for table in tables {
            let (offset, length) = span(&table.entry);
            let digest = self.full_copy(
                offset,
                length,
                (table.entry[..4] == *b"head").then_some(adjustment),
                output,
                &mut scratch,
                limits,
                cancellation,
            )?;
            if digest != table.digest {
                return Err(invalid(CHANGED));
            }
            output.put(&[0; 3][..(length.next_multiple_of(4) - length) as usize])?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
