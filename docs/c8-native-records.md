<!-- SPDX-License-Identifier: MIT -->

# Observed C8 native records

Implementation tracking: #232 (parser), #233 (rendering), parent #229.
This note records source observations, not an accepted rendering profile.
The production converter must keep rejecting required unknown semantics.

## Six-page inventory

The issue-66 source has SHA-256
`90e7b47716c32ef7a67cde8094f312e0ee7f1a7e0ed50a25830e8ba64a84f6a6`.
Each indexed text span ends with a four-byte `8004` record. There are no
remaining bytes inside those spans after that record. The file still has
1,449 bytes after the last indexed text span; they are the application-info
block classified below.

| Page | Text offset | Text bytes | Images | Drawing starts `8006` | Additional unresolved high words |
| --- | ---: | ---: | ---: | ---: | --- |
| 1 | 200 | 4,756 | 1 | 2 | `8072` |
| 2 | 7,421 | 7,944 | 0 | 73 | `8072`, `8073`, `8074` |
| 3 | 15,365 | 3,924 | 2 | 6 | `8072`, `8073`, `8074` |
| 4 | 21,969 | 5,336 | 0 | 2 | `c054`, `8072`, `8073`, `8074` |
| 5 | 27,305 | 4,584 | 0 | 25 | `c054`, `8072`, `8073`, `8074` |
| 6 | 31,889 | 7,536 | 0 | 1 | `8010`, `c053`, `8072`, `8073`, `8074` |

These are aligned-word observations with the three measured 28-byte image
records excluded from the four-byte survey. They do not prove all record
boundaries: drawing points and unknown payloads must not be counted as glyphs.
Each page also contains `8001`, `8002`, `801d`, and `8067` words.

The measured `8006/a381` and `8006/a38b` sequences have two coordinate pairs
followed by `ffff/0005`. The `8006/a383` sequence instead has two pairs followed
immediately by a state-setting record; a parser must not require or scan for a
universal `ffff` drawing terminator. Page 6 contains an `8010/0001` sequence
with two pairs and `ffff/0005`. Style, stroke and drawing semantics remain
unverified. The `c053`/`c054` words occur among text-like records; treating every
high word as a drawing opcode or every following pair as a glyph is unsafe.

## Additional controlled observations

Four source copies each change one little-endian 16-bit word. A whole-file
comparison verifies that no byte outside that word differs. All four control
captures and both source navigation states repeat identically in their own
state, using the pinned offline CAJViewer 9.0.0 at 96 DPI and 100% zoom.

| Word offset | Mutation | Observed effect on page 2 |
| ---: | --- | --- |
| 28 | 4652 → 4752 | Content shifts left approximately 13 pixels; page frame remains fixed. |
| 30 | 4274 → 4374 | Content shifts up approximately 13 pixels; page frame remains fixed. |
| 7,451 | `a0c4` → `a0da` | The selected Latin D becomes Z. This tests the selected pair, not all A0 codes. |
| 7,465 | 5335 → 5435 | The first short horizontal drawing changes, supporting a drawing-point interpretation. Stroke semantics are unresolved. |

Control file SHA-256 values, in the same order:

- `052813daadb19321e67c085d35667b0695722b6456f8f4725702430b94f3e5ee`
- `0f1f3821ff94e93f1fd3c3b15f2ba9f4ace3ca84cd3a5fa973c5205c3cc48469`
- `69a46ec552544bba86a8e168e190ba610705dfed4c05783b12cce0af159b858b`
- `8ca5ef6bdb6b1ac7f6a11bb20e627b28c808d6c7eb2a5e152f906efcc29df50f`

These observations support subtractive origin words and selected native field
interpretations. They do not establish physical units or exact PDF transforms.
Returning to the source tab and navigating again changed 23,719 pixels within
the unchanged 661×967 page crop. Some control differences also extend beyond
the intentionally changed glyph/segment. Retain both navigation phases;
these are field-semantics controls, not an approved pixel-parity baseline.
No alignment, resizing or tolerance was used to hide those differences.

External evidence directory: `caj2pdf-c8-fields-20261001`, including
`controls.json`, `integrity.json`, `aligned-word-inventory.json`, action logs,
original/repeated captures and both comparison phases. Source documents,
mutants, extracted text and captures are deliberately not committed.

## Implementation boundary

Reuse the existing bounded reader and character helpers. Establish explicit
record framing before exposing glyph/vector events. Track raw style words
without assigning font names or silently ignoring style changes. Verify the
remaining Latin map, image placement, header/footer controls, special text
words and font resources before admitting complete conversion. Unknown
required records remain located errors. Parser-only success does not complete
#229 or #233; all six pages must eventually preserve their visible content.

