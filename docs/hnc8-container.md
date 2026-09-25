# Bounded HN/C8 container records

This note supports [issue #61](https://github.com/rwv/caj2pdf-rust/issues/61).
The reader is an original MIT implementation of the *measured profile* in
the [#22 HN/C8 oracle note](jbig1-oracle.md), not a complete specification of
HN or C8. The 27 external files are pinned by size and SHA-256 in the
[corpus matrix](../tests/conformance/matrix.json); no document bytes are
committed here. The public HN wiki is incomplete and includes decompiled
material; neither it nor the Python, Go, private Rust, or GLWT sources supplied
code or pseudocode for this reader.

## Observed byte layout

All multibyte fields listed below are little endian. Every offset in a page
or image record is **absolute from the start of the source**, not relative to
a DIB or preceding record. Signed fields are rejected when negative.

| Variant | Signature and marker | Page count | Page-index start |
| --- | --- | --- | --- |
| C8 | `c8 00 00 00` at `0x00` | positive i32 at `0x08` | `0x50` |
| HN A | `48 4e 00 00` at `0x00`, `90 01 00 00` at `0x04` | positive i32 at `0x90` | `0x15c + 308 × nonnegative i32 at 0x158` |
| HN B | `48 4e 00 00` at `0x00`, `c8 00 00 00` at `0x04` | positive i32 at `0x90` | `0xd8` |

The value at `0x158` behaves as a count of 308-byte outline-like records
in the measured HN A layout. Their contents and full semantics are unknown.
The reader checks its count and span but does not interpret these records.
Other header bytes and the meanings of the HN marker values remain unknown.
Zero-page files and additional markers are outside this measured profile.

The page index contains one 20-byte row per page, in one-based page order:

| Row offset | Type | Reader treatment |
| --- | --- | --- |
| `+0` | signed i32 | Absolute text offset, checked |
| `+4` | signed i32 | Text length, checked |
| `+8` | signed i16 | Declared image count, checked |
| `+10` | 2 raw bytes | Unknown; retained without semantic validation |
| `+12` | 4 raw bytes | Unknown; retained without semantic validation |
| `+16` | 4 raw bytes | Unknown; retained without semantic validation |

For a page with images, the first 12-byte descriptor is at checked
`text_offset + text_length`. Its fields are signed i32 type at `+0`,
signed i32 absolute image offset at `+4`, and signed i32 image length at
`+8`. The **next descriptor is at the end of the preceding image payload**.
The payload may begin after a gap following its descriptor; the gap and
trailing bytes have unknown meaning. A payload cannot overlap its descriptor.
Only the descriptor's signed `+0` field is the image-type discriminator.
A common byte prefix inside an encoded payload is not one.

The measured types are 0, 1, 2, and 3. All four are enumerated as metadata.
Only type 0 may be passed as an unchanged full DIB-plus-coded
`Type0Span { record_type: 0, offset, length }` to the row API from #55.
Types 1–3 have no codec assignment or conversion claim here. A positive
unmeasured type produces a located unsupported error; a negative type is
malformed. This reader does not decode images or text, determine placement,
construct PDF pages, or provide a complete HN/C8 conversion command. The
separate [type-0 PDF note](hnc8-type0-pdf.md) describes the #28 core
converter built on it, which still needs a caller-supplied table.

## Bounds, work, and alias policy

`Hnc8Reader::open` uses the platform-neutral `RangedSource`, `Limits`,
and `Cancellation` contracts. The whole source size must fit
`Limits::max_input_bytes` before the first read. `max_pages` bounds the
signed page count. The HN A count uses the named
`Budget::max_outline_records` rather than `max_bookmarks`, because the
record semantics are not yet established. The default format budget is:

| Resource | Default ceiling | Scope |
| --- | ---: | --- |
| HN A outline-like records | 100,000 | Header |
| Images per page | 8,192 | Each page-index row |
| Images total | 1,000,000 | A normal cursor beginning at page 1 |
| Text span | 64 MiB | Each page |
| Image payload span | 64 MiB | Each descriptor |

These are implementation limits, not format maxima. The largest measured
valid source had 163 pages; observed maxima across valid sources were 111
outline-like records, 49 images on one page, 513 images in one document,
37,484 text bytes, and 2,015,864 image payload bytes. The malformed
`issue-100` source declares 6,378 images on page 2.
`issue-100` page 2's first descriptor has an unmeasured positive type;
the Rust reader refuses that type before assigning semantics to its remaining
bytes. The #22 black-box inventory independently identified an out-of-source
image span in that descriptor. The optional report keeps those two
observations distinct.
`Type0Decoder` additionally applies its own image, arithmetic, and row
budgets; the metadata budget does not replace those checks.

All header, index, text, descriptor, and payload intervals use checked
addition/multiplication and source-size containment before seeking.
Descriptors and payloads may not overlap the protected
`[0, page_index_end)` metadata region. Their chain advances only from a
checked nonzero payload end, so a later image cannot regress into an earlier
descriptor or payload. Text is only range-checked: its encoding and ownership
are unknown, and the malformed `issue-100` source has text ranges touching
metadata. Cross-page text, descriptor, and payload aliases or out-of-order
intervals are accepted **as separate record occurrences**, each checked
again for range, type, and identity. No global monotonicity rule is inferred.
An independent #61 read-only inventory found no such overlap among the 26
structurally valid pinned sources; this is evidence for those files only,
not a general HN/C8 invariant.

The reader retains one fixed header/page/descriptor buffer plus constant
cursor state; memory does not grow with page count or source size. Fixed
reads are split to `io_chunk_bytes`, including when that limit is one byte.
Short reads are retried, while zero progress, overreporting, cancellation,
and truncation become located errors. A failed or dropped **in-flight** read
poisons the normal cursor; cancellation detected before I/O leaves it
retryable. `next_page` refuses to skip declared unread images. A
malformed image stops that traversal, so following bytes cannot be
reattached to another page. A forward-only caller must provide a temporary
seekable spool before calling this ranged reader, or receive an explicit
random-access-required error from its adapter. Native/JS forward-only
conversion wiring belongs to #28/#10; this issue adds no spool adapter.

For diagnostics only, `probe_at_page` opens a **fresh** cursor at one
selected row. It intentionally skips earlier pages; its total-image budget
therefore covers the selected suffix, not the whole document. Conversion
must use `open` from page 1. The optional corpus tool probes each page
independently to report the three known `issue-100` failures while the
normal cursor fails at its first malformed image.

## Verification boundary

Original synthetic Rust tests cover the three variants, record chaining,
limits, structural failures, tiny reads, cancellation, fixed-seed mutations,
and a type-0 row handoff with an invented MIT arithmetic table. The
[optional runner](../tests/conformance/hnc8_container_compare.py) compares
only metadata from the Rust reader with the independently recorded #22
manifest. On the pinned external corpus it checks all 27 source hashes
before and after, 1,400 type-0 coordinates and spans, inventories types
1–3, and reports the three expected-invalid `issue-100` observations
separately. Clean CI reports `NOT_RUN` and **zero compatibility passes**.
No private T.82 state table, document, bitmap, PDF, or oracle binary is
included.
