# Unreleased

## Measured CAJ tiling-pattern matrices (#414)

- Two measured CAJ originals (264 pages) now convert by making the observed
  CAJViewer/Poppler identity fallback explicit for four malformed Pattern
  matrices. All pattern streams are retained. Valid matrices and other
  malformed profiles keep their existing behavior.
- Recovery bounds the header to 512 bytes, proves the exact dictionary and
  stream boundary, and preserves source checks and sequential ranged output.
  Native, CLI and JavaScript APIs and dependencies are unchanged.
- MuPDF interprets the malformed source differently; this is a measured source
  viewer recovery, not general scientific-notation support. See the
  [conformance scope](../conformance.md#tiling-pattern-matrix-checkpoint-414).

See [v0.6.0](v0.6.0.md) for the preceding release notes.
