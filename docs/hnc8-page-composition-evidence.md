<!-- SPDX-License-Identifier: MIT -->

# Source-page composition evidence

[Issue #117](https://github.com/rwv/caj2pdf-rust/issues/117) and draft
[PR #121](https://github.com/rwv/caj2pdf-rust/pull/121) remain incomplete.
The two experiments below have separate frozen plans, immutable external
receipts and actual counts. Neither failed experiment is a whole-profile
compatibility pass. Source documents, states, decoded arrays, PDFs, renders
and execution artifacts remain outside Git.

## Preserved first attempt

The [original protocol](hnc8-page-composition-protocol.md) ran at commit
`42125b6b2a98169083868bfa19f2595f12eb311f`. Report: 733,091 bytes, SHA-256
`7e9d0e6d43d1f0f3e428da636f82566f7fcac44e4868347093edd652735816e5`.

It remains **FAIL**. One native HN-A output completed; metadata passed for
68 source/output pages, 91 draws and 24 JPEG streams. The first required
complete Type0 comparison refused the reference dictionary's explicit
identity DecodeParms. Two Poppler extractions ran, but no complete array or
full-page pixel comparison passed. C8 and HN-B were unattempted.

A separately frozen [dictionary-only probe](hnc8-page-composition-dictionary-probe.md)
made two object queries and four startup identity probes, without conversion,
sample extraction or rendering. Its 21,470-byte report SHA-256 is
`ec6a38adda35e156ad5b67857471414a1208953c9aa8c7d3d50e0675ca513fa4`.
It established the narrow explicit Flate Predictor=1 profile. This is
dictionary evidence, not sample/page compatibility evidence.

## Controlled identity-parameter rerun

The independently reviewed [amendment](hnc8-page-composition-identity-params-rerun.md)
was committed before execution at
`8041ee43078e387c027d8ef317984fbf9d48cd04`. The Rust/Cargo fingerprint and
native binary were identical to the first attempt. The correction changed
only the original verifier, synthetic tests and evidence documentation.

- Report: 1,696,715 bytes, SHA-256
  `104cd225c58876e38a1d5a04d58de164d42a049d3a823632cadea4b7ac939291`.
- Outer receipt: 98,227 bytes, SHA-256
  `d6f8359f91cd791432aff8fb14c2845f4856daae6b0c2e79f9e4ac1a06cf1771`.
- Outer post-audit SHA-256:
  `cf4eca239a9e33fe9a624cca3a284eee557f716de63f0585874fbadad8bfa014`.

| Required work | Attempted | Passing | Failing | Remaining |
| --- | ---: | ---: | ---: | ---: |
| Profiles | 3 | 2 | 1 | 0 |
| Source rows | 81 | 81 | 0 | 0 |
| Output boxes | 77 | 77 | 0 | 0 |
| Ordered draws | 127 | 127 | 0 | 0 |
| Encoded JPEG streams | 53 | 53 | 0 | 0 |
| Complete padded Type0 arrays | 74 | 74 | 0 | 0 |
| Full-page/renderer comparisons | 151 | 150 | 1 | 3 |

Unsupported counters are zero. The 75 HN-A/C8 pages passed all 150 required
full-page comparisons with the frozen MuPDF and Poppler settings and exact
RGB equality. Every Type0 row and padding bit passed independent extraction;
all six CTM components and boxes met the frozen 0.00005 pt tolerance.

The overall result remains **FAIL** at HN-B output page 1 / MuPDF. That
2071-by-153 raster differs on 261,584 of 316,863 pixels, with maximum channel
difference 255. The other three HN-B page/renderer pairs were unattempted.
Both HN-B output boxes, transforms, dimensions, encoded streams and mapping
`[1, 6]` agree, with source rows 2–5 explicitly accounted as no-image rows.
The maximum box/CTM residual is 5.68e-14 pt.

The recorded reference image dictionaries declare DeviceRGB; native output
declares DeviceGray. Existing public JPEG inventory identifies the same
payloads as eight-bit, one-component JFIF. The metadata checks above compare
dimensions, streams and placement but exclude this color interpretation
difference. [Child #122](https://github.com/rwv/caj2pdf-rust/issues/122)
blocks the remaining reference-validity and complete HN-B page criterion.
No reference replacement, color inversion, malformed RGB declaration or
pixel-tolerance change follows from the recorded metadata alone.

## Resources and audits

This rerun made three native calls and zero Python converter calls. Its
1,838 validator launches include 302 renders. Adding 22 outer probes and
one runner launch gives 1,864 aggregate launches, below the frozen 2,048
ceiling. The two comparison attempts together made four native calls; each
has its own counts, receipt and limits.

All six integrity groups passed before and after: 27 corpus files, six
reference PDFs, caller table, inputs, environment/tools and native code.
The outer unchanged flag is true; its status remains FAIL because the
experiment failed. No watchdog cleanup was needed.

The rerun took 167.8 seconds. Native peak RSS was 24,640 KiB, validator peak
76,272 KiB and harness high-water mark 30,640 KiB. Observed owned-session
storage peaked at 129,696,193 bytes against 512 MiB. These measured process
and storage figures are distinct from checked allocation/buffer limits.

## Independent review and hosted gates

Two independent reviewers checked the frozen verifier amendment and wrapper;
both passed all 30 focused original synthetic tests. The full conformance
suite passed 348 tests. A separate metadata audit confirmed the rerun's
failure, actual counts, hashes and before/after audit statuses without reading
private PDFs, source/header/entropy bytes or decoded samples.

All four hosted jobs passed on `8041ee43078e387c027d8ef317984fbf9d48cd04`
in [CI run 36372656098](https://github.com/rwv/caj2pdf-rust/actions/runs/36372656098):
native quality, WASM/browser/Node, MIT audit and Rust LCOV. The LCOV gate
recorded 27,410/27,410 lines, with 100% in every recorded file. Native examples
are compiled and checked but are outside the standard LCOV report. Later
changes require new exact-head gates and final independent review.

Clean-clone CI reports the optional document comparison as **NOT_RUN** with
zero actual work and zero compatibility passes. Parent #10, table-rights
gates and production CLI/JavaScript family routing remain open.
