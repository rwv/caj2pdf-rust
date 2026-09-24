# Conformance baseline

The [corpus matrix](../tests/conformance/matrix.json) inventories unique inputs
from a pinned revision of the external
[CAJSamples](https://github.com/caj2pdf/CAJSamples) repository. Its
[provenance note](../tests/conformance/README.md) explains canonical paths,
type aliases, reference versions, and measured results. CAJSamples has no
redistribution grant recorded for this project. Keep its documents and every
PDF derived from them outside this repository.

## Commands and status

From a clean clone, run the unit tests and check that the original MIT
fixtures match their generator:

```sh
python3 scripts/generate_fixtures.py --check
python3 -m unittest discover -s tests/fixtures -p 'test_*.py'
python3 -m unittest discover -s tests/conformance -p 'test_*.py'
python3 scripts/conformance.py
python3 scripts/jbig1_oracle.py --json
```

Both optional commands print `NOT_RUN` for external checks when their
inputs are absent. The PDF command prints `NOT_RUN` for the external corpus when
`CAJ2PDF_CORPUS_DIR` is unset. This is a visible skip, never a compatibility
pass. To request an inventory run, point the variable at a local checkout of
the pinned CAJSamples revision:

```sh
CAJ2PDF_CORPUS_DIR=/path/to/CAJSamples python3 scripts/conformance.py --json
```

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
a recorded render hash for every page. Unknown reference outcomes or incomplete
successful rows report `NOT_RUN`; a requested `--pdf-dir` exits nonzero unless
the aggregate PDF status is `PASS`. Known reference errors are `EXCLUDED`
from the successful-conversion scope, while known unsupported inputs remain
`UNSUPPORTED`. Top-level page and outline counts are Python `show` observations;
`expected_pdf.page_count` and `expected_pdf.outline_count` are authoritative
for converted PDF output when they differ. `--json` provides a
machine-readable report for later release gating.

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

Without these inputs it reports `NOT_RUN`. A result of
`NO_MATCH_IN_TESTED_GRID` rejects only the listed settings; a match on an
all-zero image is explicitly `BLANK_MATCH_NON_DISCRIMINATING`. The
probe hashes each valid PBM both in its returned row order and with rows
reversed. In each order it compares visible pixels (unused low bits masked)
and the complete DIB stride against the manifest. Since PBM has no DIB row
padding, the stride comparison assumes zero padding while retaining the
PBM's actual unused low bits. `MATCH` requires both hashes to agree in the
same row order; `VISIBLE_MATCH_RAW_MISMATCH` means visible pixels agree but
the raw stride does not. If every setting fails to decode, the report is
`INCONCLUSIVE_NO_DECODABLE_SETTINGS` and exits nonzero. The
[experiment note](jbig1-bitstream-investigation.md) records the tested grid,
positive controls, refuted hypotheses, row-order evidence, and unresolved
CAJ-specific rules. Neither result claims full JBIG1 compatibility.

## Reference behavior

The [Python converter](https://github.com/rwv/caj2pdf) is a black-box
behavioral oracle at the revision named in the matrix, never an implementation
source. Reference `success`, `error`, `unsupported`, `skip`, and `not_run` are
distinct. A missing native decoder is an environment skip, not proof that a
format is unsupported. TEB conversion and pure-text HN are known reference
limitations. HN image output does not imply searchable text. Every release
report must state which optional corpus cases were actually run, the tool
versions, and the exact failures.
