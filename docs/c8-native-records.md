<!-- SPDX-License-Identifier: MIT -->

# Observed C8 native records

Implementation tracking: #232 (parser), #233 (rendering), parent #229.

## Current implementation and acceptance status

Draft #235 at `8a3bd4c` implements bounded native-page translation, real image
codec composition and explicit ranged font resources through CLI, Node and a
browser Worker. The pinned six-page source completes through all three public
paths with identical PDF hashes. This establishes a conversion checkpoint;
independent source-fidelity acceptance and final PR review remain open.
Required unknown records and unavailable glyph resources still fail explicitly.

The sections below are a chronological evidence log. Statements that a rule or
adapter is unimplemented describe the checkpoint in that section, not the current
code. Later controls supersede earlier hypotheses only within their tested scope.
The final runtime checkpoints describe the current resource contract and tests.

### Six-page comparison and decoration classification

Fresh pinned offline viewer captures on 2026-10-02 were inspected against the
96-DPI CLI PDF renders. Pages 2, 4 and 5 preserve the visible code/prose blocks,
subscript examples and page-4 footnote; pages 1 and 3 preserve their figures and
surrounding content; page 6 preserves both abstract/reference blocks and their
divider. This is page-level visual inspection, not an independent transcription
of every character. Explicit diagnostic font substitution changes face, weight,
bearings and spacing, including visibly crowded punctuation. It is not a claim
of source-font fidelity or universal pixel parity.

The page-6 viewer is bottom-aligned: its inspected interior is
`(495,203,1154,1168)`, not the common page-1–5 crop. The corrected interior repeats
identically. The previous fixed crop included a tail of the preceding page and
must not be cited as full page-6 coverage.

The apparent 96-versus-45 decoration discrepancy has two distinct causes:

- Replacing only the viewer's decoration resource with the existing original
  quarter-em rectangle produces 48 separate marks at 100% and 51 at 57% on the
  same source page. Repeated captures match. The source's active style is `10a5`
  and the horizontal span is 4,800 source units. Observed start steps are about
  13 and 7 pixels, consistent with the previously controlled integer screen-em
  requests. The fixed PDF nominal em is `42 * 75 / 301` points; its span admits
  45 marks. A screen-zoom-dependent count is not a fixed document glyph count.
- Original `decoration-size5-single-double` contains only two spans, 105 and
  210 source units, at style `10a5`. At confirmed 1448% zoom, the rectangle
  resource produces one/two marks while the unmodified viewer resource produces
  two/four arrow shapes. Both captures repeat. This independently distinguishes
  glyph outline multiplicity from repeat count without reading vendor outlines.
  The public CLI converts this same original fixture with the original rectangle
  font; qpdf accepts its PDF. The screenshot names include an attempted `900`
  entry, but only the visibly confirmed 1448% zoom is used as evidence.

Keep the nominal physical-unit decoration rule and explicit caller alias; do
not double the repeat count or add a source-specific offset to imitate one
screen zoom. A caller-supplied single triangle remains a visible substitution
for the viewer's two-arrow glyph. These controls classify this discrepancy;
they do not establish arbitrary font equivalence or exact raster parity.

Independent MuPDF text tracing of the six-page CLI PDF yields exactly the
source visitor's Unicode sequence on each page: 1,015 / 1,338 / 840 / 986 / 801 /
1,658 glyphs (6,638 total). The 45 nonsemantic decoration marks are separate.
This verifies that PDF transport did not lose/reorder decoded glyphs; it does
not independently validate the decoder's character mapping. Original mapping
controls remain the evidence for those rules.

External evidence stays in `caj2pdf-c8-six-page-review-20261002`,
`caj2pdf-c8-decoration-source-control-20261002` (`measurement.json` and
`discriminating-controls.json`), and `caj2pdf-c8-render-preview-20261001`
(`unicode-transport-checkpoint.json`). No source text, font or capture is committed.

## Historical observations

## Six-page inventory

The issue-66 source has SHA-256
`90e7b47716c32ef7a67cde8094f312e0ee7f1a7e0ed50a25830e8ba64a84f6a6`.
Each indexed text span ends with a four-byte `8004` record. There are no
remaining bytes inside those spans after that record. The file still has
1,449 bytes after the last indexed text span; they are the application-info
block classified below.

| Page | Text offset | Text bytes | Images | Drawing starts `8006` | High words unresolved at initial inventory |
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


The generator also reproduces the original `size-squares` control: eight rows
with both five-bit size fields set to 3 through 10, fixed high bits `0x1000`,
and the original string `■□田国中`. Its SHA-256 is
`7d89555d34b6a5e4d3d0aee357b4c03db6197354b4690029a3394fa4391a860f`.
Generation was checked byte-for-byte against the externally observed control.
The manifest records every raw row style and character code. This is a size
comparison fixture, not an accepted physical-size mapping.

A bounded zoom follow-up on the baseline records CNKI pixel-height requests
12, 14, 17, 20 and 23 at requested zooms 110%, 125%, 150%, 175% and 200%.
Requests at 400% and 800% were not accepted: the visible zoom field remained
200%; they provide no high-zoom size evidence. Empty traces at previously
rendered sizes also do not establish absence of glyph rendering. Derive a
source-space size rule from independent controls before adopting a PDF font
size; do not use the final pixel request as a zoom-independent point size.


### Viewer font character aliases

A follow-up original forwarding shim also observes public `FT_Get_Char_Index`
and `FT_Load_Glyph` arguments. The original baseline at 100% remains
pixel-identical to the earlier uninstrumented capture. For its Chinese glyphs,
observed HGHT lookups include Unicode U+4E2D and U+6587. For its visible Latin
A, however, HGBZ is queried with U+7CA4 (31908), yielding glyph ID 4851.

The independent `letter-a` fixture contains only eight occurrences of raw code
A0C1 at fixed positions. It confirms the HGBZ U+7CA4-to-4851 lookup and actual
loads of glyph 4851 at several pixel sizes. The existing generator reproduces
this 264-byte control; SHA-256 `930089a6d884f58e5b4365b6e1d1d8073b676822ce83bd5498589739a9298b1d`.

This does not change A0C1's independently verified textual meaning, Unicode A.
It demonstrates that the vendor font's character slots cannot be assumed to
match document Unicode. The shared PDF writer's standard Unicode font contract
is still valid for explicitly supplied Unicode fonts; passing a viewer font
unchanged is not established as faithful rendering. No generic font remapper
or proprietary font data is added. Font choice/substitution must stay explicit,
and baseline/size rules remain unverified. External traces, shim and captures
remain in `caj2pdf-c8-glyphtrace-20261001`; no outlines or font programs are copied
into the repository.


An independent `digit-one` control (eight raw A0B1 codes) confirms actual HGBZ
glyph-2578 loads. Its SHA-256 is `71368e44e4cae4133f6d61cac050c9a37963670cebec70a7debde708f90e8402`; the generator reproduces it exactly.
Metadata-only reads of the SFNT directory, loca, ten-byte glyph headers and
horizontal advances reveal a material distinction: glyph 2578 has advance
2048 and bounds `[664,288,1428,1672]`, while ordinary Unicode 1 (glyph 18) has
advance 1024 and bounds `[216,0,792,1368]`, in 2048 units/em. No outline
coordinates or font programs are exported. A/ M's observed aliases have the
same bounds and advances as their ordinary slots; this does not prove outline
identity. Consequently neither universal alias equivalence nor one uniform
compensation factor is justified. The raw character-to-Unicode mapping remains
unchanged. Measurements are external in `actual-glyph-metrics.json` and
`digit-one-trace.tsv` under the glyph-trace directory.

## Size ladder and first text-page preview

The original `size-ladder` fixture repeats `中文AM1` on twelve fixed rows,
using equal size fields 1 through 12 and high bits `0x1000`. It is 536 bytes,
SHA-256 `648a2074f743d3c588e48058f5847a9cc1775aba0c320fdbd9c3244987d04f7e`.
The generator reproduces the observed input exactly. The pinned viewer's
100% captures repeat identically. A metadata-only extension of the public
FreeType forwarding shim records bitmap dimensions/bearings after loading;
its baseline page is pixel-identical to the prior uninstrumented baseline.
No glyph bitmap or outline is exported by the shim.

A fresh six-page source inventory narrows the required equal size fields to
**2, 3, 4, 5, 6 and 8**. Page 2 uses field 5 for all 1,338 glyph records. This
bounds the first renderer's required size investigation; it does not establish
all 32 possible field values or their physical units.

An external Rust preview now exercises the existing record visitor and shared
PDF writer on actual page 2, which has no source images. It emits all 1,338
glyphs and the 73 observed `8006` coordinate records. The experiment explicitly
substitutes caller-provided WenQuanYi Zen Hei and Latin/symbol fonts. It tests
candidate source-space sizes `[60,70,80,90,105,120,140,160,180,210,240,280]`,
subtractive header origins, and baseline `raw_y + candidate_height`, using the
existing empirical coordinate factor. It models the coordinate records as
thin segments. **Those size/baseline/stroke choices remain hypotheses, not an
admitted production rendering profile.**

The preview PDF passes qpdf validation and text extraction, and renders to
661×967 pixels at 96 DPI. The independently captured viewer page frame is
`(494,156,1155,1123)` after navigating to page 2; it is not the synthetic
single-page frame at y=178. All four edges are visible and the page capture
repeats identically. The first comparison reveals visible font-shape, spacing
and stroke differences. Switching the explicit Latin substitute from DejaVu
Serif to Liberation Serif does not eliminate those differences. No alignment,
resizing or tolerance is used to turn this preview into a fidelity claim.

External receipts are in `caj2pdf-c8-metrics-20261001` (original controls,
public-call metadata and captures), `caj2pdf-c8-full-record-probe` (style
inventory), and `caj2pdf-c8-render-preview-20261001` (original experimental
harness, explicit font resources, PDF/raster/text and hashes). These artifacts
stay outside Git. The preview is restricted to an actual image-free page and
refuses any source image, so it cannot silently omit diagrams. It is not a
six-page conversion, a public CLI/JS route, or a successful completion of
#233. Next compare the candidate glyph geometry against the original controls
before admitting rules, then integrate the existing image emitters in source
draw order for pages 1 and 3.


## Original geometric-font control (#240)

`tools/cajviewer/c8_geometric_font.py OUTPUT` generates three original fonts
with 1000 units per em, an empty missing glyph, a square and two half-square
outlines. It reads no font input. It requires the external MIT fontTools tool
(observed version 4.62.1), not a converter runtime dependency. Resource-slot
family names and observed character aliases select the controlled glyphs;
these files contain no vendor outlines. Generate outside the repository and
mount over the corresponding viewer resources read-only in the isolated
viewer container. Never replace installed host fonts.

The style generator's `size-profile` has six rows, equal size fields
2, 3, 4, 5, 6 and 8, two glyphs per row, row spacing 350 and page height 3200.
Its SHA256 is `ff55ebd71ec505dc0ec26d971571126308544445878f642858ef3fc7dcd95db0`.
This shorter original page keeps all edges visible at 200% zoom. It and all
three generated fonts reproduce the external experiment byte for byte.

In the pinned offline viewer, repeated captures at each zoom were identical.
The original square's measured ink widths, rounded to integer pixels, were:

| Zoom | Field 2 | 3 | 4 | 5 | 6 | 8 |
| --- | --- | --- | --- | --- | --- | --- |
| 193% | 17 | 19 | 22 | 26 | 30 | 40 |
| 194% | 18 | 19 | 22 | 27 | 30 | 40 |
| 197% | 18 | 20 | 22 | 27 | 31 | 41 |
| 198–200% | 18 | 20 | 23 | 27 | 31 | 41 |

These observations contradict interpreting the preview's candidate table as
`floor(size * 320 / 2473 * zoom / 100)`. For example, size 105 at 194%
predicts 26 pixels, while field 5 measured 27. They do **not** establish a
replacement size table or an unmodified vendor-font rendering rule. Font
hinting, viewer size policy and page rasterization must remain distinct.
The public font-call trace contains intermediate scales and cached glyphs;
its order is not sufficient to associate every call with a final-page row.

