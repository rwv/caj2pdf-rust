# Unreleased

Changes after [v0.8.0](v0.8.0.md):

- Derive Rust and JavaScript bundled-font resource constants from one manifest
  and reject stale copies in CI (#551). Font bytes, mappings, public APIs and
  conversion output are unchanged; installed packages remain self-contained.
- Reuse the core font configuration in CLI/WASM adapters and transfer it into
  conversion without cloning maps and identities (#552). JavaScript resolves
  optional role indices once; custom fonts, TTC faces and public APIs retain
  their existing behavior.
