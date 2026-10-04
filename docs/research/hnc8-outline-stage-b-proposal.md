<!-- SPDX-License-Identifier: MIT -->

# HN outline field validation plan

This is the completed investigation plan for
[#137](https://github.com/rwv/caj2pdf-rust/issues/137), which supplied field
evidence for the original Rust reader delivered in
[#119](https://github.com/rwv/caj2pdf-rust/issues/119). It replaced
the earlier Stage B proposal. Historical Stage A observations and failures remain
in [their report](hnc8-outline-stage-a-results.md); no result is reclassified.

The completed observations and native handoff are in
[HN-A outline fields](hnc8-outline-fields.md). The steps below explain the method;
implementation and broader format support remain separate work.

## Small investigation

1. Record a candidate field table using the existing 308-byte record observations.
   Treat each field as a hypothesis until positive documents and useful controls
   support it. Matching CAJ record sizes do not prove matching semantics.
2. Check the two additional HN-A samples below. Hash inputs, read bounded record
   windows, convert through the pinned Python reference as a black box, and parse
   its PDF outlines with the existing qpdf/MuPDF comparison functions.
3. Use copied inputs for a few specific title, hierarchy and destination controls.
   Record changed bytes and expected effects before each run. Check whether
   framing/checksums need updates. Keep failures and revise hypotheses explicitly.
4. Publish a short field table, actual comparisons and unresolved limitations.
   Then implement #119 using the existing visitor and PDF APIs.

| Sample | SHA-256 | Expected records from earlier inventory |
| --- | --- | ---: |
| issue-29 | `ede5eddb0e8ec1dea46c32a06e16ac12b874a141ca2d6669736495d2ac549261` | 48 |
| issue-69 | `57a3c60e1d8639955c625452398d2c8a32a46a51b815a44cc9a5e0351fcb8ef4` | 111 |

A missing, empty or failed reference is reported as such. It cannot establish
positive outline parity. C8/HN-B rules need their own evidence; an empty reference
outline does not prove the source lacks outlines.

## Candidate and controls

Start with these explicitly provisional HN-A fields: a title beginning at record
byte zero in a possible 256-byte region; a NUL-terminated decimal page string at
280 in a possible 12-byte region; and a possible little-endian level integer at
304. GB18030 decoding is a hypothesis based on the prior title matches. Check
termination, padding, field widths and invalid cases; do not infer them from a
small set of low numeric values alone.

Useful controls include a long original ASCII title, a two-byte/four-byte Unicode
title, one legal hierarchy change, and one distinct destination. Negative controls
should answer specific questions about bounds, malformed encoding or parent gaps.
Do not expand the matrix just to accumulate proof artifacts. Preserve exact title
bytes, order, whitespace and nullable `/XYZ` parameters in comparisons.

## Implementation requirements

Use ranged input and one record at a time. Bound record/title counts, decoded title
bytes, nesting depth and the source/output page map; check offsets and cancellation.
Reuse the original title decoder only for independently justified encodings.
Bookmarks aimed at omitted source pages must follow an explicit policy, never an
implicit neighboring-page substitution. Preserve current image/page output.

Use ordinary isolated output directories, source hashes, subprocess timeouts and
resource limits. Existing user authorization covers this work; no additional
execution-token, runtime-inventory or per-phase approval framework is required.
The older scripts' frozen execution paths remain historical tools, not mandatory
entry points for new observations. Reuse their pure parsing functions where useful.

All new source and synthetic controls are original MIT. External documents,
mutations, titles, PDFs and query output remain outside Git. Review and simplify
changes, test their behavior and pass the existing CI gates. Field validation alone
does not complete native implementation, JS integration or release parity.
