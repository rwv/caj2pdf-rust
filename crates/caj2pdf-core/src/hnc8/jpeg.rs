// SPDX-License-Identifier: MIT

//! Strict, bounded marker traversal for one checked HN/C8 type-1/type-2 JPEG span.
//! This is a structural profile reader, not a JPEG entropy decoder or a PDF
//! color-management decision.

use super::{ErrorKind, ImageRecord, Location, Result, Span};
use crate::{Cancellation, Error, Limits, RangedSource, read_exact_at};

const BUFFER_BYTES: usize = 4096;

/// Color interpretation established by an explicit interchange marker.
/// Three-component JPEG without such a marker is rejected as unsupported.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JpegColor {
    Gray,
    Ycbcr,
}

/// Checked properties of one supported 8-bit baseline, single-scan JPEG.
/// Entropy symbols and decoded pixels have not been validated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JpegInfo {
    pub payload: Span,
    pub width: u16,
    pub height: u16,
    pub precision: u8,
    pub components: u8,
    pub color: JpegColor,
    pub app0_jfif: bool,
    pub restart_interval: Option<u16>,
    pub scans: u32,
}

struct Cursor<'a, S: RangedSource, C: Cancellation> {
    source: &'a mut S,
    limits: &'a Limits,
    cancellation: &'a C,
    location: Location,
    end: u64,
    position: u64,
    buffer: [u8; BUFFER_BYTES],
    buffered: usize,
    used: usize,
}

