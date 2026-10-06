// SPDX-License-Identifier: MIT

//! Test-only arithmetic encoders for the standard T.82 and T.88 states.
//!
//! Written from the interval descriptions of T.82 §6.8 and T.88 Annex E:
//! each encoder keeps its code value exactly as a bit vector, so it needs
//! neither carry stacks nor byte-out procedures. It produces the bytes a
//! decoder in this crate must read back as the same decisions in the same
//! contexts; it is a test vector generator, not a conforming encoder.
//!
//! This file is shared by the crate's unit tests (`crate::test_support`) and
//! the integration tests (`tests/common`), so it names no crate items: the
//! caller passes the probability rows.

use std::collections::HashMap;

/// One probability state: `(Qe, next index after an MPS, next index after an
/// LPS, switch MPS)`.
pub type StateRow = (u16, u8, u8, bool);

/// The exact lower bound of the coding interval as one bit per entry. Bit
/// `i` weighs `2^-(i + 1)`.
#[derive(Clone, Default)]
struct Bits(Vec<u8>);

impl Bits {
    /// Add `value * 2^-(fraction_bits + shift)`.
    fn add(&mut self, value: u32, fraction_bits: usize, shift: usize) {
        let top = fraction_bits + shift;
        if self.0.len() < top {
            self.0.resize(top, 0);
        }
        for j in 0..32 {
            if value & (1 << j) == 0 {
                continue;
            }
            let mut index = top - 1 - j;
            loop {
                self.0[index] += 1;
                if self.0[index] < 2 {
                    break;
                }
                self.0[index] = 0;
                index -= 1;
            }
        }
    }

    /// The first `count` bits, and whether any later bit is set.
    fn truncated(&self, count: usize) -> (Vec<u8>, bool) {
        let mut head = self.0.clone();
        head.resize(count.max(head.len()), 0);
        let rest = head.split_off(count);
        (head, rest.contains(&1))
    }
}

fn increment(bits: &mut [u8]) {
    for bit in bits.iter_mut().rev() {
        if *bit == 0 {
            *bit = 1;
            return;
        }
        *bit = 0;
    }
    panic!("code value overflowed the unit interval");
}

fn decrement(bits: &mut [u8]) {
    for bit in bits.iter_mut().rev() {
        if *bit == 1 {
            *bit = 0;
            return;
        }
        *bit = 1;
    }
    panic!("code value underflowed the unit interval");
}

/// Context states created on first use, like a freshly reset bank.
#[derive(Clone, Default)]
struct Contexts(HashMap<usize, (u8, bool)>);

impl Contexts {
    fn get(&self, context: usize) -> (u8, bool) {
        self.0.get(&context).copied().unwrap_or((0, false))
    }

    fn update(&mut self, context: usize, row: StateRow, mps: bool, lps: bool, renormalized: bool) {
        let (_, next_mps, next_lps, switch) = row;
        if lps {
            self.0.insert(context, (next_lps, mps ^ switch));
        } else if renormalized {
            self.0.insert(context, (next_mps, mps));
        }
    }
}

/// T.82 encoder for one stripe's SCD bytes (before any marker framing). The
/// decoder places the MPS in the lower subinterval `[0, A - Qe)` unless
/// `A - Qe < Qe`, and reads zero bytes after the coded end.
#[derive(Clone)]
pub struct QmEncoder {
    states: Vec<StateRow>,
    contexts: Contexts,
    low: Bits,
    interval: u32,
    shift: usize,
    decisions: u64,
}

impl QmEncoder {
    pub fn new(states: &[StateRow]) -> Self {
        Self {
            states: states.to_vec(),
            contexts: Contexts::default(),
            low: Bits::default(),
            interval: 0x1_0000,
            shift: 0,
            decisions: 0,
        }
    }

    pub fn decisions(&self) -> u64 {
        self.decisions
    }

    pub fn encode(&mut self, context: usize, bit: bool) {
        let (index, mps) = self.contexts.get(context);
        let row = self.states[usize::from(index)];
        let qe = u32::from(row.0);
        let narrowed = self.interval - qe;
        let lower_is_mps = narrowed >= qe;
        if (bit == mps) == lower_is_mps {
            self.interval = narrowed;
        } else {
            self.low.add(narrowed, 16, self.shift);
            self.interval = qe;
        }
        let renormalized = self.interval < 0x8000;
        self.contexts
            .update(context, row, mps, bit != mps, renormalized);
        while self.interval < 0x8000 {
            self.interval <<= 1;
            self.shift += 1;
        }
        self.decisions += 1;
    }

