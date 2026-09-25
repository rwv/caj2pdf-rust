// SPDX-License-Identifier: MIT

//! Bounded, byte-oriented PDF syntax parsing outside stream payloads.

use super::super::types::PdfRef;
use crate::fallible::reserve_exact;
use std::ops::Range;

pub(super) const MAX_SYNTAX_DEPTH: u32 = 64;
const MAX_DICTIONARY_ENTRIES: usize = 65_536;
const MAX_REFERENCES_PER_OBJECT: usize = 262_144;

#[derive(Clone, Debug)]
pub struct DictEntry {
    pub(super) name: Vec<u8>,
    pub(super) pair: Range<usize>,
    pub(super) value: Range<usize>,
}

impl DictEntry {
    pub fn name(&self) -> &[u8] {
        &self.name
    }

    pub fn raw_pair<'a>(&self, dictionary: &'a [u8]) -> &'a [u8] {
        &dictionary[self.pair.clone()]
    }

    pub(super) fn value<'a>(&self, dictionary: &'a [u8]) -> &'a [u8] {
        &dictionary[self.value.clone()]
    }
}

#[derive(Clone, Debug)]
pub(super) struct Dictionary {
    pub bytes: Vec<u8>,
    pub entries: Vec<DictEntry>,
}

impl Dictionary {
    pub fn entry(&self, name: &[u8]) -> Option<&DictEntry> {
        self.entries.iter().find(|entry| entry.name == name)
    }

    pub fn entries_named<'a>(&'a self, name: &'a [u8]) -> impl Iterator<Item = &'a DictEntry> {
        self.entries.iter().filter(move |entry| entry.name == name)
    }

