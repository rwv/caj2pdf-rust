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
```

The final command prints `NOT_RUN` for the external corpus when
`CAJ2PDF_CORPUS_DIR` is unset. This is a visible skip, never a compatibility
pass. To request an inventory run, point the variable at a local checkout of
the pinned CAJSamples revision:

```sh
CAJ2PDF_CORPUS_DIR=/path/to/CAJSamples python3 scripts/conformance.py --json
```

The runner checks all canonical files, including size and Git blob hash, using
bounded reads. A missing file, changed hash, unreadable file, or path escaping
the corpus root fails the requested run. Type aliases do not cause duplicate
runs. The concise report distinguishes `PASS`, `FAIL`, `UNSUPPORTED`, and
`NOT_RUN` for the inventory and PDF checks. An inventory `PASS` means only
that the local corpus matches the pinned matrix. It is not a Rust conversion
result.

Once a converter produces PDFs, place them outside the repository and pass
`--pdf-dir /path/to/output`. Each output path mirrors the canonical input path
with a `.pdf` suffix: `issue-1/a.caj` maps to `issue-1/a.pdf`. The runner
compares available page counts, page dimensions, outline hierarchy and
destinations, and rendered-page hashes against recorded expectations. A
requested PDF comparison fails if an expected output or required inspection
tool is missing. Fields without a measured baseline are reported `NOT_RUN`;
they do not pass by default. `--json` provides a machine-readable report for
later release gating.

PDF inspection and rendering use a separately installed, version-recorded
`mutool` command. Its source and output PDFs are never vendored here. Exact
render hashes are comparable only with the recorded rendering options and
tool version; a different version requires rebaselining and review. The
synthetic [fixture manifest](../tests/fixtures/manifest.json) includes PDF
structure cases that can test these checks without an external document.

## Reference behavior

The [Python converter](https://github.com/rwv/caj2pdf) is a black-box
behavioral oracle at the revision named in the matrix, never an implementation
source. Reference `success`, `error`, `unsupported`, `skip`, and `not_run` are
distinct. A missing native decoder is an environment skip, not proof that a
format is unsupported. TEB conversion and pure-text HN are known reference
limitations. HN image output does not imply searchable text. Every release
report must state which optional corpus cases were actually run, the tool
versions, and the exact failures.
