<!-- SPDX-License-Identifier: MIT -->

# Bounded HN/C8 type-2 JPEG marker profile

This note documents the marker reader added for [issue #102](https://github.com/rwv/caj2pdf-rust/issues/102), a child of [HN/C8 page conversion #10](https://github.com/rwv/caj2pdf-rust/issues/10). The intended caller obtains a page/image descriptor and absolute payload span from `Hnc8Reader`; the type-2 reader rechecks public record fields and then scans that span through end of image. It reports a JPEG profile; it does not decode pixels, write a PDF, determine source page placement, or establish that a full HN/C8 document converts correctly.

## Format basis and supported subset

[CCITT Recommendation T.81 / ISO/IEC 10918-1, Annex B](https://www.w3.org/Graphics/JPEG/itu-t81.pdf) defines JPEG marker and interchange syntax. The [ITU catalog](https://www.itu.int/rec/T-REC-T.81) identifies the 1992 edition and its corrigendum. [Recommendation T.871](https://www.itu.int/rec/T-REC-T.871-201105-I/en) and the original [JFIF 1.02 specification](https://www.w3.org/Graphics/JPEG/jfif.pdf) describe the JFIF application marker and YCbCr interpretation. These are sources for functional format facts only. Their text, tables, diagrams, sample bytes, and any reference software have not been copied into this MIT project.

T.81's nonhierarchical interchange form begins with SOI, has one frame with one or more scans, and ends with EOI. This reader recognizes an intentionally smaller profile: an 8-bit baseline SOF0 frame with one scan, one grayscale component or three components identified as YCbCr by a recognized JFIF APP0 marker and component IDs 1, 2, and 3. An Adobe APP14 marker is classified as unsupported without interpreting its application-defined fields. The reader checks length-delimited segments, marker ordering, scan component identities, marker-level entropy byte stuffing and restart ordering, and exact EOI/span termination. It does not verify entropy symbols or MCU counts. More than one scan can be valid T.81 baseline JPEG; it is currently reported as unsupported by this profile. Progressive, arithmetic, lossless, four-component/CMYK, changing-height DNL, unsupported precision, and uncertain three-component color interpretation are also outside this profile. A three-component count alone does not prove PDF `/DeviceRGB` color or a safe YCbCr conversion.

The pinned private HN/C8 observations use JFIF APP0 version 1.01, units 0, densities 1/1, and no thumbnail. The reader treats that as an observed compatibility profile; it does not call those files T.871-conformant version 1.02. The JFIF sources establish field meaning and color intent, while the observed version and exact marker order are corpus facts.

The HN/C8 descriptor's type field is the only source image-type discriminator. A JPEG-looking prefix inside a differently typed payload does not change its type. The reader's checked result may be used to select a later PDF passthrough path; that later work must independently verify color, dimensions, placement, page count, and rendered output. The existing `PdfDocument::add_image_page` can stream a JPEG span without collecting it, but this marker check alone does not prove that its PDF output is correct for the observed corpus.

## I/O and failure model

`hnc8::read_type2_jpeg_info` takes a ranged source, a checked `ImageRecord`, shared `Limits`, a `Cancellation`, and a JPEG-specific budget. It returns checked geometry, precision, components, marker/profile observations, and the absolute payload span. It rejects a non-type-2 or zero-identity record with a located error. Reads remain inside the checked span and use bounded buffers; segment lengths and scan work are checked before advancing. The reader does not retain a source-wide index or an image-sized byte vector. Caller-owned forward-only streams need a bounded seekable spool before this API can be used.

The default `JpegBudget` allows a payload of at most 64 MiB, at most 100,000 markers, and at most 64 MiB of source bytes fetched for this traversal. `Limits::max_input_bytes` bounds the whole source. The parser's fixed input buffer is 4,096 bytes, and each ranged request is further capped by `Limits::io_chunk_bytes`; it keeps only small frame/table flags and counters besides that buffer. These are implementation ceilings, not JPEG format maxima. A caller can lower them for untrusted workloads.

Malformed and truncated streams return errors with the source page/image identity and an absolute byte offset. A valid JPEG feature outside the measured subset is classified as unsupported, separate from malformed input. Cancellation, short reads without progress, and resource-limit failures stop scanning. Error classification is a parser observation, not a JPEG decoder's pixel verdict.

## Verification boundary

Original synthetic tests exercise HN-A, HN-B, and C8 descriptors, marker ordering, dimensions, components, application/comment segments, entropy stuffing/restarts, short reads, cancellation, limits, and located failures. An ordinary clean clone runs the inventory's public self-check and reports `NOT_RUN` with zero checked or matched private records. A requested run requires the external corpus, checks each pinned source size and full SHA-256 before and after its images, and checks each selected JPEG payload SHA-256 immediately before and after reading it. Missing or changed requested inputs fail the run. The test enumerates checked descriptors in the structurally valid sources, counts other record types and three known malformed container discoveries separately, and compares each checked type-2 descriptor and parsed header with the hash-only inventory. It does not decode or compare JPEG pixels.

```sh
CAJ2PDF_CORPUS_DIR=/private/CAJSamples \
cargo test --release -p caj2pdf-core --test hnc8_type2_jpeg_external --locked -- \
  --ignored --exact all_pinned_type2_jpeg_headers --nocapture
```

On 2026-09-27, the final-source release-mode opt-in test **passed** in 5.34 seconds. It used the unchanged #22 source catalog SHA-256 `e88401f0d9cbd08608004c2a9e58577ab85c916b9a9891a4dd5466e346e9203a` and the new [hash-only type-2 inventory](../tests/conformance/hnc8_type2_jpeg_inventory.tsv) SHA-256 `f582ffeb068eb32f7c6bcb0619a3bbd567008739a7f6b719b7c6e9eee97db0ac`. The parser source SHA-256 was `a16713f231d6b4f949a626f35d6c641b61b254d38d7570306c7eadeb629cfadf`, the opt-in test source SHA-256 was `e466171cf4628b7a8c209c200c812677fc6b594db9aaebcbde9d91245ccc2cad`, and the local release test executable SHA-256 was `926e7def8d16542d76c31a81bc52eb3e580b78a1305b2e5aea02580e6e7bca99`.

| Measured result | Count |
| --- | ---: |
| Type-2 descriptors whose checked spans and parsed headers matched | 1,085/1,085 |
| Pinned source identities unchanged before and after | 27/27 |
| Failed / unsupported / skipped type-2 records | 0 / 0 / 0 |
| HN-A grayscale / YCbCr | 739 / 314 |
| HN-B grayscale / YCbCr | 2 / 0 |
| C8 grayscale / YCbCr | 3 / 27 |
| Other checked descriptor types 0 / 1 / 3 | 1,400 / 6 / 546 |
| Separately pinned malformed container discoveries | 3 |

All 1,085 type-2 records in this pinned subset have SOI/EOI, one 8-bit SOF0 frame and one SOS, JFIF APP0 version 1.01 directly after SOI, no APP14, and no DRI. Their observed widths range from 143 to 2,481 pixels, heights from 101 to 3,598 pixels, and payload lengths from 1,824 to 2,015,864 bytes. Those are corpus measurements, not format limits or a claim that entropy data is valid.

| Measured resource | Maximum or total |
| --- | ---: |
| One parser/container ranged-source request | 4,096 bytes maximum |
| One selected JPEG payload | 2,015,864 bytes maximum |
| Parser plus container reader bytes fetched | 182,116,898 bytes total |
| Rust test process Linux `VmHWM` | 3,368 KiB |

The reader-byte total excludes the separate fixed 65,536-byte SHA-256 verification reads. No JPEG decoder or external executable runs in this test. The private CAJSamples documents and JPEG payload bytes remain outside Git, CI artifacts, and releases; the harness prints only counts, coordinates, status, and metadata hashes. Clean-clone corpus compatibility remains `NOT_RUN`/zero. No Table 24 or Annex E arithmetic-state data is needed by this marker reader, and it does not change the #30/#44 rights decisions.
