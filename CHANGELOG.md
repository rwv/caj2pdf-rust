# Changelog

## Unreleased

- Fix marked image coordinates in paired compressed HN-A composition; retain
  raw coordinate words for inspection. Shortened-page clipping fidelity remains
  under investigation.


- **Breaking:** use verified HN-A paired `8003` per-page dimensions for page
  frames and image placement instead of always using document-header dimensions.
  `hnc8::TextCoordinates` gains `page_size`; update explicit struct literals.
  Raw and compressed paired pages share the rule. Other framing retains header
  fallback; zero per-page extents fail explicitly during composition.

- Recover an interrupted indirect Flate prefix anchored by an exact repeat of
  its preceding Length object, only when the final scan proves a complete
  counterpart. Reuse exact-prefix validation for earlier unique counterparts.
  This enables the observed 141-page issue-30 conversion; visual acceptance
  remains pending.

- Recover short interrupted ASCII85 CAJ streams when their immediately
  following Length object uniquely determines a validated complete replay.
  Also recover short cut `stream`/`endobj` keywords only when a fully parsed
  object proves the exact prefix, including an indirect reference cut before
  its `R` token. Direct-Length Flate replay additionally requires a bounded
  tail-derived boundary or exact preceding-object repeat and codec validation.
  Validate Flate Length repairs before accepting them; preserve explicitly
  counted line endings. ASCII85 prefix comparison remains bounded to 4 KiB.
  Unproved corruption remains an error.

- Recover bounded interrupted CAJ objects when a later complete copy is
  independently parsed from a page-table span and confirmed by the full scan.
  Ambiguous copies and unresolved corruption remain errors.
- Correct off-page image placement for verified raw HN-A `800a/d300` records
  carrying coordinate marker bits. Raw inspection values remain unchanged;
  this does not claim complete HN-A pixel fidelity.
- Add bounded caller-supplied TrueType resources and sequential PDF glyph,
  image and vector content pages. This shared API does not enable native
  C8/HN-B conversion or production JavaScript font resources.
- **Breaking:** raise the minimum Rust version to 1.88.0 for the maintained
  MIT `xberg-ttf-parser` font metadata dependency.

- **Breaking:** add `NativeRecord::ImageReference` for the measured C8
  `810a/d300` profile. Exhaustive matches must handle its coordinates and
  opaque source-span reference. Names are never opened as external files;
  complete native-page conversion remains unsupported.
- Admit verified 28-byte HN-B native image records with bounded reads and
  exact descriptor-count checks, plus independently verified following drawing
  and style controls. Mixed-page rendering remains unsupported;
  raw traversal does not establish full document conversion.

- Extend bounded HN-B native-record traversal across both verified index
  layouts, preserving observed run controls, raw numeric values, the atomic
  `c052/a385` prefix and 12-byte drawing records. Implicit glyph styles are
  explicitly unsupported rather than reported as malformed. This does not
  enable complete HN-B rendering or expand CLI/JavaScript conversion support.

- **Breaking:** add `NativeRecord::ExtendedControl` and preserve additional
  verified C8 control records. Exhaustive native matches must handle their
  raw payloads; exact transform/resource semantics remain unimplemented.
  This does not expand public CLI/JavaScript conversion support.

- **Breaking:** add `NativeRecord::EncodedString` for verified bounded C8
  `80cc/01xx` framing. Exhaustive native record matches must handle this raw
  event; unknown rendering semantics remain unsupported. CLI/JS behavior
  and complete-document support are unchanged.

- Read the verified compact HN-B page index using its explicit layout marker.
  Native-text conversion for these pages remains unsupported.
- **Breaking:** validate the HN-B layout marker at offset 136; unknown values
  and nonzero compact-row third words now fail explicitly.

- Admit the measured paired raw HN-A page-prefix profile through the bounded
  text reader; C8 raw framing remains unsupported.

- Add a bounded raw C8 native-record visitor for incremental parser work;
  add allocation-free decoding of verified native character codes. Complete
  native text rendering remains unsupported.

- Support validated type-1 JPEG image records in experimental HN-A/C8 conversion.
- Select the existing, narrowly scoped HN/C8 JBIG2 text-header compatibility
  policy in CLI/WASM; generic decoder defaults remain strict.
- Record the current full-format baseline and fixed HN/C8 regression set.

## v0.3.1

- Attest release files and the exact GHCR image digest using GitHub Actions OIDC.
- Verify signatures and workflow/commit/tag identity before publication.
- Include downloadable Sigstore bundles and consumer verification instructions.
- Preserve v0.3.0 conversion behavior and platform coverage.

See [v0.3.1 release notes](docs/releases/v0.3.1.md).

## v0.3.0

- Expand native runtime-tested CI/release targets, including LoongArch64,
  ARMv5/v6, MIPS32/64, PowerPC32/64, SPARC64 and additional libc variants.
- Expand the tested static Docker image matrix and retain complete release
  archive checksums and registry manifest verification.
- **Breaking:** bound OPFS cleanup retries for transient file locks and expose persistent
  spool removal failures with the original error retained as the cause.
- Keep format support, bounded conversion I/O and PDF encoding unchanged.

See [v0.3.0 release notes](docs/releases/v0.3.0.md) for precise runtime limits
and candidate platforms that are not released.

## v0.2.0

- Implement the Windows CLI using the shared converter, native file identity and
  cooperative console cancellation. Preserve Unicode paths and input protection.
- Add tested Linux GNU/musl, macOS, Windows, extended Linux/QEMU and FreeBSD
  targets. See [platform baselines](docs/platforms.md); no universal OS claim.
- Package dependency MIT notices, require the complete native matrix before
  publication and checksum every asset. Windows archives use ZIP.
