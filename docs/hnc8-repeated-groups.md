<!-- SPDX-License-Identifier: MIT -->

# Repeated HN/C8 image groups

## Byte-derived rule

Some raw HN-A pages begin with the same compact image/control records used
by direct compressed frames. The shared 28-byte record consumer handles both;
the older glyph-first and tagged compressed profiles retain their checks.

The public `read_text_coordinates` API still requires one coordinate per
descriptor. Composition may accept fewer compact coordinates only when the
descriptor count is a positive integral number of coordinate groups. Each
additional descriptor must have the same type, payload length and every
payload byte as its corresponding first-group image. Two fixed 1 KiB buffers
compare the streams, respecting the configured I/O chunk size and cancellation.
Conflicts and incomplete groups are located errors before page image emission.
No sample hash, guessed repetition factor or image registry selects behavior.

Emit the first group once. Keep every source descriptor in visitor order;
aliases point to the original one-based image number. Ordinary repeated images
with their own coordinates are still drawn separately. Existing bounded page
metadata and scratch budgets apply; no decoded-document buffer is introduced.

## Real document and reference deviation

The external issue-76 HN-A document has source SHA-256
`46779c74e34f1508125fe94f482672b4eb518436bc663dc5470df814cb41f0aa`.
Its 163 indexed pages contain 368 descriptors but 210 coordinate records.
All 158 additional descriptors, across 26 pages, repeat their first groups
byte for byte. Groups contain two through six images.

Black-box Python revision `8cbc3c5721acb762f739434eb3d206171dbb022a`
emits 321 pages, including in a fresh working directory. The reference has
one complete page per source page followed by one extra single-image page
per repeated descriptor: `163 + 158 = 321`. Every predicted main page's image
count and every extra page's image dimensions were checked against qpdf.
Rust emits the 163 source pages with 161 type-3 and 49 JPEG draws, and reports
158 aliases. It intentionally does not reproduce the extra reference pages.

The first comparison also found invalid reference color declarations.
ImageMagick independently identifies 159 reference JPEG streams as Gray,
although their PDF dictionaries declare RGB. A separate reference changes
only those declarations to DeviceGray; all 368 encoded image streams,
their page order and dimensions are verified unchanged. The original output
and its failed comparisons remain available. Rust already emits Gray for
single-component JPEGs; no converter color change was needed.

Both original reference and native PDF pass qpdf 12.2.0. MuPDF 1.25.1 renders
selected complete pages at 300 ppi, grayscale, `-A 0`. Comparison uses every
pixel with no alignment or tolerance:

| Source page | Reference page | Draws | Original changed pixels | Gray-corrected changed pixels |
| ---: | ---: | ---: | ---: | ---: |
| 1 | 1 | 1 | 0 | 0 |
| 23 | 23 | 2 | 1,542,126 | 0 |
| 69 | 73 | 3 | 1,718,104 | 0 |
| 78 | 102 | 4 | 2,133,220 | 0 |
| 144 | 232 | 5 | 2,529,991 | 0 |
| 150 | 278 | 6 | 3,025,717 | 0 |
| 163 | 321 | 1 | 0 | 0 |

These are selected-page comparisons against an explicitly corrected Python
reference, not whole-document pixel parity or CAJViewer validation. Bookmarks
were not requested in this run. CAJViewer and production routing remain #129
and #10 work respectively.

## Resources and reproducibility

The complete native run took 107.35 seconds and measured 10,864 KiB child
process peak RSS using `resource.getrusage(RUSAGE_CHILDREN)` for the sole Rust
child. Validators ran separately. Aggregate scratch peak was 1,089,966 bytes;
all four backing stores were empty afterward. This is one document/run, not
a scaling claim. The diagnostic output limit is now 512 MiB to accommodate
the uncompressed 163-page output; production defaults are unchanged.

Native example binary SHA-256:
`d57c2bd847ec5cee9e42b15c15e803261d555b07017e5a6f1aeab9968ce2ea0d`.
Output PDF SHA-256:
`4c21130eb561fb48b22aa29701823df9066d015c3f137b120d849333d14f5708`.
The external `caj2pdf-issue118/reference76` bundle retains mapping observations,
original/corrected PDFs, comparison scripts/results and resource logs. The
example uses separately supplied T.82/MQ fixtures; no tables, documents,
reference implementation or derived pixels are committed.

## v0.x migration and tests

`ComposedImage::duplicate_of` is `None` for an emitted draw and `Some(n)` for
a verified alias of first-group image `n`. Consumers that count or display
draws should filter `duplicate_of.is_none()`. The alias transform and anomaly
report refer to that original draw. `ComposeReport::duplicate_image_records`
counts aliases; codec image counters count emitted images. Update exhaustive
report literals/patterns for the new field. CLI and JS routing is still gated.

Original MIT unit fixtures cover raw/direct framing, repeated type-3/JPEG
groups, byte-identical output versus one group, alias reporting, conflicting
types/lengths/payloads, partial/empty groups, short reads, I/O errors and
cancellation. Existing mixed 0/2/3 and cleanup tests remain in the normal
native and coverage gates. Missing external fixtures remain NOT_RUN.
