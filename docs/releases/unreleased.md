# Unreleased

Changes after [v0.6.1](v0.6.1.md):

- Recover the measured redundant CAJ xref/header-copy suffix and two unused
  interruption profiles (#409, #434). The suffix must match original header
  bytes and terminate at a pending indirect Length object. Interrupted images
  require an independently anchored complete counterpart and exact prefix;
  unfinished openers contain no value bytes. Full graph validation must prove
  that neither is referenced. Referenced, ambiguous, encrypted, unmeasured or
  incomplete-graph cases remain errors.
- Allow at most 64 PDF whitespace bytes after a declared or resolved stream
  payload, requiring exact endstream/endobj tokens. Payload bytes and Length
  values are retained; unrelated stream-repair restrictions still apply.

Native, CLI, browser and Node.js share these core rules. There are no API,
dependency, license or publication changes. See the
[input profile](../pdf-input.md#redundant-caj-framing) and
[provenance](../provenance.md#redundant-caj-framing-409-434) for bounds and
external evidence. Other unresolved samples are not classified as impossible.

Verification: the unchanged 60-page original produces identical native, Node
and Chromium PDFs; qpdf, all 189 raw streams, 36 bookmarks and 60 independently
framed page renders agree. Source-viewer checks cover pages 1–3. The fresh
1,277-original native run adds one pass with no changed previous-success
output hashes (1,240 PASS, 28 FAIL, nine UNSUPPORTED). Ancillary warnings and
unexecuted checks are retained in the
[pinned report](https://github.com/rwv/caj2pdf-samples/blob/26720c5d76fbb05b39400a65b35b3974b31c1931/research/notes/redundant-caj-framing-20261008.md).

Recover the measured stream-byte expansion in the accepted 63-page issue-20
CAJ (#436). Only damaged simple Flate streams whose original checksums,
Lengths and all page-table anchors corroborate the inverse substitution are
changed. Native, CLI, Node and browser share the bounded ranged recovery;
there is no API or dependency change. Other codec payloads remain opaque.
See the [admission bounds](../pdf-input.md#checksum-confirmed-caj-stream-substitution)
and [production evidence](../provenance.md#bounded-production-recovery).

The unchanged original now produces identical native/Node/Chromium PDFs with
clean qpdf and 63 clean Poppler renders. All independently framed diagnostic
pages, object values and payloads agree; 93 bookmarks are retained. Previous
vendor checks cover modified-source pages 3/5/39 and the identical PDF.
Original damaged-source page 39 remains inaccessible, and no independently
obtained intact alternative or general corruption repair is claimed.


Recover three measured CAJs containing a complete empty Form inside an
identical same-number empty Form wrapper (#439). The bounded scanner retains
the inner object and omits only redundant framing; nonempty, conflicting,
incomplete and unmeasured profiles remain errors. Native, CLI, Node and browser
share the rule, with no API or dependency change. See the
[admission bounds](../pdf-input.md#nested-duplicate-empty-forms) and
[provenance](../provenance.md#nested-duplicate-empty-forms-439).
