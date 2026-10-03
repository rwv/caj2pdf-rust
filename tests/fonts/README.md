# Original geometric test font

`geometric.ttf` is an original MIT fixture: empty `.notdef`, a rectangle
labelled U+0041, and a triangle labelled U+4E2D. The character labels are
synthetic; the fixture proves resource transport and visible geometry, not
real-language typeface fidelity. No external font data or outlines are used.

The generator is shared with the core font tests:

```sh
rustc --edition 2024 crates/caj2pdf-core/tests/common/font_fixture.rs -o /tmp/caj2pdf-font-fixture
/tmp/caj2pdf-font-fixture > tests/fonts/geometric.ttf
```

`shared_cross_runtime_font_matches_original_generator` checks the committed
bytes against the generator. Rust, Node and browser Worker tests use the
same resource. This font is test data and is not bundled as a conversion font.
