# Unreleased

## C8 native article records (#391)

- The measured decoration, unequal CJK title, style-5 book-title marks and
  arrow, table-line records and metadata value now preserve the complete
  four-page article. Unknown neighboring profiles remain explicit errors.
- Native, Node and Chromium outputs match. The [checkpoint](../conformance.md#c8-native-article-checkpoint-391)
  records full glyph-order and scoped geometry checks, including the default
  decoration substitution limit. No API shape, dependency or memory-allocation
  class changes.

## C8 generic-only JBIG2 images (#392)

- HN/C8 type-3 images with the measured page-information plus full-page
  generic-region profile now decode through the existing bounded row decoder.
  Apparently blank images retain their actual pixels; unknown profiles and
  corrupt bodies still fail.
- All 9 affected documents (224 pages) convert. The [checkpoint](../conformance.md#c8-generic-only-jbig2-checkpoint-392)
  records full image-pixel comparisons and neighboring-page checks. No native,
  CLI or JavaScript API shapes or dependencies change.

## C8 uncompressed image records (#390)

- C8 pages starting with the measured single unmarked image record now use
  the existing bounded tagged-record parser. Compressed frame validation and
  the separate native-text path keep their existing requirements.
- All 13 affected documents (227 pages) convert. The [checkpoint](../conformance.md#c8-uncompressed-image-record-checkpoint-390)
  records source-image, page geometry and scoped rendering evidence. No native,
  CLI or JavaScript API changes or new dependencies.

## C8 JBIG2 empty content (#389)

- The explicit HN/C8 text-header policy now permits ordinary signed
  displacement values with the measured unused refinement-template bit.
  HN/C8 composition also accepts a terminal-only empty symbol dictionary
  after validating zero counts and the exact marker. Strict standalone
  JBIG2 decoding keeps its existing requirements.
- Fifteen of the 25 affected documents now convert; ten reach separate
  page-2 refusals tracked by #390/#392. All 28 affected image pixel comparisons
  pass. See the [checkpoint](../conformance.md#c8-jbig2-empty-content-checkpoint-389).
  No native, CLI or JavaScript API shape changes.

## HN-A full-page JPEG regions (#388)

- HN-A pages containing one full-page JPEG and one or two additional region
  records now convert with the image, page dimensions and ordering preserved.
  The parser validates the complete measured record profile and zlib frame;
  it does not substitute blank pages or drop unknown decoder failures.
- No native, CLI or JavaScript API changes. See the
  [conformance checkpoint](../conformance.md#hn-a-jpeg-region-checkpoint-388)
  for the 12-document run and scoped fidelity evidence.

## Explicit damaged CAJ conversion (#297)

- CLI: opt in with `caj2pdf damaged.caj --allow-damaged -o partial.pdf`.
  Exit 3 means the PDF was committed with reported blank substitutions.
  Default strict behavior is unchanged.
- Browser and Node: pass `{ allowDamaged: true }` and inspect
  `report.omittedPages`, an array of `{ pageIndex, offset }` (zero-based
  index, absolute `bigint` input offset).
- Rust breaking change: `ConversionOptions` adds `allow_damaged`;
  `ConversionReport` adds `omitted_pages` and is no longer `Copy`.
  HN/C8 report wrappers containing it are also no longer `Copy`.
  Migrate option literals using `..ConversionOptions::default()`;
  borrow reports or explicitly clone them. No whole-file conversion API is added.
- Raw WASM: the existing start option word uses bit 0 for bookmarks and bit 1
  for damaged CAJ mode. New count/index/offset exports expose omissions.
  Use the JavaScript module with its matching WASM build.

See [partial conversion behavior](../pdf-input.md#explicit-partial-conversion-of-damaged-caj-inputs)
for remaining hard errors and shared-resource limitations. This is an unstable
feature intended for the next minor release, not a change to v0.4.0 artifacts.

## HN-B magnesium article conversion (#381)

- The measured 12-page HN-B profile now converts with all native glyphs and
  type-3 images. Additional size/symbol/metadata rules and bilevel composition
  preserve source order; unknown profiles remain errors.
- One observed private-use code retains U+E6C7. If the caller's Latin font lacks
  it, a documented visual substitute is drawn with the original code in PDF
  ActualText. The CLI warns; Rust `substituted_glyphs` and JavaScript
  `substitutedGlyphs` report the count. This does not identify its semantics.
- Rust breaking change: `ConversionReport` adds `substituted_glyphs: u64`.
  Migrate complete report literals by adding `substituted_glyphs: 0` or using
  `..ConversionReport::default()`. JavaScript reports add a `bigint` property;
  use the JS package with its matching rebuilt WASM module.
- See the [conformance checkpoint](../conformance.md#hn-b-magnesium-article-checkpoint-381)
  for selected comparisons, font limitations and NOT_RUN optional-corpus status.
  This changes unreleased builds, not existing v0.4.0 artifacts.