## Rust parser boundary

`Hnc8Reader::visit_native_records` visits the current page using existing
`TextBudget`, range reads and cursor poisoning. It retains one fixed 28-byte
record buffer and run state, allocates no parser-owned heap storage, and awaits
each visitor before reading the next record. Reads respect `Limits`; the largest
record-payload request is 24 bytes. This low-level API is deliberately raw:
`NativeRecord::Glyph::code` is not a Unicode scalar, and `Control` values are
never silently discarded. No CLI/JavaScript conversion route is enabled yet.
The renderer must resolve or reject unknown style/character semantics.

The admitted framing is `8001`, `8002`, observed `801d` values 0/4, observed
`8067` values 5/6/8/9, the three measured `8006` forms, the 28-byte `800a/d300`
image record, and an exact-end `8004`. Drawings and images are atomic visitor
events, including marker-looking payload bytes. Unknown tag/value pairs stop.
The current page's declared image count must agree before end-of-page success.

A native file-backed probe of all six source pages gives the following
**incomplete prefixes**, not successful extraction or conversion:

| Page | Visited records | Raw glyph records | Drawings | Images | First unsupported byte/tag |
| --- | ---: | ---: | ---: | ---: | --- |
| 1 | 1,064 | 929 | 1 | 1 | 4,492 / `8072` |
| 2 | 1,729 | 1,317 | 72 | 0 | 15,201 / `8072` |
| 3 | 909 | 810 | 5 | 2 | 19,109 / `8072` |
| 4 | 1,201 | 890 | 0 | 0 | 26,773 / `c054` |
| 5 | 167 | 130 | 4 | 0 | 28,021 / `c054` |
| 6 | 902 | 793 | 0 | 0 | 35,497 / `8010` |

The source hash was rechecked after the probe. The external harness and report
live in `caj2pdf-c8-native-probe`; neither extracted glyph codes nor source text
are included in the report. Original synthetic tests cover state changes,
asymmetric glyph order, marker-like image/drawing payloads, both drawing end
forms, every record truncation boundary, span/count/working limits, unknown
controls, short reads, source/visitor errors, cancellation and an abandoned
suspended visitor. Raw non-ASCII, Latin and invalid codes are preserved, not
misrepresented as decoded characters. The character helper below now covers the verified alphanumeric/GB18030 subset.
The still-required special characters, controls and rendering remain #229/#233.

## Character mapping and ordinary-copy controls

`decode_native_character` reuses the existing factual GB18030 table without
allocating a String per glyph. It returns the standard two-byte character for
ordinary codes, and ASCII letters/digits for the verified A0-prefixed codes.
The 62 alphanumeric codes are `a0b0..a0b9`, `a0c1..a0da`, and `a0e1..a0fa`.
Other A0 codes, private-use mappings and invalid sequences return `None`.
A visitor that requires complete text can reject `None`; the reader retains
the failing source-record offset and poisons the cursor. Raw glyph events still
preserve the original code. The helper is not a full-page text extractor.

Evidence was acquired with the same pinned offline viewer, at 100% zoom and
96 DPI, using the visibly labeled ordinary Copy menu, **not enhanced copy or
OCR**. A fresh distinct clipboard sentinel was installed before each copy.
The page-2 selection rectangle was `(548,218)` to `(1137,1060)`; it excludes
the final body paragraphs and page furniture. Original bytes remain external.

- Source selection: 2,459 UTF-8 bytes / 1,523 code points, SHA-256
  `b5bd244865f329c798b5dd92a371c16f1171832e45d7865eb721d586e87ea240`.
- Original uppercase/lowercase/digit alphabet control: 62 non-space glyph-code
  words replaced with the corresponding A0 codes; all other bytes unchanged.
  Control SHA-256 `a214562e8474b6e7579c67b8c3efa0e2cc1dc0be2fa97c49c5d84ec6313a4549`.
- Control ordinary copy: 2,417 bytes / 1,542 code points,
  SHA-256 `7def702a95f42df4291c190a344bcd8804e478bac83254786af677864d2b3137`. Its first 62 non-whitespace
  characters exactly equal `ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789`.
  Whitespace removal was used only to align this invented alphabet; retained
  raw clipboard bytes were not normalized or substituted as a text baseline.

