# Unreleased

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