    pub fn value(&self, name: &[u8]) -> Option<&[u8]> {
        self.entry(name).map(|entry| entry.value(&self.bytes))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ParseIssue {
    pub at: usize,
    pub incomplete: bool,
    pub ambiguous: bool,
    pub limit: Option<(&'static str, u64, u64)>,
    pub reason: &'static str,
}

type ParseResult<T> = std::result::Result<T, ParseIssue>;

fn malformed(at: usize, reason: &'static str) -> ParseIssue {
    ParseIssue {
        at,
        incomplete: false,
        ambiguous: false,
        limit: None,
        reason,
    }
}

fn incomplete(at: usize, reason: &'static str) -> ParseIssue {
    ParseIssue {
        at,
        incomplete: true,
        ambiguous: false,
        limit: None,
        reason,
    }
}

fn ambiguous(at: usize, reason: &'static str) -> ParseIssue {
    ParseIssue {
        at,
        incomplete: false,
        ambiguous: true,
        limit: None,
        reason,
    }
}

fn limit_issue(at: usize, resource: &'static str, limit: u64, attempted: u64) -> ParseIssue {
    ParseIssue {
        at,
        incomplete: false,
        ambiguous: false,
        limit: Some((resource, limit, attempted)),
        reason: "PDF syntax resource limit exceeded",
    }
}

pub(super) struct Syntax<'a> {
    bytes: &'a [u8],
    pub pos: usize,
    pub max_reference: u32,
    pub references: Vec<PdfRef>,
}

impl<'a> Syntax<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            pos: 0,
            max_reference: 0,
            references: Vec::new(),
        }
    }

    pub fn skip_space(&mut self) {
        loop {
            while self.pos < self.bytes.len() && is_space(self.bytes[self.pos]) {
                self.pos += 1;
            }
            if self.bytes.get(self.pos) != Some(&b'%') {
                break;
            }
            while self.pos < self.bytes.len()
                && self.bytes[self.pos] != b'\r'
                && self.bytes[self.pos] != b'\n'
            {
                self.pos += 1;
            }
        }
    }

    fn peek(&self) -> ParseResult<u8> {
        self.bytes
            .get(self.pos)
            .copied()
            .ok_or_else(|| incomplete(self.pos, "PDF value is truncated"))
    }

    fn expect_byte(&mut self, expected: u8, reason: &'static str) -> ParseResult<()> {
        let found = self.peek()?;
        if found != expected {
            return Err(malformed(self.pos, reason));
        }
        self.pos += 1;
        Ok(())
    }

    fn keyword(&mut self, keyword: &[u8]) -> ParseResult<()> {
        if self.bytes.len() - self.pos < keyword.len() {
            return Err(incomplete(self.pos, "PDF keyword is truncated"));
        }
        if &self.bytes[self.pos..self.pos + keyword.len()] != keyword {
            return Err(malformed(self.pos, "unexpected PDF keyword"));
        }
        self.pos += keyword.len();
        if self
            .bytes
            .get(self.pos)
            .is_some_and(|byte| !is_delimiter(*byte))
        {
            return Err(malformed(self.pos, "PDF keyword lacks a delimiter"));
        }
        Ok(())
    }

    pub fn consume_keyword(&mut self, keyword: &[u8]) -> ParseResult<bool> {
        if self.pos == self.bytes.len() {
            return Err(incomplete(self.pos, "PDF keyword is truncated"));
        }
        if self.bytes.len() - self.pos < keyword.len() {
            if keyword.starts_with(&self.bytes[self.pos..]) {
                return Err(incomplete(self.pos, "PDF keyword is truncated"));
            }
            return Ok(false);
        }
        if !self.bytes[self.pos..].starts_with(keyword) {
            return Ok(false);
        }
        self.keyword(keyword)?;
        Ok(true)
    }

    fn read_number_token(&mut self) -> ParseResult<(Range<usize>, bool)> {
        let start = self.pos;
        if matches!(self.peek()?, b'+' | b'-') {
            self.pos += 1;
        }
        let mut digits = 0;
        while self.bytes.get(self.pos).is_some_and(u8::is_ascii_digit) {
            self.pos += 1;
            digits += 1;
        }
        let mut integer = true;
        if self.bytes.get(self.pos) == Some(&b'.') {
            integer = false;
            self.pos += 1;
            while self.bytes.get(self.pos).is_some_and(u8::is_ascii_digit) {
                self.pos += 1;
                digits += 1;
            }
        }
        if digits == 0 {
            return Err(malformed(start, "PDF number has no digits"));
        }
        if self
            .bytes
            .get(self.pos)
            .is_some_and(|byte| !is_delimiter(*byte))
        {
            return Err(malformed(self.pos, "PDF number lacks a delimiter"));
        }
        Ok((start..self.pos, integer))
    }

    pub fn unsigned(&mut self) -> ParseResult<u64> {
        self.skip_space();
        let (span, integer) = self.read_number_token()?;
        if !integer || self.bytes[span.start] == b'-' {
            return Err(malformed(span.start, "expected a nonnegative PDF integer"));
        }
        let mut value = 0_u64;
        for &byte in &self.bytes[span] {
            if byte == b'+' {
                continue;
            }
            value = value
                .checked_mul(10)
                .and_then(|result| result.checked_add(u64::from(byte - b'0')))
                .ok_or_else(|| malformed(self.pos, "PDF integer overflows 64 bits"))?;
        }
        Ok(value)
    }

    pub fn reference(&mut self) -> ParseResult<PdfRef> {
        let start = self.pos;
        let number = self.unsigned()?;
        let generation = self.unsigned()?;
        self.skip_space();
        self.keyword(b"R")?;
        let number = u32::try_from(number)
            .map_err(|_| malformed(start, "PDF object number exceeds 32 bits"))?;
        let generation = u16::try_from(generation)
            .map_err(|_| malformed(start, "PDF generation exceeds 16 bits"))?;
        if number == 0 {
            return Err(malformed(start, "PDF object zero cannot be referenced"));
        }
        let reference = PdfRef { number, generation };
        self.remember_reference(reference)?;
        Ok(reference)
    }

    fn remember_reference(&mut self, reference: PdfRef) -> ParseResult<()> {
        if self.references.len() >= MAX_REFERENCES_PER_OBJECT {
            return Err(limit_issue(
                self.pos,
                "PDF object references",
                MAX_REFERENCES_PER_OBJECT as u64,
                self.references.len() as u64 + 1,
            ));
        }
        self.max_reference = self.max_reference.max(reference.number);
        self.references.push(reference);
        Ok(())
    }

    fn name(&mut self) -> ParseResult<Vec<u8>> {
        let mut result = Vec::new();
        self.scan_name(|byte| result.push(byte))?;
        Ok(result)
    }

    fn scan_name(&mut self, mut accept: impl FnMut(u8)) -> ParseResult<()> {
        self.expect_byte(b'/', "expected PDF name")?;
        while let Some(&byte) = self.bytes.get(self.pos) {
            if is_delimiter(byte) {
                break;
            }
            if byte == b'#' {
                if self.bytes.len() - self.pos < 3 {
                    return Err(incomplete(self.pos, "escaped PDF name is truncated"));
                }
                let high = hex_digit(self.bytes[self.pos + 1])
                    .ok_or_else(|| malformed(self.pos + 1, "invalid PDF name escape"))?;
                let low = hex_digit(self.bytes[self.pos + 2])
                    .ok_or_else(|| malformed(self.pos + 2, "invalid PDF name escape"))?;
                accept((high << 4) | low);
                self.pos += 3;
            } else {
                accept(byte);
                self.pos += 1;
            }
        }
        Ok(())
    }

    pub fn skip_value(&mut self, depth: u32) -> ParseResult<()> {
        if depth > MAX_SYNTAX_DEPTH {
            return Err(limit_issue(
                self.pos,
                "PDF syntax depth",
                MAX_SYNTAX_DEPTH as u64,
                depth as u64,
            ));
        }
        self.skip_space();
        match self.peek()? {
            b'/' => {
                self.scan_name(|_| {})?;
            }
            b'[' => {
                self.pos += 1;
                loop {
                    self.skip_space();
                    if self.peek()? == b']' {
                        self.pos += 1;
                        break;
                    }
                    self.skip_value(depth + 1)?;
                }
            }
            b'<' if self.bytes.get(self.pos + 1) == Some(&b'<') => {
                self.dictionary(depth + 1)?;
            }
            b'<' => {
                self.pos += 1;
                loop {
                    let byte = self.peek()?;
                    self.pos += 1;
                    if byte == b'>' {
                        break;
                    }
                    if !is_space(byte) && hex_digit(byte).is_none() {
                        return Err(malformed(self.pos - 1, "invalid PDF hexadecimal string"));
                    }
                }
            }
            b'(' => {
                self.pos += 1;
                let mut nesting = 1_u32;
                while nesting != 0 {
                    let byte = self.peek()?;
                    self.pos += 1;
                    match byte {
                        b'\\' => {
                            self.peek()?;
                            self.pos += 1;
                        }
                        b'(' => {
                            nesting += 1;
                            if nesting > MAX_SYNTAX_DEPTH {
                                return Err(limit_issue(
                                    self.pos,
                                    "PDF string nesting",
                                    MAX_SYNTAX_DEPTH as u64,
                                    nesting as u64,
                                ));
                            }
                        }
                        b')' => nesting -= 1,
                        _ => {}
                    }
                }
            }
            b'+' | b'-' | b'.' | b'0'..=b'9' => {
                let (first, integer) = self.read_number_token()?;
                if integer && self.bytes[first.start] != b'-' {
                    let saved = self.pos;
                    self.skip_space();
                    if self.bytes.get(self.pos).is_some_and(u8::is_ascii_digit) {
                        let second_start = self.pos;
                        if let Ok((second, true)) = self.read_number_token() {
                            self.skip_space();
                            if self.pos == self.bytes.len() {
                                return Err(incomplete(self.pos, "PDF reference may be truncated"));
                            }
                            if self.bytes.get(self.pos) == Some(&b'R')
                                && self
                                    .bytes
                                    .get(self.pos + 1)
                                    .is_none_or(|byte| is_delimiter(*byte))
                            {
                                let number = parse_u32(&self.bytes[first.clone()]);
                                let generation = parse_u16(&self.bytes[second]);
                                if let (Some(number), Some(generation)) = (number, generation) {
                                    self.remember_reference(PdfRef { number, generation })?;
                                    self.pos += 1;
                                    return Ok(());
                                }
                            }
                        }
                        self.pos = second_start;
                    }
                    self.pos = saved;
                }
            }
            b't' => self.keyword(b"true")?,
            b'f' => self.keyword(b"false")?,
            b'n' => self.keyword(b"null")?,
            _ => return Err(malformed(self.pos, "invalid PDF value token")),
        }
        Ok(())
    }

    pub fn dictionary(&mut self, depth: u32) -> ParseResult<Vec<DictEntry>> {
        if depth > MAX_SYNTAX_DEPTH {
            return Err(limit_issue(
                self.pos,
                "PDF dictionary depth",
                MAX_SYNTAX_DEPTH as u64,
                depth as u64,
            ));
        }
        if self.bytes.len() - self.pos < 2 {
            return Err(incomplete(self.pos, "PDF dictionary is truncated"));
        }
        if &self.bytes[self.pos..self.pos + 2] != b"<<" {
            return Err(malformed(self.pos, "expected PDF dictionary"));
        }
        self.pos += 2;
        let mut entries = Vec::new();
        loop {
            self.skip_space();
            if self.bytes.len() - self.pos < 2 {
                return Err(incomplete(self.pos, "PDF dictionary is truncated"));
            }
            if &self.bytes[self.pos..self.pos + 2] == b">>" {
                self.pos += 2;
                break;
            }
            let start = self.pos;
            if entries.len() >= MAX_DICTIONARY_ENTRIES {
                return Err(limit_issue(
                    self.pos,
                    "PDF dictionary entries",
                    MAX_DICTIONARY_ENTRIES as u64,
                    entries.len() as u64 + 1,
                ));
            }
            let name = self.name()?;
            self.skip_space();
            let value_start = self.pos;
            self.skip_value(depth + 1)?;
            entries.push(DictEntry {
                name,
                pair: start..self.pos,
                value: value_start..self.pos,
            });
        }
        if depth > 0 && entries.len() > 1 {
            let mut order = Vec::new();
            let refused = malformed(
                self.pos,
                "PDF nested dictionary key index allocation failed",
            );
            reserve_exact(&mut order, entries.len(), refused)?;
            order.extend(0..entries.len());
            order.sort_unstable_by(|left, right| entries[*left].name.cmp(&entries[*right].name));
            if let Some(pair) = order
                .windows(2)
                .find(|pair| entries[pair[0]].name == entries[pair[1]].name)
            {
                return Err(ambiguous(
                    entries[pair[1]].pair.start,
                    "duplicate nested PDF dictionary keys have undefined value",
                ));
            }
        }
        Ok(entries)
    }

    pub fn at_end(&mut self) -> bool {
        self.skip_space();
        self.pos == self.bytes.len()
    }
}

