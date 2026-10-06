// SPDX-License-Identifier: MIT

//! CFF outlines of OpenType fonts (Adobe Technical Notes #5176 and #5177).
//!
//! Only the structures a subset needs are read, by range and with bounds:
//! the Top DICT, the Global Subr, CharStrings and FDArray INDEXes, FDSelect
//! and each Private DICT with its local Subr INDEX. A subset is written as
//! a CID-keyed CFF whose glyph `n` has CID equal to its Unicode code point,
//! so a PDF `CIDFontType0` font needs no CID-to-glyph map. Type 2
//! charstrings are desubroutinized: neither global nor local subroutines
//! are carried, and no charstring is evaluated beyond operand counting.

use super::{OpenTypeFont, invalid, read};
use crate::fallible::{reserve, reserve_exact};
use crate::{Cancellation, Error, Limits, RangedSource, Result};
use std::collections::HashMap;
use std::rc::Rc;

/// At most 256 font DICTs (FDSelect stores FD indices in one byte).
const MAX_FONT_DICTS: usize = 256;
/// Bound on each Top, Font and Private DICT.
const MAX_DICT_BYTES: usize = 64 * 1024;
/// Type 2 limits: subroutine nesting and argument stack depth.
const MAX_SUBR_DEPTH: usize = 10;
const MAX_STACK: usize = 48;
/// Bound on one desubroutinized charstring.
const MAX_CHARSTRING_BYTES: usize = 256 * 1024;
/// Custom string IDs of "Adobe" and "Identity" after the 391 standard ones.
const SID_ADOBE: i32 = 391;
const SID_IDENTITY: i32 = 392;

const OP_CHARSTRINGS: u16 = 17;
const OP_PRIVATE: u16 = 18;
const OP_SUBRS: u16 = 19;
const OP_CHARSTRING_TYPE: u16 = 1206;
const OP_FONT_MATRIX: u16 = 1207;
const OP_ROS: u16 = 1230;
const OP_FD_ARRAY: u16 = 1236;
const OP_FD_SELECT: u16 = 1237;

/// An INDEX: `count` objects whose offsets start at `offsets`, with data
/// offsets relative to `data` (the byte before the first object).
#[derive(Clone, Copy, Debug, Default)]
struct Index {
    count: u32,
    off_size: u8,
    offsets: u64,
    data: u64,
    end: u64,
}

/// One font DICT: its Private DICT without `Subrs`, the local Subr INDEX and
/// the raw `FontMatrix` entry, if any.
#[derive(Debug)]
struct FontDict {
    private: Vec<u8>,
    subrs: Option<Index>,
    matrix: Vec<u8>,
}

/// The parsed structures of one CFF table.
#[derive(Debug)]
pub(crate) struct Cff {
    end: u64,
    global_subrs: Index,
    charstrings: Index,
    fonts: Vec<FontDict>,
    /// FDSelect as `(first glyph, FD)` ranges starting at glyph 0, each FD
    /// checked to exist.
    select: Vec<(u16, u8)>,
    /// Raw Top DICT `FontMatrix` entry, if any.
    matrix: Vec<u8>,
}

/// One parsed DICT entry: its operator, raw bytes (operands and operator)
/// and integer operand values (zero for real operands).
struct Entry {
    op: u16,
    start: usize,
    end: usize,
    values: Vec<i64>,
}

