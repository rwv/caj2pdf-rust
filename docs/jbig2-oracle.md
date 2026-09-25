<!-- SPDX-License-Identifier: MIT -->

# External HN/C8 JBIG2 pixel oracle

This note supports [issue #43](https://github.com/rwv/caj2pdf-rust/issues/43),
under [issue #9](https://github.com/rwv/caj2pdf-rust/issues/9). The oracle
records black-box tool agreement for externally held type-3 images. It does
not implement JBIG2 decoding, prove that the two tools have independent
decoder code, or claim that the Rust converter renders these images. The
original MIT script and tests contain no external documents or decoder code.

## Input and segment profile

The [pinned CAJSamples matrix](../tests/conformance/matrix.json) identifies
revision `7e1c35e7b6de34e21972fcd1752c2a7e99b4ad07`. The optional run
rehashes **all 27 HN/C8 sources** before scanning any image record. It then
consumes the [repository-owned directory inventory](../scripts/jbig2_directory_inventory.py),
which verifies five source hashes again, discovers type-3 image records, and
uses the Rust `read_embedded_directory` API to check every segment header and
exact span boundary. The oracle does not duplicate HN/C8 container parsing.

| Source | Variant | Type-3 images | Source SHA-256 |
| --- | --- | ---: | --- |
| `issue-43` | HN | 105 | `826608ef34b850926d1291ddba1773305bd44a0bf4170b4a9ae8c7d83c0b7134` |
| `issue-58` | C8 | 4 | `8974d024e0cbb54009419aa8c91c9ba286dd74f056c3b19524ee5c626c947c85` |
| `issue-66` | C8 | 1 | `90e7b47716c32ef7a67cde8094f312e0ee7f1a7e0ed50a25830e8ba64a84f6a6` |
| `issue-76` satellite sample | HN | 208 | `46779c74e34f1508125fe94f482672b4eb518436bc663dc5470df814cb41f0aa` |
| `pull-72` | HN | 228 | `01558ff7e30c3131c79fc3267e6eb99c055cc9cb8e1b0e710bc6bd0791c888ab` |

Each observed type-3 image record starts with a 48-byte DIB and two-color
palette. The rest is a headerless, contiguous JBIG2 segment stream with five
segment numbers `0..4`, types `48/0/0/6/38`, page association 1, and
references `[]/[]/[1]/[2]/[]`. Segment types and fields follow
[ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en)
§§7.2–7.4. The outer DIB dimensions match the JBIG2 page-information segment
and both region dimensions in the observed set. No end-of-page or end-of-file
segment occurs inside these image records. Their exact, enclosing record
length bounds the final segment.

The first and second symbol dictionaries use flags `0x0800` and `0x1802`;
the immediate generic region uses flags `0x04`. The text region uses
arithmetic coding and fifteen observed text-flag values. One specific image,
HN `issue-43` page 11 image 1 at source offset `930673` and length `2731`,
has flags `0xa40c`: `SBREFINE=0` while `SBRTEMPLATE=1`. T.88 §7.4.3.1.1
requires that template bit to be zero when refinement is disabled. The
manifest records this anomalous combination and its actual tool outcome;
the script does not silently classify its payload as conforming.

## Pixel method and status

For each image, the script copies only that image's encoded segment span in
64 KiB chunks into a temporary, one-image PDF with `/JBIG2Decode` and an
explicit one-bit DeviceGray decode array. `qpdf --check` must exit zero **and**
emit no `warning:` diagnostic before either pixel result is used. Poppler `pdfimages` extracts a binary PBM;
MuPDF `mutool draw` renders the same PDF to a binary PBM at 72 dpi. The
temporary PDF, PBMs, and tool logs are deleted after each case, including on
error. Source documents remain in the caller's external corpus directory.
The script hashes the DIB wrapper and the exact JBIG2 bytes copied into the
PDF, then compares that digest to the earlier image-span hash before running
any external pixel tool. This detects a source change between the profile
read and PDF spooling.

The canonical pixel bytes are PBM `P4` rows from top to bottom. The most
significant bit is the first pixel, `1` denotes black, and the unused low bits
at the end of each row are set to zero before comparison and SHA-256 hashing.
Both tools must report the DIB width and height and identical canonical rows.
The manifest stores the encoded span hash, canonical pixel hash, black-pixel
count, dimensions, flags, and a per-image `PASS` status. A `PASS` means
**these two tools agreed**; it does not establish independent decoder
implementations. A failure identifies source ID, page, image, and absolute
span, and prevents manifest replacement. No failed or skipped image is
silently omitted. Manifest validation rejects unknown fields at every level,
including sample, image, profile, and toolchain objects, so accidental raw
payload or bitmap fields cannot enter the metadata-only JSON.

On the measured Linux toolchain, `ldd` shows MuPDF dynamically linking
`libjbig2dec.so.0`, while it shows no such dynamic link for Poppler
`pdfimages`. This does not rule out shared or related decoder source in
Poppler. The manifest therefore labels implementation independence
`UNVERIFIED`. No decoder source was inspected or copied; the programs and
their libraries are external development-only black-box tools, absent from
the Rust native, WASM, CLI, and JavaScript dependency graphs.

Run the optional oracle from the repository root:

```sh
python3 scripts/jbig2_oracle.py --corpus-dir /path/to/CAJSamples --json
```

The first evidence-producing run can write the metadata manifest after all
546 cases agree; review its diff before committing:

```sh
python3 scripts/jbig2_oracle.py --corpus-dir /path/to/CAJSamples --write-manifest --json
```

Without the external corpus or any required black-box tool, the runner
reports `NOT_RUN` with zero tool agreements. A supplied but altered source,
bad inventory, tool error, PDF validation error, pixel mismatch, or semantic
manifest drift reports `FAIL`. Tool version and executable-hash changes are
reported as `toolchain_drift` and in `toolchains`; identical normalized pixels
remain `PASS` under a different tool build. The required clean-clone unit
tests exercise those control paths using only original tiny synthetic data;
they do not stand in for the 546-image external run.

The script holds at most one 64 KiB encoded copy chunk plus one PBM row per
tool in its pixel comparison path. It keeps one bounded PDF and two PBMs on
disk for a single case, then removes them. The selected image-record cap is
64 MiB, and the decoded PBM cap is 128 MiB per tool. The actual corpus spans
range from 2,453 to 106,323 bytes, with dimensions from 848 × 251 to
2,496 × 3,526 pixels. This bound does not constrain the internal memory use
of the external programs; a future decoder must measure its own memory.

## Measured external run

On 2026-09-24, a full run after the source-mutation and schema checks were
added, on the branch rebased onto the merged #42 and #47 implementations,
checked all 27 HN/C8 source hashes before and after decoding.
All 546 images were visited; 546 produced matching canonical pixels, with
zero failures and zero skips. The qpdf validation step returned zero and
emitted no warning for every temporary PDF. This is
**black-box tool agreement**, with backend independence still `UNVERIFIED`.
The manifest records each image's source span, encoded SHA-256, pixel SHA-256,
black-pixel count, and the exact anomaly outcome. The anomalous HN `issue-43`
page 11 image 1 also agreed, with pixel SHA-256
`eda12b7621370ec2b5687337b8ab20f52df0b4089c47197c7d835b6cb6709a06`
and 25,929 black pixels.

| Measurement | Result |
| --- | ---: |
| qpdf / Poppler `pdfimages` / MuPDF `mutool` | 12.2.0 / 25.03.0 / 1.25.1 |
| Oracle processing time, excluding the inventory subprocess | 142.476 s |
| End-to-end wall time, including the inventory subprocess | 144.065 s |
| Slowest single image, including validation and both pixel tools | 0.298 s |
| Largest temporary PDF | 106,983 bytes |
| Largest single PBM pixel raster | 1,098,864 bytes |
| Parent Python process `/proc/self/status` `VmHWM` | 23,536 KiB |
| `RUSAGE_CHILDREN.ru_maxrss` with prebuilt inventory | 26,536 KiB |

The parent high-water mark measures the Python oracle process. The child
`ru_maxrss` is the largest child-process resident-set high-water mark observed
by that process, across the inventory invocation and PDF tools; it is **not**
a sum and does not isolate the decoders. The table measures a run with an
already-built inventory executable. A prior first run in this worktree
compiled the Rust example and measured `RUSAGE_CHILDREN.ru_maxrss` at
328,008 KiB and parent `VmHWM` at 23,952 KiB; that child figure includes
build-time memory. The largest PBM raster size is
`ceil(width / 8) × height`; each case can have two such PBMs, and the table
does not claim a simultaneously measured disk peak.

The current Rust implementation parses headers and directories only. Its
JBIG2 pixel decoding and HN/C8 page assembly remain open under #9. Exact
T.88 Annex E arithmetic-state constants require a separate MIT provenance
decision before inclusion in project source; [#30](https://github.com/rwv/caj2pdf-rust/issues/30)
concerns a different T.82 table.