#[derive(Debug)]
pub(super) enum ObjectTail {
    EndObject { end: usize },
    Stream { data_start: usize },
}

#[derive(Debug)]
pub(super) struct ObjectHead {
    pub reference: PdfRef,
    pub dictionary: Option<Dictionary>,
    /// Offset of the dictionary within `bytes`, when the value is a dictionary.
    pub dictionary_start: Option<usize>,
    pub scalar: Option<Range<usize>>,
    pub bytes: Vec<u8>,
    pub tail: ObjectTail,
    pub max_reference: u32,
    pub references: Vec<PdfRef>,
}

pub(super) fn parse_object_head(bytes: Vec<u8>) -> ParseResult<ObjectHead> {
    let mut parser = Syntax::new(&bytes);
    let number = parser.unsigned()?;
    let generation = parser.unsigned()?;
    parser.skip_space();
    parser.keyword(b"obj")?;
    parser.skip_space();
    let value_start = parser.pos;
    let dictionary = if bytes.get(value_start..value_start + 2) == Some(b"<<".as_slice()) {
        let entries = parser.dictionary(0)?;
        let value_end = parser.pos;
        let mut adjusted = entries;
        for entry in &mut adjusted {
            entry.pair.start -= value_start;
            entry.pair.end -= value_start;
            entry.value.start -= value_start;
            entry.value.end -= value_start;
        }
        Some(Dictionary {
            bytes: bytes[value_start..value_end].to_vec(),
            entries: adjusted,
        })
    } else {
        parser.skip_value(0)?;
        None
    };
    let value_end = parser.pos;
    parser.skip_space();
    let tail = if parser.consume_keyword(b"stream")? {
        let byte = parser.peek()?;
        match byte {
            b'\n' => parser.pos += 1,
            b'\r' => {
                parser.pos += 1;
                if bytes.get(parser.pos) == Some(&b'\n') {
                    parser.pos += 1;
                }
            }
            _ => {
                return Err(malformed(
                    parser.pos,
                    "PDF stream requires an end-of-line marker",
                ));
            }
        }
        ObjectTail::Stream {
            data_start: parser.pos,
        }
    } else if parser.consume_keyword(b"endobj")? {
        ObjectTail::EndObject { end: parser.pos }
    } else {
        return Err(malformed(parser.pos, "PDF object lacks endobj or stream"));
    };
    let reference = PdfRef {
        number: u32::try_from(number)
            .map_err(|_| malformed(0, "PDF object number exceeds 32 bits"))?,
        generation: u16::try_from(generation)
            .map_err(|_| malformed(0, "PDF generation exceeds 16 bits"))?,
    };
    if reference.number == 0 {
        return Err(malformed(0, "PDF object zero is reserved"));
    }
    let max_reference = parser.max_reference;
    let references = std::mem::take(&mut parser.references);
    drop(parser);
    Ok(ObjectHead {
        reference,
        dictionary,
        dictionary_start: bytes[value_start..value_end]
            .starts_with(b"<<")
            .then_some(value_start),
        scalar: if value_start == value_end || bytes[value_start..value_end].starts_with(b"<<") {
            None
        } else {
            Some(value_start..value_end)
        },
        bytes,
        tail,
        max_reference,
        references,
    })
}