Short pages are vertically centered: the observed outer frame at 200% is
(164, 247, 1485, 1076), unlike the earlier tall-page frame. Always measure
all four page edges again. No content alignment or fitted baseline offset
was applied. External captures, traces and measurement receipts remain in
`caj2pdf-c8-square-font-20261001`; none are bundled as passing fidelity tests.
The isolated transition and coordinate controls below supersede this earlier
missing observation.
Production font size, baseline and segment rules remain unapproved.


### Isolated coordinates and segment controls (#240)

External receipt `caj2pdf-c8-isolated-size-20261001` uses the same original
geometric fonts. A single field-5 glyph requests/renders 26 pixels at 193%
and 27 at 194%; before/after trace snapshots isolate those calls. Revisiting
each zoom reproduces the screenshot exactly. Changing page width from 5105
to 4000 preserves both requests. The transition is not a page-width effect.

Automatic fit-to-width accepts much higher magnifications than the earlier
manual zoom attempt. Six original 300 × 230 pages at displayed 2896% use
origin (4652,4274) and one square glyph at (4672,4294). For fields 2,3,4,5,6,8,
the final bitmap sizes are 269,298,336,404,461,606 pixels. All frames are
(449,231,1574,1093); all dark glyph bounds start at (599,250). The UI zoom is
rounded and dark bounds are not exact fractional outline bounds; these
measurements alone still do not define a point-size table.

With field 5 fixed, adding 20 to glyph x moves the dark left edge from 599
to 674; adding 20 to glyph y moves the top from 250 to 325. Adding 20 to
header x origin moves the left edge to 524. Adding 20 to both origins and
both glyph coordinates preserves all glyph bounds. The isolated positive
y-origin control clips the glyph at the page edge and is not a full-height
measurement. These establish subtractive origins and translation direction
without fitting document-specific offsets.

A segment at the same raw starting point as the square begins at the same
horizontal position but a different vertical position. Text baseline and
segment placement must therefore not share an unverified y correction.

Three original diagonal segments, tag 8006 with styles a381/a383/a38b, were
rendered on one page. Each has dx=100 and dy=20, with starts at x=4672 and
y=4304,4354,4404. The a383 record has no ffff/0005 terminator; the other two
do. All three remain thin solid segments at both displayed 2896% and 200%.
At an interior column, integrated darkness relative to black is approximately
1.04 pixels at both magnifications (1.047 for all three at 200%). Thus the
preview's fixed positive source-space stroke width is contradicted: it would
grow with zoom. PDF's existing zero-width hairline operation is the candidate
for this observed profile; endpoint, color and independent PDF rendering
comparison remain to be completed before admitting a production rule.
The style generator now includes `draw06compact` and `draw06alternate` with
their correct record framing for further independent controls. No screenshots,
external font data or source-document text are committed.

### Core origin metadata and first segment PDF

`Hnc8Reader::header().native_origin` now exposes the two unsigned C8 words
at offsets 28/30 using one bounded four-byte read. HN-A/HN-B return `None`:
their corresponding bytes have not been established as native origins.
Subtract in signed or floating-point arithmetic; a coordinate below the
origin is not unsigned overflow. The field does not apply a viewer margin,
font baseline correction or an image transform. Explicit `Header` literals
must include the new field in this unstable API.

An original external Rust diagnostic passes the three-segment control through
the native-record visitor and existing PDF writer. It uses the candidate
20-unit x/y margin and zero-width hairlines, with the existing empirical page
scale. qpdf validates the resulting PDF. Reading the origin through the core
instead of the diagnostic's ad hoc header read produces identical bytes.

Opened in the same pinned viewer, source and PDF both have the complete frame
(449,231,1574,1093). At x=800, their three ink-weighted y positions are
458.232/458.162, 645.368/645.462 and 832.989/832.736 respectively. Integrated
black-equivalent width is approximately 1.04 pixels for both. These are
unregistered measurements, not an exact pixel match: rasterization and
subpixel placement still differ. This supports continued hairline comparison
but does not complete segment fidelity or native page conversion. Diagnostic
source is under `caj2pdf-c8-render-preview-20261001/src/bin/segments.rs` and
captures/receipts remain in `caj2pdf-c8-isolated-size-20261001`, outside Git.


### Horizontal decoration and complete diagnostic preview

The earlier diagonal `8010/1` control did not establish that this record was
ignorable. The actual page-6 record has equal endpoint y coordinates. Removing
only this record removes a visible repeated chevron separator; repeated source
and modified captures are individually identical. The changed page-relative
region is (19,600,641,606), with full frame (494,202,1155,1169).
The original `draw10horizontal` fixture reproduces visible decoration without
source-document content. Its geometry manifest explicitly sets `drawing_dy=0`.
Do not silently discard this required record or assume its diagonal behavior.

The PDF content writer now supplies a bounded black filled polygon primitive
(three to eight vertices), streaming coordinates through fixed scratch. Tests
cover concavity, closure, short writes, invalid coordinates, cancellation and
failed-output poisoning. This primitive does not establish C8 decoration shape
or spacing; those remain profile-specific work in #240.

Original geometric font controls now offer `extended-metrics` and
`shifted-outline` variants. Changing ascent/descent alone preserved the tested
CJK and Latin page crops. Moving original outlines up by 250/1000 em moved the
field-5 glyph up by 101 pixels for a 404-pixel em. This supports ordinary outline
placement relative to a baseline, but does not resolve the observed class-specific
Latin placement or the exact size table. No vendor outlines are copied.

An external six-page diagnostic now emits all 6638 mapped glyphs, three decoded
images and the required separator through shared PDF primitives. qpdf accepts
its 14,845,250-byte PDF, and MuPDF renders all six pages. Images were decoded by
the existing Rust codecs into temporary PDFs and extracted losslessly into
external row sidecars. This is not the production streaming image integration.
The diagnostic still uses hypothetical size/baseline rules, explicit substitute
fonts and an original approximate chevron shape. Visible text-spacing differences
remain. It therefore does not pass #233/#240 fidelity or public-adapter acceptance.

External diagnostics and outputs remain under
`caj2pdf-c8-render-preview-20261001`; viewer controls are under
`caj2pdf-c8-nativefont-page6-20261001`,
`caj2pdf-c8-ascent-control-20261001` and
`caj2pdf-c8-outline-shift-20261001`. No external fonts, captures, extracted text,
image rows or preview PDFs are repository fixtures.


### Direct codec and content-page integration control

The original `decoded_images_share_a_content_page_with_glyphs_and_vectors`
control decodes type-0, JPEG and type-3 descriptors directly into a document,
then interleaves their image handles with original Latin/CJK glyphs, a segment
and a filled polygon. It uses the same private checked-image emitter as the
image-only composer, including its scratch accounting and cleanup. No emitted
PDF is parsed to supply the mixed page's images.

The control checks original decoded bilevel rows, drawing order, cleared scratch
and bounded requests with short source/output calls. qpdf validation and a
72-DPI MuPDF raster independently check interior pixels: JPEG covers the earlier
segment, and the final polygon covers the white portion of a type-3 image.
The portable core assertions and external raster check are separate tests;
targets without local validators explicitly filter only the latter.
This verifies codec/content-writer integration, not the still-unverified source
C8 size, baseline or decoration rules. The external six-page diagnostic has
not yet been switched from sidecars to this internal path.


### Independently controlled native image coordinates

`tools/cajviewer/c8_image_fixture.py` generates seven original controls with
one asymmetric 32 × 24 JPEG. They share a 300 × 230 source page and vary only
one placement field or the declared origin. The generated files reproduce the
external viewer inputs byte-for-byte. No external document image is used.

At displayed 2896%, all seven complete page frames are (449,231,1574,1093).
Each capture repeats identically. Bounds below are half-open screen ink bounds,
not fractional mathematical edges:

| Control | Ink bounds |
| --- | --- |
| Baseline (relative x=30, y=40, width=80, height=50) | (562,380,863,569) |
| x + 20 | (637,380,938,569) |
| y + 20 | (562,455,863,644) |
| width + 20 | (562,380,938,569) |
| height + 20 | (562,380,863,644) |
| Both header origins + 20 | (486,305,788,494) |
| Both origins and image x/y + 20 | (562,380,863,569) |

The last page crop is byte-identical to the baseline. These controls establish
subtractive origins, independent axes/extents and absence of a text-specific
20-unit margin for this image profile. One-pixel edge rounding is visible in
the origin-only control; do not infer fractional edges from threshold bounds.
They independently support the earlier actual-source image-x observation.

`decode_native_image_coordinate` admits the observed `d300` prefix, removes
`c000` high bits from x/width, and preserves unsigned y/height. Unknown prefixes
or zero extents return `None`; a renderer must report that unsupported profile
explicitly. Remaining payload words are still opaque. The helper allocates
nothing and does not infer fonts, units or complete-page support.

The viewer vertically reverses the original JPEG relative to its encoded rows
(black source top border appears below). Row orientation therefore remains a
codec/emitter concern, as in the existing image-only composer; one universal
positive-height image transform is not justified. The six-page diagnostic uses
already decoded sidecars, whose representation must be distinguished from raw
source JPEG/type-0 storage. This observation alone does not identify a defect
in its existing diagrams.

External receipts: `caj2pdf-c8-image-controls-20261001/manifest.json`,
`measurements.json`, original inputs and repeated captures. This advances
image placement under #233/#240; font size/baseline and separator fidelity
remain unresolved.


## Held-out font-size model (#240)

The six required equal-size fields admit a common empirical model
`floor(zoom_percent * step * k)`, with steps 28/31/35/42/48/63 for fields
2/3/4/5/6/8. Intersecting the observed 3420%, 2896%, 1563% and 193–200%
integer-height intervals gives `0.003321494343593791 <= k <
0.0033228076692877633`. These step assignments are a model inferred from
observations, not a recovered format specification.

Before opening a new original field-7 control, a step of 56 predicted exactly
636 pixels at 3420% throughout that interval. Both CJK and Latin glyphs request
636 pixels through the public FreeType API and render four 636-pixel squares;
the full page crop repeats identically. Field 7 is a validation control, not
an expansion of the supported production profile. The existing generator
reproduces its bytes as `anchor-field7-large-page.caj`.

A separate original PDF emitted through the existing Rust font/PDF writer
uses the convenient empirical candidate `step * 75 / 301` points, which lies
inside the measured interval. Its CJK top is `y - 15`, with x offset 20 source
units; Latin additionally uses `em / 8` horizontally and
`min(9, round(17 - em / 10))` vertically. At independently verified page bounds,
the first Latin box agrees exactly with the C8 control. The other measured
edges differ by 0–2 pixels at this high magnification. Repeated PDF captures
are stable, and qpdf accepts the PDF. The source square interior is RGB
68/68/68 while the PDF is black: this is a geometry experiment, not pixel parity.
Neither that color difference nor the remaining edge differences are classified
as renderer-only by this experiment. Do not enable a production profile from
this result alone.

Pre-capture prediction, public API trace and both geometry reports are external:
`caj2pdf-c8-anchor-ft-metrics-20261002/field7-{prediction,result,pdf-result}.json`
and `field7-heldout.tsv`. The original diagnostic writer is
`caj2pdf-c8-render-preview-20261001/src/bin/candidate_geometry.rs`. No external
font outlines, source text or screenshots enter the repository.

### Core empirical glyph geometry

`empirical_c8_glyph_transform` now evaluates this measured geometry in the
existing placement module. It takes checked page geometry, raw header origin,
raw glyph position, style and an explicit CJK/Latin glyph class. It validates
page geometry and rejects unknown high style bits or size fields outside 2–8.
The two size axes remain independent; origins are subtracted without unsigned
underflow, and off-page positions are preserved. `C8GlyphClass` describes a
geometry class only: it does not identify a font resource or classify arbitrary
source character codes.

The evaluator uses the documented empirical size model and observed Latin
offsets (9/9/8/6/5/3/1 source units for fields 2–8). It is allocation-free and
performs no I/O. An external original PDF control now calls this core helper
instead of repeating its geometry expressions. qpdf accepts the result, and
MuPDF rasters at 72 and 3283.2 DPI are byte-identical to the earlier independently
constructed gray PDF control. This validates the implementation against that
control, not exact parity with CAJViewer: the documented source/PDF edge and
frame differences remain. Production native-text conversion is still disabled
until the remaining style, decoration and complete-page checks are satisfied.
Receipts are in `caj2pdf-c8-core-geometry-20261002/results.json`.

