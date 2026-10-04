# Changelog

## Unreleased

- **Breaking:** native C8/HN-B pages now need only a CJK and a Latin font.
  `C8PageFonts::alternate_latin` becomes `Option<usize>`; wrap existing
  values in `Some`. If a role is absent, or its font does not map a
  character, the glyph uses the CJK font for CJK-coded characters and the
  Latin font otherwise (`is_cjk_coded`, `C8_DEFAULT_DECORATION_ALIAS`). A
  glyph missing from that font still fails with its location. Before, an
  absent optional role or unmapped glyph failed. The CLI adds `--fonts DIR`,
  which reads `cjk.ttf`, `latin.ttf`, `alternate-latin.ttf`,
  `decoration.ttf`, `symbols.ttf` and `latin-state{3,28,31}.ttf`; per-role
  flags override it. JS `hnc8.fonts.alternateLatin` is optional. Matching
  WASM accepts `0xffffffff` as an absent alternate role. `docs/cli.md`
  documents a tested free recipe (Droid Sans Fallback and DejaVu Sans). It
  converts all six pinned C8/HN-B corpus inputs.

- **Behavior change:** C8 and HN-B inputs no longer fail when bookmarks are
  requested (the CLI default). Their outline layout is unverified, so the PDF
  has no outline, the CLI prints one warning, and JS reports
  `outlineOmitted: true`; `--no-bookmarks` / `includeBookmarks: false` give
  the same PDF silently. `OutlineReport` gains `unverified`.

