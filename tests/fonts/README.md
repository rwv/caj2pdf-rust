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

## Symbol transport fixture

`symbols.ttf` uses the same original shapes, labelled U+0020 (rectangle)
and U+FF1A (triangle). Its deliberately visible space catches accidental
space-record omission; it is not a real-language typeface.

```sh
/tmp/caj2pdf-font-fixture symbols > tests/fonts/symbols.ttf
```

The core font test also checks these bytes against the original generator.

## Collection fixture

`collection.ttc` is a two-face TrueType collection of the same original
fonts: face 0 is `geometric.ttf` and face 1 is `symbols.ttf`, with table
offsets made file-relative.

```sh
/tmp/caj2pdf-font-fixture collection > tests/fonts/collection.ttc
```

## CFF fixture

`geometric.otf` has CFF outlines for the same shapes as `geometric.ttf`
(`A`, `中`) plus a hinted square (`B`), drawn through global and local
subroutines with `hintmask`/`cntrmask`.

```sh
/tmp/caj2pdf-font-fixture cff > tests/fonts/geometric.otf
```