### Glyph gray level and writer support

A follow-up public FreeType trace on the same original field-7 fonts reports
maximum bitmap coverage 255 with 256 gray levels. The instrumented source
crop is byte-identical to the earlier capture, whose uniform glyph interiors
are RGB 68/68/68. The retained original `c8-axis-reference-5` print raster also
has that interior gray level (867,306 pixels); therefore the difference cannot
be dismissed as screen-only behavior. This does not infer a general color rule
for every native profile or unknown style.

`ContentPageWriter::glyph_with_gray` now supplies a local DeviceGray value,
with zero black and 255 white. It shares font lookup, glyph validation and
streaming text output with `glyph`, restoring graphics state after the draw.
The writer itself contains no C8-specific gray constant. Original mixed-page
tests cover short writes, a gray glyph followed by a black glyph, cancellation,
abandoned draws and failure while restoring graphics state. The independent
qpdf/MuPDF export renders RGB 68/68/68 for the gray glyph and 0/0/0 for the next
glyph. Existing plain glyph calls retain their output representation.

The held-out candidate PDF now uses gray 68. In the pinned viewer, source and
PDF glyph interiors agree at 68/68/68 and each page crop repeats identically.
The unaligned full-page crops still differ at 13,656 pixels, including page
frame and glyph edges. Color preservation is implemented; geometry and full
native conversion acceptance remain open. Receipts are external under
`caj2pdf-c8-gray-20261002` and
`caj2pdf-c8-anchor-ft-metrics-20261002/field7-gray-result.json`.

## Independent size fields and original glyph anchors (#240)

The style fixture generator now reproduces eight original `axis-*` controls:
CJK `中` and Latin `A`, each with horizontal/vertical fields `(3,3)`, `(3,5)`,
`(5,3)` and `(5,5)`. They use a 150×100 page, raw position `(4672,4294)`
and the existing asymmetric origin. Generated inputs match the observed
controls byte for byte; all prior generated fixtures remain unchanged.

The pinned viewer restarted in its default smaller window. Its inspected
page interior is `(648,537,1023,787)` (375×250 pixels), not the earlier
maximized-window frame. With original geometric fonts, thresholded ink
left/top positions relative to that interior are:

| Horizontal, vertical | CJK | Latin |
| --- | --- | --- |
| 3, 3 | 99, 12 | 124, 34 |
| 3, 5 | 99, 12 | 124, 27 |
| 5, 3 | 99, 12 | 132, 34 |
| 5, 5 | 99, 12 | 132, 27 |

All eight repeat captures match. The high five-bit field changes width and
Latin horizontal placement; the low field changes height and Latin vertical
placement. The original CJK square keeps its upper-left anchor. Larger squares
clip at page edges, so these measurements do not establish their full extent.

A separate font control doubles only `hmtx` advance widths. Table comparison
finds changes only in `hmtx` and the expected `head` checksum. For equal fields
3, 5 and 8, both CJK and Latin page crops are identical to the normal-advance
controls; repeated captures also match. Thus advance width does not explain
the observed anchor difference for these controls. Vertical-metric and outline
controls described above remain separate evidence.

These observations establish independent field effects, not an exact PDF
font-size or Latin baseline formula. A proposed `ceil(em/12)` vertical offset
is inconsistent with the observed size-dependent deltas and is not admitted.
Production rendering acceptance remains open. External receipts are in
`caj2pdf-c8-advance-control-20261001/{comparison,axis-comparison,axis-inputs}.json`;
no captures or font binaries are committed.

The original `anchor-field2-same-page` control places CJK/Latin square pairs
on the same 300×250 page, at source y=20 and y=120 relative to the origin.
This removes tab-specific frame/zoom differences from the comparison. With
page interiors 770×641 and 1127×938 pixels, both rows have Latin-minus-CJK
vertical offsets of 23 and 34 pixels respectively (approximately 9 source
units). Repeat captures match. The earlier candidate
`17 * page_scale - em_height / 10` predicts 25.33 and 36.86 pixels and is
not supported by this control. The discrepancy is not explained by source-y
position or separate tabs. This is a field-2 observation, not a replacement
rule for other sizes or fonts. The generator reproduces the captured input
byte for byte; external receipts are in `caj2pdf-c8-same-page-anchor-20261002`.


### Same-page anchors across the required six sizes

The original `anchor-field{2,3,4,5,6,8}-large-page` controls keep a 500×500
page and two CJK/Latin square pairs fixed. Relative glyph positions are
(20,20)/(250,20) and (20,250)/(250,250); only the equal size fields change.
All four squares remain visible. The generator adds no external font data.

Pinned offline CAJViewer with the existing original geometric fonts, 96 DPI
and fit-height (displayed 1563%) gives the following thresholded vertical
extents and Latin-minus-CJK top offsets. Both rows agree and all repeated
page crops are identical:

| Field | Square height (pixels) | Latin top offset, both rows (pixels) |
| --- | --- | --- |
| 2 | 145 | 18 |
| 3 | 160 | 18 |
| 4 | 181 | 16 |
| 5 | 218 | 12 |
| 6 | 249 | 10 |
| 8 | 327 | 2 |

The inspected full page interior is 1009×1009 pixels. Field 2 has a different
horizontal screen origin after closing its wider sidebar; each page's own
physical bounds are recorded, without content registration. Initial automatic
fit-width captures had different tab widths and sometimes clipped the page;
those are excluded from these measurements.

This independently confirms that source-y position does not explain the
class offset for any required size. The equal offset for fields 2/3 also
prevents treating the earlier simple linear baseline candidate as established.
A size-indexed source offset and a size-dependent rule with rounding remain
competing explanations. Distinguishing their predicted transition at another
scale is required before implementing either; these integer pixel observations
are not an exact point-size table or a production support claim.
External inputs/captures and measurements remain in
`caj2pdf-c8-six-anchor-inputs-20261002` and `caj2pdf-c8-six-anchor-20261002`.

A held-out scale uses the identical six inputs and fonts on a 3200×2400,
96-DPI display. Fit-height is displayed as 3420%, with inspected interiors
2210×2209 pixels. Measured square heights are 318/352/397/477/545/715 pixels;
both rows have Latin-minus-CJK offsets 40/40/36/27/22/5 pixels, and all repeats
match. Predictions were written before capturing: source offsets
9/9/8/6/5/1 versus `17 * page_scale - max(square_height, 80 * page_scale) / 10`.
The latter predicts 20.64 and 3.64 pixels for fields 6 and 8, outside a
one-pixel final-raster rounding difference from the observed 22 and 5. Thus
that candidate plus final rounding is insufficient. The integer-offset
candidate remains consistent with these controls; exact font-size mapping
and the viewer's intermediate rounding are still unresolved. Do not infer
an arbitrary-font baseline rule solely from the geometric font.
Receipts and pre-capture predictions are in
`caj2pdf-c8-six-anchor-highscale-20261002`. Its external capture helper uses
the explicitly configured 3200×2400 grid; repository capture defaults and
production code are unchanged.


A later check rereads the original fixture records (row y values 4294 and
4524, a delta of 230) and verifies the vertical ink bounds in the retained
captures. Across all six styles, the row delta is 465 pixels at displayed
1563% and 1018 pixels at displayed 3420%. Dividing the recorded page-interior
height by 500 instead predicts 464.14 and 1016.14 pixels. Thus the cropped
page height must not be treated as an exact content scale when deriving
subpixel font metrics. This observation does not establish which intermediate
rounding or frame convention causes the difference, and is not a new font
size rule. The follow-up below measures several known coordinate intervals on one page;
repeating the same six style screenshots would not decide that question.
The external `caj2pdf-c8-scale-check-20261002/check.py` and `results.json`
record source hashes and the calculation. Vertical bounds are independently
rechecked at grayscale threshold 128; horizontal edge thresholds are not used
for this conclusion.


The original `coordinate-grid` fixture supplies nine CJK squares at x values
20/170/320 and y values 20/21/22, 170/171/172 and 400/401/402. At displayed
3420%, their page-relative top coordinates are 21/25/30, 685/689/694 and
1702/1707/1711; left coordinates are 175/839/1503 for each row. Each square
is 318 pixels high. The full 2210×2209 page crop repeats identically.

These coordinates are consistent with the existing empirical source scale
`320 / 2473 * 34.2` pixels per unit, a fixed origin per axis and final integer
rounding. All nine y positions admit a common floor-model origin interval
[-67.167813, -67.157703), without changing that scale. The frame-height-derived
scale fails this same check. This supports reusing the existing coordinate
scale for this control; it does not prove the viewer's exact rounding pipeline
or establish a font-size table. An earlier unmaximized capture at displayed
1739% does not fit the same single-floor model vertically and has no repeat
capture; retain it as an unresolved observation, not acceptance evidence.
External `caj2pdf-c8-anchor-ft-metrics-20261002/grid-results.json` records the
nine points and stable final crop. The generator reproduces the 192-byte
fixture with SHA256
`37e7cfc47015ec29e12649e8df353042c841538eac08925bffb9c98b85442c47`.


A public FreeType forwarding trace on the same six controls confirms requested
pixel heights 318/352/397/477/545/715 at displayed 3420%. Both original geometric
faces request the same size, use the identity 16.16 transform, and produce
bitmap left bearing zero with top bearing, width and rows equal to that size.
The first instrumented page crops exactly match the uninstrumented captures.
At displayed 200%, requests and bitmap sizes are 18/20/23/27/31/41, matching
predictions recorded before capture; all six repeated page regions agree.
The later field-2 return from 200% to fit-height differs at five pixels from
its initial high-scale crop, while the other five high-scale repeats agree.
This navigation-dependent difference is retained, not treated as exact repeat
parity. An attempted 1000% edit was rejected by the UI and is excluded.

These observations locate the controlled square extents at the requested
pixel-size stage rather than a subsequent bitmap transform. They do not
establish exact source-space font units, arbitrary-font baselines or a PDF
point-size table. Resolve that mapping before promoting preview sizes into
production rules. External traces, pre-capture predictions and comparison
receipts are in `caj2pdf-c8-anchor-ft-metrics-20261002`; no font outlines or
bitmap payloads were extracted by the trace.


### Additional raw drawing value `8006/a385`

An original one-row control changes only the existing `8006/a381` record's
value to `a385`, retaining two asymmetric endpoints, the trailing `ffff/0005`
pair and five following glyphs. Both controls render the line and all five
geometric glyphs in pinned CAJViewer; page crops and their repeated captures
are identical at displayed 57% zoom. The inspected page frame is
`(647,386,1024,937)`. This low-zoom equality establishes record framing and
preservation of following content, not identical stroke semantics at every
scale. The generator's `draw06a385` reproduces the observed input exactly.

The initial C8 visitor treated this as one 16-byte `NativeRecord::Drawing`.
The independent in-run boundary controls below supersede that assumption:
the drawing is 12 bytes and `ffff/0005` is a separate raw control. Short-read tests
use marker-like payload words followed by a glyph; the existing truncation
sweep now covers every shortened length of this form. Adjacent unverified
value `a384` remains unsupported. No allocation or new rendering rule is added.

HN-B inventories in #241 also encounter `a385`; that observation alone does
not enable the C8 visitor for HN-B. HN-B variant semantics and complete-document
conversion remain open. External receipts are the `segment-a381` and
`segment-a385` controls/captures and `segment-inputs.json` in
`caj2pdf-c8-advance-control-20261001`.

## Corrected in-run drawing boundaries

Eleven original C8 controls test `8006/a381`, `a385` and `a38b` separately:
12-byte drawing, drawing followed by `ffff/5`, and drawing followed by
`8001/5000`, plus a no-drawing baseline and standalone `ffff/5`. For each
style, bare and footer variants have identical first-page pixels. A following
y record moves the five glyphs while preserving the visible segment.
Standalone `ffff/5` matches the baseline. All repeat crops match at 57%,
page interior `(648,387,1023,936)`. This independently confirms the boundary
without assuming that HN-B semantics apply to C8.

