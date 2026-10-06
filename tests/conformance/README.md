# External corpus metadata

These three files are the metadata that this repository's own optional corpus
checks read. They contain identities, hashes and measurements only, never
documents, PDFs, pixels or text from the external corpus.

| File | Read by |
| --- | --- |
| [`matrix.json`](matrix.json) | `js/scripts/corpus.mjs` (and its tests), `crates/caj2pdf-core/tests/hnc8_type2_jpeg_external.rs` |
| [`jbig1_oracle.json`](jbig1_oracle.json) | `crates/caj2pdf-core/tests/{hnc8_type2_jpeg,qm_caj_oracle}_external.rs` |
| [`hnc8_type2_jpeg_inventory.tsv`](hnc8_type2_jpeg_inventory.tsv) | `crates/caj2pdf-core/tests/hnc8_type2_jpeg_external.rs` |

`matrix.json` inventories the external
[CAJSamples](https://github.com/caj2pdf/CAJSamples) repository at commit
`7e1c35e7b6de34e21972fcd1752c2a7e99b4ad07`; CAJSamples records no
redistribution grant for this project, so its documents stay outside Git. The
Rust tests hash-pin these files; change them only together with those pins.

The Rust tests are `#[ignore]`d and need a local corpus checkout, for example:

```sh
CAJ2PDF_CORPUS_DIR=/path/to/CAJSamples \
  cargo test --locked -p caj2pdf-core --test hnc8_type2_jpeg_external -- --ignored
CAJ2PDF_CORPUS_DIR=/path/to/CAJSamples node js/scripts/corpus.mjs
```

An unset corpus is a visible skip (`NOT_RUN`), never a compatibility pass.

The rest of this directory — the Python oracle and conformance harness tests,
the other oracle manifests and baselines, and the full provenance of the
matrix — moved to
[caj2pdf-samples `research/conformance/`](https://github.com/rwv/caj2pdf-samples/tree/main/research/conformance)
in [#360](https://github.com/rwv/caj2pdf-rust/issues/360).
