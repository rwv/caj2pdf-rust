<!-- SPDX-License-Identifier: MIT -->

# Selected HN/C8 type-2 JPEG PDF diagnostic

This note covers [issue #104](https://github.com/rwv/caj2pdf-rust/issues/104), a narrow child of [HN/C8 page conversion #10](https://github.com/rwv/caj2pdf-rust/issues/10). One checked type-2 source image is emitted as one PDF page. The resulting page is a diagnostic view of that image, not evidence that an HN/C8 source page with text or several images has been composed at its original positions. Full-document type-0 rejection policy, other image types, outlines, and released CLI/browser/Node HN/C8 conversion remain separate work.

## API and selection boundary

`hnc8::convert_type2_image_pdf(source, sink, selection, options, limits, cancellation)` takes a `RangedSource`, `SequentialSink`, one-based `Type2ImageSelection { page_number, image_number }`, `Type2PdfOptions`, and the shared `Limits` and `Cancellation`. It returns a `Type2SelectedPdfReport` with conversion byte/page counts, source variant and declared page count, the actual checked `ImageRecord`, and the JPEG marker profile. Errors identify the source page/image and absolute byte offset when known. Bytes already accepted by a failed caller-owned sink form a partial PDF and must be discarded.

The selected entry point uses `Hnc8Reader::probe_at_page`. It checks the selected source page and the descriptor chain through the requested image, intentionally skipping earlier page rows and payloads. It does not decode or represent neighboring images; later descriptors are not traversed. The chosen descriptor must be type 2, and [the bounded marker reader](hnc8-type2-jpeg.md) must accept its observed single-scan SOF0/JFIF profile before a PDF object is written. The converter reads source bytes through bounded ranges and writes PDF bytes sequentially. A forward-only caller must first provide bounded seekable backing.

The PDF writer streams the checked JPEG span unchanged into a `/DCTDecode` image XObject. `Type2PdfOptions::default()` sets 300 pixels per inch; the PDF MediaBox is `JPEG width × 72 / pixels_per_inch` by `JPEG height × 72 / pixels_per_inch` points. This is an explicit diagnostic scale, not a recovered HN/C8 page size or image placement. A grayscale image uses `/DeviceGray` and the three-component JFIF subset uses `/DeviceRGB` with `/DecodeParms << /ColorTransform 1 >>`. [Adobe PDF Reference 1.7](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.7old.pdf) §§3.3.7 and 4.8 describes the filter and image dictionary; its three-component default alone is not proof of correct displayed color. The [T.81/JFIF marker note](hnc8-type2-jpeg.md) records why APP14, uncertain color, progressive, and multiple-scan JPEGs are outside this observed profile. The converter does not add a JPEG decoder or rely on an executable at runtime.

The selected JPEG is checked once as a marker stream and hashed through bounded reads before PDF copying. The marker parser requires EOI at the declared span end and reads all skipped marker and entropy bytes, so its digest covers the full selected span. A second bounded digest of bytes actually read for the PDF must match, so a source changed between preflight and copy returns a located error instead of reporting successful passthrough. The caller must still keep its source stable outside the call, and the private conformance harness independently checks source and span SHA-256 values before and after each conversion. No whole JPEG or PDF buffer is retained.

## Verification boundary

Original synthetic HN-A/HN-B/C8 tests cover checked selection, grayscale and asymmetric color JPEGs, exact embedded stream bytes, page geometry, source changes, short reads, cancellation, sink errors, and limits. On an ordinary clone the optional private-corpus PDF check reports `NOT_RUN` with zero private matches. Requested runs require external CAJSamples files and independent PDF/JPEG tools, fail on missing or changed inputs, and keep sources, encoded JPEG bytes, PDFs, and rendered pixels outside Git, CI artifacts, and release packages.

Run the opt-in check only with a separately held copy of the pinned corpus:

```sh
CAJ2PDF_CORPUS_DIR=/private/CAJSamples \
cargo test --release -p caj2pdf-core --test hnc8_type2_pdf_external --locked -- \
  --ignored --exact all_pinned_type2_pdf_images --nocapture
```

Set `CAJ2PDF_HN_PDF_RENDER_ALL=1` for a separate requested run that renders and checks **every** pinned JPEG, rather than only the three fixed canaries. Both modes retain the same 1,085-image encoded-byte, source-identity, structure, and geometry checks. The baseline mode remains useful if an external renderer cannot process an otherwise valid ordinary image; its encoded result and the all-image pixel result are never conflated.

The requested harness sets 72 pixels per inch so one PDF page point maps to one source JPEG pixel in the render comparison; this does not change the API's 300-pixel-per-inch default. It uses installed `qpdf`, Poppler `pdfinfo`/`pdfimages`/`pdftoppm`, MuPDF `mutool`, and libjpeg-turbo `djpeg` as independent validators. They are test-time executables, not production dependencies. A missing validator must fail a requested run. A normal clone runs only the public metadata self-check and prints `NOT_RUN` with zero private cases.

Embedded JPEG byte equality and displayed pixel parity are separate results. `pdfimages -j` can confirm that the PDF carries the selected encoded span; independent direct JPEG decoding and Poppler/MuPDF page rendering must also check color and orientation on specified canaries. The observed #102 header count is not a PDF compatibility result. The exact #30/#44 arithmetic-state rights questions are unrelated to this JPEG-only diagnostic and remain unresolved for their codec paths.

The displayed-pixel rule was fixed after a three-image smoke diagnosis and before the final full-corpus run. Direct `djpeg` pixels are compared at the same coordinates with Poppler's decoded `pdfimages` output and with MuPDF's one-page raster; each comparison allows at most 8 levels in one channel and a mean of at most 1.5 levels across channels, to accommodate decoder rounding. The smoke cases were exact in both comparisons. Poppler's `pdftoppm` page raster showed one-pixel-local differences at high-contrast edges even at the selected 72-pixel-per-inch scale, consistent with resampling. Its raw pointwise worst and mean are reported, but pass/fail instead requires every channel to fall within the direct JPEG's clipped, same-coordinate 3×3 neighborhood with **zero** additional slack. Fixed colorful canaries also require at least ten stable, chromatic interior pixels that match within 8 levels at the same coordinates. Exact dimensions and nonblank horizontal/vertical asymmetry are required for the fixed canaries. This Poppler rule is renderer-specific and does not label its page pixels as exact matches; direct decoded and MuPDF comparisons provide the stricter color and orientation evidence. The image-sample orientation follows the [PDF image model](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.7old.pdf) §4.8 and the [JFIF interchange convention](https://www.itu.int/rec/T-REC-T.871-201105-I/en); the observed orientation result is measured by the asymmetric canaries.

The private harness caps each selected source at 8 GiB, JPEG at 64 MiB, temporary PDF at 16 MiB, decoded raster file at 64 MiB, and captured child-process text at 32 KiB. Verification hashes use a fixed 65,536-byte buffer; the core JPEG marker reader uses a fixed 4,096-byte buffer and its range requests are also bounded by the caller's `Limits::io_chunk_bytes`. One selected image owns one private temporary directory, removed when its check ends, before the next image starts. Measured Linux `VmHWM` is the Rust test process only; qpdf, Poppler, MuPDF, and libjpeg-turbo child-process residency is not included. Temporary disk and accounted row/I/O scratch maxima are reported separately below.

## Final-source private evidence

On 2026-09-27, both requested release-mode runs **passed** on the final source. They used the unchanged [#22 source catalog](../tests/conformance/jbig1_oracle.json), SHA-256 `e88401f0d9cbd08608004c2a9e58577ab85c916b9a9891a4dd5466e346e9203a`, and the [#102 type-2 inventory](../tests/conformance/hnc8_type2_jpeg_inventory.tsv), SHA-256 `f582ffeb068eb32f7c6bcb0619a3bbd567008739a7f6b719b7c6e9eee97db0ac`. The new converter source SHA-256 was `8531a9d35c1903db47984c12ff718a1b651f5a9a1a253a1d8667442f3c693767`; the PDF writer source was `124fcc1db9b3ceff48335b572ed0e2010c6affe4ae96ed6cf4328f91ad3e34ba`; the synthetic test was `929791c58f6693dad678b6c063be9f8592808ee517ff8620d061527ea3f802f5`; the optional harness was `afae9aef9103a52d54aec02859d353b46f2d35e8db1e66fb6e5398c278b8306a`. The local release test executable used for both modes had SHA-256 `26b763f31212848edb91c67c380f9204bcf6d86d15c0ae8855dbd80782730d6e`.

| Result | Fixed-canary mode | All-image mode |
| --- | ---: | ---: |
| Exact embedded JPEG SHA-256 matches after `pdfimages -j` | 1,085/1,085 | 1,085/1,085 |
| Direct JPEG versus decoded `pdfimages` pixels | 3/3; worst difference 0 | 1,085/1,085; worst difference 0 |
| Direct JPEG versus MuPDF page pixels | 3/3; worst difference 0 | 1,085/1,085; worst difference 0 |
| Poppler page rasters within zero-slack 3×3 neighborhood | 3/3; 0 violating channels | 1,085/1,085; 0 violating channels |
| Rendered grayscale / YCbCr images | 1 / 2 | 744 / 341 |
| Poppler raw pointwise worst / largest per-image mean | 198 / 5.2103 | 253 / 25.9543 |
| Stable chromatic pixels observed among rendered images | 6,350,150 | 144,550,013 |
| Fixed colorful-canary stable-chromatic violations | 0 | 0 |
| Pinned source identities unchanged before and after conversion | 27/27 | 27/27 |
| Failed / unsupported / skipped images | 0 / 0 / 0 | 0 / 0 / 0 |
| Separately reported malformed container discoveries | 3 | 3 |

Every selected PDF passed `qpdf --check`, one-page `qpdf`/`pdfinfo` geometry, one-image `pdfimages -list`, `/DCTDecode` and color dictionary checks, and exact encoded JPEG extraction before any rendered result was counted. The three fixed nonblank and orientation-asymmetric identities are source index 0, page 1/image 1 (HN-A color); index 5, page 1/image 2 (C8 color from a multi-image page); and index 12, page 1/image 1 (HN-B gray). Their direct-versus-decoded and MuPDF differences were all zero. Their Poppler raw worst/mean pairs were respectively 145/2.0320, 67/2.8152, and 198/5.2103, with no neighborhood or stable-color violations. Across all 1,085 images, the decoded and MuPDF pixels were pointwise exact against `djpeg`; Poppler page pixels were **not** pointwise exact, and the reported raw worst and largest per-image mean need not come from the same image. The Poppler local-envelope result is a separate, weaker renderer-specific check.

| Measured value | Fixed-canary mode | All-image mode |
| --- | ---: | ---: |
| Elapsed wall time | 301.227 s | 587.355 s |
| Selected JPEG throughput, including verification and child tools | 0.576 MiB/s | 0.296 MiB/s |
| Largest converter source request | 65,536 bytes | 65,536 bytes |
| Largest one-image PDF | 2,016,897 bytes | 2,016,897 bytes |
| Largest extracted JPEG | 2,015,864 bytes | 2,015,864 bytes |
| Largest one-image raster | 24,075,344 bytes | 26,117,504 bytes |
| Largest simultaneous one-image temporary storage | 97,452,664 bytes | 106,889,641 bytes |
| Accounted row/I/O scratch bound | 296,012 bytes | 296,012 bytes |
| Converter ranged-source bytes read | 364,237,044 bytes | 364,237,044 bytes |
| Separate full-source/span identity hash reads | 48,618,356,582 bytes | 48,618,356,582 bytes |
| Rust test-process Linux `VmHWM` | 3,692 KiB | 3,740 KiB |

The tools were qpdf 12.2.0, Poppler `pdfinfo`/`pdfimages`/`pdftoppm` 25.03.0, MuPDF `mutool` 1.25.1, and libjpeg-turbo `djpeg` 2.1.5. The peak request covers metered converter reads; the separate source/span SHA-256 verification uses the fixed 65,536-byte buffer. The scratch bound is an accounting of named buffers, not process residency. Both requested tests exited successfully, and no per-image private temporary directory remained afterward. The external corpus, raw JPEG/PDF/raster outputs, and private logs remain outside Git, CI artifacts, and releases. Clean-clone tests report `NOT_RUN` with zero checked, matched, or rendered private images; they do not establish corpus compatibility. This evidence validates one selected image per diagnostic PDF, not full HN/C8 page composition or standalone HN/C8 conversion.