impl<S: RangedSource, C: Cancellation> Cursor<'_, S, C> {
    fn at(&self, offset: u64) -> Location {
        self.location.at(offset)
    }

    fn fill(&mut self, field: &'static str) -> Result<()> {
        if self.position == self.end {
            return Err(self.at(self.position).error(ErrorKind::Truncated {
                field,
                expected: 1,
                available: 0,
            }));
        }
        let count = (self.end - self.position)
            .min(BUFFER_BYTES as u64)
            .min(self.limits.io_chunk_bytes as u64) as usize;
        let offset = self.position;
        read_exact_at(
            self.source,
            offset,
            &mut self.buffer[..count],
            self.limits,
            self.cancellation,
        )
        .map_err(|error| match error {
            Error::Cancelled => self.at(offset).error(ErrorKind::Cancelled),
            Error::TruncatedInput { available, .. } => self
                .at(offset.saturating_add(available))
                .error(ErrorKind::Truncated {
                    field,
                    expected: count as u64,
                    available,
                }),
            source => self.at(offset).error(ErrorKind::Source { field, source }),
        })?;
        self.buffered = count;
        self.used = 0;
        Ok(())
    }

    fn byte(&mut self, field: &'static str) -> Result<u8> {
        if self.used == self.buffered {
            self.fill(field)?;
        }
        let value = self.buffer[self.used];
        self.used += 1;
        self.position += 1;
        Ok(value)
    }

    fn skip(&mut self, length: u64, field: &'static str) -> Result<()> {
        // Every caller derives `length` from a checked segment end or calls
        // `require_remaining` first.
        debug_assert!(length <= self.end - self.position);
        let target = self.position + length;
        while self.position < target {
            if self.used == self.buffered {
                self.fill(field)?;
            }
            let count = (target - self.position).min((self.buffered - self.used) as u64) as usize;
            self.used += count;
            self.position += count as u64;
        }
        Ok(())
    }

    fn marker(&mut self, field: &'static str) -> Result<(u64, u8)> {
        let offset = self.position;
        if self.byte(field)? != 0xff {
            return Err(self.at(offset).malformed(field, "expected marker prefix"));
        }
        let code = loop {
            let code = self.byte(field)?;
            if code != 0xff {
                break code;
            }
        };
        if code == 0 {
            return Err(self
                .at(offset)
                .malformed(field, "stuffed byte outside entropy data"));
        }
        Ok((offset, code))
    }

    fn segment_end(&mut self, field: &'static str) -> Result<u64> {
        let offset = self.position;
        let high = self.byte(field)?;
        let low = self.byte(field)?;
        let length = u16::from_be_bytes([high, low]);
        if length < 2 {
            return Err(self
                .at(offset)
                .malformed(field, "segment length is below two"));
        }
        let content = u64::from(length - 2);
        let available = self.end - self.position;
        if content > available {
            return Err(self.at(self.position).error(ErrorKind::Truncated {
                field,
                expected: content,
                available,
            }));
        }
        Ok(self.position + content)
    }

    fn require_remaining(&self, end: u64, count: u64, field: &'static str) -> Result<()> {
        let available = end - self.position;
        if count > available {
            return Err(self.at(self.position).error(ErrorKind::Truncated {
                field,
                expected: count,
                available,
            }));
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct Frame {
    marker_offset: u64,
    width: u16,
    height: u16,
    precision: u8,
    components: u8,
    ids: [u8; 3],
    quant_tables: [u8; 3],
}

struct Parser<'a, S: RangedSource, C: Cancellation> {
    cursor: Cursor<'a, S, C>,
    /// Markers read so far; each takes at least two payload bytes.
    markers: u32,
    frame: Option<Frame>,
    quant_mask: u8,
    dc_mask: u8,
    ac_mask: u8,
    app0_jfif: bool,
    restart_interval: Option<u16>,
}

impl<S: RangedSource, C: Cancellation> Parser<'_, S, C> {
    fn next_marker(&mut self) -> Result<(u64, u8)> {
        let (offset, code) = self.cursor.marker("JPEG marker")?;
        self.markers = self.markers.saturating_add(1);
        Ok((offset, code))
    }

    fn app(&mut self, marker: u8, marker_offset: u64) -> Result<()> {
        let end = self.cursor.segment_end("JPEG APP segment")?;
        if marker == 0xee {
            return Err(self.cursor.at(marker_offset).error(ErrorKind::Unsupported {
                field: "JPEG APP14 marker",
                value: u64::from(marker),
            }));
        }
        let remaining = end - self.cursor.position;
        if marker == 0xe0 && remaining >= 5 {
            let mut name = [0; 5];
            for value in &mut name {
                *value = self.cursor.byte("JPEG APP0 identifier")?;
            }
            if &name == b"JFIF\0" {
                if self.app0_jfif {
                    return Err(self
                        .cursor
                        .at(marker_offset)
                        .malformed("JFIF APP0", "duplicate JFIF marker"));
                }
                if self.markers != 2 {
                    return Err(self.cursor.at(marker_offset).error(ErrorKind::Unsupported {
                        field: "JFIF APP0 placement",
                        value: u64::from(self.markers),
                    }));
                }
                self.cursor.require_remaining(end, 9, "JFIF APP0 fields")?;
                let fields_offset = self.cursor.position;
                let mut fields = [0; 9];
                for value in &mut fields {
                    *value = self.cursor.byte("JFIF APP0 fields")?;
                }
                if fields[0] != 1 || fields[1] > 2 || fields[2] > 2 {
                    return Err(self.cursor.at(fields_offset).error(ErrorKind::Unsupported {
                        field: "JFIF version or density unit",
                        value: u64::from_be_bytes([0, 0, 0, 0, 0, fields[0], fields[1], fields[2]]),
                    }));
                }
                if fields[3..5] == [0, 0] || fields[5..7] == [0, 0] {
                    return Err(self
                        .cursor
                        .at(fields_offset + 3)
                        .malformed("JFIF density", "zero density"));
                }
                let thumbnail_bytes = u64::from(fields[7]) * u64::from(fields[8]) * 3;
                if end - self.cursor.position != thumbnail_bytes {
                    return Err(self
                        .cursor
                        .at(self.cursor.position)
                        .malformed("JFIF thumbnail", "length differs from dimensions"));
                }
                self.app0_jfif = true;
            }
        }
        self.cursor
            .skip(end - self.cursor.position, "JPEG APP segment")
    }

    fn dqt(&mut self) -> Result<()> {
        let end = self.cursor.segment_end("JPEG DQT")?;
        if self.cursor.position == end {
            return Err(self.cursor.at(end).malformed("JPEG DQT", "empty segment"));
        }
        while self.cursor.position < end {
            let at = self.cursor.position;
            let selector = self.cursor.byte("JPEG DQT selector")?;
            if selector >> 4 != 0 || selector & 15 > 3 {
                return Err(self.cursor.at(at).error(ErrorKind::Unsupported {
                    field: "JPEG DQT selector",
                    value: u64::from(selector),
                }));
            }
            self.cursor.require_remaining(end, 64, "JPEG DQT values")?;
            self.cursor.skip(64, "JPEG DQT values")?;
            self.quant_mask |= 1 << (selector & 15);
        }
        Ok(())
    }

    fn dht(&mut self) -> Result<()> {
        let end = self.cursor.segment_end("JPEG DHT")?;
        if self.cursor.position == end {
            return Err(self.cursor.at(end).malformed("JPEG DHT", "empty segment"));
        }
        while self.cursor.position < end {
            let at = self.cursor.position;
            let selector = self.cursor.byte("JPEG DHT selector")?;
            let class = selector >> 4;
            let table = selector & 15;
            if class > 1 || table > 3 {
                return Err(self.cursor.at(at).error(ErrorKind::Unsupported {
                    field: "JPEG DHT selector",
                    value: u64::from(selector),
                }));
            }
            self.cursor.require_remaining(end, 16, "JPEG DHT counts")?;
            let mut symbols = 0_u16;
            for _ in 0..16 {
                symbols += u16::from(self.cursor.byte("JPEG DHT counts")?);
            }
            if symbols > 256 {
                return Err(self
                    .cursor
                    .at(at)
                    .malformed("JPEG DHT", "more than 256 symbols"));
            }
            self.cursor
                .require_remaining(end, u64::from(symbols), "JPEG DHT symbols")?;
            self.cursor.skip(u64::from(symbols), "JPEG DHT symbols")?;
            if class == 0 {
                self.dc_mask |= 1 << table;
            } else {
                self.ac_mask |= 1 << table;
            }
        }
        Ok(())
    }

    fn dri(&mut self, marker_offset: u64) -> Result<()> {
        let end = self.cursor.segment_end("JPEG DRI")?;
        if end - self.cursor.position != 2 {
            return Err(self
                .cursor
                .at(marker_offset)
                .malformed("JPEG DRI", "expected two-byte restart interval"));
        }
        let high = self.cursor.byte("JPEG DRI")?;
        let low = self.cursor.byte("JPEG DRI")?;
        self.restart_interval = Some(u16::from_be_bytes([high, low]));
        Ok(())
    }

    fn sof0(&mut self, marker_offset: u64) -> Result<()> {
        if self.frame.is_some() {
            return Err(self
                .cursor
                .at(marker_offset)
                .malformed("JPEG frame", "duplicate frame header"));
        }
        let end = self.cursor.segment_end("JPEG SOF0")?;
        self.cursor.require_remaining(end, 6, "JPEG SOF0 fields")?;
        let precision_offset = self.cursor.position;
        let precision = self.cursor.byte("JPEG precision")?;
        let height = u16::from_be_bytes([
            self.cursor.byte("JPEG height")?,
            self.cursor.byte("JPEG height")?,
        ]);
        let width = u16::from_be_bytes([
            self.cursor.byte("JPEG width")?,
            self.cursor.byte("JPEG width")?,
        ]);
        let components_offset = self.cursor.position;
        let components = self.cursor.byte("JPEG components")?;
        if end - self.cursor.position != u64::from(components) * 3 {
            return Err(self.cursor.at(components_offset).malformed(
                "JPEG SOF0",
                "component field count differs from segment length",
            ));
        }
        if precision != 8 {
            return Err(self
                .cursor
                .at(precision_offset)
                .error(ErrorKind::Unsupported {
                    field: "JPEG precision",
                    value: u64::from(precision),
                }));
        }
        if height == 0 {
            return Err(self
                .cursor
                .at(precision_offset + 1)
                .error(ErrorKind::Unsupported {
                    field: "JPEG DNL height",
                    value: 0,
                }));
        }
        if width == 0 {
            return Err(self
                .cursor
                .at(precision_offset + 3)
                .malformed("JPEG width", "zero width"));
        }
        if components != 1 && components != 3 {
            return Err(self
                .cursor
                .at(components_offset)
                .error(ErrorKind::Unsupported {
                    field: "JPEG components",
                    value: u64::from(components),
                }));
        }
        let mut ids = [0; 3];
        let mut quant_tables = [0; 3];
        let mut blocks = 0_u16;
        for index in 0..usize::from(components) {
            let at = self.cursor.position;
            let id = self.cursor.byte("JPEG component ID")?;
            let sampling = self.cursor.byte("JPEG sampling")?;
            let quant = self.cursor.byte("JPEG quantization selector")?;
            if ids[..index].contains(&id) {
                return Err(self
                    .cursor
                    .at(at)
                    .malformed("JPEG component ID", "duplicate component ID"));
            }
            let horizontal = sampling >> 4;
            let vertical = sampling & 15;
            if !(1..=4).contains(&horizontal) || !(1..=4).contains(&vertical) {
                return Err(self
                    .cursor
                    .at(at + 1)
                    .malformed("JPEG sampling", "factor outside 1..=4"));
            }
            if quant > 3 {
                return Err(self.cursor.at(at + 2).error(ErrorKind::Unsupported {
                    field: "JPEG quantization selector",
                    value: u64::from(quant),
                }));
            }
            ids[index] = id;
            quant_tables[index] = quant;
            blocks += u16::from(horizontal) * u16::from(vertical);
        }
        if components > 1 && blocks > 10 {
            return Err(self
                .cursor
                .at(marker_offset)
                .malformed("JPEG sampling", "too many MCU blocks"));
        }
        self.frame = Some(Frame {
            marker_offset,
            width,
            height,
            precision,
            components,
            ids,
            quant_tables,
        });
        Ok(())
    }

    fn sos(&mut self, marker_offset: u64) -> Result<()> {
        let frame = self.frame.ok_or_else(|| {
            self.cursor
                .at(marker_offset)
                .malformed("JPEG SOS", "scan precedes frame")
        })?;
        let end = self.cursor.segment_end("JPEG SOS")?;
        self.cursor
            .require_remaining(end, 1, "JPEG SOS components")?;
        let components_offset = self.cursor.position;
        let components = self.cursor.byte("JPEG SOS components")?;
        if end - self.cursor.position != u64::from(components) * 2 + 3 {
            return Err(self
                .cursor
                .at(components_offset)
                .malformed("JPEG SOS", "scan field count differs from segment length"));
        }
        if components != frame.components {
            return Err(self
                .cursor
                .at(components_offset)
                .error(ErrorKind::Unsupported {
                    field: "JPEG scan components",
                    value: u64::from(components),
                }));
        }
        for index in 0..usize::from(components) {
            let at = self.cursor.position;
            let id = self.cursor.byte("JPEG SOS component ID")?;
            let selectors = self.cursor.byte("JPEG Huffman selectors")?;
            if id != frame.ids[index] {
                return Err(self.cursor.at(at).malformed(
                    "JPEG SOS component ID",
                    "component order differs from frame",
                ));
            }
            let dc = selectors >> 4;
            let ac = selectors & 15;
            if dc > 1 || ac > 1 {
                return Err(self.cursor.at(at + 1).error(ErrorKind::Unsupported {
                    field: "JPEG Huffman selectors",
                    value: u64::from(selectors),
                }));
            }
            if self.quant_mask & (1 << frame.quant_tables[index]) == 0
                || self.dc_mask & (1 << dc) == 0
                || self.ac_mask & (1 << ac) == 0
            {
                return Err(self
                    .cursor
                    .at(at)
                    .malformed("JPEG tables", "scan references an undefined table"));
            }
        }
        let spectral_offset = self.cursor.position;
        let start = self.cursor.byte("JPEG spectral start")?;
        let end_spectral = self.cursor.byte("JPEG spectral end")?;
        let approximation = self.cursor.byte("JPEG approximation")?;
        if (start, end_spectral, approximation) != (0, 63, 0) {
            return Err(self
                .cursor
                .at(spectral_offset)
                .error(ErrorKind::Unsupported {
                    field: "JPEG scan parameters",
                    value: u64::from_be_bytes([0, 0, 0, 0, 0, start, end_spectral, approximation]),
                }));
        }
        Ok(())
    }

    fn entropy_end(&mut self) -> Result<(u64, u8)> {
        let mut expected_restart = 0_u8;
        loop {
            let at = self.cursor.position;
            let byte = self.cursor.byte("JPEG entropy data")?;
            if byte != 0xff {
                continue;
            }
            let mut fill = false;
            let code = loop {
                let code = self.cursor.byte("JPEG entropy marker")?;
                if code != 0xff {
                    break code;
                }
                fill = true;
            };
            if code == 0 {
                if fill {
                    return Err(self
                        .cursor
                        .at(at)
                        .malformed("JPEG entropy marker", "fill byte before stuffed zero"));
                }
                continue;
            }
            self.markers = self.markers.saturating_add(1);
            if (0xd0..=0xd7).contains(&code) {
                if self.restart_interval.unwrap_or(0) == 0 {
                    return Err(self
                        .cursor
                        .at(at)
                        .malformed("JPEG restart", "restart without active DRI"));
                }
                if code != 0xd0 + expected_restart {
                    return Err(self
                        .cursor
                        .at(at)
                        .malformed("JPEG restart", "restart sequence mismatch"));
                }
                expected_restart = (expected_restart + 1) & 7;
                continue;
            }
            return Ok((at, code));
        }
    }

    fn run(mut self, payload: Span) -> Result<JpegInfo> {
        let (soi_offset, soi) = self.next_marker()?;
        if soi != 0xd8 {
            return Err(self
                .cursor
                .at(soi_offset)
                .malformed("JPEG SOI", "expected start-of-image marker"));
        }
        loop {
            let (offset, marker) = self.next_marker()?;
            match marker {
                0xe0..=0xef => self.app(marker, offset)?,
                0xfe => {
                    let end = self.cursor.segment_end("JPEG COM")?;
                    self.cursor.skip(end - self.cursor.position, "JPEG COM")?;
                }
                0xdb => self.dqt()?,
                0xc4 => self.dht()?,
                0xdd => self.dri(offset)?,
                0xc0 => self.sof0(offset)?,
                0xda => {
                    self.sos(offset)?;
                    let (next_offset, next) = self.entropy_end()?;
                    if next == 0xd9 {
                        if self.cursor.position != self.cursor.end {
                            return Err(self
                                .cursor
                                .at(self.cursor.position)
                                .malformed("JPEG trailing bytes", "bytes follow EOI"));
                        }
                        let frame = self.frame.expect("SOS required frame");
                        let color = if frame.components == 1 {
                            JpegColor::Gray
                        } else if frame.ids[..3] == [1, 2, 3] && self.app0_jfif {
                            JpegColor::Ycbcr
                        } else {
                            return Err(self.cursor.at(frame.marker_offset).error(
                                ErrorKind::Unsupported {
                                    field: "JPEG color transform",
                                    value: 255,
                                },
                            ));
                        };
                        return Ok(JpegInfo {
                            payload,
                            width: frame.width,
                            height: frame.height,
                            precision: frame.precision,
                            components: frame.components,
                            color,
                            app0_jfif: self.app0_jfif,
                            restart_interval: self.restart_interval,
                            scans: 1,
                        });
                    }
                    if next == 0xdc {
                        return Err(self.cursor.at(next_offset).error(ErrorKind::Unsupported {
                            field: "JPEG DNL marker",
                            value: u64::from(next),
                        }));
                    }
                    if next == 0xda
                        || next == 0xc4
                        || next == 0xdb
                        || next == 0xdd
                        || next == 0xfe
                        || (0xe0..=0xef).contains(&next)
                    {
                        return Err(self.cursor.at(next_offset).error(ErrorKind::Unsupported {
                            field: "JPEG multiple scans",
                            value: u64::from(next),
                        }));
                    }
                    return Err(self
                        .cursor
                        .at(next_offset)
                        .malformed("JPEG entropy marker", "unexpected marker after scan"));
                }
                0xd9 => {
                    return Err(self
                        .cursor
                        .at(offset)
                        .malformed("JPEG EOI", "end-of-image precedes scan"));
                }
                0xdc => {
                    return Err(self.cursor.at(offset).error(ErrorKind::Unsupported {
                        field: "JPEG DNL marker",
                        value: u64::from(marker),
                    }));
                }
                0xcc | 0xde | 0xdf | 0xc1..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf => {
                    return Err(self.cursor.at(offset).error(ErrorKind::Unsupported {
                        field: "JPEG frame or arithmetic marker",
                        value: u64::from(marker),
                    }));
                }
                0xd0..=0xd8 => {
                    return Err(self
                        .cursor
                        .at(offset)
                        .malformed("JPEG marker order", "standalone marker outside scan"));
                }
                _ => {
                    return Err(self.cursor.at(offset).error(ErrorKind::Unsupported {
                        field: "JPEG marker",
                        value: u64::from(marker),
                    }));
                }
            }
        }
    }
}

