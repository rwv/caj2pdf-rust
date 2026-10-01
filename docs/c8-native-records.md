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
`8067` values 5/6/8/9, raw controls `8072..8074` and `c053/c054`, the three
measured `8006` forms, the `8010/1` coordinate form, the 28-byte `800a/d300`
image record, and an exact-end `8004`. Drawings and images are atomic visitor
events, including marker-looking payload bytes. Unknown tag/value pairs stop.
The current page's declared image count must agree before end-of-page success.

The initial limited parser probe of all six source pages gave the following
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
The three explicitly verified symbols listed below are also mapped. Other A0
codes, private-use mappings and invalid sequences return `None`.
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
those comma/digit transformations or guess unknown special-code semantics.
The three symbol exceptions have since received separate original controls,
as described below; other unverified codes still fail explicitly.
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

## Original fixed-position style controls

The style subset of `tools/cajviewer/c8_style_fixture.py` builds nine original 392-byte documents
without reading an external source. Each has one text-only page, eight rows
and the test characters `中文AM1`. The observed header identifier and structural
constants are retained as format facts; their necessity is not established.
The grid varies the fields by row; the other documents repeat one field set
at identical positions, permitting direct comparisons without alignment.

```sh
python3 tools/cajviewer/c8_style_fixture.py /tmp/c8-style-controls
```

The output directory must be new. The manifest contains input hashes and
raw per-row values. Open the files with the existing pinned offline viewer
recipe, close the annotation sidebar and explicitly set `100%` zoom after
opening each tab. The initial fit-to-width zoom differs and is not a valid
comparison. All generated documents were byte-identical to the observed
inputs. The grid SHA-256 is
`5cdcadbd6559d0c50c21a16e1fa59eb0e729e383e9a0cf937bf6ec584d795d7c`.

At 96 DPI, the fixed page crop was `(494,178)` to `(1155,1145)`; each capture
repeated identically. Compared with `style=1084, 801d=0, 8067=6`:

| Changed field | Changed page pixels | Observation |
| --- | ---: | --- |
| style `1085` | 3,524 | Increased glyph height. |
| style `10a4` | 3,218 | Increased glyph width. |
| `801d=4` | 1,580 | Changes confined to the Latin/digit columns; Chinese columns unchanged. |
| `8067=5`, `8`, or `9` | 0 each | No visible difference for these glyphs and state. |
| style `0884` | 0 | No visible difference for these glyphs and state. |

There was no resizing, registration or pixel tolerance. These independent
controls corroborate the selected source mutations, but do not determine the
size lookup, font identities, baseline metrics or all state interactions.
In particular, `801d=4` is not established as a universal bold flag, and a
zero pixel difference is not permission to discard a control. Keep the raw
parser/rendering boundary until those required semantics are resolved.

External receipts are in `caj2pdf-c8-grid-20261001`: `variants.json`,
`comparison.json`, action logs and paired captures. Viewer images/fonts are
not bundled. Successful display of these original controls does not satisfy
the six-page source conversion requirement in #233.


## Remaining record framing and full-span traversal

The same original generator now also emits seven control variants and three
coordinate-record variants. Controls are inserted after each row's context
and before its glyphs: `8072/0`, `8073/38`, `8074/0`, `c053/5200`,
`c054/5200`, `c053/5700`, and `c054/5700`. Each is four bytes. All seven
preserve every following glyph and produce zero changed page pixels against
the baseline, with identical repeat captures. Combined with the aligned source
inventory, this establishes the admitted framing, not their rendering meaning.
The visitor now delivers these controls and their full u16 payload unchanged.

The coordinate variants insert, before each row, a 16-byte sequence consisting
of `8010/1`, `(5200, 4800 + row*500)`, `(6300, 4850 + row*500)`, and
`ffff/5`. A second control changes the first x to 5400. Neither produces a
visible line. Replacing only the opening pair with `8006/a381` produces eight
sloped lines (3,566 changed page pixels). All captures repeat identically.
Thus `8010/1` uses the observed two-pair/end framing but must **not** be
rendered automatically as an `8006` stroke. `NativeRecord::Drawing` preserves
its tag, style and points without promising a visible drawing. Other `8010`
values remain unsupported; terminator mismatch and truncation remain errors.

The extended file-backed parser now consumes each indexed source span exactly:

| Page | Records | Glyphs | Coordinate records | Images | Largest read request |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 | 1,178 | 1,015 | 2 | 1 | 24 |
| 2 | 1,768 | 1,338 | 73 | 0 | 20 |
| 3 | 952 | 840 | 6 | 2 | 24 |
| 4 | 1,329 | 986 | 2 | 0 | 20 |
| 5 | 1,072 | 801 | 25 | 0 | 20 |
| 6 | 1,879 | 1,658 | 2 | 0 | 20 |

The original source hash is unchanged. `caj2pdf-c8-full-record-probe/result.txt`
is the external receipt; control/drawing manifests and comparisons are in
`caj2pdf-c8-grid-20261001`. The generator reproduces all 19 viewer input files
byte for byte. Original Rust regressions check retained marker-looking control
payloads, atomic coordinate payloads, short reads and all new truncation
boundaries. No parser buffer or output API is added.

