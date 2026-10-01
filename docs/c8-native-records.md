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
1,449 bytes after the last indexed text span; their purpose is unverified.

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
misrepresented as decoded characters. Unicode decoding acceptance in #232
remains open, together with the still-required controls under #229/#233.
