<!-- SPDX-License-Identifier: MIT -->

# Issue #117: bounded dictionary-profile follow-up

Status: FROZEN BEFORE THIS PROBE, 2026-09-28 UTC. This plan follows the first
complete-page attempt and is separately reviewed before any additional private
object query. It does not modify the original attempt or authorize another
conversion, sample extraction or render.

## Preserved first attempt

The source was clean at `42125b6b2a98169083868bfa19f2595f12eb311f`.
The original report is external at
`/home/hzc/.cache/caj2pdf-issue117-validation/composition-report.json`,
733091 bytes, SHA-256
`7e9d0e6d43d1f0f3e428da636f82566f7fcac44e4868347093edd652735816e5`.
Its external execution receipt SHA-256 is
`d2c52847c7935c1b1293f8962fbd5dbcf2edcd66a679c21b5fe7ed93ddcbe68d`;
internal receipt SHA-256 is
`6271e6445f444d3a0208a38171969441ad7ceb47f58d32e9c2ff5260cf146bd0`.

That attempt is **FAIL**. One native HN-A invocation completed; 68 source rows,
68 output pages, 91 ordered draws and 24 JPEG streams passed metadata gates.
The first Type0 complete-array comparison failed at its dictionary guard:
`Type0 dictionary uses an unsupported sample interpretation`. Two Poppler
page-image extractions had already completed; the complete qpdf/Poppler/native
sample comparison did not complete. No full-page renders or pixel comparisons
ran. All 27 source and six baseline before/after checks passed; original
source, tools, helper and startup-library identities were unchanged.

The reported guard can refuse `/DecodeParms`, `/SMask`, `/Mask` or `/ImageMask`.
Report metadata does not establish which key is present or its value. Keep all
first-attempt counts, artifacts and hashes unchanged. No tolerance is increased.

## Two exact object queries

Use only the pinned `/usr/bin/qpdf` binary, SHA-256
`30b2389c3ed0ba73434244fff5b149d4d919279ea7a8c3b1cd9fc25f57ec1792`.
Perform exactly these two bounded dictionary-only queries, without retries:

1. `qpdf --json --json-key=qpdf --json-stream-data=none --json-object=128 /tmp/caj2pdf-layout-reference-final/hnc8-layout-run-mi3kj_io/hn_a-run1-3rgsr25v/output.pdf`
2. `qpdf --json --json-key=qpdf --json-stream-data=none --json-object=10 /home/hzc/.cache/caj2pdf-issue117-validation/hnc8-composition-n_hzvxga/hn_a.pdf`

These are the first Type0 draw on output page 2. Both metadata observations
declare width 2304, height 3425 and one bit per component. The baseline PDF is
8239244 bytes, SHA-256
`833f0c40881f0baf7f76f3505b6679e2dfa0b16c0e469dfbf10f791e687dec40`.
The native PDF is 68697906 bytes, SHA-256
`54960ff59252a5acfbc9b2a0d283d7c96b1ad9ef699a8edfe7ca49177a7cf8b7`.
Object associations come from the completed three-tool metadata observation;
their numeric IDs are diagnostic inputs only, never conversion inputs.

The original MIT probe validates qpdf's JSON stream-object structure, requires
only the selected object entry and its `dict`, rejects `data`/`datafile`, and
returns only dictionary key names, numeric/boolean/name fields, the bounded
numeric/name structure of Decode/DecodeParms, and color-space names plus a
hash/length of an encoded palette. Raw JSON stays external. Unknown fields
are identified by name/type rather than arbitrary private strings. Do not
decode a stream or return any source text, table value, sample or pixel.

## Limits, audit and interpretation

- Maximum converter/native launches: zero. Maximum dictionary-tool launches:
  two. Maximum sample extractions and renders: zero.
- Record attempted/completed/failed/skipped object queries separately. Record
  up to four startup-identity probes (`ldd` for qpdf and Python before/after)
  separately; they do not inspect private inputs or decode streams.
- Each child: 45-second timeout, 1 GiB virtual memory, 65536-byte stdout and
  32768-byte stderr ceilings. Drain in chunks at most 65536 bytes; no input.
  Whole probe deadline: 120 seconds. Kill its whole group on failure.
- Keep the new probe directory outside Git under
  `/home/hzc/.cache/caj2pdf-issue117-dictionary-probe`, mode 0700. Maximum owned
  files: 1 MiB; no raw diagnostic data is committed.
- Freeze the probe source, this committed plan, execution environment, argv,
  qpdf and report/PDF identities before the first object query. Hash the two
  PDFs, old report, qpdf, probe source and plan again in `finally`. Record both
  successful and failing attempted queries, exact argv, exit status, byte
  counts/hashes, elapsed time and per-child `wait4` peak RSS. A mismatched
  audit prevents a passing probe result. Recheck the frozen effective
  environment and canonical startup-library identities in `finally`.
- This probe establishes only dictionary framing. It cannot establish image
  compatibility, orientation, padding or page pixels and cannot close #117.
  Any supported-profile change must be justified with independent primary
  documentation and original actual-tool synthetic fixtures, reviewed, then
  frozen in a separate follow-up full-comparison protocol. Retain the first
  FAIL and preserve the full-array and full-page zero-difference gates.