pub(super) fn exact_unsigned(bytes: &[u8]) -> Option<u64> {
    let mut parser = Syntax::new(bytes);
    let value = parser.unsigned().ok()?;
    parser.at_end().then_some(value)
}

pub(super) fn exact_reference(bytes: &[u8]) -> Option<PdfRef> {
    let mut parser = Syntax::new(bytes);
    let value = parser.reference().ok()?;
    parser.at_end().then_some(value)
}

pub(super) fn exact_name(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut parser = Syntax::new(bytes);
    parser.skip_space();
    let name = parser.name().ok()?;
    parser.at_end().then_some(name)
}

pub(super) fn unsigned_array(bytes: &[u8], max_items: usize) -> Option<Vec<u64>> {
    let mut parser = Syntax::new(bytes);
    parser.skip_space();
    parser
        .expect_byte(b'[', "expected PDF integer array")
        .ok()?;
    let mut result = Vec::new();
    loop {
        parser.skip_space();
        if parser.peek().ok()? == b']' {
            parser.pos += 1;
            break;
        }
        if result.len() >= max_items {
            return None;
        }
        result.push(parser.unsigned().ok()?);
    }
    parser.at_end().then_some(result)
}

pub(super) fn valid_id_array(bytes: &[u8]) -> bool {
    let mut parser = Syntax::new(bytes);
    parser.skip_space();
    if parser.expect_byte(b'[', "expected PDF ID array").is_err() {
        return false;
    }
    for _ in 0..2 {
        parser.skip_space();
        if !matches!(parser.bytes.get(parser.pos), Some(b'(' | b'<'))
            || parser.bytes.get(parser.pos..parser.pos.saturating_add(2)) == Some(b"<<".as_slice())
            || parser.skip_value(0).is_err()
        {
            return false;
        }
    }
    parser.skip_space();
    parser
        .expect_byte(b']', "expected PDF ID array end")
        .is_ok()
        && parser.at_end()
}