fn dict(bytes: &[u8]) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    let mut values = Vec::new();
    let mut start = 0;
    let mut at = 0;
    while at < bytes.len() {
        let b0 = bytes[at];
        let size = match b0 {
            32..=246 => 1,
            247..=254 => 2,
            28 => 3,
            29 => 5,
            30 => {
                // Real number: nibbles until the 0xf terminator.
                let end = bytes[at + 1..]
                    .iter()
                    .position(|byte| byte & 0x0f == 0x0f || byte >> 4 == 0x0f)
                    .ok_or(invalid("CFF real number is truncated"))?;
                end + 2
            }
            12 => {
                let op =
                    1200 + u16::from(*bytes.get(at + 1).ok_or(invalid("CFF DICT is truncated"))?);
                entries.push(Entry {
                    op,
                    start,
                    end: at + 2,
                    values: std::mem::take(&mut values),
                });
                at += 2;
                start = at;
                continue;
            }
            0..=21 => {
                entries.push(Entry {
                    op: u16::from(b0),
                    start,
                    end: at + 1,
                    values: std::mem::take(&mut values),
                });
                at += 1;
                start = at;
                continue;
            }
            _ => return Err(invalid("invalid CFF DICT byte")),
        };
        let token = bytes
            .get(at..at + size)
            .ok_or(invalid("CFF DICT is truncated"))?;
        if values.len() == MAX_STACK {
            return Err(invalid("CFF DICT has too many operands"));
        }
        values.push(match b0 {
            29 => i64::from(i32::from_be_bytes([token[1], token[2], token[3], token[4]])),
            30 => 0,
            _ => integer(token),
        });
        at += size;
    }
    if !values.is_empty() {
        return Err(invalid("CFF DICT ends with operands"));
    }
    Ok(entries)
}

/// The value of an integer operand token shared by DICTs and charstrings:
/// one byte (32-246), two bytes (247-254) or a 16-bit `28` shortint.
fn integer(token: &[u8]) -> i64 {
    let b0 = i64::from(token[0]);
    match token[0] {
        32..=246 => b0 - 139,
        247..=250 => (b0 - 247) * 256 + i64::from(token[1]) + 108,
        251..=254 => -(b0 - 251) * 256 - i64::from(token[1]) - 108,
        _ => i64::from(i16::from_be_bytes([token[1], token[2]])),
    }
}

/// The raw bytes (operands and operator) of an entry, if present.
fn raw(entries: &[Entry], bytes: &[u8], op: u16) -> Vec<u8> {
    find(entries, op)
        .map(|entry| bytes[entry.start..entry.end].to_vec())
        .unwrap_or_default()
}

fn find(entries: &[Entry], op: u16) -> Option<&Entry> {
    entries.iter().find(|entry| entry.op == op)
}

/// An entry's `count` leading nonnegative integer operands.
fn operands<const N: usize>(entry: &Entry) -> Result<[u64; N]> {
    let mut out = [0; N];
    if entry.values.len() != N {
        return Err(invalid("CFF DICT operator has the wrong operands"));
    }
    for (out, value) in out.iter_mut().zip(&entry.values) {
        *out = u64::try_from(*value).map_err(|_| invalid("CFF DICT offset is negative"))?;
    }
    Ok(out)
}

/// Subroutine number bias for an INDEX of `count` subroutines.
fn bias(count: u32) -> i64 {
    match count {
        0..1240 => 107,
        1240..33900 => 1131,
        _ => 32768,
    }
}

/// Reads CFF structures through the font source, counting bytes read.
struct Reader<'s, 'l, S, C> {
    source: &'s mut S,
    limits: &'l Limits,
    cancellation: &'l C,
    start: u64,
    end: u64,
    read: u64,
}

impl<S: RangedSource, C: Cancellation> Reader<'_, '_, S, C> {
    async fn bytes(&mut self, offset: u64, length: usize, limit: usize) -> Result<Vec<u8>> {
        if length > limit {
            return Err(Error::LimitExceeded {
                resource: "CFF structure bytes",
                limit: limit as u64,
                attempted: length as u64,
            });
        }
        if offset < self.start || offset + length as u64 > self.end {
            return Err(invalid("CFF structure is outside its table"));
        }
        self.limits.check_allocation(length as u64)?;
        let mut bytes = Vec::new();
        let refused = self
            .limits
            .allocation_refused("CFF structure bytes", length as u64);
        reserve_exact(&mut bytes, length, refused)?;
        bytes.resize(length, 0);
        read(
            self.source,
            offset,
            &mut bytes,
            self.limits,
            self.cancellation,
        )
        .await?;
        self.read += length as u64;
        Ok(bytes)
    }

