<!-- SPDX-License-Identifier: MIT -->

# Bounded JBIG2 segment-header reader

This note records the first, header-only slice of [issue #40](https://github.com/rwv/caj2pdf-rust/issues/40),
under [issue #9](https://github.com/rwv/caj2pdf-rust/issues/9). The source of
standard format facts is the English [ITU-T Recommendation T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en),
§§5.4.2, 7.1–7.3, and Annex D. The original code and synthetic test bytes
were authored for this MIT repository. No official example bytes, arithmetic
state table, conformance vector, external decoder code, or CAJSamples document
are included. Later amendments and the 2018 edition require separate review
before making claims about them.

## Input and result

`jbig2::read_segment_header` receives a `RangedSource` and an exact
`SegmentSpan` containing **one contiguous header followed immediately by its
declared data**. It reads only header bytes and returns the validated segment
number, type code, deferred-retention flag, reference numbers and retention
bits, page association, header length, and absolute data offset/length. A
declared data range shorter or longer than the supplied span is an error. The
data bytes are not read or decoded. The offsets refer to the supplied source;
an enclosing format must map them to original document coordinates if it
provides a logical or spooled source.

The reader handles the §7.2 short and long reference-count forms, their
retention bits, reference-number widths of one, two, or four bytes according
to the current segment number, and one- or four-byte page associations. It
recognizes the §7.3 segment type codes as **header metadata only**. It checks
reference numbers are lower than the current number, single-header reference
count rules, and page association rules that can be decided without other
headers. The segment number zero and global page association zero remain
valid where the standard permits them. Reserved count forms and segment types
are unsupported; a noncanonical long count is malformed. An unknown data
length is unsupported, including the immediate generic region form that T.88
permits. Errors identify the source offset, segment number when read, and the
malformed field, unsupported feature, or exhausted limit.

`HeaderLimits` defaults to 64 KiB of header bytes, 4,096 references, and
64 MiB of declared segment data. The shared `Limits` also caps the selected
input range, I/O request size, and **combined** retained-reference metadata
allocation. Header sizes and data boundaries are checked before allocating
or reading the corresponding fields. Reads tolerate short progress, respect
the I/O chunk cap, and check cancellation after each source call. Work is
bounded by the configured header and reference limits; retained memory is
`O(reference count)` and no image or document buffer is allocated.

## Deliberate boundary

This single-header API does not parse a standalone JBIG2 file header,
discover segment boundaries, support unknown-length termination, validate
cross-segment references, or decode segment data. The companion
[embedded-directory reader](jbig2-directory.md) indexes an exact contiguous
span and checks cross-segment header rules. T.88 Annex D allows standalone
random-access files whose headers and data are stored separately; those
files cannot be passed as one contiguous segment span without a format-aware
adapter. HN/C8 record framing and page composition remain separate.

The optional [directory inventory](jbig2-directory.md#optional-external-metadata-inventory)
checks the pinned HN/C8 type-3 segment spans through the Rust directory API
when the external corpus is supplied. A clean clone reports **NOT_RUN**.
Neither this header API nor the directory inventory runs a decoded-pixel
oracle, so neither makes an HN/C8 pixel-parity claim. Issue #9 remains open.
Arithmetic decoding would also need a separate provenance decision before any exact T.88 Annex E
numeric-state table enters MIT source or released artifacts; the existing
[#30 decision](https://github.com/rwv/caj2pdf-rust/issues/30) concerns T.82
Table 24 instead.