pub(super) fn valid_text_string(bytes: &[u8]) -> bool {
    let mut parser = Syntax::new(bytes);
    parser.skip_space();
    if !matches!(parser.bytes.get(parser.pos), Some(b'(' | b'<'))
        || parser.bytes.get(parser.pos..parser.pos.saturating_add(2)) == Some(b"<<".as_slice())
    {
        return false;
    }
    parser.skip_value(0).is_ok() && parser.at_end()
}

pub(super) fn reference_array(bytes: &[u8], max_items: usize) -> Option<Vec<PdfRef>> {
    let mut parser = Syntax::new(bytes);
    parser.skip_space();
    parser.expect_byte(b'[', "expected PDF array").ok()?;
    let mut result = Vec::new();
    loop {
        parser.skip_space();
        if parser.peek().ok()? == b']' {
            parser.pos += 1;
            break;
        }
        if result.len() >= max_items {
            return None;
        }
        result.push(parser.reference().ok()?);
    }
    parser.at_end().then_some(result)
}

pub(super) fn destination_page(bytes: &[u8]) -> Option<PdfRef> {
    let mut parser = Syntax::new(bytes);
    parser.skip_space();
    parser
        .expect_byte(b'[', "expected PDF destination array")
        .ok()?;
    let page = parser.reference().ok()?;
    parser.skip_space();
    let mode = parser.name().ok()?;
    if mode.is_empty() {
        return None;
    }
    loop {
        parser.skip_space();
        if parser.peek().ok()? == b']' {
            parser.pos += 1;
            break;
        }
        parser.skip_value(0).ok()?;
    }
    parser.at_end().then_some(page)
}

pub(super) fn media_box(bytes: &[u8]) -> Option<[f64; 4]> {
    let mut parser = Syntax::new(bytes);
    parser.skip_space();
    parser.expect_byte(b'[', "expected PDF array").ok()?;
    let mut box_values = [0.0; 4];
    for value in &mut box_values {
        parser.skip_space();
        let (span, _) = parser.read_number_token().ok()?;
        let number = std::str::from_utf8(&bytes[span])
            .ok()?
            .parse::<f64>()
            .ok()?;
        if !number.is_finite() {
            return None;
        }
        *value = number;
    }
    parser.skip_space();
    parser.expect_byte(b']', "expected PDF array end").ok()?;
    (parser.at_end() && box_values[2] > box_values[0] && box_values[3] > box_values[1])
        .then_some(box_values)
}

fn is_space(byte: u8) -> bool {
    matches!(byte, 0 | b'\t' | b'\n' | 12 | b'\r' | b' ')
}

