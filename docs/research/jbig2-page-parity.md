<!-- SPDX-License-Identifier: MIT -->

# Observed HN/C8 full-page JBIG2 pixel parity

This is the optional, private diagnostic for [issue #95](https://github.com/rwv/caj2pdf-rust/issues/95). It compares Rust's caller-table dictionaries, text composition, generic rows, and [bounded page OR composition](t88-observed-page-composition.md) with the SHA-pinned [#43 full-image pixel oracle](jbig2-oracle.md). The supported input is the observed five-segment type-3 profile: ordered segment numbers #0–#4, types 48/0/0/6/38, one page association, full-page regions at (0,0), and external OR. It is a corpus profile, not a claim about every legal JBIG2 stream.

## Private run and failure rules

From the repository root, provide the pinned CAJSamples checkout and the private 47-state table fixture:

```sh
python3 scripts/jbig2_page_parity.py \
  --corpus-dir /path/to/CAJSamples \
  --table-fixture /tmp/caj2pdf-t88-2000-h2-private.fixture \
  --json
```

The table file must stay under `/tmp`; its permitted diagnostic digest is `bdf6eeeca3bc5d5a8dc1a13acc7698ec356c886b27f6526f3e09fc2c8520ac57`. The runner checks the selected manifest, all 27 source SHA-256 values, and the table digest before decoding, then rehashes sources and table afterward. It also records the Rust diagnostic executable's SHA-256 before and after the run. Expected full-page hashes and black-pixel counts are compared outside Rust. The native diagnostic receives selected ranges and bounds, never expected output pixels. It uses bounded requests and caller-owned scratch instead of a whole-file input buffer or second full-page bitmap. A missing or changed *explicitly requested* corpus, manifest, or table is a failure; a clean clone with no private inputs reports `NOT_RUN` and zero checked or matching cases.

Strict T.88 text-header validation applies to 545 standards-valid records. The single observed raw-`0xa40c` header must be refused in a strict run. Its full-page pixel comparison requires the explicit `--text-header-policy hn-c8-unused-refinement-template` option and must be reported separately from the 545 strict cases. A decoder refusal, incomplete output, changed source, hash mismatch, unrecognized segment/profile, or failed resource bound remains a failure; it cannot be counted as skipped or matched.

The JSON result must separate attempted, completed, matching, failed, skipped, and unsupported counts for the standard and opt-in cases; retain each first failure and typed location; show 27 source hashes, the table hash, and the executable hash before and after; and report the largest I/O request, bounded working-set and scratch/output bytes, peak RSS, and one representative larger page. `peak_resident_bytes` is an accounted estimate of row, context, and composition working memory; it is not whole-process residency. `peak_rss_kib` is Linux process `VmHWM` across the cumulative 546-case run. `source_bytes` counts metered ranged decoder reads, excluding independent SHA verification; `max_request_bytes` includes the verifier's 64 KiB chunks. Synthetic fault tests cover missing or altered inputs, manifest drift, output protocol errors, unsupported topology/association, and short, zero, or overreported I/O. Ordinary CI asserts only the clean-clone `NOT_RUN` state.

## Measured evidence

On 2026-09-27, the final-source Rust diagnostic **passed** both private modes against the unchanged #43 full-page oracle. Each command submitted 546 records:

```sh
python3 scripts/jbig2_page_parity.py --corpus-dir /tmp/caj2pdf-ranged-corpus \
  --table-fixture /tmp/caj2pdf-t88-2000-h2-private.fixture \
  --rust-bin target/release/examples/jbig2_page_parity \
  --text-header-policy strict --json > /tmp/caj2pdf-issue95-strict-final-source.json

python3 scripts/jbig2_page_parity.py --corpus-dir /tmp/caj2pdf-ranged-corpus \
  --table-fixture /tmp/caj2pdf-t88-2000-h2-private.fixture \
  --rust-bin target/release/examples/jbig2_page_parity \
  --text-header-policy hn-c8-unused-refinement-template --json \
  > /tmp/caj2pdf-issue95-optin-final-source.json
```

| Mode | Standard full-page matches | Anomaly result | Failures, skips, unsupported |
| --- | ---: | --- | ---: |
| Strict | 545/545 completed and matched | One `malformed_text_header` refusal at the pinned header offset | 0 / 0 / 0 |
| Named opt-in | 545/545 completed and matched | 1/1 completed and matched, with `HN_C8_UNUSED_REFINEMENT_TEMPLATE` marker | 0 / 0 / 0 |

Both reports have no first failure. Their 27 source ID-to-SHA-256 maps are identical before and after and agree with the [pinned matrix](../../tests/conformance/matrix.json), whose raw SHA-256 is `af42132133911f3597eed9318f613494c4047353cb7e15fc4d1008cf59ef44a9`. The canonical UTF-8 JSON digest of either sorted source map (`sort_keys=True`, `separators=(",", ":")`, `ensure_ascii=False`) is `2399bddb744e94041f7b4c3e3938717780c97a803d5ee793bd83dbef8d205416`. The private table digest before and after both runs is `bdf6eeeca3bc5d5a8dc1a13acc7698ec356c886b27f6526f3e09fc2c8520ac57`. The final-source Rust executable digest before and after both runs is `8477ebb4259e572a5d40c913a750613f6d43e0dab9b81c632615d46e4d10aaa8`.

| Measured maximum | Strict | Named opt-in |
| --- | ---: | ---: |
| Metered ranged decoder reads per case | 106,399 bytes | 106,399 bytes |
| Largest individual I/O request, including SHA verifier | 65,536 bytes | 65,536 bytes |
| Accounted row/context/composition working set | 17,632 bytes | 17,632 bytes |
| Logical scratch | 1,132,471 bytes | 1,132,471 bytes |
| Packed output | 1,098,864 bytes | 1,098,864 bytes |
| Process `VmHWM` across all cases | 3,032 KiB | 2,896 KiB |

The representative larger page is the `issue-43` sample at page 3, image 1: **2,496 × 3,522** pixels, 1,098,864 packed output bytes, 1,112,371 scratch bytes, and 17,632 accounted working-set bytes. Its process `VmHWM` was 2,944 KiB in the strict run and 2,820 KiB in the opt-in run. The #95 reports and all private inputs remain under `/tmp`; none are committed.

The clean-clone status remains exactly `NOT_RUN`, with zero submitted, checked, or matched cases. The demonstrated boundary is **packed pixels for these observed full-page JBIG2 records only**. It does not prove HN/C8 page placement, PDF output, universal JBIG2 support, or independence of the #43 oracle's external decoder backends. The [#44 exact MQ table rights record](t88-mq-rights.md) remains `UNRESOLVED`: the table stays caller supplied and external, and no default standard-compatibility or bundled-state claim follows from this diagnostic. No source document, table row, official vector, temporary bitmap, or decoded pixel is committed or released.

The #95 final-source repository coverage gate measured **23,200/23,201 lines** (99.99569%, displayed as 100.00%); exactly one existing `text_composer` formatter-error branch remained uncovered at that time. [Issue #98](https://github.com/rwv/caj2pdf-rust/issues/98) subsequently added a formatter-failure test; the required gate now measures **23,201/23,201** unique source lines and requires exact 100% per file.