The visitor now emits each `8006` drawing as a 12-byte record and preserves
`ffff/5` as its own `Control`. It does not discard the control or infer that
it is always a no-op. The already admitted `a383` remains 12 bytes. The
separate `8010/1` form was subsequently verified below with its own controls.
Existing sources containing `ffff/5` therefore yield one additional raw event
per occurrence: exhaustive event/count consumers must accommodate it. This is
a documented unstable v0.x parser behavior correction, not a conversion claim.

The generator reproduces all eleven inputs. Tests cover immediate y/end
records, marker-like coordinates, short reads and truncated drawings. Earlier
16-byte boundary descriptions in this investigation are superseded for these
`8006` forms. Font size, baseline and stroke fidelity remain unresolved under
#240. External receipts are `c8-drawing-boundary-{inputs,comparison}.json` in
`caj2pdf-c8-advance-control-20261001`; external captures remain outside Git.

### 8010 boundary and resource-controlled replay

The `8006` conclusion was not automatically applied to `8010/1`. Initial original
horizontal controls with substituted geometric fonts showed no decoration.
Replaying the known-positive `draw10horizontal` input and three new long
horizontal controls in the same pinned offline image with its shipped font
resources restores the visible repeated ornament. Those font files remain in
the external viewer image; no font outlines or implementation are copied.

Bare 12-byte `8010/1` and the variant followed by `ffff/5` have identical page
pixels. A following `8001/5000` moves only the glyph row; the ornament remains.
All four replayed inputs match their own repeats at 57%, page interior
`(648,387,1023,936)`. The generator reproduces the three new controls exactly.
This establishes the 12-byte boundary and independent following control, allowing
the reader to use one fixed drawing read without a guessed mandatory footer.
Tests cover following y/end records, marker-like points and truncation.

The resource comparison establishes sensitivity to font replacement for this
control; it does not identify a redistributable glyph or approve a substitute
pattern. `8010/1` must still not be silently dropped or rendered as a plain
segment. Its rendering semantics remain in #240. External receipts are in
`caj2pdf-c8-decoration-default-20261001`, including `comparison.json` and the
recorded launch arguments. Earlier mandatory-footer descriptions are superseded.

### Original glyph control for the 8010 resource dependency

Public FreeType call observations for the original horizontal `8010/1` control
show a character query of decimal 23812 in HGBZ_CNKI, returning glyph 1862 in
the viewer's bundled font. These are API metadata, not extracted outlines.
The numeric query is a resource alias; it does not establish the ornament's
Unicode text meaning or a stable glyph ID across fonts.

The geometric font generator now offers `--variant decoration-alias`. It adds
only that character-map entry, pointing at the existing original upper-half
rectangle. Against otherwise identical generated baseline fonts, the long
horizontal control gains a solid visible strip; the baseline has no strip.
Both captures repeat exactly at 57% with page interior `(648,387,1023,936)`.
The changed region is `(17,34,335,39)` relative to that interior. This isolates
the alias as necessary for visible decoration in this control and provides an
original positive resource fixture. It does not establish the complete pattern
placement, scaling or repetition rule and does not authorize rendering the
source ornament as a plain line. Those remaining rules stay under #240.

The default generator and previous metric/outline variants retain their output.
Receipts and captures remain external in
`caj2pdf-c8-decoration-{alias,no-alias}-20261001`. No vendor font data is added.

### Decoration axes and repetition control

Five original `decoration-geometry-*` inputs vary only the two `8010/1`
coordinate pairs. With the original alias font, shortening the horizontal
span changes the decoration width; translating both points by 300 source x
units moves its visible region 22 pixels, and 500 source y units moves it
37 pixels at the same 57% page frame. A vertical span is visible. The sloped
control matches the no-decoration baseline; this is an observation, not
permission to silently discard arbitrary diagonal records. All repeat captures
match. Raw coordinate endpoints remain preserved by the parser.

`--variant decoration-narrow` narrows the original upper-half rectangle to a
quarter em while retaining its one-em advance and the same alias. The long
and short horizontal controls now show separated repeated marks (53 and 27
connected runs at desktop row 423, threshold 200), rather than one stretched
rectangle. Vertical repetition is also visible; all three captures repeat.
The generator reproduces the tested glyph/cmap/metric tables and all five
source controls. This establishes axis sensitivity and glyph repetition, but
not an exact source-space step, endpoint clipping or font-size formula. Do not
promote pixel-run counts into a document-independent repetition rule.

External receipts are `geometry-{inputs,comparison}.json` in
`caj2pdf-c8-decoration-alias-20261001` and `comparison.json` in
`caj2pdf-c8-decoration-narrow-20261001`. The next discriminator is the source
step and end clipping under an independently changed zoom/advance; reuse these
controls rather than introducing another renderer or screenshot framework.

### Advance and zoom discriminate decoration spacing

Doubling the original narrow glyph's horizontal advance, leaving its
outline and mapping unchanged, produces an identical whole page at 57%.
The generated font also updates `hhea.advanceWidthMax` to remain consistent.
A fresh viewer replay with both metrics at 2000 confirms the same result.
Reproduce with `--variant decoration-narrow --decoration-advance-multiplier 2`.
Thus normal font advance is not the spacing rule for this observed decoration.

At independently selected and visually confirmed 100% zoom, all page edges
remain visible (interior `(858,179,1518,1145)`). The same long control has 51
separated runs on desktop row 245, with 11-pixel spacing in its unobstructed
tail, versus 53 runs and 6-pixel spacing at 57%. Repeated captures match.
This disproves a zoom-invariant glyph count derived from the 57% screenshot.
The viewer applies raster-dependent spacing/rounding; exact pixel equality at
one zoom cannot establish a source-space repetition count for PDF output.
Nominal symbol size and end clipping still need an independent rule before
production admission. Font outline substitution remains explicit. Receipts
are in `caj2pdf-c8-decoration-advance-20261001/comparison.json`; the
consistent-metrics replay is in
`caj2pdf-c8-decoration-advance-valid-20261001/comparison.json`.

## Print-path limit for physical-size evidence (#240)

An isolated CUPS-PDF destination was attached by Unix socket to the pinned,
network-disabled viewer. The original `c8-axis-reference-5` control printed
successfully. Two explicitly selected actual-size jobs and one automatic-fit
job produced byte-identical 160,762-byte PJL-wrapped PostScript spools (SHA256
`2db49b940f3ae532f3e3c4f2207ec3b307d0bcfac53d860c520c4d46ee94e6f4`).
Each spool contains one 2310×3059 RGB raster placed using `28 28 translate`
and `555 736 scale`; it contains no `/PageSize` request. The resulting PDF
passes qpdf, has no font objects, and uses an A4 MediaBox despite the Letter
label in the application's dialog.

These controlled print settings do not establish native vector geometry or
physical font units. Do not infer a point-size table from the dialog labels,
use the backend's paper size as the source page size, or promote this output
as an exact page-fidelity baseline. The print path is usable as a separately
identified raster appearance reference only. This bounded experiment is
complete; repeated screen/print ratio fitting does not resolve the missing
source-space style rule.

External receipts: `caj2pdf-c8-print-size-20261001` (build/server configuration)
and `caj2pdf-c8-print-output-20261001` (settings, raw spools, converted PDF and
`repeat-print-result.json`). The print service is experiment infrastructure,
not a runtime dependency. Original controls only; no captures or PDFs are
committed.
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

The following HN-B `8006/a383` drawing is independently verified as 12 bytes:
original bare and `ffff/5`-suffixed controls render identically, and a following
y control changes the glyph row independently. The visible segment occupies
only the added drawing region. `8072/cdc1` bare/next-y controls preserve their
following glyphs; unchanged pixels do not establish that this control is a no-op.
Both records reuse the existing raw events. Seven controls in
`hnb_index_fixture.py` reproduce the external inputs byte-for-byte, with repeated
identical crops (`remaining-record-comparison.json`).

After these admissions, the pinned issue-65 page 6 traverses all four raw records.
Page 1 traverses 283 records (235 raw glyphs) before another unsupported control
at offset 1500. Other pages retain explicit style/control failures. No complete
page rendering or document-conversion acceptance is claimed.

Four further original bare/next-y controls verify raw `801d/0003` and
`8070/001c` framing. All repeat identically and preserve the following row;
changes are confined to the affected first-row glyphs. The generator reproduces
captured input bytes exactly (`style-inputs.json`, `style-comparison.json`).
Only the observed raw values are admitted; this does not establish physical
font units or admit the corresponding untested `8071` value. The issue-65 probe
then reaches page-1 offset 1532 and page-3/page-4 offsets 53366/57046 before the
next unsupported controls; implicit-style failures on pages 2/5 remain.


### Decoration inherits active size (2026-10-02)

Three original `decoration-inherited-size{2,4,8}` controls place the explicit
style before the same horizontal `8010/1` record, with no ordinary glyphs.
They use the existing original quarter-width, upper-half decoration outline
with doubled advance. At the same displayed 57% zoom and physical page bounds
`(648,387,1023,936)`, all three repeated page captures are byte-identical.
The final document-rendering FreeType requests are respectively 5, 6 and 11
pixels in both axes; thumbnail requests are excluded. The 6/11 predictions
were recorded before their captures using `floor(step * 57 / 301)`.

| Size field | Requested pixels | Dark-pixel bounds relative to page | Separate x runs | Observed start spacing |
| --- | --- | --- | --- | --- |
| 2 | 5 | `(17,35,334,36)` | 64 | 5 or 6 pixels |
| 4 | 6 | `(17,34,331,37)` | 53 | 6 or 7 pixels |
| 8 | 11 | `(17,32,329,37)` | 29 | 11 or 12 pixels |

Bounds are exclusive; runs use RGB channels all below 128, without registration
or resizing. Fractional sampling can move a thresholded run start; these values
must not be misreported as uniformly integer-spaced glyph origins. This isolates
active-size inheritance and disproves a fixed-size/fixed-count decoration.
It does not yet establish exact source-space repetition, endpoint clipping,
color or font selection. Earlier advance controls still show that doubling the
font advance does not double the repetition step. Keep these distinctions when
implementing the PDF rule; arbitrary zoom pixel parity is not a physical-unit
specification, and the production profile remains gated on required content.

External receipts: `caj2pdf-c8-decoration-metrics-20261002`, including input
hashes, predictions, public font-API traces, repeated captures and `results.json`.
Only the original generator and these findings are committed.


The two unequal-axis controls `decoration-inherited-axis2-8` and `axis8-2`
separate width and height. Both repeat identically at the same page bounds and
57% zoom. Width field 2 gives 64 runs with dark bounds `(17,32,334,36)`;
width field 8 gives 29 runs with bounds `(17,35,329,37)`. Horizontal count and
spacing therefore follow the width field independently of height; changing
height changes the mark's vertical extent.

The predicted *effective* dimensions of 5×11 and 11×5 pixels hold, but the
prediction of anisotropic FreeType size requests is false. The viewer requests
`(width=0,height=11)` or `(0,5)`, producing square em metrics, then sets x scale
`29789/65536` (approximately 5/11) or `144179/65536` (approximately 11/5),
with identity y scale. This explains the final bitmap dimensions without
mistaking `FT_Size` for effective glyph width. Preserve this failed prediction
alongside the observation: PDF transforms can represent independent axes, but
these screen operations still do not specify a zoom-independent repeat count.
Receipts are `axis-predictions.json`, `axis-results.json` and the corresponding
traces/captures in the same external metrics directory.


### Horizontal decoration endpoint clipping

The original `decoration-endpoints` control places six otherwise identical
horizontal `8010/1` spans of 10, 50, 89, 91, 180 and 430 source units on a
600×600-unit page, with explicit style `1084` and no ordinary glyphs. Predictions
were recorded before capture. Both inspected 486% and 993% views show respectively
1, 1, 1, 2, 3 and 5 marks, including clipped marks at the right endpoint. All
page crops repeat identically without registration or resizing.

