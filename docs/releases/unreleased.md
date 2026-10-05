# Unreleased

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