fn is_delimiter(byte: u8) -> bool {
    is_space(byte)
        || matches!(
            byte,
            b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
        )
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn parse_u32(bytes: &[u8]) -> Option<u32> {
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

fn parse_u16(bytes: &[u8]) -> Option<u16> {
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_values_keep_dictionary_names_and_indirect_references() {
        let raw = b"12 0 obj\r<< /Ty#70e /Page /Values [true false null -2 .5 <ABC> (a\\(b\\)c) << /Child 3 1 R >>] /Parent 2 0 R >>\rendobj";
        let head = parse_object_head(raw.to_vec()).unwrap();
        assert_eq!(
            head.reference,
            PdfRef {
                number: 12,
                generation: 0
            }
        );
        let dictionary = head.dictionary.unwrap();
        assert_eq!(dictionary.value(b"Type"), Some(b"/Page".as_slice()));
        assert_eq!(
            head.references,
            vec![
                PdfRef {
                    number: 3,
                    generation: 1
                },
                PdfRef {
                    number: 2,
                    generation: 0
                }
            ]
        );
        assert_eq!(head.max_reference, 3);
        assert!(matches!(head.tail, ObjectTail::EndObject { .. }));
    }

    #[test]
    fn stream_head_stops_at_binary_payload_with_crlf() {
        let raw = b"5 0 obj\n<< /Length 8 >>\nstream\r\nendobj!!\nendstream\nendobj";
        let head = parse_object_head(raw.to_vec()).unwrap();
        assert!(matches!(
            head.tail,
            ObjectTail::Stream { data_start }
                if &head.bytes[data_start..data_start + 8] == b"endobj!!"
        ));
        let lone_cr = parse_object_head(b"5 0 obj << /Length 1 >> stream\rX".to_vec()).unwrap();
        assert!(matches!(
            lone_cr.tail,
            ObjectTail::Stream { data_start }
                if &lone_cr.bytes[data_start..data_start + 1] == b"X"
        ));
        assert!(parse_object_head(b"5 0 obj << /Length 0 >> stream X".to_vec()).is_err());
    }

    #[test]
    fn invalid_values_and_limits_fail_without_guessing() {
        for raw in [
            b"1 0 obj <ABG> endobj".as_slice(),
            b"1 0 obj (unterminated endobj",
            b"1 0 obj /Bad#GG endobj",
            b"1 0 obj /Bad#",
            b"1 0 obj 1.2.3 endobj",
            b"1 0 obj << /A [1 2 >> endobj",
            b"0 0 obj null endobj",
            b"4294967296 0 obj null endobj",
        ] {
            assert!(
                parse_object_head(raw.to_vec()).is_err(),
                "accepted malformed input: {raw:?}"
            );
        }
        let mut nested = Vec::from(b"1 0 obj ".as_slice());
        nested.extend(std::iter::repeat_n(b'[', MAX_SYNTAX_DEPTH as usize + 1));
        nested.push(b'0');
        nested.extend(std::iter::repeat_n(b']', MAX_SYNTAX_DEPTH as usize + 1));
        nested.extend_from_slice(b" endobj");
        assert!(parse_object_head(nested).is_err());
    }

    #[test]
    fn destination_and_box_helpers_reject_ambiguous_values() {
        let page = PdfRef {
            number: 7,
            generation: 2,
        };
        assert_eq!(destination_page(b"[7 2 R /XYZ 0 300 null]"), Some(page));
        assert_eq!(destination_page(b"[7 2 R null]"), None);
        assert_eq!(destination_page(b"/named"), None);
        assert_eq!(reference_array(b"[7 2 R 8 0 R]", 2).unwrap().len(), 2);
        assert!(reference_array(b"[7 2 R 8 0 R]", 1).is_none());
        assert_eq!(media_box(b"[0 0 100 200]"), Some([0.0, 0.0, 100.0, 200.0]));
        assert!(media_box(b"[0 0 0 200]").is_none());
        assert!(media_box(b"[0 0 1 NaN]").is_none());
        assert!(exact_unsigned(b"18446744073709551616").is_none());
        assert!(exact_reference(b"0 0 R").is_none());
    }

    #[test]
    fn nested_duplicate_names_and_invalid_id_forms_are_rejected() {
        let issue = parse_object_head(
            b"1 0 obj << /Resources << /ProcSet [/PDF] /ProcSet [/Text] >> >> endobj".to_vec(),
        )
        .unwrap_err();
        assert!(issue.ambiguous);
        assert!(valid_id_array(b"[<0123> (second)]"));
        assert!(valid_id_array(b"[(first) <ABCD>]"));
        for value in [
            b"/Bogus".as_slice(),
            b"[]",
            b"[<01>]",
            b"[1 2]",
            b"[<01> <02> <03>]",
            b"[<<>> <02>]",
        ] {
            assert!(
                !valid_id_array(value),
                "accepted invalid trailer ID: {value:?}"
            );
        }
    }

    fn head_issue(raw: &[u8]) -> ParseIssue {
        let result = parse_object_head(raw.to_vec());
        assert!(result.is_err(), "accepted malformed object {raw:?}");
        result.unwrap_err()
    }

    #[test]
    fn comments_are_whitespace_between_tokens() {
        let head = parse_object_head(
            b"% leading comment\r\n+7 % number comment\n0 obj%\r<< /Kind /Note % trailing\n>> % tail\nendobj"
                .to_vec(),
        )
        .unwrap();
        assert_eq!(
            head.reference,
            PdfRef {
                number: 7,
                generation: 0
            }
        );
        assert_eq!(
            head.dictionary.unwrap().value(b"Kind"),
            Some(b"/Note".as_slice())
        );
        assert!(matches!(head.tail, ObjectTail::EndObject { .. }));
        assert_eq!(exact_unsigned(b" +42 % explicit sign\n"), Some(42));
    }

    #[test]
    fn object_keywords_distinguish_truncation_from_bad_syntax() {
        let truncated = head_issue(b"1 0 ob");
        assert!(truncated.incomplete);
        assert_eq!(truncated.reason, "PDF keyword is truncated");
        assert_eq!(truncated.at, 4);

        let glued = head_issue(b"1 0 objx null endobj");
        assert!(!glued.incomplete);
        assert_eq!(glued.reason, "PDF keyword lacks a delimiter");
        assert_eq!(glued.at, 7);

        let missing_tail = head_issue(b"1 0 obj null trailer");
        assert!(!missing_tail.incomplete);
        assert_eq!(missing_tail.reason, "PDF object lacks endobj or stream");
        assert_eq!(missing_tail.at, 13);

        let dangling_pair = head_issue(b"1 0 obj 5 0");
        assert!(dangling_pair.incomplete);
        assert_eq!(dangling_pair.reason, "PDF reference may be truncated");

        let other = head_issue(b"1 0 xyz null endobj");
        assert!(!other.incomplete);
        assert_eq!(other.reason, "unexpected PDF keyword");
        assert_eq!(other.at, 4);
    }

    #[test]
    fn name_escapes_decode_or_report_their_position() {
        let head = parse_object_head(b"1 0 obj << /A#42 /V#43 >> endobj".to_vec()).unwrap();
        let dictionary = head.dictionary.unwrap();
        assert_eq!(dictionary.value(b"AB"), Some(b"/V#43".as_slice()));
        assert_eq!(exact_name(b"/V#43").as_deref(), Some(b"VC".as_slice()));
        // Keys are decoded by `name`; values are skipped by `skip_value`.
        for (raw, at, incomplete, reason) in [
            (
                b"1 0 obj << /A#4".as_slice(),
                13,
                true,
                "escaped PDF name is truncated",
            ),
            (
                b"1 0 obj << /A#G1 1 >> endobj",
                14,
                false,
                "invalid PDF name escape",
            ),
            (
                b"1 0 obj << /A#4G 1 >> endobj",
                15,
                false,
                "invalid PDF name escape",
            ),
            (
                b"1 0 obj << /A /V#4",
                16,
                true,
                "escaped PDF name is truncated",
            ),
            (
                b"1 0 obj << /A /V#G1 >> endobj",
                17,
                false,
                "invalid PDF name escape",
            ),
            (
                b"1 0 obj << /A /V#4G >> endobj",
                18,
                false,
                "invalid PDF name escape",
            ),
        ] {
            let issue = head_issue(raw);
            assert_eq!(
                (issue.at, issue.incomplete, issue.reason),
                (at, incomplete, reason),
                "{raw:?}"
            );
        }
    }

    #[test]
    fn oversized_reference_components_are_not_remembered() {
        for raw in [
            b"1 0 obj [4294967296 0 R] endobj".as_slice(),
            b"1 0 obj [1 65536 R] endobj",
        ] {
            let issue = head_issue(raw);
            assert_eq!(issue.reason, "invalid PDF value token", "{raw:?}");
            assert_eq!(raw[issue.at], b'R');
        }
        let head = parse_object_head(b"1 0 obj [4294967295 65535 R] endobj".to_vec()).unwrap();
        assert_eq!(
            head.references,
            [PdfRef {
                number: u32::MAX,
                generation: u16::MAX
            }]
        );
    }

    #[test]
    fn string_nesting_is_bounded() {
        let nesting = MAX_SYNTAX_DEPTH as usize;
        let nested = |depth: usize| {
            let mut object = b"1 0 obj ".to_vec();
            object.extend(std::iter::repeat_n(b'(', depth));
            object.extend(std::iter::repeat_n(b')', depth));
            object.extend_from_slice(b" endobj");
            object
        };
        assert!(parse_object_head(nested(nesting)).is_ok());

        let issue = head_issue(&nested(nesting + 1));
        assert_eq!(
            issue.limit,
            Some((
                "PDF string nesting",
                MAX_SYNTAX_DEPTH as u64,
                MAX_SYNTAX_DEPTH as u64 + 1
            ))
        );
        assert_eq!(issue.at, 8 + nesting + 1);
    }

    #[test]
    fn dictionary_entry_point_rejects_depth_truncation_and_wrong_tokens() {
        let issue = Syntax::new(b"<< >>")
            .dictionary(MAX_SYNTAX_DEPTH + 1)
            .unwrap_err();
        assert_eq!(
            issue.limit,
            Some((
                "PDF dictionary depth",
                MAX_SYNTAX_DEPTH as u64,
                MAX_SYNTAX_DEPTH as u64 + 1
            ))
        );

        let short = Syntax::new(b"<").dictionary(0).unwrap_err();
        assert!(short.incomplete);
        assert_eq!(short.reason, "PDF dictionary is truncated");

        let array = Syntax::new(b"[1]").dictionary(0).unwrap_err();
        assert!(!array.incomplete);
        assert_eq!(array.reason, "expected PDF dictionary");
        assert_eq!(array.at, 0);

        let open = Syntax::new(b"<< /A 1").dictionary(0).unwrap_err();
        assert!(open.incomplete);
        assert_eq!(open.reason, "PDF dictionary is truncated");
        assert_eq!(open.at, 7);
    }

    #[test]
    fn dictionary_entry_count_is_bounded() {
        let mut bytes = b"<<".to_vec();
        for _ in 0..MAX_DICTIONARY_ENTRIES {
            bytes.extend_from_slice(b"/A 1 ");
        }
        let full = bytes.len();
        bytes.extend_from_slice(b"/B 2 >>");
        let issue = Syntax::new(&bytes).dictionary(0).unwrap_err();
        assert_eq!(
            issue.limit,
            Some((
                "PDF dictionary entries",
                MAX_DICTIONARY_ENTRIES as u64,
                MAX_DICTIONARY_ENTRIES as u64 + 1
            ))
        );
        assert_eq!(&bytes[issue.at..issue.at + 2], b"/B");

        // Without the extra entry the same dictionary is accepted.
        bytes.truncate(full);
        bytes.extend_from_slice(b">>");
        assert_eq!(
            Syntax::new(&bytes).dictionary(0).unwrap().len(),
            MAX_DICTIONARY_ENTRIES
        );
    }

    #[test]
    fn references_per_object_are_bounded() {
        let mut bytes = b"1 0 obj [".to_vec();
        for _ in 0..MAX_REFERENCES_PER_OBJECT {
            bytes.extend_from_slice(b"2 0 R ");
        }
        let full = bytes.len();
        bytes.extend_from_slice(b"3 0 R] endobj");
        let issue = head_issue(&bytes);
        assert_eq!(
            issue.limit,
            Some((
                "PDF object references",
                MAX_REFERENCES_PER_OBJECT as u64,
                MAX_REFERENCES_PER_OBJECT as u64 + 1
            ))
        );
        assert_eq!(issue.reason, "PDF syntax resource limit exceeded");

        // Without the extra reference the same object is accepted.
        bytes.truncate(full);
        bytes.extend_from_slice(b"] endobj");
        let head = parse_object_head(bytes).unwrap();
        assert_eq!(head.references.len(), MAX_REFERENCES_PER_OBJECT);
        assert_eq!(head.max_reference, 2);
    }

    #[test]
    fn value_helpers_reject_extra_items_and_unusable_numbers() {
        assert_eq!(unsigned_array(b"[1 +2 3]", 3), Some(vec![1, 2, 3]));
        assert_eq!(unsigned_array(b"[1 2 3]", 2), None);
        assert_eq!(unsigned_array(b"[1 -2]", 2), None);
        assert_eq!(destination_page(b"[7 0 R / 0 0 0]"), None);
        let overflowing = format!("[0 0 1{} 1]", "0".repeat(400));
        assert_eq!(media_box(overflowing.as_bytes()), None);
        assert_eq!(media_box(b"[-1.5 +0 .5 2.]"), Some([-1.5, 0.0, 0.5, 2.0]));
        assert!(valid_text_string(b"<feff00e9>"));
        assert!(valid_text_string(b"(plain)"));
        assert!(!valid_text_string(b"<< /Not /Text >>"));
        assert!(!valid_text_string(b"<fg>"));
    }
}
