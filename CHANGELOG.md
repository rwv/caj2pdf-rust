# Changelog

## Unreleased — v0.1.0 preparation

This is an unstable development build, not a published release. Native Rust,
CLI, browser and Node.js APIs may break during v0.x. Codec state distribution
(#30/#44), final release-artifact verification and publication remain pending.

### Capabilities and limits

- Bounded ranged input and sequential PDF output; forward-only input can spool
  to capped temporary storage. Platform adapters stay separate from the core.
- CLI and browser/Node WASM convert the documented CAJ, PDF and KDH profiles.
  HN/C8 image-page conversion is experimental and needs caller-provided states
  for arithmetic images. Project source is MIT; those external states are not
  bundled or relicensed by this project.
- [The support matrix](docs/conformance.md#v01-support-and-release-status)
  lists actual sample scope, page counts, bookmarks and rejected profiles.
  HN-A/C8 page-frame dimensions now match selected CAJViewer pages, but exact
  pixels still differ. C8/HN-B require explicit bookmark omission. Image-less
  HN-B rows are rejected by public conversion, not silently dropped.
- OCR, searchable HN, TEB and optional legacy Python ordering are outside v0.1.
  Missing optional corpus checks are `NOT_RUN`, never compatibility passes.

### Migration from development snapshots

These changes occurred before the first release. The table covers the
breaking commits on main; links retain detailed API and diagnostic scope.

| Change | Old → new behavior and migration |
| --- | --- |
| PDF input (#19) and CAJ conversion | Rust `Error` gains located PDF, `Caj` and `CajLimitExceeded` variants; raw WASM gains error categories. Update exhaustive matches and use the current [error API](crates/caj2pdf-core/src/error.rs). |
| Streaming JavaScript API | `copyRangeProof` / `convertKdhProof` become `copyRange` / `convert`; raw `caj2pdf_kdh_start` becomes `caj2pdf_start`. Replace `convertKdhProof(instance, source, sink)` with `convert(instance, source, sink, { format: "kdh" })`. |
| Type-0 row decoding | `ArithmeticSnapshot` gains `source_bytes_fetched`. Add that field to explicit literals; use a rest pattern when inspecting snapshots. See [row API](docs/jbig1-type0-rows.md). |
| Arithmetic budgets | Counter budgets above `MAX_BUDGET_COUNT` (2^48) are rejected. Replace native `u64::MAX` sentinel values with `MAX_BUDGET_COUNT`, e.g. `MqBudget { max_work: MAX_BUDGET_COUNT, ..Default::default() }`. CLI/JS do not expose these budgets. |
| JBIG2 header policy (#92) and page composition (#96) | `TextRegionHeader` / `TextComposeReport` gain source identity, raw flags, anomaly and complete-header metadata; mismatched identities fail. Update explicit literals from the parsed header. Strict parsing stays default; the narrow anomaly needs an explicit native policy. See [page profile](docs/t88-observed-page-composition.md). |
| Selected type-0 PDF (#101) | Exhaustive `Type0PdfErrorKind` matches must handle `InvalidSelection`; ordinary `convert_type0_pdf` does not emit it. See [selection API](docs/hnc8-type0-pdf.md). |
| Outline observation tool | Opt-in diagnostic contracts/reports move from schema 1 to schema 2 with a three-profile field scope. Replace old contracts with reviewed schema-2 contracts; production Rust APIs are unchanged. See [observation record](docs/hnc8-outline-observation.md). |
| HN-A outlines (#162) | `ComposeOptions` gains `include_bookmarks`; set it explicitly or use defaults. HN-A output can carry source outlines; C8/HN-B require `false`. See [outline fields](docs/hnc8-outline-fields.md). |
| Raw HN composition (#164) | Additional uncompressed text profiles are parsed, and image-width behavior changes. Reconvert affected documents; update explicit coordinate/report literals from the current API. The later #184 geometry rule below supersedes intermediate padded-width behavior. |
| Type-3 composition (#174) | `ComposeOptions` gains `type3`, and reports add type-3 counts/anomalies. Use `type3: Type3PdfOptions::default()` or `..Default::default()`. Existing scratch calls remain valid; see [migration](docs/hnc8-page-composition.md#v0x-api-migration). |
| Repeated image groups (#176) | Verified aliases no longer become extra draws/pages. `ComposedImage` gains `duplicate_of`; count draws with `duplicate_of.is_none()`. Reports add `duplicate_image_records`; update exhaustive literals/patterns and regenerate old duplicate-page outputs. |
| WASM (#179) and CLI (#180) HN/C8 routing | HN/C8 now attempt supported conversion instead of unconditional rejection. Supply needed states and bounded scratch, handle located HN/C8/configuration errors, and explicitly omit unknown outlines. Raw WASM hosts must implement scratch statuses 6–9; use [the JS migration guide](js/README.md#v0x-migration). |
| Metadata inspection (#181) | Valid HN-A outlines are reported instead of unknown values; malformed outlines fail with location. Known empty HN-A outlines return zero/false/empty entries; C8/HN-B remain unknown. `conversion_supported` means a route exists, not that a document will convert. Update consumers of [CLI JSON](docs/cli.md#v0x-migration) and JS error matches. |
| Source geometry (#184) | HN-A/C8 use declared page/display extents instead of first-image pixel dimensions and omit DIB storage padding. Zero extents fail. Rust `Header` gains `page_size`; `RawTextCoordinate` gains `width`/`height`. Update literals and regenerate PDF snapshots/hashes. The physical-unit factor remains empirical; see [geometry migration](docs/cli.md#source-geometry-correction-breaking-v0x). |

### Build and usage

```sh
cargo build --locked --release -p caj2pdf-cli
./target/release/caj2pdf paper.caj -o paper.pdf
./target/release/caj2pdf paper.c8 --mq-states mq.txt --no-bookmarks -o paper.pdf
./target/release/caj2pdf inspect paper.caj --json --bookmarks
```

For JavaScript, build with `npm run build:wasm` inside `js/`, then follow the
[Node and browser examples](js/README.md). State files are caller-owned inputs;
these commands do not download them. The npm package remains private and
Cargo publishing is disabled pending release acceptance.

### Development artifact verification

These are local, unpublished audit artifacts built from package sources at
`e794c4b890e562ba82e3773b7a3beed741507170`. They are not download links or a
promise of reproducible binaries on another host. Regenerate checksums from
the exact final artifacts after any release-input change, including removing
npm's `private` flag.

| Artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| `caj2pdf` (Linux development host) | 2,833,080 | `7e886dd497314a370987b464f6d9ddfbc3b382424bacc368d2fcf3a3c6981bb1` |
| `caj2pdf_wasm.wasm` | 1,932,215 | `6de6986d730d3ec55d6462bd43436ddc6c812ae61bdeb2b11e816c391a12a05c` |
| `caj2pdf-rust-0.1.0.tgz` | 637,749 | `5af27603f55f9f33b7e9a1f353ab4e2787b8e6cfc814380debcd3586b3ce5b5c` |

The tarball contains the declared 12 files, including MIT license and WASM.
[Package/example tests and memory observations](docs/js-validation.md) and
[viewer results](docs/cajviewer-fixtures.md) record their separate scopes.
#186/#187 passed Native, WASM, MIT audit and 100% Rust line-coverage gates.
The final release commit and its artifacts must pass the existing
[release policy](docs/release-policy.md); this draft does not close #14.