    async fn index(&mut self, offset: u64) -> Result<Index> {
        let head = self.bytes(offset, 2, 2).await?;
        let count = u32::from(u16::from_be_bytes([head[0], head[1]]));
        if count == 0 {
            return Ok(Index {
                end: offset + 2,
                ..Index::default()
            });
        }
        let off_size = self.bytes(offset + 2, 1, 1).await?[0];
        if !(1..=4).contains(&off_size) {
            return Err(invalid("invalid CFF INDEX offset size"));
        }
        let offsets = offset + 3;
        let data = offsets + u64::from(count + 1) * u64::from(off_size) - 1;
        let mut index = Index {
            count,
            off_size,
            offsets,
            data,
            end: 0,
        };
        let (_, end) = self.object(&index, count - 1).await?;
        index.end = end;
        Ok(index)
    }

    /// Absolute byte range of object `item` of `index`.
    async fn object(&mut self, index: &Index, item: u32) -> Result<(u64, u64)> {
        if item >= index.count {
            return Err(invalid("CFF INDEX object is out of range"));
        }
        let size = usize::from(index.off_size);
        let at = index.offsets + u64::from(item) * size as u64;
        let bytes = self.bytes(at, 2 * size, 8).await?;
        let offset = |bytes: &[u8]| {
            bytes
                .iter()
                .fold(0_u64, |value, byte| value << 8 | u64::from(*byte))
        };
        let (start, end) = (offset(&bytes[..size]), offset(&bytes[size..]));
        if start == 0 || start > end || index.data + end > self.end {
            return Err(invalid("invalid CFF INDEX offsets"));
        }
        Ok((index.data + start, index.data + end))
    }

    async fn object_bytes(&mut self, index: &Index, item: u32, limit: usize) -> Result<Vec<u8>> {
        let (start, end) = self.object(index, item).await?;
        self.bytes(start, (end - start) as usize, limit).await
    }

    /// The Private DICT a font DICT's entries reference, without its `Subrs`
    /// entry, and its local Subr INDEX.
    async fn private(&mut self, font: &[Entry]) -> Result<(Vec<u8>, Option<Index>)> {
        let [size, offset] =
            operands(find(font, OP_PRIVATE).ok_or(invalid("CFF font has no Private DICT"))?)?;
        let start = self.start + offset;
        let bytes = self.bytes(start, size as usize, MAX_DICT_BYTES).await?;
        let entries = dict(&bytes)?;
        let mut private = Vec::new();
        let mut subrs = None;
        for entry in &entries {
            if entry.op == OP_SUBRS {
                let [relative] = operands(entry)?;
                subrs = Some(self.index(start + relative).await?);
            } else {
                private.extend_from_slice(&bytes[entry.start..entry.end]);
            }
        }
        Ok((private, subrs))
    }
}

