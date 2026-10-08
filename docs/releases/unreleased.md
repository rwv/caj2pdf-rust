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
