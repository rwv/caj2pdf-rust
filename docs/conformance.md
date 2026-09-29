# Conformance baseline

Current selected-page vendor results are in [CAJViewer fixtures](cajviewer-fixtures.md).
CAJ/PDF/KDH selected pages match. After #184, HN-A/C8 page-frame sizes match,
but exact pixels still differ; these profiles remain experimental. Capture
repeatability is scoped to each report. Historical launch/fixture issues
#124–#129 are closed or consolidated into #123.

The [corpus matrix](../tests/conformance/matrix.json) inventories unique inputs
from a pinned revision of the external
[CAJSamples](https://github.com/caj2pdf/CAJSamples) repository. Its
[provenance note](../tests/conformance/README.md) explains canonical paths,
type aliases, reference versions, and measured results. CAJSamples has no
redistribution grant recorded for this project. Keep its documents and every
PDF derived from them outside this repository.

## v0.1 support and release status

This is the current CLI, Node and browser support summary. “Supported” is
limited to each documented input profile, not every file with that signature.
The same conversion core serves all three interfaces.

| Profile | Status on all three interfaces | Verified scope and limits |
| --- | --- | --- |
| PDF | Supported within the [PDF input profile](pdf-input.md) | Representative 11-page output is identical across interfaces; selected viewer pages 1 and 11 match. |
| CAJ | Supported within the [CLI profile](cli.md) | Representative 75-page output with 58 bookmarks is identical across interfaces; selected viewer pages 1 and 75 match. Optional legacy Python ordering is deferred (#21). |
| KDH | Supported for validated embedded PDFs | Representative one-page output is identical across interfaces and matches the selected viewer page. |
| HN-A | Experimental image-page conversion | Complete 163-page, 96-bookmark output is identical across interfaces. Declared page/display extents are used; selected frame sizes match, but exact pixels differ. Physical units remain empirical. |
| C8 | Experimental image-page conversion | Complete four-page output is identical across interfaces. Explicit bookmark omission is required; outline semantics are unverified. Selected frame sizes match, but exact pixels differ. |
| HN-B | Experimental single-JPEG image-bearing rows | Unknown outlines require explicit omission. A source row without image content is rejected by public conversion, never silently dropped. No vendor rendering parity is claimed here. |
| TEB, unrecognized layouts, unsupported image modes, pure-text/searchable HN | Rejected | No OCR or silent omission fallback. Located errors identify unsupported HN/C8 content. |

Arithmetic HN/C8 images use built-in standard QM/MQ states. Optional custom
state overrides remain supported. The owner-directed adoption and upstream
practice are recorded in [provenance](provenance.md); #189 completed #30/#44.
JS arithmetic image decoding still needs bounded caller-owned scratch stores.

[Viewer results](cajviewer-fixtures.md) record the pinned application and
selected-page scope. [Complete HN/C8 checks](js-validation.md#source-geometry-correction-repeat)
record output hashes, page counts, outline retention and image-stream checks.
Python-reference corpus expectations below are a separate compatibility
baseline, not a CAJViewer verdict. Missing optional inputs are `NOT_RUN`;
known pixel failures are not passing baselines.

### Release evidence and remaining work (#14)

- Existing original tests cover short I/O, malformed input, cancellation,
  bounded scratch, output cleanup and source-page omission refusal. The
  [JavaScript delivery report](js-validation.md#verified-delivery-paths) covers
  examples and conversion from the extracted npm artifact on Node and Chromium.
- [Memory measurements](js-validation.md#memory-and-temporary-storage) cover
  small/large original PDF inputs and the complete HN-A/C8 repeats. Native
  child RSS, sampled Node RSS, WASM linear memory, scratch caps and cleanup
  have distinct scopes; they do not establish a browser-process RSS peak or
  constant memory for arbitrary documents.
- #186 passed Native, WASM, MIT audit and the 100% Rust line-coverage gate.
  This is development-head evidence, not certification of a future release.
- Release remains pending codec integration acceptance (#8/#9), final artifact
  inspection and checksums, and English release notes with breaking changes
  and migration examples. Keep npm private and Cargo publishing disabled until
  the release commit follows the [release policy](release-policy.md).

## Commands and status

From a clean clone, run the unit tests and check that the original MIT
fixtures match their generator:

```sh
python3 scripts/generate_fixtures.py --check
python3 -m unittest discover -s tests/fixtures -p 'test_*.py'
python3 -m unittest discover -s tests/conformance -p 'test_*.py'
python3 scripts/conformance.py
python3 scripts/jbig1_oracle.py --json
python3 scripts/jbig2_directory_inventory.py --json
```

The optional commands print `NOT_RUN` for external checks when their
inputs are absent. The PDF command prints `NOT_RUN` for the external corpus when
`CAJ2PDF_CORPUS_DIR` is unset. This is a visible skip, never a compatibility
pass. To request an inventory run, point the variable at a local checkout of
the pinned CAJSamples revision:

```sh
CAJ2PDF_CORPUS_DIR=/path/to/CAJSamples python3 scripts/conformance.py --json
```

The [JBIG2 directory inventory](jbig2-directory.md#optional-external-metadata-inventory)
uses the same pinned corpus to check the HN/C8 type-3 segment headers through
the Rust core API. It reports `NOT_RUN` when the corpus is absent and does
not claim decoded-pixel compatibility.

The runner checks all canonical files, including size and Git blob hash, using
bounded reads. A missing file, changed hash, unreadable file, or path escaping
the corpus root fails the requested run. Type aliases do not cause duplicate
runs. The concise report distinguishes `PASS`, `FAIL`, `UNSUPPORTED`,
`EXCLUDED`, and `NOT_RUN` for the inventory and PDF checks. An inventory
`PASS` means only that the local corpus matches the pinned matrix. It is not
a Rust conversion result.

Once a converter produces PDFs, place them outside the repository and pass
`--pdf-dir /path/to/output`. Each output path mirrors the canonical input path
with a `.pdf` suffix: `issue-1/a.caj` maps to `issue-1/a.pdf`. The runner
compares available page counts, page dimensions, outline hierarchy and
destinations, and rendered-page hashes against recorded expectations. A
requested PDF comparison fails if an expected output or required inspection
tool is missing. A complete output `PASS` requires all five checks and a
recorded render hash for every page. Unknown reference outcomes or incomplete
successful rows report `NOT_RUN`; a requested `--pdf-dir` exits nonzero unless
the aggregate PDF status is `PASS`. Known reference errors are `EXCLUDED`
from the successful-conversion scope, while known unsupported inputs remain
`UNSUPPORTED`. Top-level page and outline counts are Python `show` observations;
`expected_pdf.page_count` and `expected_pdf.outline_count` are authoritative
for converted PDF output when they differ. `--json` provides a
machine-readable report for later release gating.

To verify one format as its implementation lands, add `--only-format KDH`
(or another detected format). The runner still validates the full matrix,
then checks only the selected corpus files and output PDFs. The JSON report
names `selected_format` and counts only that subset. Missing selected inputs
or requested outputs fail; omitted formats are outside the reported result. For KDH,
the selected baseline has three Python-success documents and 74 fully
fingerprinted pages:

```sh
CAJ2PDF_CORPUS_DIR=/path/to/CAJSamples \
  python3 scripts/conformance.py --only-format KDH \
  --pdf-dir /path/to/output --json
```

PDF inspection and rendering use a separately installed, version-recorded
`mutool` command. Its source and output PDFs are never vendored here. Exact
render hashes are comparable only with the recorded rendering options and
tool version; a different version requires rebaselining and review. The
synthetic [fixture manifest](../tests/fixtures/manifest.json) includes PDF
structure cases that can test these checks without an external document.

## JBIG1-like image oracle

The separate [type-0 image manifest](../tests/conformance/jbig1_oracle.json)
contains only pinned source/image metadata and decoded pixel hashes. Its
[observation note](jbig1-oracle.md) records the independent HN/C8 byte layout,
external decoder provenance, hash definitions, and secondary PDF extraction
checks. The opt-in runner uses the same external corpus plus a separately
built, **non-distributed** black-box native oracle:

```sh
python3 scripts/jbig1_oracle.py \
  --corpus-dir /path/to/CAJSamples \
  --oracle-lib /path/to/external/libjbigdec.so \
  --json
```

The ordinary Rust build and CI do not require that external library. A
clean-clone run validates the manifest's schema and reports the image work as
`NOT_RUN`; it does not claim pixel compatibility. A requested run verifies
input hashes, rediscovers image spans, and compares every requested image's
raw-stride and visible-bit hashes with isolated, timed decoder calls. The
pinned corpus currently has 1,400 measured type-0 images, plus three
separately recorded discovery errors in `issue-100` (one image descriptor and
two page rows). Even when all
1,400 images match, the three expected invalid records remain visible in the
discovery report as `expected_invalid_records: 3`, separate from pixel passes.
A changed or new invalid record fails discovery. No corpus
document, decoded bitmap, derived PDF, or differently licensed decoder binary
belongs in this repository or its release artifacts.

The optional [standard T.82 probe](../scripts/jbig1_standard_probe.py) tests a
finite set of constructed BIH/stripe settings against one selected HN/C8
image from that manifest. It requires an external standard `jbgtopbm` binary
and the pinned corpus; the executable stays outside this repository. This
optional probe runs on Linux/POSIX because it limits child output with
`RLIMIT_FSIZE`:

```sh
python3 scripts/jbig1_standard_probe.py \
  --corpus-dir /path/to/CAJSamples \
  --decoder /path/to/jbgtopbm \
  --sample-id issue-33/test1.caj --page 1 --json
```

With no probe options, it reports `NOT_RUN`; an incomplete explicit request
fails. `NO_MATCH_IN_TESTED_GRID` means none of the decoded outputs matched
the oracle's visible pixels. `VISIBLE_ONLY_IN_TESTED_GRID` means at least one
setting matched visible pixels but no setting matched the complete stride
hash in the same orientation. A match on an all-zero image is explicitly
`BLANK_MATCH_NON_DISCRIMINATING`; a visible-only blank result is
`VISIBLE_ONLY_BLANK_NON_DISCRIMINATING` and is not a full match. The
probe hashes each valid PBM both in its returned row order and with rows
reversed. In each order it compares visible pixels (unused low bits masked)
and the complete DIB stride against the manifest. Since PBM has no DIB row
padding, the stride comparison assumes zero padding while retaining the
PBM's actual unused low bits. `MATCH` requires both hashes to agree in the
same row order; `VISIBLE_MATCH_RAW_MISMATCH` means visible pixels agree but
the raw stride does not for an individual setting. The parser follows the
[Netpbm raw PBM format](https://netpbm.sourceforge.net/doc/pbm.html) for P4
magic, decimal dimensions, and whitespace or comments before the dimensions.
It also accepts a comment directly after the height digits. The first
whitespace after height is always the single raster delimiter; a `#` after
that byte belongs to the raster, so whitespace-then-comment after height is
rejected. The header is limited to 1,024 bytes, and exactly one image of the
expected raster length is required. If every setting fails to decode, the
report is `INCONCLUSIVE_NO_DECODABLE_SETTINGS` and exits nonzero. The
[experiment note](jbig1-bitstream-investigation.md) records the tested grid,
positive controls, refuted hypotheses, row-order evidence, and unresolved
CAJ-specific rules. Neither result claims full JBIG1 compatibility.

## Selected HN/C8 type-3 PDF pixels

The [#106 selected type-3 PDF diagnostic](hnc8-type3-pdf.md) converts one
checked HN/C8 JBIG2 image record into one bilevel PDF page with a
caller-supplied, privately held T.88 MQ table. The optional
[`jbig2_page_pdf_parity.py`](../scripts/jbig2_page_pdf_parity.py) runner
requires the pinned external CAJSamples corpus and private table, then checks
each selected PDF using `qpdf`, Poppler, and fixed MuPDF/Poppler render
canaries against the [#43 hash-only pixel oracle](jbig2-oracle.md). It keeps
strict-valid image matches separate from the single named opt-in `0xa40c`
case, and reports the expected strict refusal separately. A clean clone
reports `NOT_RUN` and zero PDF pixel compatibility matches. Source documents,
MQ state rows, generated PDFs, and bitmaps are never committed. This check
does not establish multi-image HN/C8 page placement or independence of the
external oracle's decoder backends; [#107](https://github.com/rwv/caj2pdf-rust/issues/107)
tracks source-page layout measurement.

## HN/C8 source-page layout metadata

The [#107 layout oracle](hnc8-layout-oracle.md) is an opt-in, metadata-only
black-box comparison against a fixed Python reference revision. It checks
27 SHA-pinned HN/C8 sources, three deterministic reference PDFs, the original
75 pages/125 ordered image draws and a separate two-page HN-B omission case.
qpdf, MuPDF and Poppler independently check boxes, image order, transforms,
types and encoded-stream hashes. The committed oracle contains coordinates,
dimensions and hashes only; no private documents, PDFs, text or pixels. A
clean clone reports `NOT_RUN` and zero layout matches. That metadata-only
phase measured 50 extra-image placements without identifying their source
fields. Later [#112 empirical placement rules](hnc8-placement-rule.md) and
[#117 page composition](hnc8-page-composition.md) establish a bounded
caller-table diagnostic for the selected profiles. Their Python-reference
basis does not establish vendor page fidelity or full-family conversion.

## CAJViewer vendor fixtures

Follow the [simplified fixture plan](cajviewer-fixtures.md) and
[epic #123](https://github.com/rwv/caj2pdf-rust/issues/123). Prove one practical
capture recipe, save a small external image baseline, and collect ordinary-copy
text where available. Manual initial capture is acceptable. #128 comparison
can start with original fixtures; it depends only on completed #125.
Text unavailability is recorded and does not block the image route.

Actual complete-page/text compatibility remains **`NOT_RUN`, zero passes**.
The [V14 inventory](cajviewer-runtime-view-v14.md) and
[twelve earlier launch observations](cajviewer-linux-startup.md) are retained;
they do not prove document compatibility. #153 is cancelled as a standalone
source-loading prerequisite. The revised plan supersedes the old future
proof/launch-approval requirements, without changing historical outcomes.

Keep vendor/corpus artifacts external, preserve full-page geometry and raw
text, and distinguish native capture from print-derived images and OCR.
Original fixtures run in ordinary CI. Requested missing inputs fail; optional
missing corpus is NOT_RUN. Publish actual coverage and limitations separately
from Python regression results. Preserve independent bookmark, licensing and
converter-memory release checks.

## Reference behavior

The [Python converter](https://github.com/rwv/caj2pdf) is a black-box
behavioral oracle at the revision named in the matrix, never an implementation
source. Reference `success`, `error`, `unsupported`, `skip`, and `not_run` are
distinct. A missing native decoder is an environment skip, not proof that a
format is unsupported. TEB conversion and pure-text HN are known reference
limitations. HN image output does not imply searchable text. Every release
report must state which optional corpus cases were actually run, the tool
versions, and the exact failures.