impl Cff {
    /// Parse the CFF table at `offset` with `length` bytes for a font of
    /// `glyphs` glyphs.
    pub(crate) async fn read<S: RangedSource, C: Cancellation>(
        source: &mut S,
        (offset, length): (u64, u64),
        glyphs: u16,
        limits: &Limits,
        cancellation: &C,
    ) -> Result<Self> {
        let mut reader = Reader {
            source,
            limits,
            cancellation,
            start: offset,
            end: offset + length,
            read: 0,
        };
        let header = reader.bytes(offset, 4, 4).await?;
        if header[0] != 1 || header[2] < 4 {
            return Err(invalid("unsupported CFF header"));
        }
        let names = reader.index(offset + u64::from(header[2])).await?;
        let tops = reader.index(names.end).await?;
        if tops.count != 1 {
            return Err(invalid("a CFF table must hold exactly one font"));
        }
        let strings = reader.index(tops.end).await?;
        let global_subrs = reader.index(strings.end).await?;
        let top_bytes = reader.object_bytes(&tops, 0, MAX_DICT_BYTES).await?;
        let top = dict(&top_bytes)?;
        if find(&top, OP_CHARSTRING_TYPE).is_some_and(|entry| entry.values != [2]) {
            return Err(invalid("only Type 2 CFF charstrings are supported"));
        }
        let matrix = raw(&top, &top_bytes, OP_FONT_MATRIX);
        let [charstrings] =
            operands(find(&top, OP_CHARSTRINGS).ok_or(invalid("CFF font has no CharStrings"))?)?;
        let charstrings = reader.index(offset + charstrings).await?;
        if charstrings.count != u32::from(glyphs) {
            return Err(invalid(
                "CFF CharStrings count differs from the glyph count",
            ));
        }
        let mut fonts = Vec::new();
        let select = if find(&top, OP_ROS).is_some() {
            let (Some(array), Some(select)) = (find(&top, OP_FD_ARRAY), find(&top, OP_FD_SELECT))
            else {
                return Err(invalid("CID-keyed CFF needs an FDArray and FDSelect"));
            };
            let ([array], [select]) = (operands(array)?, operands(select)?);
            let array = reader.index(offset + array).await?;
            // FDSelect stores one-byte FD indices: later font DICTs are
            // unreachable, and FDSelect checks that each FD exists.
            for item in 0..array.count.min(MAX_FONT_DICTS as u32) {
                let bytes = reader.object_bytes(&array, item, MAX_DICT_BYTES).await?;
                let entries = dict(&bytes)?;
                let (private, subrs) = reader.private(&entries).await?;
                let matrix = raw(&entries, &bytes, OP_FONT_MATRIX);
                fonts.push(FontDict {
                    private,
                    subrs,
                    matrix,
                });
            }
            let at = offset + select;
            let format = reader.bytes(at, 1, 1).await?[0];
            let mut ranges: Vec<(u16, u8)> = Vec::new();
            let mut reserve_ranges = |count: usize| {
                let refused = limits.allocation_refused(
                    "CFF FDSelect ranges",
                    (count * size_of::<(u16, u8)>()) as u64,
                );
                reserve_exact(&mut ranges, count, refused)
            };
            let sentinel = match format {
                0 => {
                    let fds = reader
                        .bytes(at + 1, usize::from(glyphs), usize::from(glyphs))
                        .await?;
                    reserve_ranges(fds.len())?;
                    for (glyph, fd) in fds.into_iter().enumerate() {
                        push_range(&mut ranges, glyph, fd);
                    }
                    glyphs
                }
                3 => {
                    let head = reader.bytes(at + 1, 2, 2).await?;
                    let count = usize::from(u16::from_be_bytes([head[0], head[1]]));
                    let bytes = reader.bytes(at + 3, 3 * count + 2, 3 * 65536 + 2).await?;
                    reserve_ranges(count)?;
                    let (entries, sentinel) = bytes.as_chunks::<3>();
                    ranges.extend(
                        entries
                            .iter()
                            .map(|[high, low, fd]| (u16::from_be_bytes([*high, *low]), *fd)),
                    );
                    u16::from_be_bytes([sentinel[0], sentinel[1]])
                }
                _ => return Err(invalid("unsupported CFF FDSelect format")),
            };
            if ranges.first().is_none_or(|(first, _)| *first != 0)
                || !ranges.is_sorted_by(|(a, _), (b, _)| a < b)
                || ranges.iter().any(|(_, fd)| usize::from(*fd) >= fonts.len())
                || sentinel != glyphs
            {
                return Err(invalid("invalid CFF FDSelect"));
            }
            ranges
        } else {
            let (private, subrs) = reader.private(&top).await?;
            fonts.push(FontDict {
                private,
                subrs,
                matrix: Vec::new(),
            });
            vec![(0, 0)]
        };
        Ok(Self {
            end: offset + length,
            global_subrs,
            charstrings,
            fonts,
            select,
            matrix,
        })
    }

