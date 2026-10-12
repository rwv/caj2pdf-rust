# Wine grayscale JPEG decode and interpolation controls

Related: [#534](https://github.com/rwv/caj2pdf-rust/issues/534),
[#541](https://github.com/rwv/caj2pdf-rust/issues/541), and parent
[#406](https://github.com/rwv/caj2pdf-rust/issues/406).
The [metadata receipt](wine-jpeg-decode-controls-20261011.json) retains identities,
page bounds, exact comparisons, numeric UI observations, cleanup and CI results.

## Result

Replacing grayscale JPEG streams with full-resolution decoded Flate streams
does not change any of nine selected Wine PDF frames. Setting `/Interpolate
true`, independently or together with that replacement, also changes none.
All **27 treatment frames equal their PDF baselines exactly**. Every baseline
still differs from the native source. These alternatives are not adopted.

Seventeen independent cold sessions yield 51 complete, stable, numeric-confirmed
page visits. Three additional source-repeat views and six comparisons with
historical source/PDF views are exact. A deliberately inverted-image control
changes all three selected frames, confirming that the acquisition detects
image-content changes. No document, font, PDF or screenshot body is published.

This result is specific to one grayscale document and these zooms. It does not
prove universal interpolation-flag disregard, native/PDF decoder equivalence,
correct image placement, or a renderer-only cause. It adds no fidelity pass.

## Inputs and protocol

The unchanged C8 original has SHA-256
`7e3f2c0faebb8ddac2f4feff3cfc9249476ab170b43250ac00610b10aa46c74b`.
Its ten-page PDF from the alphabet implementation
`7f8b7e75e4a5a13dd041011b4ddc759a2b288b31` has SHA-256
`ff3caa836050b94ce07ef826f94561d305a4dfddc959e06ecd45ea5b849b6a07`.
The working branch is at `d836ae7` before this research; the intervening changes
are research documentation and a JavaScript test-race fix. This is not a new
current-main baseline for #534.

Pages 1–9 each contain one 2480 × 3365 grayscale JPEG and no text draw.
Page 10 contains eight 708 × 897 JPEGs and 2,040 text draws. The experiment
selects pages **1, 5 and 10**, distinguishing image-only and mixed content.
It processes all 17 images in the PDF, one at a time, with a ten-million-pixel
bound per decoded image. This is an external research utility, not the
converter's bounded production I/O path.

All sessions use the established offline CAJViewer 7.3.151 Simple / Wine 8.0
32-bit profile, fixed 1600 × 1200 display at 96 DPI, and single-page mode.
At each of 50%, 75% and 100%, separate cold sessions open the source, rewritten
baseline, Flate, JPEG-with-interpolation and Flate-with-interpolation PDFs.
The initial plan also includes a second cold source session at 75%. After the
unchanged results, one additional 75% session checks an intentionally inverted
Flate PDF. That positive control is not a fidelity candidate.

The numeric page/zoom fields must agree before and after each pair of captures,
taken 0.6 seconds apart. Complete page crops include their observed borders;
only identically sized, stable crops enter exact RGB comparison. There is no
registration, resizing, fitted tolerance, JPEG screenshot normalization or
candidate-specific coordinate correction. Every session records intact inputs,
no observed OOM kill and container removal. All 17 sessions are admitted, with
no replacement or omitted startup attempt in this batch.

## Isolated changes

The [original MIT generator](../../scripts/research/wine_jpeg_decode_diagnostic.py)
creates five PDFs from an external input:

| Variant | Image representation | Interpolation |
| --- | --- | --- |
| Baseline | Original `/DCTDecode` bytes | Original setting, absent here |
| Flate | Pillow's full-resolution grayscale decode, zlib-compressed | Original setting |
| JPEG/interpolate | Original JPEG bytes | Explicitly true |
| Flate/interpolate | Same decoded grayscale bytes | Explicitly true |
| Positive | Deliberately inverted decoded grayscale bytes | Original setting |

All page bounds, decoded content-stream bytes and eleven original embedded font
programs remain identical. Thus image matrices, text identities, glyph selection,
paint order and drawing operators are unchanged. Flate round trips reproduce
the exact Pillow-decoded bytes. The helper rejects non-gray, masked or
decode-remapped images rather than silently converting their color semantics.

The public generator reproduces all four originally observed diagnostic PDFs
byte-for-byte, along with their full metadata; it also produces the observed
positive control. All five PDFs pass `qpdf --check`. This validation concerns
PDF structure and the declared changes, not visual correctness or independent
JPEG codec conformance. Pillow 12.1.1 uses libjpeg-turbo 3.1.3 here; pikepdf is
10.5.1 and zlib is 1.3.1. Native-viewer decode rows are not observed.

## Exact page comparisons

These counts are differing RGB pixels against Wine's native source. All three
treatments retain each baseline count and its exact raster.

| Zoom | Page | Baseline and each treatment |
| --- | ---: | ---: |
| 50% | 1 | 60,368 |
| 50% | 5 | 83,964 |
| 50% | 10 | 52,162 |
| 75% | 1 | 128,982 |
| 75% | 5 | 183,080 |
| 75% | 10 | 78,226 |
| 100% | 1 | 183,148 |
| 100% | 5 | 284,197 |
| 100% | 10 | 163,116 |

The three 75% fresh source frames also equal the retained historical source
frames. The three rewritten baseline frames equal the retained original PDF
frames, controlling for PDF reserialization. The extra cold source session
repeats all three frames exactly. Positive-control differences and individual
frame hashes are retained in the receipt, separately from fidelity comparisons.

## Validation and remaining acceptance

Before this research, CI on `35aec83` failed the existing file-truncation test:
the worker could reject while the test awaited `writeFile`, before
`assert.rejects` had attached its handler. Commit `d836ae7` attaches the same
assertion before that I/O yield and awaits both operations. It changes neither
production code nor accepted error codes. The normal concurrent local JS suite
passes **192 tests, zero failures and zero skips**, including real Chromium.
The initially sandboxed invocation that reported only a file-level result is
not counted. CI on `d836ae7` has all four quality gates and nine regular native
jobs successful: 15 successes including inputs/assets, with 17 optional/release
skips excluded from passing evidence. The old failed job remains linked in
the receipt; the fresh research commit has its own CI status.

This experiment narrows one hypothesis for two image-only pages and one mixed
page. **Status correction:** #534 had already closed through the explicit
unresolved-classification path in merged [#538](https://github.com/rwv/caj2pdf-rust/pull/538),
with original asymmetric controls and metadata in
[samples #115](https://github.com/rwv/caj2pdf-samples/pull/115). That earlier work
covers the 58 width cases and 14 selected image-only cases and retains their
unresolved visual differences; closure did not establish pixel parity.

The fresh main baseline, historical width cases and independent image controls
are outside this JPEG batch, rather than missing from #534's completed
classification. Future fidelity work should reuse those controls under #406;
font/placement work remains #541. No production defect is established here.
The 77-page font cohort retains its previously reported improvements and
remaining differences; it is not recounted as 77 new observations.

For reproduction, install the versions above and qpdf, then run:

```sh
python3 scripts/research/wine_jpeg_decode_diagnostic.py \
  /external/baseline.pdf /external/new-output-directory
```

Use the established cold-session protocol for selected pages at all three
zooms, retain both captures and numeric fields, and compare complete native
pixel arrays. Keep input/output binaries external. The generator and analysis
are independently authored MIT code; no vendor implementation, private HN/JBIG
or foreign converter code is inspected or copied. No production dependency,
API, allocation strategy, output behavior, format support or release changes.