    /// Rows of a CAJ type-0 image, top first, with its observed model: one
    /// row-control decision in context 457 (one copies the preceding row, a
    /// blank row above the first), else each pixel with ten neighbours. A
    /// row equal to its predecessor is copied only when `copy_rows`.
    pub fn type0_rows(&mut self, rows: &[Vec<bool>], copy_rows: bool) {
        let at = |y: isize, x: isize| -> usize {
            if y < 0 || x < 0 {
                return 0;
            }
            rows.get(y as usize)
                .and_then(|row| row.get(x as usize))
                .map_or(0, |&bit| usize::from(bit))
        };
        for (y, row) in rows.iter().enumerate() {
            let blank = vec![false; row.len()];
            let previous = if y == 0 { &blank } else { &rows[y - 1] };
            let copy = copy_rows && row == previous;
            self.encode(457, copy);
            if copy {
                continue;
            }
            let y = y as isize;
            for (x, &bit) in row.iter().enumerate() {
                let x = x as isize;
                let mut context = 0;
                for (dy, dx) in [
                    (0, -2),
                    (0, -1),
                    (-1, -2),
                    (-1, -1),
                    (-1, 0),
                    (-1, 1),
                    (-1, 2),
                    (-2, -1),
                    (-2, 0),
                    (-2, 1),
                ] {
                    context = (context << 1) | at(y + dy, x + dx);
                }
                self.encode(context, bit);
            }
        }
    }

    /// The shortest SCD whose zero-extended value lies in the final interval.
    pub fn finish(self) -> Vec<u8> {
        // The final interval is at least 2^-(shift + 1) wide.
        let (mut bits, rest) = self.low.truncated(self.shift + 1);
        if rest {
            increment(&mut bits);
        }
        let mut bytes = pack(&bits, false, 0);
        while bytes.last() == Some(&0) {
            bytes.pop();
        }
        bytes
    }
}

/// T.88 Annex E encoder for one MQ coding unit, including the `FF AC`
/// terminator. The decoder places the LPS in the lower subinterval `[0, Qe)`
/// unless `A - Qe < Qe`, and reads one bits after the terminal marker.
#[derive(Clone)]
pub struct MqEncoder {
    states: Vec<StateRow>,
    contexts: Contexts,
    low: Bits,
    interval: u32,
    shift: usize,
    decisions: u64,
}

impl MqEncoder {
    pub fn new(states: &[StateRow]) -> Self {
        Self {
            states: states.to_vec(),
            contexts: Contexts::default(),
            low: Bits::default(),
            interval: 0x8000,
            shift: 0,
            decisions: 0,
        }
    }

    /// Decisions encoded so far, as the decoder's symbol counter.
    pub fn decisions(&self) -> u64 {
        self.decisions
    }

    pub fn encode(&mut self, context: usize, bit: bool) {
        let (index, mps) = self.contexts.get(context);
        let row = self.states[usize::from(index)];
        let qe = u32::from(row.0);
        let narrowed = self.interval - qe;
        let lower_is_mps = narrowed < qe;
        if (bit == mps) == lower_is_mps {
            self.interval = qe;
        } else {
            self.low.add(qe, 15, self.shift);
            self.interval = narrowed;
        }
        let renormalized = self.interval < 0x8000;
        self.contexts
            .update(context, row, mps, bit != mps, renormalized);
        while self.interval < 0x8000 {
            self.interval <<= 1;
            self.shift += 1;
        }
        self.decisions += 1;
    }

    /// One T.88 Annex A.2 integer in the 512-context bank at `base`; `None`
    /// is the out-of-band value.
    pub fn integer(&mut self, base: usize, value: Option<i64>) {
        let mut prev = 1_usize;
        let mut put = |encoder: &mut Self, bit: bool| {
            encoder.encode(base + prev, bit);
            let next = (prev << 1) | usize::from(bit);
            prev = if prev < 256 { next } else { (next & 511) | 256 };
        };
        let Some(value) = value else {
            for bit in [true, false, false, false] {
                put(self, bit);
            }
            return;
        };
        let magnitude = value.unsigned_abs();
        const BANDS: [(u32, u64); 6] = [(2, 0), (4, 4), (6, 20), (8, 84), (12, 340), (32, 4436)];
        let band = BANDS
            .iter()
            .rposition(|&(_, start)| magnitude >= start)
            .unwrap();
        put(self, value < 0);
        for _ in 0..band {
            put(self, true);
        }
        if band < BANDS.len() - 1 {
            put(self, false);
        }
        let (width, start) = BANDS[band];
        let payload = magnitude - start;
        for bit in (0..width).rev() {
            put(self, payload >> bit & 1 != 0);
        }
    }

    /// One T.88 Annex A.3 fixed-length symbol ID in the `2^code_len`
    /// contexts at `base`.
    pub fn iaid(&mut self, base: usize, code_len: u32, id: u64) {
        let mut prev = 1_usize;
        for shift in (0..code_len).rev() {
            let bit = id >> shift & 1 != 0;
            self.encode(base + prev, bit);
            prev = (prev << 1) | usize::from(bit);
        }
    }