    /// Desubroutinize the charstring of `glyph` with the subroutine bodies
    /// in `cache`, and return it with its FD.
    async fn glyph<S: RangedSource, C: Cancellation>(
        &self,
        reader: &mut Reader<'_, '_, S, C>,
        cache: &mut SubrCache,
        glyph: u16,
    ) -> Result<(Vec<u8>, usize)> {
        let range = self.select.partition_point(|(first, _)| *first <= glyph) - 1;
        let fd = usize::from(self.select[range].1);
        let body = reader
            .object_bytes(&self.charstrings, u32::from(glyph), MAX_CHARSTRING_BYTES)
            .await?;
        let mut frames: Vec<(Rc<[u8]>, usize)> = vec![(body.into(), 0)];
        let mut out = Vec::new();
        // Each operand's start in `out` and its integer value, if any.
        let mut stack: Vec<(usize, Option<i64>)> = Vec::new();
        let mut stems = 0_usize;
        // Bytes interpreted, including subroutine calls and returns: this
        // bounds the work and the output however subroutines nest.
        let mut executed = 0_usize;
        loop {
            let last = frames.len() - 1;
            let (frame, at) = (Rc::clone(&frames[last].0), frames[last].1);
            let Some(&b0) = frame.get(at) else {
                return Err(invalid("CFF charstring ends without endchar or return"));
            };
            let size = match b0 {
                32..=246 => 1,
                247..=254 => 2,
                28 => 3,
                255 => 5,
                12 => 2,
                19 | 20 => 1 + (stems + stack.len() / 2).div_ceil(8),
                _ => 1,
            };
            let token = frame
                .get(at..at + size)
                .ok_or(invalid("CFF charstring is truncated"))?;
            frames[last].1 += size;
            executed += size;
            if executed > MAX_CHARSTRING_BYTES {
                return Err(Error::LimitExceeded {
                    resource: "CFF charstring bytes",
                    limit: MAX_CHARSTRING_BYTES as u64,
                    attempted: executed as u64,
                });
            }
            match b0 {
                28 | 32..=255 => {
                    if stack.len() == MAX_STACK {
                        return Err(invalid("CFF charstring argument stack overflows"));
                    }
                    stack.push((out.len(), (b0 != 255).then(|| integer(token))));
                    out.extend_from_slice(token);
                }
                10 | 29 => {
                    let (start, value) = stack
                        .pop()
                        .ok_or(invalid("CFF subroutine call has no number"))?;
                    let (key, subrs) = if b0 == 10 {
                        (Some(fd), self.fonts[fd].subrs)
                    } else {
                        (None, Some(self.global_subrs))
                    };
                    let subrs = subrs.ok_or(invalid("CFF charstring calls missing subroutines"))?;
                    let number = value.ok_or(invalid("CFF subroutine number is not an integer"))?
                        + bias(subrs.count);
                    let number = u32::try_from(number)
                        .map_err(|_| invalid("CFF subroutine number is out of range"))?;
                    if frames.len() > MAX_SUBR_DEPTH {
                        return Err(invalid("CFF subroutines nest too deeply"));
                    }
                    out.truncate(start);
                    let body = match cache.bodies.get(&(key, number)) {
                        Some(body) => Rc::clone(body),
                        None => {
                            let body: Rc<[u8]> = reader
                                .object_bytes(&subrs, number, MAX_CHARSTRING_BYTES)
                                .await?
                                .into();
                            cache.bytes += body.len() as u64;
                            reader.limits.check_allocation(cache.bytes)?;
                            cache.bodies.insert((key, number), Rc::clone(&body));
                            body
                        }
                    };
                    frames.push((body, 0));
                }
                11 => {
                    if frames.len() == 1 {
                        return Err(invalid("CFF charstring returns outside a subroutine"));
                    }
                    frames.pop();
                }
                14 => {
                    if stack.len() >= 4 {
                        return Err(invalid("CFF accented-character endchar is not supported"));
                    }
                    out.push(14);
                    return Ok((out, fd));
                }
                1 | 3 | 18 | 23 | 19 | 20 => {
                    stems += stack.len() / 2;
                    stack.clear();
                    out.extend_from_slice(token);
                }
                12 if matches!(token[1], 0 | 34..=37) => {
                    stack.clear();
                    out.extend_from_slice(token);
                }
                12 => return Err(invalid("unsupported CFF charstring operator")),
                4..=8 | 21 | 22 | 24..=27 | 30 | 31 => {
                    stack.clear();
                    out.extend_from_slice(token);
                }
                _ => return Err(invalid("invalid CFF charstring operator")),
            }
        }
    }
}

/// Subroutine bodies read for one subset, keyed by FD (`None` for global
/// subroutines) and number, so each is read once however often it is called.
#[derive(Default)]
struct SubrCache {
    bodies: HashMap<(Option<usize>, u32), Rc<[u8]>>,
    bytes: u64,
}

/// A CID-keyed CFF subset: glyph 0 is `.notdef` and every other glyph has
/// the CID of the Unicode character it draws. The desubroutinized
/// charstrings are held once, between the structures before and after them.
pub(crate) struct CffSubset {
    head: Vec<u8>,
    charstrings: Vec<u8>,
    tail: Vec<u8>,
}