At 486%, full marks occupy 15 thresholded pixels horizontally; the 10-unit
span leaves 8 pixels. At 993%, corresponding widths are 30 and 13 pixels.
The 91-unit span's second mark and 180-unit span's third mark each occupy only
2 thresholded pixels at both scales. Thus the renderer starts a mark even for
less than one em of remaining span, then clips its visible extent. A floor-count
loop that drops partial final marks is contradicted, as is emitting whole glyphs
past the endpoint. Threshold is RGB channels below 200; these are raster bounds,
not exact geometric clipping coordinates.

Consecutive complete mark starts differ by 56 pixels at 486% and 115 pixels at
993%, consistent with the observed font em requests. The nominal empirical
width (35×75/301 PDF points, about 89.86 source units) predicts the same six
counts at both scales. Prior long-line observations still demonstrate
zoom-dependent counts; nominal PDF spacing must not claim pixel parity at all
viewer zooms. Implementation should stream repeated font marks with endpoint
clipping, keep decoration separate from semantic Unicode text, and use the
verified independent width/height fields. This evidence does not admit reversed
or diagonal endpoints, or establish arbitrary font selection.

Receipts: `endpoint-predictions.json`, `endpoint-results.json`, traces and repeated
captures in `caj2pdf-c8-decoration-metrics-20261002`. The first screenshot is named
`endpoints600` after an attempted UI entry, but the visible confirmed zoom is
486%, not 600%; the maximized view confirms 993%. No inference uses the attempted
zoom. The generator reproduces the captured input bytes exactly.


The PDF writer now exposes `glyph_with_clip` with a local, positive-extent
rectangle in page coordinates. Independent original-fixture rendering verifies
cutoff at x=35 and an unaffected later glyph outside that rectangle. Required
clipping mechanics are available without page-content buffering. The method
retains ordinary Unicode mapping: C8 repetition, font selection, placement and
nonsemantic alias handling still need integration before production admission.


`decoration_glyph` now supplies the nonsemantic counterpart of the clipped
glyph operation. It uses the same bounded writer, font lookup and failure path,
with Artifact/Span marking and empty ActualText. Original output retains its
visible clipped glyph while both Poppler and MuPDF omit that glyph's alias and
preserve subsequent ordinary text. It does not infer a font resource, position
or repetition count; those remain C8 translation responsibilities. Missing font
glyphs still fail explicitly. This primitive does not enable production native
conversion or close six-page acceptance.


### Nominal decoration PDF control

An original external Rust control now generates the six endpoint rows using the
actual `decoration_glyph` writer and the same original narrow alias font. For
style `1084`, its nominal em is `35 * 75 / 301` points. The first glyph's x is
the source x minus source origin, in empirical coordinate units, with no text
margin; its baseline is page top minus relative source y minus half the em.
Successive glyphs advance one em. The count is the ceiling of span width divided
by em, with the clip at the exact source endpoints. These are empirical candidate
placement rules, not authoritative format units or production admission.

Pinned CAJViewer displays source and generated PDF at confirmed 993%, with the
same inspected interior `(803,277,1573,1046)`. Repeated PDF crops match; a fresh
source crop matches the prior source capture. Both have counts 1/1/1/2/3/5 and
partial final marks. Without registration or scaling, PDF-minus-source row y
bounds differ by 0 pixels for row one and +1 for the other five. X run boundaries
mostly agree; differences reach +1 for later repetitions and +2 at one clipped
endpoint. These residuals remain recorded, not hidden by a tolerance or corrected
with sample offsets. The model preserves the demonstrated repetition/clipping
behavior but does not establish universal pixel parity.

qpdf accepts the generated PDF. Poppler extraction is empty apart from the page
separator, as expected for a page containing only decoration. External receipts:
`caj2pdf-c8-decoration-metrics-20261002/endpoint-pdf-result.json`, source/PDF
captures and the authored `decoration_endpoints.rs` in the existing external
render-preview package. The first PDF captures used different automatic zooms;
only the final visibly confirmed 993% capture is compared quantitatively.


`empirical_c8_horizontal_decoration` now evaluates this model in the core. It
returns the first glyph matrix, one endpoint clip and a bounded repetition count;
callers stream each mark through `decoration_glyph`, incrementing x by index times
nominal width. It shares size validation with ordinary glyph placement and has
no allocations or I/O. Only forward horizontal nonempty spans are admitted.
Original control counts, unequal axes, origin translation, signed off-page
positions, maximum raw span, unsupported directions/styles and invalid pages
are covered. Replacing the external diagnostic's manually assembled placement
with this helper produces a byte-identical PDF, preserving the recorded source
comparison rather than starting a new calibration. Font/style selection and
complete-document integration remain open.


### Decoration resource selection controls

Two original isolated controls change only `801d` from 0 to 4 or `8067` from
6 to 9 before the same horizontal decoration. Both remain byte-identical to
the style-4 baseline page crop at confirmed 57% and repeat identically. Public
FreeType traces independently identify `HGBZ_CNKI`, alias 23812, original glyph
2 and final 6-pixel em loads in both cases. Thus the ordinary Latin font switch
observed for `801d=4` must not be applied to this decoration; the tested `8067`
change also retains its resource. These controls do not establish every possible
font/state value or authorize ignoring those controls on ordinary text.

The generator reproduces `decoration-resource-state4` and
`decoration-resource-font9` exactly. External input hashes, repeated screenshots
and metadata-only traces are in `caj2pdf-c8-decoration-metrics-20261002`, with
`resource-results.json`. No font programs or outlines are extracted or committed.
The explicit caller-resource contract should identify the decoration separately
from semantic Latin text, even when a provider supplies both from one font.


### Independent horizontal/vertical segment control

The original `segment-axes` fixture contains one horizontal and one vertical
segment for each of `8006/a381`, `a383`, `a38b`, with asymmetric coordinates
and no ordinary text. A separately authored PDF uses the existing black,
zero-width segment writer with the candidate +20 source-unit margin in both
axes. At confirmed 993% and the same physical page bounds
`(803,277,1573,1046)`, source and PDF both show all six segments in the expected
positions. Repeated captures match. Threshold-250 bounds differ by at most one
pixel at endpoints/edges; no registration or sample correction is applied.

The comparison exposes a material raster difference that the earlier diagonal
controls did not: source horizontal cross-sections integrate to 1.0078 black
pixels, while the same viewer's PDF path integrates to 2.0039. Source/PDF
vertical cross-sections both integrate to approximately 1.00. Independently,
MuPDF renders this same PDF at 953.28 DPI with horizontal/vertical cross-section
masses of 0.2667 and 0.1765–0.2353. Thus a zero-width PDF stroke is not a promise
of a uniform one-black-pixel line across these rendering paths. These results
must not be described as pixel-equivalent or used to tune a fixed gray value.
The same PDF's differing renderings establish renderer sensitivity; they do
not establish every remaining source stroke property.

Together with the existing diagonal controls across zooms, this supports a
nominal hairline representation with explicit raster limits, not a fixed
positive source-space width that grows with zoom. Unknown styles remain
unsupported. qpdf accepts the authored PDF. External receipts are
`segment-axes-input.json`, `segment-axes-result.json`, `segment-mupdf-result.json`
and repeated source/PDF captures in `caj2pdf-c8-decoration-metrics-20261002`.
The generator reproduces the observed input byte-for-byte; no external content
or raster captures are committed.


`empirical_c8_segment` now implements the measured endpoint evaluator for
`a381/a383/a38b`, sharing the empirical coordinate unit and retaining source
endpoint order and signed off-page coordinates. Callers separately establish
the `8006` record and emit the endpoints using zero-width PDF segments. Unknown
styles, including framing-only `a385`, and invalid page geometry fail explicitly.
The original horizontal/vertical control generated through this helper renders
byte-identical pixels to the independently constructed PDF at 953.28 DPI; qpdf
accepts it. This confirms implementation of the model, not removal of the
recorded viewer hairline differences. Receipts: `segment-core-result.json` in
the existing external metrics directory. No production profile is enabled here.


### Admitted glyph style prefixes

Original `style-flags-{0800,0c00,1000}` controls change only the high style bits,
with a Chinese and Latin geometric glyph at each of size fields 2 and 8. At
confirmed 993%, all three complete page interiors `(803,277,1573,1046)` are
byte-identical and repeated captures match. The previously observed field-4
controls provide an independent size case. Original font-call traces retain the
HGHT/HGBZ resources and the 92/207 pixel requests at the final matched scale.
Initial new-tab captures used 1448% with a different sidebar width; those are
excluded from the matched comparison, not scaled or registered afterwards.

The glyph geometry evaluator now explicitly admits these three prefixes,
normalizing only its private size calculation. Raw parser styles remain intact;
unknown prefixes and unknown size fields still fail. This proves the measured
glyph geometry for the selected profile, not a universal no-op interpretation
of high bits. Decoration admission is unchanged because these controls contain
ordinary glyphs. No default font-selection or full-page acceptance follows.

The generator reproduces all three observed inputs exactly. External inputs,
matched repeats and traces are under `caj2pdf-c8-decoration-metrics-20261002`;
`style-flags-matched-results.json` supersedes the initial unequal-zoom comparison.


## Initial state and combined ordinary font controls (2026-10-02)

A fresh bounded inventory at draft `1f4ab71` visits all six issue-66 pages:
6,638 glyphs decode without an unknown character, and the existing glyph,
segment and horizontal-decoration evaluators reject none of their geometry.
This diagnostic provisionally classifies ASCII alphanumerics as Latin; it
neither proves all glyph classes nor admits control semantics or conversion.
Some glyphs precede the first `8067` control, and page 6 combines `801d/4`
with `8067` values 5, 6, 8 and 9. Initial absence must not be mistaken for an
explicit source `8067/0` record.

The original style generator now supplies three omitted-initial-state controls
(`initial-default`, `initial-font-default`, `initial-weight-default`) and
`weight-font5/8/9`. Each contains eight fixed-position rows of `中文AM1` at
style `1084`. Only the identified controls change relative to the existing
baseline (`801d/0`, `8067/6`) or weight (`801d/4`, `8067/6`) fixture. The
omission is applied to every row; it does not simulate a reset after a prior
nondefault state or establish cross-page state persistence.

In the pinned offline viewer with the existing original geometric fonts, at
57% and page interior `(648,387,1023,936)`, each omission control is byte-equal
to the baseline page crop. Each combined-weight control equals the weight
crop. All repeated captures agree. Incremental public FreeType metadata shows
HGHT for Chinese and HGBZ for the baseline/default Latin glyphs; the weight
controls use HGHT and HGHZ. Because the original resources intentionally share
outlines, crop equality alone would not identify resource selection.

The initial combined-weight 8/9 traces were cached and therefore insufficient
for resource attribution. Fresh viewer processes reproduce both crops and
independently load HGHT glyphs 1/2 and HGHZ glyphs 1/3, matching the weight
control. Only appended trace lines after the initial unrelated document are
used for those observations. No vendor outline or bitmap data was extracted.

This supports the observed initial ordinary resource choice and these tested
weight/font combinations. It does not establish a universal bold bit, arbitrary
font values, punctuation/symbol role selection, or ignored semantics for other
controls. Keep the decoration resource separate, as established above. The next
integration work must preserve these explicit roles and resolve required
remaining controls rather than adding a blanket no-op branch.

External receipts: `caj2pdf-c8-render-preview-20261001/current-role-inventory.txt`
and `caj2pdf-c8-decoration-metrics-20261002/state-results.json`,
`state-fresh-results.json`, paired captures and incremental traces. Generated
inputs are in `caj2pdf-c8-state-controls-20261002`; all six new fixtures reproduce
byte for byte from the committed generator, and omitted control tags are absent.
These are original control observations, not complete-document compatibility
passes. Source documents, captures, external fonts and traces remain outside Git.


## Symbol resource roles and nonzero control payloads (2026-10-02)

The original `symbol-role-*` controls isolate six source codes, each repeated
on two rows at `1084`, with `801d` either 0 or 4 and `8067/6`. Unlike the
previous geometric-font experiments, this offline run uses the pinned viewer's
own installed resources. Only public font-call metadata and visible output are
observed; no glyph program or outline is extracted or bundled.

Actual raster-request loads identify these resource selections:

| Raw code | Verified Unicode | `801d/0` resource | `801d/4` resource |
| --- | --- | --- | --- |
| `a0a6` | U+FF06 | HGBZ | HGHZ |
| `aab3` | U+2217 | HGBZ | HGBZ |
| `aca3` | U+25BA | HGBZ | HGBZ |
| `a3a6` | U+FF06 | HGHT | HGHT |
| `a3aa` | U+FF0A | HGHT | HGHT |
| `a3ac` | U+FF0C | HGBZ | HGHZ |

The ampersand observation at state zero comes from the initial document;
reopening the same tab generated no new calls. The first comma run reached the
shim's bounded trace limit and is excluded from complete attribution. A fresh
process records both comma states and stable repeated page captures. Relevant
loads are distinguished from the numerous resource-initialization metric loads
by the observed raster-request flag and associated bitmap metadata. Resource
names describe this viewer execution, not redistributable font requirements.

These observations contradict a Unicode-only resource choice: both ampersand
codes have U+FF06 text but select different resources. They also contradict
applying the ordinary Latin state switch to every symbol. Preserve the raw code
until role selection, then emit its verified Unicode. A role and a character
must remain separate inputs to the existing PDF font/glyph path. Resource
identity alone does not establish glyph placement class, vendor-alias outline
identity or the appearance of a caller-supplied replacement font.

Eight additional original controls exercise required nonzero `8072` and `8074`
payloads: `1042/a3a8/a0f2` and `b4a2/d4b4/24a7/a1a1/a3a9`, respectively.
Each is inserted after row context and before the existing `中文AM1` glyphs.
At matched 57% zoom and interior `(648,387,1023,936)`, all eight page crops
are byte-identical to the baseline, with identical repeats. Together with the
previous zero-payload and independent control tests, this establishes that
these payloads neither draw extra characters nor change these following glyphs.
It does not establish their metadata meaning, arbitrary control tags or effects
on untested operations. Preserve located errors for unresolved required content.

The generator reproduces all twelve symbol-role inputs exactly. External
receipts are in `caj2pdf-c8-symbol-role-20261002` (incremental and fresh-process
traces, captures, `required-control-results.json`), with generated input manifests
in `caj2pdf-c8-symbol-role-generated-20261002` and
`caj2pdf-c8-required-controls-20261002`. These controls refine the production
resource contract; they are not a complete six-page conversion or fidelity pass.


## Native records now drive PDF page content (2026-10-02)

`write_c8_native_page` consumes the current C8 reader page and finishes a mixed
PDF content page. It directly connects the bounded record visitor to the
existing glyph, segment, decoration and image writer. Only current style,
ordinary Latin alternate state and image ordinal are retained. There is no
whole-page record/glyph vector or second renderer. Already embedded font
handles are reusable across pages; explicit role indices may share a handle.

The initial translation admits ordinary decoded ASCII alphanumerics and CJK
ideographs with their controlled geometry/resource roles, the three measured
segment styles, forward horizontal decoration with a caller-selected
nonsemantic alias, and the controlled raw image form. C8 origins are subtracted
without the text margin for images. The caller supplies existing decoded image
handles in descriptor order and their row representation (JPEG/type-0 false,
type-3 true); the image count must agree before content output. Whole-document
codec orchestration and CLI/Node/browser font transport remain #233/#252.

Unresolved glyph classes, controls, image prefixes and end payloads fail at the
source record's page/offset. In particular, the known same-Unicode symbol role
distinctions are not replaced by a CJK fallback. The API does not yet admit the
complete six-page profile. A checkpoint with that source and existing diagnostic
image row sidecars stops at page 1, byte 232, raw code `a3ba` (fullwidth colon):
its resource/placement translation remains unresolved. Finishing the PDF after
that error fails. This checkpoint is not a successful conversion and sidecars
are not the production codec path.

Original Rust fixtures now traverse actual native records into the PDF writer,
interleaving CJK/Latin glyphs, two images with opposite row representations, a
segment, a Latin resource switch and decoration. Three-byte source reads and
seven-byte sink writes exercise short I/O. Unknown content, missing glyphs or
resources, invalid image coordinates/prefixes, missing header fields, source
failure, output failure and cancellation cannot publish an unfinished page as a
valid PDF. Cancellation and output failure are also triggered during traversal,
after resource preparation. The reader retains its existing poisoned state.

The existing independent mixed-content raster test additionally renders this
native-record fixture with qpdf/MuPDF. At 741.9 DPI, one source unit is one pixel:
all four image quadrants prove that the second image covers the first in the
correct row orientation, and interior CJK/Latin/blank pixels are checked against
independent coordinates. An external Poppler extraction retains `中AAA` after
whitespace normalization and excludes decorative aliases. These checks exercise
the original fixture, not external-document fidelity.

### Controlled image trailing words

Two new original `c8_image_fixture.py` variants change all eight trailing
`c0xx` words, once to ASCII-like low bytes and once to zero/high-bit/extreme low
bytes. The image payload and geometry are unchanged. In the pinned viewer at
971%, page interior `(648,518,1023,805)`, both variants equal the baseline crop
byte for byte and all repeated captures agree. This independently supports
rendering the tested trailing-word class without matching one literal payload.
Other high prefixes remain rejected; these words are never opened as paths or
URLs and their metadata meaning is not inferred.

External inputs are in `caj2pdf-c8-image-tail-20261002`; captures and
`image-tail-results.json` are under `caj2pdf-c8-symbol-role-20261002`. The real
source checkpoint is in `caj2pdf-c8-render-preview-20261001` as
`native-page-checkpoint.txt`; its partial PDF is explicitly incomplete. No
external document, image/font data, extracted text or capture is committed.

## Fullwidth colon and original resource markers (#240)

The `role-markers` variant of `tools/cajviewer/c8_geometric_font.py`
contains only original rectangular outlines. Each resource has the same outer
em square and advance; a different interior white notch identifies the resource.
Its diagnostic format-13 cmap maps every BMP alias to that original outline.
This is a viewer probe, not a production font or an expansion of the Rust font
reader's supported cmap formats. No vendor outline is used.

`c8_style_fixture.py` reproduces 52 original `role-*` controls, ten
`role-sizes-*` controls and `role-colon-heldout`. All 63 byte streams reproduce
the observed external inputs exactly. Fixed CJK/Latin anchors separate resource
selection from glyph placement. These controls are an inventory, not blanket
admission of all tested codes. In particular, preliminary horizontal-offset
hypotheses for other punctuation do not explain all size observations and are
not implemented.

For raw `a3ba` (U+FF1A), the field-4 weight controls select the ordinary Latin
resource for `801d/0` and the alternate Latin resource for `801d/4`. Placement
is separate from that resource choice. Starting from the existing CJK matrix,
leave x unchanged and adjust the PDF baseline by:

```
y += em_height / 8 - 15 * EMPIRICAL_COORDINATE_POINTS_PER_UNIT
```

At the matched 337% view, six size fields 2/3/4/5/6/8 give target-minus-CJK
visible top offsets of 3/3/2/1/0/-2 pixels, with the same nominal horizontal
origin. This rejects a fixed vertical shift. The separately authored held-out
input changes position, uses alternate resource state, and exercises width/
height fields (7,7), (2,8), (8,2). Its SHA-256 is
`cb81b33cea4545ba4cc5f93648437128882c3a12f7771fef1c09ceee4e9b5816`.
At 429%, the corresponding top offsets are -1/-2/4 pixels; all three horizontal
displacements are 195 pixels for the authored 350-unit separation. The original
notch identifies the alternate resource in all three rows. Repeated captures
are byte-identical. All glyphs fit the viewport; the right page frame is clipped,
so this control establishes relative glyph placement, not full-page bounds.
The empirical model retains the previously documented raster-edge and physical-
unit limitations; these integer box measurements do not establish pixel parity.

Receipts remain external in `caj2pdf-c8-required-glyph-roles-20261002`, including
`size-component-measurements.json`, `colon-heldout-input.json` and
`colon-heldout-result.json`. Initial batches after viewer exit are excluded.
The later colon capture is populated and repeats identically; its anchor boxes
match the earlier size controls, but 302 edge pixels in the anchor crop differ.
It is not counted as an identical-anchor capture or used to claim pixel parity.

The native-page translator now admits this raw code using the existing explicit
font roles and matrix evaluator. Tests exercise the three held-out size pairs,
both resource states, emitted U+FF1A and explicit missing-glyph failure through
actual ranged record traversal and PDF output. This removes one required-record
blocker; complete six-page and public-adapter acceptance remain open.

### Shared baseline for required fullwidth digits and symbols

Seven further original `role-common-heldout-*` controls test all 41 codes below
with alternate resource state and independently varied width/height fields
(7,3)/(3,7). The authored positions differ from the field-4 controls. Every
capture repeats byte-identically; all reference and target glyphs lie within
the visible page. At the matched 315% view, every target is 163 pixels to the
right and 6 pixels below its CJK anchor. The authored differences are 400 raw
x units and the predicted 15 raw y units. The two em heights differ, while the
baseline displacement remains the same. Interior markers independently verify
the selected resources, including the five invariant-role symbols.

| Raw codes | Resource | Placement relative to existing CJK matrix |
| --- | --- | --- |
| `a0a6`, `a1aa`, `a1ad`, `a1ae`, `a2d9..a2df`, `a3a3`, `a3a5`, `a3ab..a3b9`, `a3bb..a3bf`, `a3dc`, `a3fb`, `a3fd` | Active ordinary/alternate Latin | x unchanged; PDF y minus 15 coordinate units |
| `a1c6`, `a1c8`, `a9aa`, `aab3`, `aca3` | Ordinary Latin, independent of the alternate state | Same placement |

The source-size controls for `a0a6` and `aca3` additionally vary the six required
size fields. The new held-out `aca3` observation is valid independently of the
excluded earlier black capture. None of these observations admits the other
punctuation offsets, nor do they equate source aliases with emitted Unicode.
The same decoder remains responsible for the verified Unicode mappings.

The generator reproduces all seven input byte streams exactly. External
`common-heldout-inputs.json` pins each hash and the prediction recorded before
capture; `common-heldout-results.json` records every glyph box, resource marker
and repeat result under `caj2pdf-c8-required-glyph-roles-20261002`.

The native translator groups these codes explicitly and reuses the existing
matrix/font/PDF path. Tests traverse every admitted code in both ordinary and
alternate states with both independent-size configurations, checking resource
operators, Unicode output and serialized baselines. Missing or unknown required
content remains an error. With an explicitly supplied broad Unicode font reused
for the ordinary roles, the real six-page diagnostic now reaches page 1 byte 280
(`a3a8`, opening parenthesis); its partial PDF remains unfinished. This checkpoint
still uses previously decoded image sidecars and does not claim production
orchestration, public font transport or complete-document acceptance.

### Parentheses, ideographic space and comma

The earlier em-fraction hypothesis is insufficient for raw `a3a8`/`a3a9`
(fullwidth opening/closing parentheses). Original `paren-detail-*` controls
compare identical marker outlines across all seven admitted size fields at a
matched 2233% view. The measured offsets are expressed in source coordinate
units relative to the existing CJK matrix, not tuned to the external document:

| Size field | Opening x | Closing x | Downward y (both) |
| --- | --- | --- | --- |
| 2 | 18 | 16 | 3 |
| 3 | 19 | 18 | 1 |
| 4 | 22 | 21 | 0 |
| 5 | 26 | 25 | -4 |
| 6 | 30 | 28 | -7 |
| 7 | 35 | 33 | -10 |
| 8 | 39 | 37 | -14 |

A single small table supplies x from the width field and y from the height
field, after the existing matrix evaluator validates both fields. Both codes
use the active ordinary/alternate Latin resource. No new public geometry API,
font lookup or document-specific correction is introduced.

Before admitting this table, `paren-axes-heldout` changes the authored position,
uses alternate resource state and independent width/height pairs (3,7), (7,3),
(2,8), (8,2). At 919%, its opening/closing target-minus-reference box origins are
(261,-12)/(498,-12), (280,1)/(516,1), (260,-17)/(495,-17) and
(285,3)/(520,3) pixels for authored horizontal separations of 200/400 units.
All resource markers identify alternate Latin; the CJK reference retains its
own marker. Repeat captures are identical. All glyphs fit the held-out page.
These observations validate independent axes within the empirical coordinate
model, not pixel equality between different PDF and viewer rasterizers.