    /// A bitmap, one decision per pixel in raster order, with the 10-pixel
    /// template-2 neighbourhood (adaptive pixel at `(2, -1)`) as the context
    /// in the bank at `base`. `rows[y][x]` is one pixel.
    pub fn template2(&mut self, base: usize, rows: &[Vec<bool>]) {
        let pixel = |x: isize, y: isize| -> usize {
            if x < 0 || y < 0 {
                return 0;
            }
            rows.get(y as usize)
                .and_then(|row| row.get(x as usize))
                .map_or(0, |&bit| usize::from(bit))
        };
        for (y, row) in rows.iter().enumerate() {
            let y = y as isize;
            for (x, &bit) in row.iter().enumerate() {
                let x = x as isize;
                let mut context = 0;
                for (dx, dy) in [
                    (-1, -2),
                    (0, -2),
                    (1, -2),
                    (-2, -1),
                    (-1, -1),
                    (0, -1),
                    (1, -1),
                    (2, -1),
                    (-2, 0),
                    (-1, 0),
                ] {
                    context = (context << 1) | pixel(x + dx, y + dy);
                }
                self.encode(base + context, bit);
            }
        }
    }

    /// A refinement target, one decision per pixel in raster order, with the
    /// template-1 neighbourhood (T.88 Figure 13) as the context in the bank
    /// at `base`: three target pixels above, the one to the left, and the
    /// reference pixels above, left, centre, right, below and below right
    /// of `(x - dx, y - dy)`. Pixels outside either bitmap are zero.
    pub fn template1(
        &mut self,
        base: usize,
        target: &[Vec<bool>],
        reference: &[Vec<bool>],
        (dx, dy): (i64, i64),
    ) {
        let pixel = |rows: &[Vec<bool>], x: i64, y: i64| -> usize {
            if x < 0 || y < 0 {
                return 0;
            }
            rows.get(y as usize)
                .and_then(|row| row.get(x as usize))
                .map_or(0, |&bit| usize::from(bit))
        };
        for (y, row) in target.iter().enumerate() {
            let y = y as i64;
            for (x, &bit) in row.iter().enumerate() {
                let x = x as i64;
                let (rx, ry) = (x - dx, y - dy);
                let context = [
                    pixel(target, x - 1, y - 1),
                    pixel(target, x, y - 1),
                    pixel(target, x + 1, y - 1),
                    pixel(target, x - 1, y),
                    pixel(reference, rx, ry - 1),
                    pixel(reference, rx - 1, ry),
                    pixel(reference, rx, ry),
                    pixel(reference, rx + 1, ry),
                    pixel(reference, rx, ry + 1),
                    pixel(reference, rx + 1, ry + 1),
                ]
                .into_iter()
                .fold(0, |context, pixel| (context << 1) | pixel);
                self.encode(base + context, bit);
            }
        }
    }

    /// The shortest data whose value, followed by the one bits a decoder
    /// supplies from the terminator on, lies in the final interval; then
    /// `FF AC`.
    pub fn finish(self) -> Vec<u8> {
        let mut bytes = self.finish_data();
        bytes.extend_from_slice(&[0xff, 0xac]);
        bytes
    }

    /// [`finish`](Self::finish) without the terminal marker.
    pub fn finish_data(self) -> Vec<u8> {
        // The final interval is at least 2^-shift wide; a value at most one
        // `2^-(shift + 1)` step above the lower bound lies inside it.
        let count = self.shift + 1;
        let (mut bits, rest) = self.low.truncated(count);
        // Choose `D` with `D + 2^-count` in `[low, low + 2^-count]`.
        if !rest && bits.contains(&1) {
            decrement(&mut bits);
        }
        let mut bytes = pack(&bits, true, 1);
        // A trailing all-one byte is what the terminator supplies anyway, and
        // an `FF` data byte must not precede the terminator.
        loop {
            let after_ff = bytes.len() >= 2 && bytes[bytes.len() - 2] == 0xff;
            match bytes.last() {
                Some(0xff) => {}
                Some(0x7f) if after_ff => {}
                _ => break,
            }
            bytes.pop();
        }
        bytes
    }
}

impl MqEncoder {
    /// Data bytes whose value, followed by any number of zero bytes, lies
    /// in the final interval: for a test that pads the coded data before
    /// its own (possibly invalid) terminal pair.
    pub fn finish_zero_padded(self) -> Vec<u8> {
        let (mut bits, rest) = self.low.truncated(self.shift + 1);
        if rest {
            increment(&mut bits);
        }
        let mut bytes = pack(&bits, true, 0);
        while bytes.last() == Some(&0) {
            bytes.pop();
        }
        bytes
    }
}

/// Pack bits into bytes, filling the last byte with `fill` bits. With `mq`,
/// a byte after `FF` carries seven bits under a zero MSB.
fn pack(bits: &[u8], mq: bool, fill: u8) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut at = 0;
    while at < bits.len() {
        let width = if mq && bytes.last() == Some(&0xff) {
            7
        } else {
            8
        };
        let mut byte = 0_u8;
        for _ in 0..width {
            let bit = bits.get(at).copied().unwrap_or(fill);
            byte = (byte << 1) | bit;
            at += 1;
        }
        bytes.push(byte);
    }
    bytes
}