This completes traversal of the sample's records, not conversion, Unicode
coverage or rendering semantics. The public route remains disabled pending
#233. A renderer must explicitly interpret or reject each required control;
none is silently dropped by the parser.


## Three verified symbol exceptions

A complete raw-glyph inventory across the six source pages found only three
codes rejected by the character helper: `a0a6` (8 occurrences on page 2),
`aab3` (2/17/5/12 on pages 1/2/4/5), and `aca3` (2 on page 3).
Counts are decimal. The 6,638 total glyph records include spacing characters;
this is not a reading-order or text-layout claim.

The original generator's `symbols` fixture places `aab3 a0a6 aca3 a3a6 a3aa`
in five columns and repeats them on eight rows. `symbols-permuted` places
`aca3 a3aa a0a6 a3a6 aab3` and changes `801d` to 4. In the pinned offline
viewer at 100% and 96 DPI, the first three symbols visibly appear as an
asterisk operator, ampersand and filled right-pointing triangle. The standard
GB18030 fullwidth ampersand/asterisk columns serve as separate controls.

Fresh distinct clipboard sentinels preceded each selection. The visibly
labeled ordinary **Copy** menu (not enhanced copy/OCR) produced exactly eight
rows of the expected symbol order, allowing whitespace only between symbols:

| Native code | Unicode mapping |
| --- | --- |
| `a0a6` | U+FF06 FULLWIDTH AMPERSAND |
| `aab3` | U+2217 ASTERISK OPERATOR |
| `aca3` | U+25BA BLACK RIGHT-POINTING POINTER |

The earlier real-source ordinary-copy observations independently agree for
`a0a6` and `aab3`. The reordered original control confirms the association for
all three and rules out a stale clipboard result. Font appearance and Unicode
identity are separate: do not derive glyph width from the word “FULLWIDTH”,
and do not extend this exception to arbitrary A0 punctuation or private-use
codes. Tests retain rejection of neighboring unverified codes.

Fixture SHA-256 values:

- `symbols`: `e9c413d7b70494fa3ec699baf350bcf281946cf47a6c50d85bc6974f46cefdcf`
- `symbols-permuted`: `c825648f82dba851333c081b31ec8126434ff98bf423132dc81e426566d993c2`

Raw ordinary-copy SHA-256 values, respectively:

- `d4dc44574a9e80fd3abc52af8f62733ff27ff613d1ec56e6598605e849e5e824`
- `4f5b050cf0d83b3830be642a850f87e2558bbd41c55a5a4010556f0fdf5fa846`

The helper now maps these three explicit exceptions without allocations.
The same six-page source probe reports zero unmapped glyph codes; this does
not prove complete visual conversion, correct font resources or reading order.
The generator reproduces all 21 original inputs byte for byte. Captures,
clipboard bytes and probe outputs remain external under the existing evidence
directories.


## Runtime font-call observations (2026-10-01)

An original external MIT `LD_PRELOAD` shim forwards the public FreeType
size/transform calls unchanged and logs only face family/style, units per em,
size arguments and matrices. It reads no viewer implementation or glyph
outlines. The pinned offline viewer opened the original baseline, vertical,
horizontal, weight and size-square controls. At 100% zoom, each captured page
rectangle `(494,178,1155,1145)` is pixel-identical to its earlier capture without
the shim. This checks the observed rendering, not arbitrary instrumentation
transparency.

For the baseline's `中文AM1` glyphs, the observed CNKI faces are `HGHT_CNKI`
(256 units/em) and `HGBZ_CNKI` (2048 units/em). With only `801d` changed from
0 to 4, calls instead include `HGHT_CNKI` and `HGHZ_CNKI`. This agrees with
the previously measured change confined to the Latin/digit columns, but does
not establish a universal bold flag or the font mapping for every character.
Neither face names nor units/em identify a redistributable font resource.

The vertical control produces nonuniform horizontal matrices while changing
the requested pixel height; the horizontal control changes the horizontal
matrix while retaining the corresponding baseline pixel heights. The final
recorded CNKI pixel-height requests are 11 for baseline, 13 for vertical, 11
for horizontal and 11 for weight. The final vertical/horizontal matrix xx
values are 55453 and 77451 respectively (yy is 65536). These describe this
viewer execution; they are not a physical point-size lookup table. Opening
and resizing also produces thumbnail/intermediate-scale calls, so the full
trace must not be treated as a list of document font sizes.

FreeType documents pixel sizing and 16.16 transform matrices in its
[sizing and scaling reference](https://freetype.org/freetype2/docs/reference/ft2-sizing_and_scaling.html).
The next rendering step still requires associating each admitted raw style
with reproducible font resources, source-space dimensions and baseline
placement. Do not infer those values solely from hinted raster bounds.
External receipts are in `caj2pdf-c8-fonttrace-20261001`: original shim,
per-control traces, captures and action log. No proprietary font, source
content or runtime trace is committed.
