# Unreleased

Changes after [v0.6.1](v0.6.1.md). [v0.6.2](v0.6.2.md) was cut from v0.6.1 on
`release/v0.6.x` and contains only the Worker fix (#517), which `main` also
carries; everything below ships in v0.7.0:

### Breaking CLI inspection value: TEB support reason (#469)

For TEB, `inspect --json` now reports `"unsupported_reason":"not-implemented"`
in place of `"drm-encrypted"`. Update callers matching `drm-encrypted` to
accept `not-implemented`; `format: "TEB"`, `conversion_supported: false` and
unknown page/outline metadata are unchanged. CLI text and Node/browser errors
now say conversion is unsupported. Recognizing a TEB prefix does not prove
encryption, corruption or impossibility of recovery. JavaScript retains
`UnsupportedFormatError`, `code: "UNSUPPORTED_FORMAT"` and `format: "teb"`;
callers should match these fields instead of diagnostic prose. No supported
conversion/PDF output, I/O limits, dependency or core/WASM API changes.


### Breaking Rust API: native symbol glyph maps (#518)

`caj2pdf_core::Fonts` gains `symbol_glyphs: Vec<NativeSymbolGlyph>` and now
implements `Default`; `hnc8::C8FontSources` gains
`symbol_glyphs: &[NativeSymbolGlyph]`. Each entry selects the `symbols`
font character-map entry drawn for one raw HN-B mode-0 symbol code
(`hnc8::is_mode_zero_symbol`) while extracted text keeps the decoded
character. Codes that decode alike can draw distinct glyphs, and one glyph
can carry different text. Rust callers building these structs with literals
add `symbol_glyphs: Vec::new()` / `symbol_glyphs: &[]`, or use
`..Fonts::default()`. An empty map leaves output byte-identical.

A non-empty map must name distinct symbol codes, needs a `symbols` role
whose font maps every glyph, and applies only to HN-B mode-0 native text.
Native conversion refuses anything else before a PDF is finished, with no
fallback to another glyph or font. Like the fonts themselves, the map is
unread for documents routed to image composition. A `symbols` source shared
with other roles is embedded as one mapped font for all of them. The core
ships no mappings. Binding a map to a pinned resource identity and
verification against measured source resources remain open in #518. No fonts
are bundled, fetched or inferred.

### Symbol font identity binding (#518)

A map can be bound to the font it was measured on. Rust `Fonts::symbol_font`
and `C8FontSources::symbol_font`, CLI `--symbol-font-identity NAME:CHECKSUM`,
JavaScript `symbolFontIdentity` and raw WASM `caj2pdf_hnb_set_symbol_font`
all take the font's PostScript name and `head` checkSumAdjustment. A
different `symbols` font is refused before output is finished, and an
identity without a map is refused. The check uses retained metadata, so no
full-font hash is computed. This guards against using the wrong font by
mistake, not against deliberate tampering: a tool that edits outlines but
leaves the checksum field unchanged would still pass. Rust struct literals
of `C8FontSources`, and `Fonts` literals without `..Fonts::default()`, add
`symbol_font: None`.

### CLI and JavaScript symbol glyph options (#518)

The CLI accepts repeatable `--symbol-glyph CODE=CHAR` (four hex digits, then
one BMP character or `U+XXXX`) with `--font-symbols` or `--fonts DIR`.
Node and browser accept `hnc8.fonts.symbolGlyphs`, an iterable of
`{ code, glyph }` that requires `symbols`. The raw WASM ABI adds
`caj2pdf_hnb_add_symbol_glyph(code, glyph)` after the role calls. All three
use the same core map: the listed codes draw exactly the chosen glyph, and
text keeps the decoded character. Existing options and outputs are unchanged.

### Breaking Rust API: original programs and ordinary glyph aliases (#541)

`Fonts` and `C8FontSources` gain `embedding` and `glyph_aliases`. For previous
behavior, use `FontEmbedding::Subset` and an empty `Vec`/slice, or
`..Fonts::default()`. `write_c8_native_page` takes an additional alias slice
after decoration glyphs; pass `&[]` when no aliases are needed. Nonempty
`NativeFontGlyphAlias { font, character, glyph }` lists must be strictly sorted
by source index and BMP character; their target glyphs must exist. Aliases
preserve semantic text and do not mutate embedded font cmaps.

CLI adds `--font-embedding subset|full` and repeatable
`--font-glyph ROLE:CHAR=GLYPH` with explicit font resources. Node/browser add
`hnc8.fonts.embedding` and `glyphAliases: [{ role, character, glyph }]`.
Raw WASM adds `caj2pdf_c8_set_font_embedding` and
`caj2pdf_c8_add_font_glyph` after base-role registration. Defaults retain
subsetting and empty maps. Complete TrueType programs preserve original bytes
(or selected TTC face tables) using bounded reads, with larger PDFs; used CFF
full embedding is refused. See [CLI usage](../cli.md) and
[host configuration](../io-architecture.md). Remaining Wine differences are
tracked in #541; using original fonts does not establish pixel parity.

### Breaking PDF geometry: HN-B mode-0 page padding (#541)

Native mode-0 page extents now add 10 PDF points after converting raw dimensions
with the measured `72/742` point unit. Previously they added 100 raw units,
about 9.7035 points. Width and height each increase by about 0.2965 point, and
bottom-origin drawing coordinates follow the corrected page height. This
applies to native Rust, CLI, Node and browser, with either subset or full fonts.
Update stored page-box expectations and PDF hashes by reconverting mode-0
inputs; there is no new option or API field. Font bytes and semantic text are
unchanged. Mode-2 pages and other conversion routes are unaffected. Ten original
Wine boundary comparisons match, while four actual mode-0 pages retain their
previous 75% raster. Remaining glyph placement differences are tracked in #541.

### Native mode-2 ASCII digit placement (#541)

Raw A0B0–A0B9 digits in ordinary Latin placement now use their recorded origin
instead of the alphabet's horizontal and vertical offsets. Font selection,
explicit glyph aliases and semantic ASCII text remain intact. This changes PDF
coordinates/hashes for affected C8/HN-B input in native, CLI, Node and browser,
with both subset and full font programs; reconvert affected files. No option
or API change. CJK character mode, mode 0, fullwidth A3 digits and letters keep
their existing behavior. Independent Wine controls and three actual pages
improve; other alphabet and raster differences remain tracked in #541.

### Native mode-2 ASCII letter placement (#541)

Raw A0C1–A0DA and A0E1–A0FA letters in ordinary Latin placement now use the
character-dependent integer origins measured in Wine, replacing the previous
class-wide width compensation. Existing style/axis admission, font programs,
glyph aliases and semantic text remain intact. This changes PDF coordinates
and hashes for affected C8/HN-B input across native, CLI, Node and browser,
with both subset and full programs; reconvert affected files. No option or API
change. CJK character mode, mode 0, fullwidth A3 letters and digits keep their
separate placement rules. In the existing 77-page Wine cohort, 37 pages improve,
40 retain their prior rasters and none regress. All pages still have pixel
differences; device rounding and unequal-axis rasterization remain open in #541.

### Other unreleased changes

- Native C8/HN-B header, table and other admitted axis-aligned segments use
  gray-127 device hairlines to match Wine CAJViewer's lighter appearance
  (#544). Diagonals remain black; coordinates, font programs, semantic text
  and admission rules are unchanged. This changes affected PDF hashes across
  native, CLI, Node and browser; reconvert to receive the correction. No API,
  dependency or buffering change. All 77 observed pages retain or improve
  diagnostic agreement, with every changed region visually reviewed. Small
  vertical-coverage and overlapping-line raster differences remain.

- The Rust PDF writer adds `pdf::FontEmbedding` and
  `PdfDocument::embed_font_with_embedding(&handle, &mut font, FontEmbedding::Full)`
  (#541). Used standalone TrueType programs retain their exact bytes; TTC
  faces retain original glyph IDs and all rendering tables in a rebuilt
  standalone font, omitting DSIG and updating the checksum. Both ordinary
  Unicode and mapped glyph/text resources retain correct PDF widths and
  ToUnicode. Complete fonts use their original PostScript names, without a
  subset prefix. Reads are chunked and output sequential; an allocation
  never scales with the whole font. Full programs can greatly increase PDF
  size and are read twice, with source-digest validation between passes.
  Used CFF resources refuse `Full`; unused resources keep small subsets.
  Existing font-admission rules apply. `embed_font` and all conversion
  defaults still subset fonts. Native conversion, CLI, Node/browser and raw
  WASM now expose this option with external glyph aliases, as described above.

- Correct vertically inverted type-1 JPEGs in HN-A/C8 image and native pages
  (#530). Type 1 now draws top-first; type 2 retains its measured bottom-first
  placement. Encoded JPEG bytes, image bounds, APIs and memory limits are
  unchanged across native, CLI, Node and browser. Wine 7.3 controls and four
  affected pages verify orientation; residual raster differences remain open.

- The Rust PDF writer can register an explicit mapped font with
  `PdfDocument::add_mapped_font` and draw it with
  `ContentPageWriter::mapped_glyph(font, glyph_character, text, transform, gray)`
  (#519). A supported BMP font cmap character selects the outline; an
  independent Unicode scalar supplies extracted text through ToUnicode.
  Distinct glyphs may carry the same text, and one glyph may carry different
  text. Ordinary font registration and conversion defaults remain unchanged.
  Pair storage is bounded to 65,535 entries for TrueType or 65,534 for CFF,
  subject to `Limits`; fonts still use ranged input and sequential output.
  This additive low-level API does not yet provide native CAJ raw-code maps
  or CLI/JavaScript options. Those integrations and source-font verification
  remain in #518. No fonts are bundled, fetched or inferred.

- Accept the measured named local outline destinations in indexed PDFs (#449):
  byte-string keys in an indirect Dests name tree, resolving through indirect
  XYZ arrays to live pages. Validate complete ordering, Limits and graph shape
  under bounded metadata budgets; reject missing/ambiguous targets and
  unobserved named profiles. Clean PDFs remain byte-identical. Native, CLI,
  Node and browser share this core behavior, with no API or dependency change.

- Recover the measured redundant CAJ xref/header-copy suffix and two unused
  interruption profiles (#409, #434). The suffix must match original header
  bytes and terminate at a pending indirect Length object. Interrupted images
  require an independently anchored complete counterpart and exact prefix;
  unfinished openers contain no value bytes. Full graph validation must prove
  that neither is referenced. Referenced, ambiguous, encrypted, unmeasured or
  incomplete-graph cases remain errors.
- Allow at most 64 PDF whitespace bytes after a declared or resolved stream
  payload, requiring exact endstream/endobj tokens. Payload bytes and Length
  values are retained; unrelated stream-repair restrictions still apply.

Native, CLI, browser and Node.js share these core rules. There are no API,
dependency, license or publication changes. See the
[input profile](../pdf-input.md#redundant-caj-framing) and
[provenance](../provenance.md#redundant-caj-framing-409-434) for bounds and
external evidence. Other unresolved samples are not classified as impossible.

Verification: the unchanged 60-page original produces identical native, Node
and Chromium PDFs; qpdf, all 189 raw streams, 36 bookmarks and 60 independently
framed page renders agree. Source-viewer checks cover pages 1–3. The fresh
1,277-original native run adds one pass with no changed previous-success
output hashes (1,240 PASS, 28 FAIL, nine UNSUPPORTED). Ancillary warnings and
unexecuted checks are retained in the
[pinned report](https://github.com/rwv/caj2pdf-samples/blob/26720c5d76fbb05b39400a65b35b3974b31c1931/research/notes/redundant-caj-framing-20261008.md).

Recover the measured stream-byte expansion in the accepted 63-page issue-20
CAJ (#436). Only damaged simple Flate streams whose original checksums,
Lengths and all page-table anchors corroborate the inverse substitution are
changed. Native, CLI, Node and browser share the bounded ranged recovery;
there is no API or dependency change. Other codec payloads remain opaque.
See the [admission bounds](../pdf-input.md#checksum-confirmed-caj-stream-substitution)
and [production evidence](../provenance.md#bounded-production-recovery).

The unchanged original now produces identical native/Node/Chromium PDFs with
clean qpdf and 63 clean Poppler renders. All independently framed diagnostic
pages, object values and payloads agree; 93 bookmarks are retained. Previous
vendor checks cover modified-source pages 3/5/39 and the identical PDF.
Original damaged-source page 39 remains inaccessible, and no independently
obtained intact alternative or general corruption repair is claimed.


Recover three measured CAJs containing a complete empty Form inside an
identical same-number empty Form wrapper (#439). The bounded scanner retains
the inner object and omits only redundant framing; nonempty, conflicting,
incomplete and unmeasured profiles remain errors. Native, CLI, Node and browser
share the rule, with no API or dependency change. See the
[admission bounds](../pdf-input.md#nested-duplicate-empty-forms) and
[provenance](../provenance.md#nested-duplicate-empty-forms-439).

All three unchanged originals produce identical native/Node/Chromium PDFs;
211 independently framed page renders, 754 raw streams and 166 bookmarks
agree. Scoped source-viewer checks retain one initial cold-session raster
disagreement despite three matching retries; its cause remains open in #441.
No full-document vendor-render fidelity is claimed.

Recover three measured CAJs with interrupted same-ID object copies (#442),
including distant later copies established by page-table anchors, partial
boolean/number values and a terminal repeated header. One payload-free
unfinished Flate declaration is omitted only after complete-graph proof that
it is unused. Comparisons use bounded ranged I/O, preserve complete object and
stream bytes, and retain strict ambiguity and indexed-PDF behavior. Native,
CLI, browser and Node share the core implementation; no API or dependency
changes. See [bounds](../pdf-input.md#proved-interrupted-copies-across-caj-page-rows)
and [source evidence](../provenance.md#proved-interrupted-caj-copies-442).

Recover the measured 141-page Lambertian CAJ with indirect-Length constraints
hidden by provisional stream spans and distant interrupted copies in the same
page row (#444). Resolve known constraints in bounded batches before checking
duplicates and missing targets; require exact complete-scan proof for every
marker-derived counterpart. Complete objects, stream bytes and bookmarks are
preserved. Native, CLI, Node and browser share the core rule without API or
dependency changes. See [bounds](../pdf-input.md#indirect-lengths-and-distant-same-row-replays)
and [provenance](../provenance.md#indirect-length-ordering-and-same-row-replays-444).

Recover the measured 66-page KDH with a differently numbered empty Form nested
inside indexed object 485 (#446). Append an empty revision under the outer ID
only after bounded dictionary, exact geometry, tail, separate live-xref and
source-stability proofs. Keep every reference and the real inner ID's object;
#439's CAJ profile is unchanged. Native, CLI, Node and browser share the core
rule, without API or dependency changes. See [bounds](../pdf-input.md#indexed-nested-empty-forms)
and [provenance](../provenance.md#indexed-nested-empty-form-446).

The unchanged original produces identical native/Node/Chromium PDFs with
qpdf exit 0. All 66 pages and the other 304 raw streams match independently
decoded source evidence; there are no outlines. Scoped source-viewer page 47
matches, with a Form-content negative control. Original malformed-stream
warnings and decoder framing disagreements remain documented; no general
nested-object recovery or whole-document vendor-render claim is made.

### Breaking edge case: conversion and spool cleanup both fail (#454)

After a spooled conversion fails or is cancelled, `convertReadableStream`,
`convertReadable` and `convertSpooled` now report a failed removal as an
`AggregateError`. Previously these wrappers suppressed the removal error and
returned only the conversion error, potentially hiding a remaining temporary
file. `cause` and `errors[0]` preserve the original failure; `errors[1]` reports
cleanup. Callers checking `name`/`code` should inspect `cause` for the original
failure and also surface the cleanup error. Successful cleanup retains the
original error object. No successful output or native/CLI behavior changes.

Cancellation now changes the shared ACK counter before waking a backpressured
Worker. This prevents a notification from being lost between its cancellation
check and wait, allowing the Worker to close its input before spool removal.
The fix preserves bounded writes and cancellation; no timeout, cleanup retry
budget or empty-directory assertion is relaxed. A deterministic real-Chromium
control reproduces the previous lost wakeup and verifies closure after repair.

Recover the measured 53-page CAJ with one unused interrupted XML metadata
opener and five payload-free missing-parent openers (#452). Require exact
boundaries, complete-graph role proofs and explicit inheritable page properties
before using existing CAJ page-tree reconstruction. Preserve complete objects,
streams and bookmarks; referenced, ambiguous and unmeasured cases remain
errors. Native, CLI, Node and browser share the bounded core change without
API or dependency changes. See [bounds](../pdf-input.md#interrupted-metadata-and-missing-parents)
and [provenance](../provenance.md#interrupted-metadata-and-missing-parent-openers-452).

Recover the measured 78-page CAJ with a retained catalog, disconnected page
tree, unused incomplete metadata declaration and missing parent prefixes
(#456). Preserve the original page-label tree, complete page content and all
40 CAJ bookmarks. Require complete-graph proofs of optional-target absence,
explicit inheritable page values and exact partial-child-list agreement;
live form/metadata, extra catalog properties and unproved profiles remain
errors. Native, CLI, Node and browser share the bounded core behavior with
no public API or dependency change. See the
[bounds](../pdf-input.md#retained-catalog-and-incomplete-page-tree) and
[provenance](../provenance.md#retained-catalog-and-incomplete-page-tree-456).

### Breaking Rust API: CJK style font roles (#541)

`hnc8::C8PageFonts` gains three optional source indices: `cjk_sans`,
`cjk_kai`, and `cjk_system`. Existing struct literals add:

```rust,ignore
cjk_sans: None,
cjk_kai: None,
cjk_system: None,
```

When supplied, these roles select the measured CJK fonts using the original
raw style, including styles whose geometry is normalized to another value.
They apply to C8 (both character modes) and HN-B native modes 0 and 2.
The admitted `801c` family control overrides the CJK family until the next
`8002` style record; it preserves geometry and ordinary Latin selection.
Missing optional fonts or missing CJK glyphs use the base `cjk` fallback.
These roles also select C8 fullwidth Latin letters, ideographic space and
fullwidth ampersand; ordinary Latin and raw low-byte `006c`/`0070` keep their
previous resource rules. No font is bundled or
looked up by these options; `cjk_system` is a caller-supplied source.

The matching CLI flags and directory stems are `--font-cjk-sans` /
`cjk-sans`, `--font-cjk-kai` / `cjk-kai`, and `--font-cjk-system` /
`cjk-system`. Node/browser use `hnc8.fonts.cjkSans`, `cjkKai`, `cjkSystem`.
Raw WASM adds `caj2pdf_c8_set_cjk_style(style, index)` after base role
registration; the selectors are `0x0800`, `0x0c00`, and `0x0400`.
These adapter additions are compatible. Rust callers should use
`hnc8::C8_MAX_FONTS` (eleven) for resource bounds. Font I/O stays ranged,
embedding stays sequential, and shared inputs are still embedded once.
The three absent roles preserve the existing eight-entry PDF resource list.

This change addresses font selection, not full Wine pixel parity. Latin
resource aliases, punctuation, decorations, positioning and rasterization
must still be assessed separately; #541 remains open.

## Breaking Rust API: ornament glyph maps (#541)

`hnc8::C8FontSources` adds `decoration_glyphs: &[NativeDecorationGlyph]` and
`decoration_font: Option<&SymbolFontIdentity>`. Add `&[]` and `None` to retain
previous output. Owned `Fonts` adds the corresponding `Vec` and owned identity;
use `..Fonts::default()` or add an empty vector and `None`.
`write_c8_native_page` takes an extra decoration-map slice immediately after
the symbol-map slice. Pass `&[]` for the legacy global decoration alias.

Each entry maps a decimal native ornament value to a nonsemantic BMP glyph of
the explicit decoration role. Four unique values (1, 2, 46, 117) are supported;
mapped glyphs must exist in that resource. Identity binding uses the same
PostScript-name/checksum representation as symbols. The map applies only to
mode 2 and preserves existing record/state restrictions, Artifact/ActualText
marking and drawing geometry. Unlisted values use the previous global alias.

CLI, Node and browser options are additive. Raw WASM adds
`caj2pdf_c8_add_decoration_glyph(value, glyph)` and
`caj2pdf_c8_set_decoration_font(checksum, length, w0, ..., w7)` after roles,
using the existing bounded font-name encoding. Use matching JS/WASM builds.
No font binaries or automatic downloads are added.


### Breaking C8 font behavior: CJK-mode alphabet aliases (#541)

Correct the glyph selection for all 62 raw `a0` letters/digits while `80ce/0`
is active. Native, CLI, Node and browser now query the measured GBK alias in
the styled CJK font; previously they queried ASCII in the base CJK font.
ActualText preserves the existing source ASCII character, and exact aliases
do not increment substitution counts. `80ce/1` restores ordinary Latin.

Callers rendering this combination must supply the measured alias in the
selected font (for example U+7283 for raw `a0c1`), including optional CJK
style fonts. An absent optional role still uses the base CJK font, but a
missing glyph in the selected font fails instead of silently drawing ASCII.
No public API, font-read bound or coordinate model changes. All 77 actual
inventory pages remain byte-identical; six original controls improve while
retaining pixel differences. See the [evidence](../research/wine-cjk-aliases-20261011.md).


### Breaking HN/C8/NH PDF geometry: refined coordinate scale (#541)

The empirical raw-coordinate scale changes from `240/2473` to `72/742`
points throughout the shared geometry path. Native Rust, CLI, Node and
browser outputs can change page dimensions, positions and PDF bytes/hashes.
Regenerate affected snapshots and coordinate expectations; for example,
`raw_width * 240 / 2473` becomes `raw_width * 72 / 742`. No API signatures,
font/payload I/O bounds, dependencies, `72/300` image-pixel scale or `75/301`
font-em scale change. The fraction is an empirical model, not a proved
physical source unit.

Independent controls and held-out dimensions/zooms predict observed page
extents. All 77 inventoried actual pages now match Wine extents, with pixel
differences still present. Among 42 previously comparable pages, 30 improve,
ten are unchanged and two worsen slightly; all 113,418 source characters
are preserved and all 13 Node/native PDFs are byte-identical. Additional
HN-A/C8/NH checks cover 43 selected pages, not all 433 NH pages. See the
[67-session evidence and remaining limits](../research/wine-page-extents-20261011.md).

### Breaking native C8 behavior: preserve link appearances (#545)

Native Rust, CLI, Node and browser conversion now emits the measured
application-info links as PDF `/Link` annotations with explicit appearance
Forms. Previously all notes were counted and visually discarded. Their source
page, rectangle, color and HTTP(S) URI are retained without accessing the
URI; body streams, font/image resources and semantic text are unchanged.
The original font setup is unaffected. Reconvert files to receive the marks.

Unknown required note styles/types/units/versions or URI profiles now produce
an unsupported-format error. Callers must handle that error instead of
assuming metadata notes are always ignored. Damaged packages remain located
warnings; inspection and the image-only diagnostic API retain their existing
metadata-only behavior. No public field, WASM ABI, option or dependency is
added. See [the admitted profile and bounds](../cli.md#c8-application-info).

The appearance is a documented physical approximation: 0.5-point filled
marks, 6.75/2.25-point dash-dot-dot runs and a 1.5-point lower gap. Wine keeps
its dash cadence and gap in device pixels, so zoom-dependent differences
remain. Exact RGB equality is not an acceptance gate. Original controlled
sources and all 11 affected actual pages are compared at reading size;
printing and target navigation remain untested.
