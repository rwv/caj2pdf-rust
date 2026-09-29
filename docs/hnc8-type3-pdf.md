<!-- SPDX-License-Identifier: MIT -->

# Selected HN/C8 type-3 JBIG2 PDF diagnostic

This note covers [issue #106](https://github.com/rwv/caj2pdf-rust/issues/106), a child of [HN/C8 page conversion #10](https://github.com/rwv/caj2pdf-rust/issues/10). One checked source image becomes one one-page PDF for pixel verification. It does not reconstruct an HN/C8 source page, place neighboring images, decode source text, create outlines, or enable a released CLI/browser/Node.js HN/C8 converter.

## Format and rights boundary

The selected path accepts the observed five-segment type-3 profile already measured in the [#43 oracle](jbig2-oracle.md) and decoded as packed page rows in [#95](jbig2-page-parity.md). The caller provides the validated 47-state T.88 MQ table. The exact normative Annex E.1 rows remain external while [#44](t88-mq-rights.md) is unresolved; this diagnostic does not grant redistribution rights or claim standalone JBIG2 support. No Python, Go, private Rust, differently licensed decoder, sample document, normative table, or generated private PDF/bitmap is copied into this repository.

The output uses the existing forward-only PDF writer's one-bit image path. Packed source rows are top-down, MSB-first, with set bits denoting black and unused low row bits zero. The PDF XObject is `/DeviceGray`, `/BitsPerComponent 1`, and `/Decode [1 0]`, so set bits display as black. The page MediaBox uses image pixels scaled by the caller's pixels-per-inch setting; this is a diagnostic scale, not a recovered HN/C8 page size. [Adobe PDF Reference 1.7](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.7old.pdf) §4.8 defines image XObjects and their decode arrays. Independent `qpdf`, Poppler, and MuPDF checks are required for private parity; structural validity and displayed-pixel agreement are different checks.

## API and memory contract

`hnc8::convert_type3_image_pdf(source, sink, table, workspaces, selection, options, limits, cancellation)` is the platform-neutral selected-image entry point. `Type3ImageSelection` uses one-based page/image numbers. `Type3PdfOptions` supplies the explicit 300-pixel-per-inch default scale, container and JBIG2 stage budgets, and text-header policy; shared `Limits` bounds the PDF and I/O. The return value includes conversion counts, source variant/page count, the checked `ImageRecord`, JBIG2 page geometry, page-composition progress, and any text-header anomaly.

`Type3Workspaces` borrows three initially empty intermediate symbol stores and one full-page text scratch. The first and second stores each expose a writer and two reader handles because decoding and page composition borrow readers at the same time; the refined store has one live reader and one writer. The second and refined stores must make appended bytes visible while decoding continues; a native `SeekableSource` snapshots its size and is unsuitable for those live views. The caller owns capacity, temporary storage, and cleanup on every success/error path. The core does not create native temp files. Native and JavaScript adapters choose their own bounded backing; a forward-only source must first be spooled to seekable storage. The image record's preceding same-page descriptor chain is checked; preceding source pages are intentionally skipped. The selected descriptor, DIB and palette, segment directory, page profile, and source identity are validated before the PDF reports success. A failure may leave bytes in the caller's sink; discard that partial PDF.

Strict T.88 text-header validation is the default. The single known raw `0xa40c` text header is refused in strict mode; the existing named HN/C8 policy admits it and preserves its anomaly marker. Other unsupported page topology, dimensions, palette, text modes, and segment associations are errors rather than silent fallbacks. This diagnostic does not alter the full-document HN/C8 converter's current support boundary.

`Type3PdfError` retains requested page/image numbers, including zero for invalid selection, and an absolute source anchor. When a decoder knows a precise source location it is lifted to the outer error; scratch and PDF output positions remain in the nested stage error and are never labeled as source bytes. A zero selection has source anchor zero because no descriptor has been selected.

## Shared image emission for source-page integration

The selected-image wrapper now uses three internal steps: `preflight_type3`
checks the image metadata and source digest, `prepare_type3_image` decodes
symbols/text into the existing stores, and `emit_type3_xobject` appends the
combined rows to a caller's existing `PdfDocument`. Page creation and
placement remain the caller's responsibility.

Preparation writes no PDF bytes. Its result borrows the text scratch until
emission finishes, so that storage cannot be reset between these steps.
Symbol contexts and catalogs are released before the generic row pass. The
wrapper retains its three source-hash passes, error locations, strict/anomaly
policy, and existing rejection-before-output behavior.

An original unit test emits two differently sized asymmetric images into
one document, reuses the text scratch between images, and checks packed
pixels and page-placement commands. Its synthetic byte builder is shared
with the existing selected-image integration tests. This is the reusable
image boundary for #118; complete HN/C8 source-page composition and external
full-page comparisons remain that issue's next steps.

## Verification protocol and measured evidence

The clean-clone synthetic tests use independently authored tiny containers and an invented MQ table or injected packed rows. They exercise selection, errors, and the one-bit PDF polarity/row-order contract without the private table or corpus. The opt-in external harness must verify the pinned 27 source identities, all 546 selected encoded spans and the separately held table digest before and after; a missing explicitly requested input fails. It emits one temporary PDF at a time and independently checks its structure, extracted packed pixels, and selected rendered canaries. A clean clone reports `NOT_RUN` and zero private compatibility matches.

Run a requested comparison with separately held files; neither the table nor the corpus is downloaded by the test:

```sh
python3 scripts/jbig2_page_pdf_parity.py \
  --corpus-dir /private/CAJSamples \
  --table-fixture /tmp/t88-mq.fixture \
  --text-header-policy strict --json

python3 scripts/jbig2_page_pdf_parity.py \
  --corpus-dir /private/CAJSamples \
  --table-fixture /tmp/t88-mq.fixture \
  --text-header-policy hn-c8-unused-refinement-template --json
```

The private fixture must be a regular file under `/tmp`, as required by the existing diagnostic table loader; the shown name is a placeholder. The harness invokes a repository-owned Rust example that receives only the selected page/image and private table path. It opens three bounded symbol stores and one page scratch under a case-scoped temporary directory. The Python parent owns that directory even if a child times out, and limits validator logs and one PDF/PBM/render set at a time. It requires warning-free `qpdf --check`, a one-page `pdfinfo` MediaBox, exactly one one-bit gray `pdfimages` XObject, exact normalized packed-row SHA-256 and black count, and nonblank fixed HN/C8/multi-image/anomaly render comparisons. MuPDF at 72 pixels per inch is compared pointwise; Poppler at 720 pixels per inch uses a predeclared centre pixel from each 10 × 10 block. The original #43 oracle's decoder-backend independence remains unverified.

The final-source runs on 2026-09-27 used the same release example binary in
both policies, built with `cargo build --locked --release -p caj2pdf-core
--example jbig2_page_pdf`. The actual commands were:

```sh
python3 scripts/jbig2_page_pdf_parity.py \
  --corpus-dir /tmp/caj2pdf-ranged-corpus \
  --table-fixture /tmp/caj2pdf-t88-2000-h2-private.fixture \
  --rust-bin target/release/examples/jbig2_page_pdf \
  --text-header-policy strict --json

python3 scripts/jbig2_page_pdf_parity.py \
  --corpus-dir /tmp/caj2pdf-ranged-corpus \
  --table-fixture /tmp/caj2pdf-t88-2000-h2-private.fixture \
  --rust-bin target/release/examples/jbig2_page_pdf \
  --text-header-policy hn-c8-unused-refinement-template --json
```

| Policy | Attempted | Complete PDF/pixel matches | Expected strict refusal | Opt-in anomaly match | Fail / skip / unsupported |
| --- | ---: | ---: | ---: | ---: | ---: |
| `strict` | 546 | 545/545 | 1/1 | 0 | 0 / 0 / 0 |
| `hn-c8-unused-refinement-template` | 546 | 546/546 | 0 | 1/1 | 0 / 0 / 0 |

All 27 pinned source-file SHA-256 identities and all 546 selected encoded-span
SHA-256 values matched before and after each run. The canonical SHA-256 of the
27-entry source hash map was
`2399bddb744e94041f7b4c3e3938717780c97a803d5ee793bd83dbef8d205416`
(`json.dumps(source_hashes_before, sort_keys=True, ensure_ascii=False,
separators=(",", ":"))`
encoded as UTF-8). The private MQ table digest was
`bdf6eeeca3bc5d5a8dc1a13acc7698ec356c886b27f6526f3e09fc2c8520ac57`
before and after each run; its state rows remain external. The example binary
digest was
`72bb21f3088a92fd1d44fc9aaccbd148075cee6d6cafa70046073c9ba903cd30`
before and after both runs. The independent validator executables were also
hashed before and after each run:

| Tool | Version | Executable SHA-256 |
| --- | --- | --- |
| qpdf | 12.2.0 | `30b2389c3ed0ba73434244fff5b149d4d919279ea7a8c3b1cd9fc25f57ec1792` |
| Poppler `pdfinfo` | 25.03.0 | `a1a371340d7b76e7d501da9136cc9256dfdb9cbdf09300520d7a3b4465343e67` |
| Poppler `pdfimages` | 25.03.0 | `213eba4a36ef021f49a0abc94292a7566baba8dfafef5017467166d9f06074f5` |
| Poppler `pdftoppm` | 25.03.0 | `f22d753dfb4c31c9f0198d608982dac5b06a3c5d9a08d7d9bf0c6004f08a1a56` |
| MuPDF `mutool` | 1.25.1 | `b9588916750d90219b1511cf329776439c68e9a1ae1e6f92dc6532e21dd96df7` |

The fixed nonblank render canaries used these exact source identities and
one-based page/image coordinates. The source SHA-256 identifies the entire
external file, not a redistributed copy:

| Canary | Manifest source | Source SHA-256 | Page / image | Pixels compared | Worst MuPDF / Poppler bit difference |
| --- | --- | --- | ---: | ---: | ---: |
| HN | `issue-43/Windows9x_NT操作系统的磁盘备份与恢复的研究与实现_张宗伟.caj` | `826608ef34b850926d1291ddba1773305bd44a0bf4170b4a9ae8c7d83c0b7134` | 2 / 1 | 8,124,608 | 0 / 0 |
| C8 | `issue-66/IDL编译器的实现_词法分析部分_于埴尧.caj` | `90e7b47716c32ef7a67cde8094f312e0ee7f1a7e0ed50a25830e8ba64a84f6a6` | 1 / 1 | 212,848 | 0 / 0 |
| Multi-image page | `issue-76/基于星载合成孔径雷达干涉测量技术的数字高程模型生成研究_任坤.caj` | `46779c74e34f1508125fe94f482672b4eb518436bc663dc5470df814cb41f0aa` | 23 / 3 | 7,731,927 | 0 / 0 |
| Opt-in anomaly | `issue-43/Windows9x_NT操作系统的磁盘备份与恢复的研究与实现_张宗伟.caj` | `826608ef34b850926d1291ddba1773305bd44a0bf4170b4a9ae8c7d83c0b7134` | 11 / 1 | 8,124,608 | 0 / 0 |

For every canary, MuPDF's 72-ppi rows matched every extracted pixel exactly;
the centre of each predeclared 10 × 10 Poppler block at 720 ppi matched its
source pixel exactly. The comparison does not require the other 99 pixels in
a Poppler block to match. The #43 oracle's external decoder backend
independence remains unverified, so agreement with its hashes does not prove
universal JBIG2 support.

| Resource maximum | Strict | Opt-in |
| --- | ---: | ---: |
| Single ranged source request | 65,536 B | 65,536 B |
| Rust converter's four scratch files at completion | 1,132,471 B | 1,132,471 B |
| Selected PDF file | 1,099,856 B | 1,099,856 B |
| Extracted PBM file | 1,098,877 B | 1,098,877 B |
| Post-child PDF/PBM/render validator files in one temporary directory | 104,605,361 B | 104,605,361 B |
| Rust child peak `VmHWM`, excluding validators | 2,544 KiB | 2,580 KiB |
| Rust ranged-reader bytes, excluding pre/post hash reads | 60,423,430 B | 60,431,570 B |
| Matched encoded bytes | 15,083,476 B | 15,086,207 B |
| Whole harness elapsed | 327.982 s | 335.083 s |
| Matched encoded MiB per whole-harness second | 0.044 | 0.043 |

The largest selected image was `issue-43` page 3/image 1: 8,790,912 pixels,
1,099,856 PDF bytes, and 1,112,371 Rust scratch bytes. The scratch maximum
belongs to a different case. The validator temporary-file maximum is measured
after the Rust child exits and its scratch files have been removed; it is not
a measured simultaneous scratch-plus-render size. A conservative
simultaneous-storage upper bound across the two phases is
`max(104,605,361, 1,132,471 + 1,099,856) = 104,605,361` bytes. Each
case-scoped temporary directory is cleaned after success or failure, including
a killed or timed-out child. Both policy runs overlapped on one machine, and
the whole-harness timing includes inventory, hashing, conversion, PDF
validation, and rendering; these figures are resource evidence, not a
converter-only throughput benchmark.
