# Unreleased

Changes after [v0.6.1](v0.6.1.md):

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
with other roles is embedded as one mapped font for all of them. The core ships no mappings. CLI and JavaScript
options, resource identity binding and verification against measured source
resources remain open in #518. No fonts are bundled, fetched or inferred.

### Other unreleased changes

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