impl<S: RangedSource> OpenTypeFont<'_, S> {
    /// Build the CFF subset of the characters set in `used`, at most
    /// `max_length` bytes. Each charstring and subroutine is read once, and
    /// the charstrings and subroutines held count as one allocation.
    pub(crate) async fn plan_cff<C: Cancellation>(
        &mut self,
        cff: &Cff,
        used: &[u8],
        max_length: u64,
        limits: &Limits,
        cancellation: &C,
    ) -> Result<CffSubset> {
        let mut glyphs = vec![(0, 0)];
        for glyph in self.used_glyphs(used)? {
            glyphs.push(glyph);
        }
        let name = self.postscript_name()?;
        let mut reader = Reader {
            source: self.source,
            limits,
            cancellation,
            start: 0,
            end: cff.end,
            read: 0,
        };
        let mut cache = SubrCache::default();
        let mut charstrings = Vec::new();
        let mut lengths = Vec::new();
        let mut fds = Vec::new();
        for (glyph, _) in &glyphs {
            let (charstring, fd) = cff.glyph(&mut reader, &mut cache, *glyph).await?;
            super::subset::too_long((charstrings.len() + charstring.len()) as u64, max_length)?;
            limits.check_allocation(cache.bytes + (charstrings.len() + charstring.len()) as u64)?;
            let refused =
                limits.allocation_refused("CFF subset charstrings", charstring.len() as u64);
            reserve(&mut charstrings, charstring.len(), refused)?;
            charstrings.extend_from_slice(&charstring);
            lengths.push(charstring.len());
            fds.push(fd);
        }
        self.subset_bytes_read += reader.read;
        let cids: Vec<u16> = glyphs.iter().map(|(_, cid)| *cid).collect();
        let subset = CffSubset::new(cff, &name, charstrings, &lengths, &fds, &cids);
        super::subset::too_long(subset.length(), max_length)?;
        Ok(subset)
    }
}

/// DICT integer in the fixed five-byte form, so offsets do not change sizes.
fn int5(out: &mut Vec<u8>, value: u64) {
    out.push(29);
    out.extend_from_slice(&(value as i32).to_be_bytes());
}

/// The count, offset size and offsets of an INDEX of objects with these
/// lengths, using the smallest offset size.
fn index_head(out: &mut Vec<u8>, lengths: &[usize]) {
    out.extend_from_slice(&(lengths.len() as u16).to_be_bytes());
    if lengths.is_empty() {
        return;
    }
    let size = offset_size(lengths.iter().sum::<usize>() + 1);
    out.push(size as u8);
    let mut offset = 1_usize;
    for length in std::iter::once(&0).chain(lengths) {
        offset += length;
        out.extend_from_slice(&offset.to_be_bytes()[size_of::<usize>() - size..]);
    }
}

/// An INDEX of `items`.
fn index(out: &mut Vec<u8>, items: &[&[u8]]) {
    let lengths: Vec<usize> = items.iter().map(|item| item.len()).collect();
    index_head(out, &lengths);
    for item in items {
        out.extend_from_slice(item);
    }
}

/// Bytes of an INDEX of objects with these lengths, as [`index`] writes it.
fn index_length(lengths: &[usize]) -> usize {
    let mut head = Vec::new();
    index_head(&mut head, lengths);
    head.len() + lengths.iter().sum::<usize>()
}

fn offset_size(largest: usize) -> usize {
    match largest {
        0..0x100 => 1,
        0x100..0x1_0000 => 2,
        0x1_0000..0x100_0000 => 3,
        _ => 4,
    }
}

/// Append `(glyph, fd)` to FDSelect ranges, starting a range when the FD
/// changes.
fn push_range(ranges: &mut Vec<(u16, u8)>, glyph: usize, fd: u8) {
    if ranges.last().is_none_or(|(_, last)| *last != fd) {
        ranges.push((glyph as u16, fd));
    }
}