Some high-magnification detail captures clip the *right edge* of the target:
opening field 8 and closing fields 7/8. Only their visible top/left origins are
used for the offset measurement; their clipped widths are not accepted as
extent evidence. The held-out independent-axis page contains complete glyphs
for these sizes. Earlier unmaximized or improperly positioned observations are
not used to derive the table. External `paren-detail-measurements.json` and
`paren-axes-results.json` retain the exact boxes and limitations.

`space-comma-axes-heldout` independently substitutes raw `a1a1` (U+3000) and
`a1a2` (U+3001), with the same four independent-axis configurations. The space
uses the CJK resource and origin under both ordinary and alternate states;
it is emitted as Unicode text, never unconditionally skipped. A caller font
may naturally have an empty space outline. The comma uses the active Latin
resource and the existing Latin baseline, but retains the CJK x origin. Thus
it reuses the Latin matrix with its horizontal em/8 addition removed.
At 919%, space-minus-CJK origins are (238,0) on every row; comma-minus-CJK origins
are (475,3), (475,10), (475,1), (475,10). Resource markers are CJK/CJK/alternate
Latin on every row, and full-page captures repeat identically. This agrees
with the earlier six-size comma controls without introducing a new size model.

The generator reproduces all 16 new inputs exactly. Predictions, input hashes,
action receipts and observations remain external in
`caj2pdf-c8-required-glyph-roles-20261002`, including
`paren-axes-prediction.json`, `space-comma-prediction.json` and
`space-comma-results.json`. Tests run actual native traversal and PDF emission,
checking Unicode, both resource states, independent axes and invalid size fields
before table indexing. The real-source diagnostic now reaches page 1 byte 2432;
remaining required symbols/controls and full-document orchestration stay open.

### Remaining required brackets and quotation marks

The original marker controls now cover all 52 required non-Han, non-ASCII-
alphanumeric raw codes in the pinned six-page inventory. This is record-level
admission, not complete-document support. The remaining additions reuse the
existing matrix, resource roles and bounded PDF path:

| Raw codes | Resource | Placement relative to CJK matrix |
| --- | --- | --- |
| `a3db`, `a3dd` (fullwidth square brackets) | Ordinary Latin, including alternate state | Size-table x/down offsets below |
| `a1b0`, `a1b1` (double quotation marks) | Active Latin | Existing closing-parenthesis offsets |
| `a1af` (right single quotation mark) | Active Latin | Small-mark x below; down 15 coordinate units |
| `a1a4` (middle dot) | Active Latin | Same small-mark x; existing colon baseline model |

| Size field | Square-bracket x | Square-bracket down | Small-mark x |
| --- | --- | --- | --- |
| 2 | 24 | 1 | 7 |
| 3 | 27 | -1 | 7 |
| 4 | 30 | -3 | 8 |
| 5 | 36 | -7 | 10 |
| 6 | 41 | -10 | 11 |
| 7 | 48 | -15 | 13 |
| 8 | 54 | -18 | 15 |

`bracket-detail-*` and `single-quote-detail-*` measure all seven fields with
original full-em marker outlines at 2233%. Their repeats are identical. As in
the parenthesis experiment, only visible top/left origins are used where the
right edge is clipped (bracket fields 6/7/8 and single-quote field 8); no clipped
extent is admitted as a complete glyph width. The independent-axis controls
below show complete glyphs at these sizes.

Three separately authored controls change position, use alternate state and
width/height pairs (3,7), (7,3), (2,8), (8,2). At 919%, target-minus-CJK origins
for their two columns are:

- `bracket-axes-heldout`: (270,-18)/(507,-18), (295,-2)/(532,-2),
  (266,-22)/(504,-22), (302,1)/(540,1). Both markers select ordinary Latin.
- `quotes-axes-heldout`: (259,-12)/(497,-12), (277,1)/(515,1),
  (257,-17)/(494,-17), (282,3)/(519,3). Both select alternate Latin.
- `marks-axes-heldout`: (246,-3)/(484,17), (253,7)/(491,17),
  (246,-5)/(484,17), (255,8)/(493,17). Both select alternate Latin; the
  horizontal correction is shared while their baselines remain different.

All page captures repeat identically. Predictions, input hashes, complete boxes
and role observations remain external under
`caj2pdf-c8-required-glyph-roles-20261002` in `bracket-axes-prediction.json`,
`remaining-marks-inputs.json`, both detail-measurement files and
`remaining-marks-axes-results.json`. The simplified fixture-generator loops
reproduce all 103 original inputs in that experiment directory byte-for-byte.
The empirical physical-unit and raster-edge limits remain; these observations
do not establish exact cross-renderer pixel parity or a general shaping model.

The actual-record tests check independently varied axes, ordinary/alternate
resource selection, explicit Unicode and retained rejection of unverified raw
variants (including `a3a6`, despite its Unicode matching admitted `a0a6`).
The real-source run first exposes a missing U+2217 in the diagnostic caller
font at byte 4256; this remains an explicit product error. An explicitly
configured external diagnostic font adds U+2217/U+25BA from an installed font,
with input/output hashes retained outside Git. It is not bundled, automatically
selected or used to claim source-font fidelity. With that resource, traversal
reaches **page 1 byte 4492, control `8072/1042`**. Its effects on the following
segment require verification; it remains rejected. No partial PDF is finished.


### Mixed-page control and end-record checkpoint (2026-10-02)

Original `c8_image_fixture.py` controls combine glyphs, all three admitted
segment styles, decoration and an asymmetric JPEG. The 23 control variants
cover admitted 8072/8073/8074 values and c053/c054 payload boundaries.
Eight additional variants use end payloads 0, 39 through 44, and 65535.
Pinned offline Viewer captures of the complete page interior at fit-width
repeat identically and differ by zero pixels from the baseline end-1 page.
The fixed interior is 376 by 564 pixels; no registration, masks or tolerance
are applied. A fresh-process baseline also matches the first batch.
External receipts are `mixed-control-results-batch1.json`,
`mixed-control-results-batch2.json`, and `mixed-end-results.json` under
`caj2pdf-c8-required-glyph-roles-20261002`. External captures stay outside Git.

The composer treats admitted controls and the end payload as nonpainting.
The reader still requires the end record to terminate the indexed span
exactly and validates image counts. Raw end payloads remain observable;
this does not assign semantic meaning to them. Regression tests preserve
explicit rejection of premature ends and unverified control values.

The pinned six-page source now completes diagnostic native-page composition.
qpdf reports no syntax/stream errors; MuPDF reports six pages and the three
expected image dimensions (848x251, 866x388, 666x172). This uses explicitly
supplied external diagnostic fonts and decoded image sidecars. It does not
prove source-font fidelity, complete visual parity, production codec
orchestration, or public CLI/Node/browser acceptance. Those remain #233/#252.


### Shared-codec document checkpoint (2026-10-02)

`convert_c8_native_pdf` now orchestrates fonts, the existing image preflight
and emitters, and the streaming native-page writer. `C8FontSources` supplies
one to four ranged sources plus role indices; several roles may reuse one
source. Each distinct supplied resource is embedded once per document.
No image sidecars, new codecs, font discovery or document-content buffers
are required. Per-page storage consists of image handles and orientations;
existing decoder scratch and PDF indexes retain their existing budgets.

An external run of the pinned six-page source completes with two type-0
images, one type-3 image and four text-only pages. It explicitly opts into
the existing HN/C8 unused-refinement-template policy. qpdf accepts the PDF;
all six page rasters at 72 DPI match the previous sidecar-based diagnostic
exactly, without registration or tolerance. This validates orchestration
against that diagnostic, not independent CAJViewer fidelity. Explicit
external fonts remain diagnostic resources, with ordinary roles sharing
one font and decoration using another. Output is 13,782,485 bytes, compared
with 37,700,531 bytes when the diagnostic embedded the same ordinary font
three times. These figures are not peak-memory measurements.

The report's page-metadata and row-store counters concern those specific
allocations; they are not whole-process/WASM peaks. Native record traversal
uses fixed buffers rather than a decompressed text allocation. The existing
CLI/Node/browser entry points still need #252 resource transport before
this core entry point establishes public-adapter support.

### Public-runtime checkpoint (2026-10-02)

The same pinned six-page source now completes through the native CLI, public
Node `convert()` and a real Chromium Dedicated Worker. All three produce
SHA-256 `907209917a1813afaf3786fad9278bf23f152b1580ca14fdbe448e8417660eea`
with the same explicit external diagnostic font resources. qpdf accepts the
CLI/Node PDF. No image sidecars are used. Runtime agreement is separate from
independent source fidelity and does not prove source-font identity.

Node uses FileHandle sources and temporary scratch. The browser diagnostic
spools forward-only document/font response streams into OPFS with the existing
64 MiB per-source limit, uses ranged sources for conversion and streams output
to an OPFS handle. Its completed output is hashed only after conversion.
All spool, decoder scratch and output files are removed; the final OPFS entry
list is empty. An earlier diagnostic using `Response.blob()` for the larger
font failed while consuming its response body; that run is excluded.

Both JS runs use 65,536-byte maximum observed read/write requests. The WASM
linear-memory high-water observed at I/O boundaries is 2,162,688 bytes; this
is not a total browser/process peak measurement. Receipts remain external:
`native-node-six-page-checkpoint.json` and
`native-browser-six-page-checkpoint.json` in
`caj2pdf-c8-render-preview-20261001`. CLI font-path syntax, output protection
and substitution limits are documented in `docs/cli.md`.

### Paired HN-B/C8 geometry controls (issue #241)

At HN-B framing checkpoint `210c846`, three original single-page control
streams were wrapped separately in C8 and compact HN-B containers. They use
original geometric fonts, not vendor glyph outlines. The controls cover:

- A fourteen-row style grid: `0000`, `04e7`, `0842`, `0884`, `0c84`, `0cc6`,
  `1042`, `1064`, `1084`, `10a5`, `14e7`, `e58c`, `08a5`, and `10c6`.
- Style zero with independent `8070/0024` and `8071/0024` axis controls.
- A nonzero header-origin change, with unchanged glyph coordinates.

In the pinned offline viewer at 57%, each C8/HN-B pair has identical pixels
inside the independently identified page interior `(648, 387, 1023, 936)`;
each capture also matches its repeat. The earlier comparison crop started
above this single-page frame and omitted part of the blank footer; the
corrected full-interior comparison retains all three matches. External input
hashes, screenshot hashes and comparisons are in
`caj2pdf-hnb-rendering-20261003/geometry-controls.json` and
`geometry-comparison.json`. External captures and fonts remain outside Git.

These controls establish agreement between the two container interpretations
for these streams. They do not establish PDF fidelity, a formula for new size
fields, or complete HN-B support. In particular, style zero visibly renders
glyphs and cannot be discarded as empty content. The next implementation
checkpoint must independently verify the missing style/axis metrics, character
mappings and required drawings before admitting the complete issue-100
document to the shared renderer. Preserve existing image-only HN-B geometry
until a change to that path has its own evidence.

A subsequent single-variable HN-B axis experiment retains positions and original
fonts: style zero alone, width-only `8070/36`, height-only `8071/36`, both axes
72, and style `1084` with both axes 72. All five captures repeat exactly.
At 57%, thresholded first-row bounds relative to the same page interior are
respectively `(40,29,147,32)`, `(40,29,150,32)`, `(40,29,147,35)`, and
`(40,29,157,42)` for both final cases. The two final page interiors are also
pixel-identical. This independently demonstrates axis-specific sizing and
that explicit axes can override a nonzero style; treating these controls as
ignorable would lose visible geometry. These screen bounds are diagnostics,
not PDF-unit constants. Derive physical sizing and baseline behavior before
extending the production transform. External predictions, original source
hashes and measurements are `axis-controls.json` and `axis-comparison.json`
in the same receipt directory.

