<!-- SPDX-License-Identifier: MIT -->

# HN/C8 type-0 PDF pixel diagnostic

This note records the optional verification for [issue #100](https://github.com/rwv/caj2pdf-rust/issues/100). It joins the checked [HN/C8 reader](hnc8-container.md), caller-table [type-0 row decoder](jbig1-type0-rows.md), and [PDF writer](hnc8-type0-pdf.md) through the selected-image API. The full-document converter keeps its documented rejection policy for zero-image, mixed-type, and multi-image pages. The selected API is a diagnostic: it validates the actual requested descriptor and its preceding same-page descriptor chain, while deliberately skipping earlier source pages. One selected type-0 record becomes one PDF page; this is not a claim about full HN/C8 page placement.

## Opt-in protocol

Ordinary clean-clone tests report `NOT_RUN`, with zero checked or matched corpus images. To request the private run, supply the separately held, SHA-pinned CAJSamples corpus and T.82 fixture:

```sh
CAJ2PDF_CORPUS_DIR=/private/CAJSamples \
CAJ2PDF_T82_VECTOR_FILE=/private/t82-vector.txt \
cargo test --release -p caj2pdf-core --test hnc8_type0_pdf_external --locked -- \
  --ignored --exact all_pinned_type0_pdf_images --nocapture
```

The ignored harness checks the [#22 hash-only manifest](../../tests/conformance/jbig1_oracle.json) and caller-supplied table digest before work. For each source, it checks its SHA-256 before and after that source's selected conversions; for each selected image, it checks its encoded-span SHA-256 before and after conversion and compares the checked `ImageRecord` returned by the core API. A requested run fails on missing or changed inputs. The three pinned malformed `issue-100` discoveries remain separate from the 1,400 type-0 images and are never counted as successful conversion.

Each image is streamed to one private temporary PDF, independently reopened with `qpdf`, and extracted as a binary PBM with `pdfimages`. The test requires one 1 bpp grayscale image and a one-page PDF with `/Decode [1 0]` (set bit paints black). The PBM's packed, top-down display rows are reversed to the #22 DIB-memory order; unused low bits are masked and zero padding is appended to the checked 32-bit DIB stride. Both visible-bit and raw-stride SHA-256 digests must match. An original asymmetric synthetic PDF tests the extraction polarity and row-order assumption; selected nonblank HN, C8, and multi-image records are rendered independently with Poppler and MuPDF. This procedure compares pixels of the observed type-0 subset, not text layers, bookmarks, other image types, or a complete HN/C8 document PDF.

The requested run requires `qpdf`, Poppler's `pdfimages`, `pdfinfo`, and `pdftoppm`, and MuPDF's `mutool`. A missing or failing validator makes the requested comparison fail; it cannot become a skipped or matched image. Ordinary clean-clone tests still report `NOT_RUN`/zero without private inputs.

The harness holds only bounded manifest/table data, per-row pixel buffers, the decoder's row/context state, and one selected temporary PDF and extraction at a time, plus render outputs for the three fixed canaries. It caps ranged source requests, the PDF spool, and raster file sizes. Each image uses a unique private temporary directory that is removed after success or failure. The reported `process_vm_hwm_kib` is the Rust harness's Linux high-water RSS and excludes child PDF tools; temporary-storage and accounted row/I/O scratch maxima are reported separately. The private corpus, exact Table 24 states, official vector, external oracle binary, PDFs, and bitmaps remain outside Git, CI artifacts, and release packages. The caller-table result cannot resolve [#30 Table 24 provenance](https://github.com/rwv/caj2pdf-rust/issues/30) or enable standalone HN/C8 conversion in the CLI, browser, or Node.js.

## Measured evidence

On 2026-09-27, the final-source, release-mode opt-in test **passed** in 285.70 seconds against the unchanged #22 manifest (`e88401f0d9cbd08608004c2a9e58577ab85c916b9a9891a4dd5466e346e9203a`) and private caller-supplied T.82 fixture (`11fe241dedbbf4faa542af4a1485566c2794fa69e5c06e2e5c8542adfe9b1ab7`). The test source SHA-256 was `575382f42cf148962eed329bde27ca8b2266d478928f3f1dc4deacf2d299c6ed`; the local release test executable SHA-256 after the run was `ff7dc44044430074d7a9c0723bc8530d7822e8af3fea9c42e634140715aef672`. The private log and all input/output files stayed outside the repository.

| Result | Final-source private run |
| --- | ---: |
| Type-0 images with exact extracted visible-bit and DIB raw-stride hashes | 1,400/1,400 |
| Pinned source identities checked before and after their images | 27/27 |
| Failed / skipped / unsupported type-0 images | 0 / 0 / 0 |
| Valid blank images matched by hash | 2 |
| Multi-image source pages whose type-0 images all matched | 6/6 |
| Separately pinned malformed discoveries, never converted | 3 |
| Fixed HN, C8, and multi-image render canaries | 3/3 passed |

Every selected PDF passed `qpdf --check`, one-page/geometry and one-bit image checks, and `pdfimages` extraction before its two hash comparisons. The three fixed nonblank canaries also passed independent Poppler and MuPDF black-pixel rendering. The tools were qpdf 12.2.0, Poppler `pdfimages`/`pdftoppm` 25.03.0, and MuPDF `mutool` 1.25.1. The fixed keys in the pinned manifest are sample index 0, page 2/image 1 (HN); index 5, page 1/image 1 (C8); and index 13, page 3/image 1 (a multi-image C8 page). Poppler rendered at 720 dpi and the test compared each 10×10 block's centre; MuPDF rendered at 72 dpi for one pixel per source pixel.

| Measured maximum | Value |
| --- | ---: |
| Converter ranged-source request | 256 bytes |
| One temporary PDF | 1,092,291 bytes |
| One extracted PBM | 1,091,312 bytes |
| One independently rendered PBM | 105,678,465 bytes |
| Simultaneous one-image temporary storage | 108,852,793 bytes |
| Accounted row/I/O scratch bound | 77,769 bytes |
| Rust harness Linux `VmHWM` | 3,864 KiB |

The 256-byte request maximum covers metered converter reads; independent SHA-256 verification uses a fixed 65,536-byte buffer. The scratch figure conservatively accounts for named row/I/O buffers, 1,024 context states, the 113 caller-supplied states, and QM lookahead; it is not total process residency. The 10× Poppler render dominates temporary disk storage and runs as a separate child process. The per-image private temporary directories were absent after the run. A clean clone still reports `NOT_RUN` with zero submitted, checked, or matched private-corpus cases. This evidence proves the selected observed type-0 pixels through a one-image PDF path only; #28 full-document integration and #30 table rights remain open.