impl CffSubset {
    /// Assemble the subset of `name` around desubroutinized `charstrings`
    /// of these `lengths`, with each glyph's source FD and CID, in subset
    /// glyph order.
    fn new(
        cff: &Cff,
        name: &str,
        charstrings: Vec<u8>,
        lengths: &[usize],
        fds: &[usize],
        cids: &[u16],
    ) -> Self {
        // Used font DICTs in first-use order.
        let mut used: Vec<usize> = Vec::new();
        let mut ranges = Vec::new();
        for (glyph, fd) in fds.iter().enumerate() {
            let index = used
                .iter()
                .position(|known| known == fd)
                .unwrap_or_else(|| {
                    used.push(*fd);
                    used.len() - 1
                });
            push_range(&mut ranges, glyph, index as u8);
        }
        let cid_count = u64::from(cids.iter().copied().max().unwrap_or(0)) + 1;

        let mut head = vec![1, 0, 4, 4];
        index(&mut head, &[name.as_bytes()]);
        // Every offset is a five-byte integer, so the DICT's length does
        // not depend on the offsets it holds.
        let top_dict = |[charset, select, charstrings, array]: [u64; 4]| {
            let mut top = Vec::new();
            for value in [SID_ADOBE, SID_IDENTITY, 0] {
                int5(&mut top, value as u64);
            }
            top.extend_from_slice(&[12, 30]);
            top.extend_from_slice(&cff.matrix);
            int5(&mut top, cid_count);
            top.extend_from_slice(&[12, 34]);
            int5(&mut top, charset);
            top.push(15);
            int5(&mut top, select);
            top.extend_from_slice(&[12, 37]);
            int5(&mut top, charstrings);
            top.push(OP_CHARSTRINGS as u8);
            int5(&mut top, array);
            top.extend_from_slice(&[12, 36]);
            top
        };
        let mut strings = Vec::new();
        index(&mut strings, &[b"Adobe", b"Identity"]);
        let mut charset = vec![0];
        for cid in &cids[1..] {
            charset.extend_from_slice(&cid.to_be_bytes());
        }
        let mut select = vec![3];
        select.extend_from_slice(&(ranges.len() as u16).to_be_bytes());
        for (first, fd) in &ranges {
            select.extend_from_slice(&first.to_be_bytes());
            select.push(*fd);
        }
        select.extend_from_slice(&(cids.len() as u16).to_be_bytes());

        let font_dict = |font: &FontDict, private_at: u64| {
            let mut dict = font.matrix.clone();
            int5(&mut dict, font.private.len() as u64);
            int5(&mut dict, private_at);
            dict.push(OP_PRIVATE as u8);
            dict
        };
        let top_length = top_dict([0; 4]).len();
        let charset_at = (head.len() + index_length(&[top_length]) + strings.len() + 2) as u64;
        let select_at = charset_at + charset.len() as u64;
        let charstrings_at = select_at + select.len() as u64;
        let array_at = charstrings_at + index_length(lengths) as u64;
        let dict_lengths: Vec<usize> = used
            .iter()
            .map(|fd| font_dict(&cff.fonts[*fd], 0).len())
            .collect();
        let mut private_at = array_at + index_length(&dict_lengths) as u64;
        let mut dicts = Vec::new();
        for fd in &used {
            let font = &cff.fonts[*fd];
            dicts.push(font_dict(font, private_at));
            private_at += font.private.len() as u64;
        }

        index(
            &mut head,
            &[&top_dict([charset_at, select_at, charstrings_at, array_at])],
        );
        head.extend_from_slice(&strings);
        index(&mut head, &[]);
        head.extend_from_slice(&charset);
        head.extend_from_slice(&select);
        index_head(&mut head, lengths);
        let mut tail = Vec::new();
        let dicts: Vec<&[u8]> = dicts.iter().map(|dict| &dict[..]).collect();
        index(&mut tail, &dicts);
        for fd in &used {
            tail.extend_from_slice(&cff.fonts[*fd].private);
        }
        Self {
            head,
            charstrings,
            tail,
        }
    }

    /// The program's bytes, in order.
    pub(crate) fn parts(&self) -> [&[u8]; 3] {
        [&self.head, &self.charstrings, &self.tail]
    }

    pub(crate) fn length(&self) -> u64 {
        self.parts().iter().map(|part| part.len() as u64).sum()
    }
}

#[cfg(test)]
mod tests;
