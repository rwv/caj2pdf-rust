<!-- SPDX-License-Identifier: MIT -->

# Bounded T.88 text-region composition

Issue [#87](https://github.com/rwv/caj2pdf-rust/issues/87) composes the
ordered [#86 instance stream](t88-text-instances.md) into a packed text-region
bitmap. The format source is [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en),
especially §§6.4.1–6.4.5 and 7.4.3.2, Tables 9–11. The consulted English
PDF has SHA-256
`a94850aa659f4c5267051d1e17081dc4ffd04531c3d659c6bc2835802035ec69`.
The Rust implementation and its tests are original MIT work. The exact MQ
Table E.1 states remain caller supplied and outside this repository pending
[#44](https://github.com/rwv/caj2pdf-rust/issues/44).

## Storage contract

`TextComposer::new` accepts the validated text header, an ordered
`TextInstanceSource` (normally `TextInstanceDecoder`), the exported symbol
catalog, caller-owned `BitmapView` ranged views of the imported, new, and refined bitmap
stores, an empty caller-owned `RandomAccessScratch`, a final
`SequentialSink`, limits, cancellation, and a composition budget. Each
bitmap handle retains its store identity, base, dimensions, row stride, and
relative offset. The composer rechecks identity, geometry, store extent,
placement order, and signed 32-bit coordinates before using a handle. A
refined instance remains in its temporary store; it is never added to the
exported dictionary catalog. The caller must connect each ranged view to the
store that produced its handles. Opaque adapters cannot prove that two
handles refer to the same physical file.

A `BitmapView` supplies a content revision that advances on every byte or
extent change. The imported and new stores must be frozen once composition
begins. The refined store may only append between decoded instances; its
existing prefix must remain unchanged. The composer checks revisions and
sizes around positioned reads and reports a located source mutation if
either changes unexpectedly. The adapter must enforce its revision promise;
a filesystem timestamp or cached file length alone cannot detect an
equal-length rewrite. The optional native diagnostic closes the dictionary
writers before composition, tracks refined writes with a shared monotonic
revision, and unlinks its private temporary paths on Unix once all handles
are open.

The scratch adapter reports its size fallibly, sets an exact length, supports
positioned reads and writes that may return a partial prefix, and flushes
buffered writes. Completed writes must be visible to later reads through
the same adapter. `set_len` must not expose uninitialized bytes. The composer
must be the scratch store's sole writer for the session; an adapter that
allows an unrelated writer to rewrite bytes at the same length violates this
ownership contract. Size-query failures and unexpected extent changes are
located errors. The composer explicitly initializes every row to
`SBDEFPIXEL`, with unused low padding
bits cleared, before it reads an instance. It flushes scratch after
initialization and after all instances, then reads rows in ascending order
and writes each packed row to the final sink. The final sink is flushed only
after all rows are written. A short successful I/O call is retried; zero
progress, overreporting, truncation, observable mutation, cancellation, and failed
flushes are located typed errors. Any failed or abandoned session poisons
the partial scratch and final output, which the caller must discard.

Random access is required because later S/T deltas may place an instance
above an earlier one. A row cannot be emitted early from the placement
stream. The caller may implement scratch using a bounded native temporary
file, a browser origin-private file or other quota-checked random-access
store, or a Node.js temporary file. Browser and Node adapters belong outside
the codec and must preserve partial-I/O, visibility, size, flush, and
cancellation semantics. A forward-only document input can be spooled by its
platform adapter before decoding. The core does not require a whole-region
`Vec<u8>` or whole-file conversion buffer.

## Pixels, limits, and progress

Instances are combined in their decoded order using the header's `SBCOMBOP`:
OR, AND, XOR, or XNOR. The #86 stream has already resolved reference corner
and transpose placement into a checked top-left coordinate. The composer
clips each source bitmap to the text-region rectangle. Negative placement,
overlap, nonmonotone order, and a wholly off-region instance are valid. An
off-region instance still counts toward the declared instance count but
does not read bitmap pixels. A zero-instance region still initializes and
emits its default pixels. Final row padding is zero even when the default
pixel is one.

`TextComposeBudget` caps region pixels, scratch and output bytes, symbol and
row bytes, each I/O request, per-instance and total touched pixels, source
and scratch traffic, I/O call counts, work, and resident row buffers.
`Limits` also applies where relevant. The default scratch/output cap is
128 MiB; the default request cap is 256 KiB and the resident codec-buffer
cap is 2 MiB. Scratch bytes and final output bytes are separate progress
counters. Peak resident bytes in the public progress describe the codec's
owned buffers, not the host runtime, filesystem cache, or an adapter's own
allocation. On Linux, the optional native diagnostic also reads process peak
RSS from `/proc/self/status`; it reports zero when that metric is unavailable.
No cap is a substitute for the host's storage quota.

The composer checks the declared instance count and accepts `None` only
after the pull decoder has validated its MQ terminal. It never interprets
the text region as a complete HN/C8 page: page information and generic
region #4 still need explicit composition under
[#9](https://github.com/rwv/caj2pdf-rust/issues/9). The malformed raw
`0xa40c` text header remains a strict refusal by default. The
[#88 compatibility policy](t88-text-header-compatibility.md) requires an
explicit caller choice, and the report retains its raw flags and typed
anomaly marker when chosen.

## Verification

Clean-clone tests use original small bitmaps to exercise corner and
transpose placement, clipping, overlap and all four operators, padding,
both dictionary stores and the refined store, zero instances, nonmonotone
order, I/O faults, cancellation, limits, and malformed input. The optional
private run compares each standards-valid text-only output against the
independent [#85 hash-only baseline](jbig2-text-oracle.md):

```sh
python3 scripts/jbig2_text_region_parity.py \
  --corpus-dir /path/to/CAJSamples \
  --table-fixture /tmp/private-t88-table.fixture --json
```

To measure the separately labeled `0xa40c` case through the same arithmetic
and composition path, add
`--text-header-policy hn-c8-unused-refinement-template`. Strict mode remains
the default. The opt-in report counts one anomaly match separately from the
545 standard matches; it does not add the two counts into a conformance
claim. See the [policy note](t88-text-header-compatibility.md) for exact
source and hash evidence.

The diagnostic checks all 27 source hashes and the private MQ-table hash
before and after execution. Its report separates completed, matching,
failing, and skipped standard cases from either the strict `0xa40c` refusal
or the one separately counted opt-in anomaly result. It also records maximum
request, scratch bytes, and process peak RSS. No external
document, decoded bitmap, or exact MQ table row enters Git. A clean clone
without optional inputs reports `NOT_RUN` with zero compatibility cases;
explicitly invalid inputs fail. A pixel match establishes this text-only
slice, not full-page/PDF parity or independence of the two external oracle
backends. Issue #88 adds `TextRegionHeader::anomaly` and the public composer
report's `text_flags_raw` and `header_anomaly` fields. This is an explicitly
marked `v0.x.y` Rust struct-literal API change.

On 2026-09-27 UTC, the local SHA-pinned run attempted all 546 images. All
**545 standards-valid regions** completed and matched both the #85
normalized pixel SHA-256 and black-pixel count; there were zero standard
refusals, mismatches, or skipped cases. The sole raw `0xa40c` header was
refused at its expected source byte. All 27 source hashes and the private
table hash matched before and after. The largest scratch bitmap was
1,098,864 bytes, the largest observed I/O request was 312 bytes, and peak
process RSS was 2,711,552 bytes. A 2496 × 3522 page used 1,098,864 bytes
of scratch and matched the text-only oracle. These measurements are for
this private run and machine, not universal runtime bounds.

A separate 2026-09-27 UTC [#88 opt-in run](t88-text-header-compatibility.md)
kept the 545 standards-valid pixel matches and decoded the single
nonconforming `0xa40c` header through the same instance and composer path.
Its 234 instances, 3,431 rows, SHA-256, and 3,718 black pixels matched
the #85 text-only baseline. This opt-in result is reported apart from the
strict-valid count and does not include page-information or generic-region
composition.