- Fix: one malformed HN-A outline entry no longer fails `convert` and
  `inspect` for the whole document (#299). Entries with an invalid title,
  page or zero level are skipped and their children re-parented; level skips
  and levels beyond the depth limit are clamped. The CLI prints one located
  `caj2pdf: warning:` line per defect (at most 16, then a count) and exits 0;
  `inspect --json` adds `outline_warnings` (schema version 1, additive) and
  `bookmark_count` reports the entries written; JS reports add
  `outlineWarnings`. Unreadable outline tables, limits and cancellation still
  fail. **Breaking (Rust):** `Hnc8Reader::visit_bookmarks` returns an
  `OutlineReport` instead of the declared count, and `ComposeReport` gains
  `outline`.

- Add `caj2pdf inspect INPUT --pages` (#301), a structure-only report for
  diagnosing unseen profiles without the file: the HN/C8 page-index layout,
  native mode/origin, page size and the presence/extent of a trailing
  `APPINFOSIGN` application-info section, then one line or JSON object per
  page with its text span, the text framing accepted by the existing readers
  (`none`, `raw`, `raw-paired`, `compresstext`, `legacy-24`, `native`) and
  record counts, image descriptor types and payload spans, and located
  per-page errors. KDH reports the observed 32-byte wrapper signature; a
  different signature is reported (exit 0) instead of failing. Nothing from
  the document's text, titles or pixels is printed. JSON stays
  `schema_version` 1 (additive `structure` and `pages` fields, only with
  `--pages`). Core adds `Hnc8Reader::inspect_text`, `page_row_bytes` and
  `application_info_tail`, and `kdh::HEADER_SIGNATURE`. JS is unchanged.
- Add a conversion-failure issue template that asks for the
  `inspect --json --pages` output instead of the document.

- C8: read the trailing application-info package (#302). `inspect` reports
  its DOI, URL and annotation count (`application_info` in `--json`, schema
  version 1, additive; `DOI:`/`URL:`/`Notes:` lines in text). Conversion of a
  C8 source whose package has a DOI or URL now writes a PDF `/Info` dictionary
  with custom `/CNKI_DOI` and `/CNKI_URL` keys; other output is
  byte-identical. A defective package is ignored with one located
  `caj2pdf: warning:` line and never fails conversion. **Breaking (Rust):**
  `ComposeReport` gains `application_info`; the new
  `PdfDocument::finish_with_info` and `PdfWriter::finish_with_info` keep
  `finish` unchanged. JavaScript conversions get the same PDF; JS inspection
  does not expose the fields yet.

- Conversion shows input progress on standard error when it is a terminal;
  `-q`/`--quiet` disables it. Redirected standard error is unchanged.

- Speed up native HN/C8 scratch I/O: `FileScratch` caches its length and
  uses positioned reads/writes, removing a `statx` and `lseek` per request
  (36.6M to 13.5M system calls on a 163-page HN-A input; output unchanged).

- Add cargo-fuzz targets for conversion and inspection of arbitrary input
  (`fuzz/`, its own workspace), run weekly in CI from the synthetic fixtures.
- Pull requests run the full release-platform matrix only when platform
  inputs change; the Linux, macOS and Windows native jobs always run.
  Dependabot proposes grouped weekly Cargo and GitHub Actions updates.
- Recognize a PDF whose `%PDF-` header follows other bytes (a newline, UTF-8
  byte-order mark, or junk line) within the first 1,024 bytes when no other
  signature matches at byte 0, in the CLI and auto-detecting JS API. Its
  offsets are read relative to the header and the leading bytes are dropped
  from the output (#300). Core adds `detect_source`, `Detection`, and
  `PDF_HEADER_SEARCH_BYTES`; `detect_format` now also matches the displaced
  header within its prefix.
- TEB diagnostics now say the input is a DRM-encrypted CNKI container whose
  document content cannot be converted; `inspect --json` adds
  `"unsupported_reason":"drm-encrypted"` for TEB. Exit status is unchanged.
- Add FreeBSD RISC-V64 GC and PowerPC64 big-endian native CLI archives,
  tested in FreeBSD 15.1 VMs. Both targets join the required release matrix,
  dependency-license audit, archive inventory and existing provenance flow.
- Add the i586 GNU CLI target using a pinned glibc 2.19 sysroot and actual
  Pentium-model execution. It joins the required test matrix, MIT dependency
  audit and release archive/provenance inventory; no old-kernel guarantee.

- **Breaking:** `C8PageFonts` adds optional `latin_state28` and `latin_state31`;
  existing Rust initializers should set them to `None` unless supplying the
  distinct C8 resources. CLI and JS expose matching optional font roles;
  existing WASM registration exports remain compatible. At most eight distinct
  ranged font sources are accepted. The additional four/five-page C8 profiles
  complete with explicit resources through CLI/Node/Worker, including native
  text, radicals and descriptor-ordered images. Bookmarks remain unsupported;
  marker-layout checks do not establish original-font or whole-format fidelity.

- Enable the independently controlled HN-B mode-0/mode-2 native profiles
  through the shared bounded renderer and explicit CLI/Node/Worker fonts.
  The selected 4/4/6-page inputs have complete runtime checkpoints and scoped
  marker-layout checks. Mode 2 preserves leading images; image-after-text and
  mode-0 images remain explicit errors. Bookmarks remain unsupported, and
  caller font substitution does not establish original-font pixel parity.
- Add optional `--font-symbols` / JS `symbols` and `--font-latin-state3` /
  JS `latinState3` resources. Missing required roles fail explicitly.
- **Breaking:** `C8PageFonts` gains `symbols` and `latin_state3` fields;
  existing literals should specify `None` unless those roles are provided.
  The raw WASM five-argument font setter remains available; additional roles
  use the exports documented in [I/O architecture](docs/io-architecture.md).

- Reject unverified large-title punctuation with a located error before
  regular-size offset lookup, avoiding an out-of-bounds panic.
- **Breaking:** `hnc8::Header` gains `native_mode: Option<u32>`; update
  explicit literals. `Engine::set_c8_fonts` gains a symbols argument; use
  `u32::MAX` when absent. This Rust change does not alter the old WASM export.

- **Breaking:** HN-B `hnc8::Header.native_origin` and `page_size` now expose
  verified raw header words as `Some`, including zero extents. Do not use their
  presence as proof of complete native rendering support. Legacy image-only
  HN-B conversion retains its image-derived page dimensions.

- **Breaking:** `hnc8::NativeRecord::End.value` is now `Option<u16>` to preserve
  HN-B two-byte page ends. Wrap existing explicit values in `Some`; handle
  `None` as an absent payload, not a default ordinal. C8 retains four-byte ends.

- Fix marked image coordinates in paired compressed HN-A composition; retain
  raw coordinate words for inspection. Original controls distinguish clipped
  raster sampling from physical geometry; no pixel-parity guarantee is made.

- **Breaking:** use verified HN-A paired `8003` per-page dimensions for page
  frames and image placement instead of always using document-header dimensions.
  `hnc8::TextCoordinates` gains `page_size`; update explicit struct literals.
  Raw and compressed paired pages share the rule. Other framing retains header
  fallback; zero per-page extents fail explicitly during composition.
- Add experimental complete-document conversion for the observed raw C8 native
  profile through Rust, CLI, Node and browser Workers. Stream text, drawings and
  images in source order using shared codecs and reusable bounded scratch.
  Supply explicit ranged TrueType fonts; missing resources/glyphs and unknown
  required records fail rather than dropping content. C8 bookmarks, other C8
  native profiles remain unsupported; HN-B scope is described above.
- Add CLI `--font-cjk`, `--font-latin`, `--font-alternate-latin`, optional
  `--font-decoration` and `--decoration-char`; JavaScript exposes the same roles
  through `hnc8.fonts`. Shared sources embed once. Forward-only fonts use existing
  bounded spooling helpers; callers retain ownership and cleanup responsibility.
- **Breaking:** `caj2pdf_wasm::engine::Request::Read` gains `resource`: 0 identifies
  the document and 1–4 identify registered fonts. Raw hosts must route reads by
  resource and register fonts before polling; bundled JavaScript handles this.
- Add measured C8 glyph, image, hairline and horizontal-decoration placement,
  including style prefixes `0800`/`0c00`/`1000`, separate size axes and clipped
  final decoration marks. Unknown styles remain errors. Font substitution and
  zoom-dependent viewer rasterization are explicit limits; no pixel-parity claim.
- Add scoped grayscale/clipped glyph output and nonsemantic decoration marking
  to the incremental PDF writer. Decoration aliases are excluded from extractors
  that honor empty `ActualText`.

- **Breaking:** frame admitted C8 `8006` and `8010/1` drawings as 12-byte records and
  preserve following `ffff/5` controls independently. Raw visitor event counts
  change; following position/style/end records are no longer consumed as footers.

- Preserve the independently controlled `8006/a385` C8 drawing record in
  bounded native traversal; controlled HN-B translation now reuses the
  shared renderer as described above.

- Add allocation-free decoding of the independently controlled C8 native image
  coordinate profile. Unknown prefixes and zero extents remain unsupported;
  the document composer applies these fields in source draw order.

- **Breaking:** add `hnc8::Header::native_origin` for the observed C8 native
  coordinate origin. Explicit header literals must include this field; use
  `None` for HN variants.

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
  image and vector content pages. C8 and admitted HN-B native conversion
  share this API.
- **Breaking:** raise the minimum Rust version to 1.88.0 for the maintained
  MIT `xberg-ttf-parser` font metadata dependency.

- **Breaking:** add `NativeRecord::ImageReference` for the measured C8
  `810a/d300` profile. Exhaustive matches must handle its coordinates and
  opaque source-span reference. Names are never opened as external files;
  complete native-page conversion remains unsupported.
- Admit verified 28-byte HN-B native image records with bounded reads and
  exact descriptor-count checks, plus independently verified following drawing
  and style controls. Native mode-2 leading-image rendering is now admitted;
  raw traversal alone does not establish full document conversion.

- Extend bounded HN-B native-record traversal across both verified index
  layouts, preserving observed run controls, raw numeric values, the atomic
  `c052/a385` prefix and 12-byte drawing records. Implicit glyph styles are
  unsupported except for the independently controlled complete axis state.
  See the complete native-rendering scope above; record admission alone
  does not establish document support.

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
  text reader.

- Add a bounded raw C8 native-record visitor for incremental parser work;
  add allocation-free decoding of verified native character codes. Preserve
  raw controls and coordinates; framing admission alone does not establish
  renderability for additional profiles.

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
before opening the image. See [measurements and limits](docs/research/bilevel-compression.md).

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
| Type-0 row decoding | `ArithmeticSnapshot` gains `source_bytes_fetched`. Add that field to explicit literals; use a rest pattern when inspecting snapshots. See [row API](docs/research/jbig1-type0-rows.md). |
| Arithmetic budgets | Counter budgets above `MAX_BUDGET_COUNT` (2^48) are rejected. Replace native `u64::MAX` sentinel values with `MAX_BUDGET_COUNT`, e.g. `MqBudget { max_work: MAX_BUDGET_COUNT, ..Default::default() }`. CLI/JS do not expose these budgets. |
| JBIG2 header policy (#92) and page composition (#96) | `TextRegionHeader` / `TextComposeReport` gain source identity, raw flags, anomaly and complete-header metadata; mismatched identities fail. Update explicit literals from the parsed header. Strict parsing stays default; the narrow anomaly needs an explicit native policy. See [page profile](docs/research/t88-observed-page-composition.md). |
| Selected type-0 PDF (#101) | Exhaustive `Type0PdfErrorKind` matches must handle `InvalidSelection`; ordinary `convert_type0_pdf` does not emit it. See [selection API](docs/research/hnc8-type0-pdf.md). |
| Outline observation tool | Opt-in diagnostic contracts/reports move from schema 1 to schema 2 with a three-profile field scope. Replace old contracts with reviewed schema-2 contracts; production Rust APIs are unchanged. See [observation record](docs/research/hnc8-outline-observation.md). |
| HN-A outlines (#162) | `ComposeOptions` gains `include_bookmarks`; set it explicitly or use defaults. HN-A output can carry source outlines; C8/HN-B require `false`. See [outline fields](docs/research/hnc8-outline-fields.md). |
| Raw HN composition (#164) | Additional uncompressed text profiles are parsed, and image-width behavior changes. Reconvert affected documents; update explicit coordinate/report literals from the current API. The later #184 geometry rule below supersedes intermediate padded-width behavior. |
| Type-3 composition (#174) | `ComposeOptions` gains `type3`, and reports add type-3 counts/anomalies. Use `type3: Type3PdfOptions::default()` or `..Default::default()`. Existing scratch calls remain valid; see [migration](docs/research/hnc8-page-composition.md#v0x-api-migration). |
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
records profile limits and known Python differences; [compression evidence](docs/research/bilevel-compression.md)
records new PDF hashes and memory observations. Optional missing corpus checks
remain NOT_RUN, never compatibility passes.

This completes candidate preparation under #14, subject to the final audit PR's
review and existing gates. npm remains private and Cargo publishing disabled.
Publishing is a separate action: removing `private` or changing any release
input requires rebuilding, rechecking the actual package and regenerating its
checksums under the [release policy](docs/release-policy.md).