The follow-up explicit-size controls establish a state transition: explicit
width/height 35 at style zero matches style `1084` pixel-for-pixel, including
Latin positions. Applying `8002/1084` after explicit axes 72 restores that same
page, so a subsequent style record resets the explicit axes. Repeated captures
are stable. The proposed field-12/explicit-112 equivalence fails and must not
be admitted from this experiment. Receipts are `axis-model-controls.json` and
`axis-model-comparison.json`; `tools/cajviewer/hnb_geometry_fixture.py`
reproduces all eleven original input byte sequences from these two experiments.
These controls support reusing verified metrics with explicit mutable axis
state, but do not establish arbitrary-size baseline rounding.

At confirmed 971% zoom, original 300×250-unit HN-B anchor pages compare
explicit 35/36 with the admitted field-4 geometry. Explicit 35 and field 4
retain identical source interiors. A separate PDF written through the existing
Rust font/PDF API uses `axis * 75 / 301` points and the existing empirical
Latin baseline model. A fresh viewer session checks both values with repeats.
Relative CJK top/left differences (PDF minus source) are `(-1,-1)` for 35
and `(-2,-1)` for 36; Latin top differences are 0 and -2 pixels. These are
thresholded diagnostics at independently identified page frames, not alignment
corrections. The known size also has edge residuals, but that does not prove
the extra vertical residual for 36 is renderer-only. Keep this qualification
when evaluating the explicit-axis model. Receipts are
`caj2pdf-hnb-rendering-20261003/axis-validation-comparison.json` and the
`axis-validation-viewer` repeated captures. No source-specific offset is added.

### Additional field-7 styles

Original HN-B controls `04e7`, `14e7`, and `10e7` produce pixel-identical
page interiors at 57%, including CJK and Latin resource-marker glyphs; each
repeats identically. Earlier paired C8/HN-B controls also agree for these
styles. The shared glyph transform therefore admits exactly `04e7` and
`14e7` using its existing field-7 metrics. Other sizes with these high bits
and decoration states remain rejected. The original generator preserves these
three controls; external hashes and repeated comparisons are in
`caj2pdf-hnb-rendering-20261003/style-equivalence-{inputs,comparison}.json`.

The same experiment finds `e58c` equivalent to explicit axes 110, while
style zero differs from explicit axes 16 in Latin placement. Those findings
do not yet extend the transform's admitted size fields. Complete HN-B
rendering remains open in #241.

### Effective style inventory for the first HN-B document

A bounded native-record traversal of the pinned issue-100 input tracks axis
controls and resets them at each `8002` style record. All 38 style-zero glyphs
on page 1 occur with explicit width and height `0x24` (36); there are no
default-zero glyphs in this document. The same page has twelve `e58c` glyphs
without explicit axes. Remaining pages use ordinary size fields. This narrows
the first-document implementation to explicit 36 and the observed large size,
in addition to remaining character/drawing rules; do not delay that delivery
for default-zero baseline calibration. The external count-only result is
`caj2pdf-hnb-rendering-20261003/effective-style-inventory.txt` and contains no
extracted source text.

Separately, original 300×250-unit controls at 971% distinguish default zero
from explicit 16 and establish pixel-identical interiors for default zero and
explicit 21, including Latin placement. Repeats are identical. This corrects
the low-zoom ambiguity; it does not yet admit default-zero PDF geometry.
The existing generator now reproduces all six high-zoom anchor inputs exactly.
Receipts are `axis-zero-anchor-comparison.json` and
`axis-zero-heldout-comparison.json` in the same external directory.

### Large-size discriminator supersedes low-zoom equality

The earlier 57% equality between `e58c` and explicit axes 110 is insufficient
to identify the size. Original single-CJK controls on a 400×400-unit page at
confirmed 729% separate them: thresholded glyph bounds relative to the same
page frame are `(37,6,298,265)` for `e58c` and `(37,6,301,268)` for explicit
110. Repeats match. Held-out explicit values 108 and 109 distinguish the
remaining candidates: 109 has a pixel-identical page interior to `e58c`; 108
does not. Use 109 for subsequent model validation, not the rejected 110.
External receipts are `large-anchor-comparison.json` and
`large-anchor-heldout-comparison.json` under
`caj2pdf-hnb-rendering-20261003`; no production size admission is made yet.

A count-only character-class inventory additionally confirms that all twelve
`e58c` glyphs in issue-100 are Han. Its 38 explicit-36 glyphs consist of thirty
ASCII alphanumerics and eight unresolved raw codes. This identifies which
baseline/resource rules are required without extracting or committing text.

The independent PDF control with size `109 * 75 / 301` points retains
source top/right boundaries at 729%; its thresholded left/bottom edges differ
by one pixel (source `(37,6,298,265)`, PDF `(36,6,298,266)` relative to the
same frame). Both repeats are stable. Glyph color is intentionally different
in this geometric experiment; no pixel-equality claim is made. Receipt:
`large-anchor109-pdf-comparison.json`. The shared transform now admits exact
style `e58c` for CJK placement using size 109 and the existing origin model.
Latin placement, other field-12 styles and decoration states remain errors.
This supplies the measured large-Han geometry needed by issue-100, not full
HN-B rendering.

### HN-B symbol-copy candidates

An original seven-glyph control brackets unresolved `a0ae`, `a0af`, `a0ba`,
`aab1`, `aab2` with known A/M glyphs. Ordinary selection and Ctrl+C after a
fresh clipboard sentinel returns, respectively, U+FF0E, U+FF0F, U+003A,
U+2219 and U+002D, with inserted spaces. A separately generated reversed
control after a different sentinel reverses the complete sequence. The
existing generator reproduces both source byte sequences. External source
hashes and exact copied code points are in `unknown-symbols-input.json`,
`unknown-symbols-reversed-input.json` and `symbol-copy-comparison.json` under
`caj2pdf-hnb-rendering-20261003`. These are original test characters.

These are semantic candidates, not production mappings: prior C8 controls
show viewer-copy punctuation normalization. Corroborate visible glyph identity
and resource/placement before admitting the symbols to rendering. No arbitrary
A0 punctuation range or private-use Unicode fallback is added.

The follow-up visible control uses the same seven authored raw codes with a
smaller canvas and fixed positions. A separate offline viewer session retains
its normal font resources (no extracted outlines or font files enter Git). At
291%, repeated captures show period, slash, colon, dot and short horizontal
stroke in the expected order, bracketed by A/M. Together with the fresh-sentinel
forward/reversed copy controls, these establish explicit mappings `a0ae` →
U+FF0E, `a0af` → U+FF0F, `a0ba` → U+003A, `aab1` → U+2219 and `aab2` →
U+002D. The decoder now preserves these exact scalars, including the fullwidth
forms, instead of inferring an ASCII punctuation range. This is character
identity evidence, not verified resource/placement or complete rendering.
External captures: `caj2pdf-hnb-rendering-20261003/symbol-visible-viewer`; the
original `unknown-symbols-visible.caj` stays in the external input directory.

Original full-em resource-marker controls establish that all five added
symbols select the active Latin resource, including after `801d/4`; their
baseline differs from ordinary A/M exactly as the existing `a0a6` symbol
class does. Two independent unequal-axis pages (`1067`, `10e3`) replace only
these five raw codes with `a0a6`, preserving coordinates and fonts. Both
source/reference page interiors are pixel-identical, and all repeats match.
External hashes and comparisons are `symbol-axis-comparison.json`; regular
and alternate resource captures are in the same `symbol-viewer` directory.
The native page writer therefore reuses its active-Latin, CJK-origin, zero
baseline-fraction branch for these five codes. Existing Unicode/resource/
unequal-axis PDF tests include them. No offsets, allocations or new rendering
abstraction are introduced. HN-B page admission remains separately incomplete.

### Explicit 36-axis composition checkpoint

The shared page writer retains two optional axis words. The independently
controlled `8070/36` and `8071/36` pair overrides the active style; `8002`
resets both. Glyph composition uses the existing empirical origin model,
`36 * 75 / 301` points per em and the observed adjacent-size Latin baseline
offset of eight source units. Original source/PDF anchor measurements and
their 1–2-pixel residuals remain recorded above; no pixel compensation or
parity claim is added. Paired C8/HN-B original controls establish this same
record behavior; the C8 visitor now also admits these exact control values.

An incomplete pair, other explicit values, unverified size-dependent
punctuation offsets and explicit-axis decorations remain located errors.
Large-style non-Han glyphs are likewise refused before indexing ordinary-size
offset tables. Tests cover actual sequential PDF matrices, axis order, reset
to ordinary style, partial pairs and failure cases with short reads/writes.
The state is constant-size and the public ordinary-style helper is unchanged.
HN-B full-page admission still depends on remaining drawing/control integration.

### Verified `a385` line start marker

The first issue-100 page contains `a385` endpoints `(53909,5026)` and
`(9235,5026)`. Treating 53909 literally would draw from outside its canvas.
Original HN-B controls compare `a381`, plain `a385`, and `a385` with only
`c000` set on the first x word. Their horizontal interiors are identical at
146%. A held-out small-page diagonal with different endpoints agrees at 729%;
all repeats match. Separately authored C8 wrappers for the ordinary and marked
diagonal also match the HN-B reference. Hashes and captures are recorded in
`line-controls.json`, `line-diagonal-inputs.json`, `line-comparison.json`,
`line-diagonal-comparison.json` and `line-diagonal-c8-comparison.json` under
`caj2pdf-hnb-rendering-20261003`. The generator reproduces all six HN-B inputs.

The existing segment evaluator now admits `a385`, clearing the paired `c000`
bits only in its first x coordinate. Other words and other styles retain raw
values. It shares the independently established endpoint/origin and hairline
output with ordinary segments. Original regressions cover both geometries and
retain distinct behavior for another style's high coordinate. Raw inspection
records are unchanged; no blanket coordinate mask or new renderer is added.

### HN-B skew state and independent-axis discriminator

Original single-glyph controls confirm `8024/281d` activates a visible tilt,
`8024/2800` restores the baseline exactly, and a subsequent `8002/1084` does
not reset the tilt. All 729% page interiors repeat identically. Two unequal
size controls (`1067`, `10e3`) distinguish the transform's axes: the narrower,
taller glyph shifts near its top by about 18 screen pixels, while the wider,
shorter glyph shifts by about 31. Thus the displacement follows width, not
height. Small-glyph edge differences alone do not establish horizontal
compression; the larger control below distinguishes that hypothesis. Do not
infer an angle directly from the payload or discard this control as a no-op.

The existing generator reproduces all eight inputs. Receipts are
`skew-inputs.json`, `skew-state-comparison.json`, `skew-axis-inputs.json`,
`skew-axis-comparison.json` and repeated `skew-viewer` captures in
`caj2pdf-hnb-rendering-20261003`. These are original full-em font controls;
no external document text or outlines enter Git.

### Held-out large-glyph skew measurement

The same original control at the independently established `e58c` size
(`109 * 75 / 301` points per em) retains its approximately 262-screen-pixel
width while shifting the top rightward by about 63 pixels at 729% zoom.
This contradicts a horizontal scale correction inferred from the smaller
rasterized glyphs. The generator includes both large baseline and tilted
controls as `skew-axis-e58c-{base,skew}.caj`.

An independently constructed PDF uses the existing original full-em marker
font, gray 68, the established origin and baseline, and the matrix
`[width, 0, width * 0.24, height, x, y]`. Source and PDF top/left positions
agree; the PDF's right and bottom edges differ by one screen pixel. Repeated
captures are stable. No compensating translation or horizontal scaling is
introduced. This supports a width-relative shear for this exact control;
it does not establish arbitrary `8024` payloads, skewed decorations or mixed
image behavior, and production admission remains separate.

External receipts in `caj2pdf-hnb-rendering-20261003` are
`skew-large-inputs.json`, `skew-large-comparison.json`, and
`skew-large-pdf-comparison.json`, with repeated `skew-viewer` captures.
The original source controls have SHA256 values
`737f359f84cdfaebfec360fc0a3fda3603009bcd3e7525d96b0a12cb1aefb046`
(baseline) and
`0234a4a08bf8ce3e1a0188a58a3be642d146620da7e76ea86f23287927a0e520`
(tilted). These controls use synthetic glyph outlines and contain no copied
source-document content.
