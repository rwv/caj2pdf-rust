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

Known source-data limitation (#436): the accepted 63-page issue-20 sample
already contains five bad Flate checksums and one invalid DEFLATE stream,
affecting image, font and page-content resources. Native/Node/Chromium outputs
preserve these bytes; matching independent source-framed renders does not
prove intact content. Original CAJViewer page 39 remains unverified. See the
[source-stream investigation](../provenance.md#damaged-source-streams-in-an-accepted-caj-436).
This adds evidence, not a new fix or compatibility pass; #436 remains open.
A subsequent [substitution diagnostic](../provenance.md#checksum-confirmed-substitution-candidate)
restores the original checksums, lengths and page-table offsets without
editing those expectations. Its PDF validates cleanly, but a bounded
production recovery rule and its negative controls remain unimplemented.