The unchanged source selection independently contains 40 distinct A0 letter
codes that agree with that rule. Ordinary copy is not a universal Unicode
oracle: it changes some fullwidth digits to ASCII, produces U+0082 for the
standard GB18030 fullwidth comma code, and maps special A0 ampersand/AA symbol
codes differently from an ordinary GB18030 decode. The helper does not copy
those clipboard transformations or guess unknown special-code semantics.
Those exceptions remain explicit prerequisites for complete text rendering.
Source record order has only been compared for this selected body region;
full-document reading order is not established.

The external evidence directory `caj2pdf-c8-copy-20261001` retains the raw
clipboard payloads, visible menu/selection captures, mutation manifest, full-file
integrity checks and comparison report. Only format facts and hashes are
recorded here. No extracted document text, source document or font is committed.

## Style-word follow-up (not yet a font contract)

Four more original controls change only the style word at source offset 210,
from `0884` to `0885`, `08a4`, `0c84`, or `1084`. Each source/control capture
repeats identically with the same page frame. The first two changes affect only
the selected five-glyph run (589 and 466 changed pixels, respectively); the
other two produce zero changed page pixels. In the first control, the low-field
increment visibly increases glyph height. The control file labels `width` and
`height` were initial hypotheses, not established field names.

All observed source style words have equal low five-bit and next five-bit
fields. The asymmetric controls support investigating independent glyph-size
fields, but do not establish size units, font selection or an accepted style
bit layout. Zero visible difference does not authorize ignoring high bits.
Runtime diagnostic font requests include Fangzheng/CNKI names; the viewer has
its own font resources. Those log observations do not identify the effective
font for each source style or license any font for redistribution. Keep style
words raw until the rendering work validates a reproducible font contract.

## Application-info tail and source coverage

The remaining interval at 39,425 begins with two little-endian u32 lengths:
10,400 decoded bytes and 1,424 compressed bytes. A bounded zlib decode consumes
exactly those 1,424 bytes, produces exactly 10,400 bytes, and leaves the 17-byte
ASCII trailer `APPINFOSIGN 39425`. The compressed/decoded SHA-256 values are:

- `33de306020c1aa748b10094d04ebb6928b4057a99b2dae2dd285387884981f17`
- `fd2920fa45820856482b89577239587e2a9a5c4ee60339fc4f9c4e11bbecd4b5`

The decoded XML is an application `Package` with a `Note-Package` and
`FileProperty-Package`. There are 23 Link entries, each containing one rectangle
and one UrlLink. Their page counts are 13 on page 1, one each on pages 2–4, and
seven on page 6. This explains the 13 entries in the viewer's page-1 annotation
panel; it is not a table of contents or embedded font resource. No URL was
followed. XML and link/text contents remain external.

An independent interval check accounts for every source byte without gaps or
overlaps: the 200-byte header/index, six indexed text spans, three image
descriptors/payloads, and this application-info block total 40,874 bytes.
No separate font resource interval is observed. Font identifiers within text
records and the effective viewer font selection are still unresolved; this
accounting does not authorize a guessed font or omission of source controls.
It also does not infer bookmark absence for every C8 variant (#221).

The external `application-info-summary.json`, `application-info-structure.json`
and `source-span-coverage.json` retain the bounded decode and interval checks.
The ordinary-copy/runtime smoke evidence supplements the frozen #223 report;
it does not retroactively make that report's intentionally limited prototype a
complete extractor.

### HN-B native image framing (issue #250)

`tools/cajviewer/hnb_image_fixture.py` generates 13 original one-page controls
outside the repository. The pinned offline CAJViewer accepts the 20-byte index,
28-byte `800a/d300` image record and chained type-2 descriptors. Changing x, y,
width or header origins independently changes the displayed image as expected;
two images consume consecutive descriptors. Text following the 28-byte record
remains visible. The visitor preserves all 13 raw words, source order and exact
image counts; this is framing support, not a new public conversion profile.
The compact 12-byte index still rejects nonzero third words.

The generated inputs reproduce the independently captured controls byte-for-byte.
External receipts are in `caj2pdf-hnb-image-controls-20261001`; every selected page
crop repeats identically at 971% zoom. No source documents or captures are bundled.

Mixed-page rendering is unresolved: a glyph followed by a green JPEG changes
`(68,68,68)` to `(0,4,0)`, consistent with bitwise AND against decoded `(1,180,0)`
and inconsistent with Multiply. However image A → glyph → image B renders B
opaquely, while glyph → A → B retains cumulative AND in the overlap. These
controls rule out a universal image blend. They do not yet establish the state
that selects the operation; do not implement a guessed global blend or claim
complete HN-B conversion from raw record admission.