/// Traverse one checked type-1 or type-2 JPEG descriptor without copying its complete JPEG.
/// A successful result proves only the documented marker/profile subset;
/// entropy code validity and PDF pixel parity require later checks. The
/// payload is bounded by `Limits::max_allocation_bytes`.
pub fn read_type2_jpeg_info<S: RangedSource, C: Cancellation>(
    source: &mut S,
    record: ImageRecord,
    limits: &Limits,
    cancellation: &C,
) -> Result<JpegInfo> {
    let location = Location {
        variant: None,
        offset: record.payload.offset,
        page: Some(record.page_number),
        image: Some(record.image_number),
    };
    if cancellation.is_cancelled() {
        return Err(location.error(ErrorKind::Cancelled));
    }
    if record.page_number == 0 || record.image_number == 0 {
        return Err(location
            .at(record.descriptor_offset)
            .malformed("image identity", "page and image numbers must be positive"));
    }
    if !matches!(record.record_type, 1 | 2) {
        return Err(location
            .at(record.descriptor_offset)
            .error(ErrorKind::Unsupported {
                field: "image type",
                value: u64::from(record.record_type),
            }));
    }
    let descriptor_end = record.descriptor_offset.checked_add(12).ok_or_else(|| {
        location
            .at(record.descriptor_offset)
            .malformed("image descriptor", "offset overflows u64")
    })?;
    if record.payload.offset < descriptor_end {
        return Err(location
            .at(record.descriptor_offset + 4)
            .malformed("image payload", "payload overlaps descriptor"));
    }
    if record.payload.length == 0 {
        return Err(location
            .at(record.descriptor_offset + 8)
            .malformed("image payload", "zero-length payload"));
    }
    let end = record.payload.checked_end().ok_or_else(|| {
        location
            .at(record.payload.offset)
            .malformed("image payload", "end overflows u64")
    })?;
    if source.size() > limits.max_input_bytes {
        return Err(location.limit("source bytes", limits.max_input_bytes, source.size()));
    }
    if end > source.size() {
        return Err(location.error(ErrorKind::Truncated {
            field: "image payload",
            expected: record.payload.length,
            available: source.size().saturating_sub(record.payload.offset),
        }));
    }
    if record.payload.length > limits.max_allocation_bytes {
        return Err(location.at(record.descriptor_offset + 8).limit(
            "JPEG payload bytes",
            limits.max_allocation_bytes,
            record.payload.length,
        ));
    }
    let cursor = Cursor {
        source,
        limits,
        cancellation,
        location,
        end,
        position: record.payload.offset,
        buffer: [0; BUFFER_BYTES],
        buffered: 0,
        used: 0,
    };
    Parser {
        cursor,
        markers: 0,
        frame: None,
        quant_mask: 0,
        dc_mask: 0,
        ac_mask: 0,
        app0_jfif: false,
        restart_interval: None,
    }
    .run(record.payload)
}