- Add non-root Docker amd64/arm64 CLI images on scratch, with read-only root and
  pipe tests. Publish the tested OCI archive without rebuilding.
- Conversion profiles, JS API and PDF encoding remain unchanged from v0.1.0.
  HN/C8 remains experimental. No npm/crates.io registry publication is included.

See [v0.2.0 release notes](docs/releases/v0.2.0.md) for downloads and installation.

## v0.1.0

See [release notes](docs/releases/v0.1.0.md) for GitHub assets and installation.
CI-built release checksums are attached to the release as `SHA256SUMS`; the local
candidate hashes below are historical audit evidence.

### Candidate preparation record

Native Rust, CLI, browser and Node.js APIs may break during v0.x. The following
record describes the unpublished candidate audit before GitHub release packaging.

### Capabilities and limits

- Bounded ranged input and sequential PDF output; forward-only input can spool
  to capped temporary storage. Platform adapters stay separate from the core.
- CLI and browser/Node WASM convert the documented CAJ, PDF and KDH profiles.
  HN/C8 image-page conversion is experimental and includes standard QM/MQ
  numerical states. Custom tables remain optional overrides. Project source is MIT.
- [The support matrix](docs/conformance.md#v01-support-and-release-status)
  lists actual sample scope, page counts, bookmarks and rejected profiles.
  HN-A/C8 page-frame dimensions now match selected CAJViewer pages, but exact
  pixels still differ. C8/HN-B require explicit bookmark omission. Image-less
  HN-B rows are rejected by public conversion, not silently dropped.
- OCR, searchable HN, TEB and optional legacy Python ordering are outside v0.1.
  Missing optional corpus checks are `NOT_RUN`, never compatibility passes.

### Review fixes and JavaScript examples

- WASM packaging follows Cargo's resolved target directory, including
  `CARGO_TARGET_DIR` and Cargo configuration, rather than copying stale builds.
- Standard QM/MQ tables are borrowed without allocation by CLI and WASM.
- CLI SIGINT/SIGTERM request cooperative cancellation and clean staged output;
  repeated signals can force termination when OS I/O is blocked.
- Browser/Node `withHnc8Scratch` scopes own four capped stores and dispose them
  after success, failure or cancellation. The browser example uses a Dedicated
  Worker and backpressured output; both examples support experimental HN/C8.

### Streaming bilevel compression (#195)

Bilevel image XObjects now use `/FlateDecode`. Visible row bits, JPEG payloads,
page geometry/order and bookmarks are unchanged; compressed PDF bytes and hashes
change. Regenerate byte snapshots and use a PDF decoder when inspecting image
streams. Set `max_allocation_bytes` / `maxAllocationBytes` to at least 512 KiB
for bilevel output; this conservative fixed compressor reservation is checked
before opening the image. See [measurements and limits](docs/bilevel-compression.md).

### Built-in codec states

HN/C8 CLI and WASM conversion now use standard QM/MQ states when overrides are
omitted. Remove `--qm-states` / `--mq-states` for ordinary CLI conversion; in
JavaScript use `hnc8: { scratch }`. Explicit custom tables still take precedence,
and partial tables still fail. Missing JS scratch now reports
`RANDOM_ACCESS_REQUIRED` rather than a missing-codec-state error.

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
./target/release/caj2pdf paper.c8 --no-bookmarks -o paper.pdf
./target/release/caj2pdf inspect paper.caj --json --bookmarks
```

For JavaScript, build with `npm run build:wasm` inside `js/`, then follow the
[Node and browser examples](js/README.md). State files are caller-owned inputs;
these commands do not download them. The npm package remains private and
Cargo publishing is disabled pending release acceptance.

### Audited v0.1 candidate artifacts

The locked release builds and package sources are revision
`b5ccff9c5e831cb0b6f6570ea2062e322eb80bad`. It includes the packaged-module example-path fix;
the Rust production sources are unchanged from merged #197. The remaining audit
changes only documentation outside the shipped package. These are local, unpublished artifacts, not download links or a
promise of identical binaries on another host. Build environment: rustc 1.98.1,
Linux x86_64, Node 24.13.0.

| Artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| `caj2pdf` | 2,921,016 | `5618ceae0c30ab98a15249cb2fee5db803ab2c0a62dbb0eb05ef01c5ddc03c4b` |
| `caj2pdf_wasm.wasm` | 1,960,522 | `b23cb2421aaefb2beecdd6984c6988d2947fe4828cbaeac40803aa0f97256090` |
| `caj2pdf-rust-0.1.0.tgz` | 650,646 | `28da4e5d04f849bf0c4520c3db5af32c4ba9f85940f15b05a27a6d1184fdc539` |

The actual offline npm tarball contains the declared 12 files, including the MIT
license and current WASM, with no external documents, captures or vendor binaries.
Extracted-package Node and Chromium tests cover CAJ and compressed C8 conversion,
built-in states, scoped scratch and the default packaged WASM URL. The native
artifact reports version 0.1.0 and passes a synthetic PDF conversion/qpdf check.

#196/#197 passed all four hosted quality gates, including Node 22/24, Chromium,
MIT dependency/source/advisory audits and 100% Rust line coverage (30,539/30,539
at #197). [The support matrix](docs/conformance.md#v01-support-and-release-status)
records profile limits and known Python differences; [compression evidence](docs/bilevel-compression.md)
records new PDF hashes and memory observations. Optional missing corpus checks
remain NOT_RUN, never compatibility passes.

This completes candidate preparation under #14, subject to the final audit PR's
review and existing gates. npm remains private and Cargo publishing disabled.
Publishing is a separate action: removing `private` or changing any release
input requires rebuilding, rechecking the actual package and regenerating its
checksums under the [release policy](docs/release-policy.md).
