<!-- SPDX-License-Identifier: MIT -->

# External HN/C8 generic-only JBIG2 pixel oracle

This note supports [issue #51](https://github.com/rwv/caj2pdf-rust/issues/51)
under [issue #50](https://github.com/rwv/caj2pdf-rust/issues/50). The oracle
measures the output of each type-38 **generic region alone**. The
[full-image oracle](jbig2-oracle.md) includes text and symbol-dictionary
composition; its hashes answer a different question. This script does not
decode pixels in Rust or claim #49 parity.

## Pinned inputs and profile

The script uses the [CAJSamples matrix](../tests/conformance/matrix.json) at
revision `7e1c35e7b6de34e21972fcd1752c2a7e99b4ad07` and rehashes **all
27 HN/C8 sources** before and after a run. The repository-owned
[#42 inventory](../scripts/jbig2_directory_inventory.py) checks each of the
five image-bearing source files again, discovers type-3 records using its
MIT HN/C8 helpers, and calls Rust `read_embedded_directory` to validate
header framing. The generic-only oracle consumes this inventory through the
[#43 helper](../scripts/jbig2_oracle.py); it does not implement another
container parser.

| Source | Variant | Images | Source SHA-256 |
| --- | --- | ---: | --- |
| `issue-43` | HN | 105 | `826608ef34b850926d1291ddba1773305bd44a0bf4170b4a9ae8c7d83c0b7134` |
| `issue-58` | C8 | 4 | `8974d024e0cbb54009419aa8c91c9ba286dd74f056c3b19524ee5c626c947c85` |
| `issue-66` | C8 | 1 | `90e7b47716c32ef7a67cde8094f312e0ee7f1a7e0ed50a25830e8ba64a84f6a6` |
| `issue-76` satellite sample | HN | 208 | `46779c74e34f1508125fe94f482672b4eb518436bc663dc5470df814cb41f0aa` |
| `pull-72` | HN | 228 | `01558ff7e30c3131c79fc3267e6eb99c055cc9cb8e1b0e710bc6bd0791c888ab` |

For each of the 546 records, the profile reader checks the 48-byte DIB and
palette, page-information segment #0, and full-page immediate generic-region
segment #4. The selected #0 header begins exactly after the DIB; the #4
header and data end exactly at the record boundary. Header-plus-data spans
must stay within that image and cannot overlap. Their exact original bytes
are SHA-256 checked before extraction and during copying. The #43 profile
helper validates page dimensions and flag `0x01`, region width, height, and
origin, generic flag `0x04`, and adaptive-template bytes `02 FF`. Under
[ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en)
§§7.4.1, 7.4.6, and 7.4.8, the observed generic flag means arithmetic
coding, template 2, and typical prediction disabled; the signed adaptive
pixel coordinate is `(2,-1)`.

## Pixel method

For one image at a time, the script writes a temporary 48-byte DIB followed
by the original complete segments #0 and #4. Each source span is copied in
at most 64 KiB chunks, and its copied-byte SHA-256 must match a separately
read source-span SHA-256. The [#43 PDF writer](../scripts/jbig2_oracle.py)
then makes a temporary one-image `/JBIG2Decode` PDF containing **only those
two segments**. It checks the exact temporary-record bytes again while
spooling the PDF. `qpdf --check` must return zero with no `warning:` line
before pixel results are used.

Poppler `pdfimages` extracts a native-size P4 PBM; MuPDF `mutool draw` renders
a P4 PBM at 72 dpi. The shared #43 normalization compares every row top to
bottom, MSB first, `1` meaning black, with unused low row-padding bits set to
zero. Both dimensions must match the DIB. The source, temporary record, PDF,
PBMs, and logs are never put into Git; all generated per-image files are
deleted after success or failure. A failed case reports source ID, page,
image, and the absolute source spans of both selected segments.

Two independently measured preliminary spots match this method:

| Image | Dimensions | Normalized P4 SHA-256 | Black pixels |
| --- | ---: | --- | ---: |
| HN `issue-43`, page 2 image 1 | 2368 × 3431 | `72170496b556f7628b436b8e924e9bc4aa2815dc8d31106ab64e8ea0dbecde8f` | 94,550 |
| C8 `issue-58`, page 1 image 1 | 2366 × 3368 | `dae0fec2ea4c15de4b70f590a6bb3629f8bf17c225f0d0d4427743a04084fcb6` | 393,170 |

The [versioned metadata manifest](../tests/conformance/jbig2_generic_oracle.json)
stores source identity, image coordinate and dimensions, both original
header-plus-data span offsets/lengths/SHA-256 values, normalized generic-only
pixel SHA-256, black-pixel count, tool identities, and per-case status. Its
validator rejects unknown fields at every object level. A repeated run fails
on semantic drift while reporting toolchain drift separately. These hashes
do not contain source, segment, PDF, or bitmap bytes.

On the measured Linux toolchain, `ldd` shows MuPDF dynamically linking
`libjbig2dec.so.0`, while it shows no such dynamic link for Poppler
`pdfimages`. That does not prove separate decoder implementations. The
manifest therefore marks backend independence `UNVERIFIED`; a `PASS` means
**two black-box tools agreed**. No decoder source was inspected or copied.

Run from the repository root:

```sh
python3 scripts/jbig2_generic_oracle.py --corpus-dir /path/to/CAJSamples --json
```

The first evidence-producing run can write the metadata manifest only after
all 546 cases agree:

```sh
python3 scripts/jbig2_generic_oracle.py --corpus-dir /path/to/CAJSamples --write-manifest --json
```

Without the external corpus or a required black-box tool, the command
reports `NOT_RUN` and zero agreements. A supplied but changed source,
malformed span, unexpected profile, qpdf warning, pixel mismatch, or semantic
manifest drift reports `FAIL`. The required clean-clone tests use original
synthetic bytes and explicitly check `NOT_RUN`; they do not substitute for
the external 546-image run.

## Bounds and measured run

The Python path holds a 64 KiB copy chunk, the bounded #42 metadata
inventory, and one PBM row per external tool during comparison. The selected
record is capped at 64 MiB; each decoded PBM is checked against a 128 MiB
raster limit after its external tool exits. One case uses a temporary selected
record, PDF, two PBMs, and tool logs; the next case starts after these are
removed. External tools can write outputs and logs before post-run size checks,
so these validation limits are **not hard execution-time disk quotas**. Their
RSS is also outside the Python buffer bounds.

The measured run on **2026-09-25 UTC** used the corpus revision and source
digests above, qpdf 12.2.0, Poppler `pdfimages` 25.03.0, and MuPDF `mutool`
1.25.1. The [manifest](../tests/conformance/jbig2_generic_oracle.json) pins
the three exact executable SHA-256 values. A first baseline generation and a
second comparison with parent/child memory accounting each passed **546/546**,
with 27 source digests checked before and after, zero failures, zero skipped
images, and no toolchain drift. The second run measured:

| Measure | Result | Interpretation |
| --- | ---: | --- |
| Whole command | 132.568 s | Includes source checks and Rust directory inventory. |
| Image processing | 130.958 s | All 546 generic-only two-tool comparisons. |
| Slowest image | 0.505 s | Python per-case wall interval. |
| Largest selected record | 76,108 B | Temporary DIB plus only segments #0 and #4. |
| Largest temporary PDF | 76,766 B | One `/JBIG2Decode` image. |
| Largest single PBM raster | 1,098,864 B | One canonical image plane, excluding its small header. |
| Parent Python `VmHWM` | 25,052 KiB | Linux `/proc/self/status`, measured after the command. |
| Maximum waited child `ru_maxrss` | 26,420 KiB | `resource.getrusage(RUSAGE_CHILDREN)`; maximum child high-water mark, including the inventory process and black-box tools, not a sum. |

Across these successful cases, the four image files' separate maxima sum to
2,350,602 bytes for selected record, PDF, and two PBM rasters. This is a
conservative upper estimate for that part of **one** case's simultaneous
temporary storage; PBM headers, logs, filesystem overhead, and any transient
tool files are additional. Parent `VmHWM` and child `ru_maxrss` describe
different processes and cannot be added to infer concurrent peak RSS. The
initial generation's image processing finished in 116.812 s; the second
command is the measured comparison above.

This baseline supplies reference hashes to #50. It makes no claim that the
#49 Rust region decoder matches them. The raw Rust region-bit polarity must
be checked during #50 comparison; P4's `1=black` is only the canonical
oracle convention. Symbol dictionaries, text regions, full-page composition,
and HN/C8 conversion remain open under #9. Exact T.88 arithmetic-state rows
stay outside this MIT repository pending #44's provenance decision.
