# Research notes

The format investigations, oracle reports and validation runs behind the
decoders now live in the
[caj2pdf-samples repository](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes)
([#360](https://github.com/rwv/caj2pdf-rust/issues/360)), next to the scripts,
conformance harnesses and CAJViewer automation that produced them. They are
evidence, not user documentation: start with the [README](../../README.md) and
the [CLI reference](../cli.md). The project-wide record for this repository's
own source is [provenance](../provenance.md).

Source comments and older documents that cite `docs/research/<note>.md` mean
`research/notes/<note>.md` there; the [note index](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/README.md)
maps each note to the code it backs. The notes cited most often from this
repository's code are:

| Note | Backs |
| --- | --- |
| [CAJ container observations](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/caj-format.md) | `caj/`, `gb18030.rs`, CAJ tests |
| [KDH PDF wrapper observations](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/kdh-format.md) | `kdh.rs` |
| [Bounded HN/C8 container records](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/hnc8-container.md) | `hnc8.rs` reader |
| [Observed C8 native records](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/c8-native-records.md) | `hnc8/native.rs`, `hnc8/native_page.rs`, `hnc8/appinfo.rs` |
| [HN-A outline fields: observed profile](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/hnc8-outline-fields.md) | `hnc8/outline.rs` |
| [T.82 arithmetic core](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/t82-arithmetic-core.md) | `qm.rs` |
| [Bounded HN/C8 type-0 rows](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/jbig1-type0-rows.md) | `jbig1.rs` |

Notes and harnesses are pinned to the last caj2pdf-rust commit that carried
them, `0abee3862f01756ee15f69a1b174a35208fc1e41`. A new investigation goes to
caj2pdf-samples; a format fact that the code depends on is summarized in
[provenance](../provenance.md) with a link to its note.

The [2026-10-07 GitHub corpus validation](https://github.com/rwv/caj2pdf-samples/blob/043e52cd37389b3f903426cd0b8c6d7564aedf43/research/notes/github-sweep-fixes-20261007.md)
records all 1,277 post-fix native results, 50 remaining refusal classifications,
scoped runtime/fidelity evidence and explicit collection/test limits (#385).

The [973-original HN/C8/NH geometry report](https://github.com/rwv/caj2pdf-samples/blob/7dd99d385d334815c49afedcfe7929f5412bed8d/research/notes/source-image-geometry-20261008.md)
records source-declared page boxes and ordered image transforms/resource
identities for 16,548 accepted pages, with new NH bitmap evidence. Native
glyph/vector placement, complete rendered-page fidelity and unknown outlines
remain separate open obligations under #406.

The [60-page native composition checkpoint](https://github.com/rwv/caj2pdf-samples/blob/5a676fa58529249136b9b276e34ec466981067ac/research/notes/native-page-composition-20261008.md)
adds complete-page normal/marker-font observations and per-page text, vector,
image and font-state inventories for all ten native originals. It retains
all pixel differences, failed acquisition attempts and one normal-font cold
disagreement. This is scoped evidence under samples #51 and #441; complete
source fidelity remains open under #406.

The [native vector follow-up](https://github.com/rwv/caj2pdf-samples/blob/4744a9360354d119facc20e7bd93d916afc54d24/research/notes/native-vector-geometry-20261008.md) verifies all 327 measured paths and
their order among glyph/image operations on those 60 pages. Original negative
controls, input/output hashes, process bounds and earlier checker refusals are
retained. That check excludes glyph placement, fonts, ornaments and raster fidelity.

The [native glyph/ornament follow-up](https://github.com/rwv/caj2pdf-samples/blob/82054fbe12e7a222a4e8e8a54f3694ee44361306/research/notes/native-glyph-model-20261009.md) verifies all 83,432 ordinary
glyphs and 212 ornament marks in both normal and original-marker PDF sets.
Positions, dimensions, shear, gray, clipping and complete paint-kind order match
existing measured models; diagnostic roles match original font programs and CID
maps. This fills the model-check gap while retaining original-font, ornament
appearance, raster, viewer, outline and remaining exception limits under #406.
