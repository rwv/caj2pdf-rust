<!-- SPDX-License-Identifier: MIT -->

# HN/C8 source-page layout measurements

This note records [issue #107](https://github.com/rwv/caj2pdf-rust/issues/107),
the independent metadata oracle needed before HN/C8 multi-image page
composition in [#10](https://github.com/rwv/caj2pdf-rust/issues/10). It
measures source records and black-box reference PDFs. It does not implement a
page compositor, decode source text, establish a general image-placement
formula, or enable HN/C8 in the released CLI, browser or Node.js package.

## Inputs, provenance and reproducible generation

The committed [27-source matrix](../tests/conformance/matrix.json) has SHA-256
`af42132133911f3597eed9318f613494c4047353cb7e15fc4d1008cf59ef44a9`.
The optional [reference runner](../scripts/hnc8_layout_reference.py) checks
every source size and digest before and after conversion. It runs the external
Python converter only as a black-box executable, in a new per-case directory.
Neither that converter, its separately held native decoder, the 27 documents,
generated PDFs, decoded images, nor raw source text is copied into Git or a
release. The reader and [PDF extractor](../scripts/hnc8_layout_pdf.py) are
original MIT measurement code based on the repository's [#61 bounded container
observations](hnc8-container.md) and independent PDF tool outputs. No
Python/Go/private Rust/third-party converter implementation was inspected or
translated for this issue.

The pinned external reference checkout was clean at
`8cbc3c5721acb762f739434eb3d206171dbb022a`. The recorded Linux
environment was Python 3.13.5, PyPDF2 1.26.0, `PYTHONHASHSEED=0`,
`LC_ALL=C.UTF-8`, `TZ=UTC`, `PYTHONDONTWRITEBYTECODE=1`,
`PYTHONNOUSERSITE=1`, and a fresh per-case `PYTHONPYCACHEPREFIX`. The runner
uses the exact invocation
`python caj2pdf convert SOURCE -o OUTPUT`, with `PYTHONPATH` pointing to the
separate PyPDF2 installation. A fresh working directory contains only a
`./libjbigdec.so` symlink to the separately held native library; this is
required by the black-box executable. Each conversion has a 180-second
timeout, bounded output, child-process-group cleanup on timeout/failure, and
a before/after executable audit. The external library has no separately
reported embedded version; its binary digest, the recorded compiler, and the
tool versions identify this run:

| Component | Version or role | SHA-256 |
| --- | --- | --- |
| Python interpreter | 3.13.5 | `889c603f0d17cb54060951bcf4c4f9b8c9ebd9e52b392c70209bbb9755d797d9` |
| Python reference CLI file | commit above | `c2bede4bd4e1308fb9f7ec7e5593106c5b41188c337158a61cfadadc6f81f739` |
| Separate PyPDF2 package tree | 1.26.0, 16 files | `4d33afdda9bb9ed730fba0355c42291d3380ce93f89cf4ec489338dcd385b36c` |
| Separate `libjbigdec.so` | test-only native library | `d370d071a4b7abdf7db4565c2bc85ac881470ee1a212459dd658d70974128de6` |
| Recorded native compiler | Debian g++ 14.2.0 | `6b3696e4dcb85e1c949c732a02befa50e3983ecf94ce7e8e58d9d503b954b79d` |
| Git | 2.47.3 | `356db14e102d68a1a37d8a1ac577dfd678d45d46e92f468bef8b7154e7bfdc60` |
| qpdf | 12.2.0 | `30b2389c3ed0ba73434244fff5b149d4d919279ea7a8c3b1cd9fc25f57ec1792` |
| MuPDF `mutool` | 1.25.1 | `b9588916750d90219b1511cf329776439c68e9a1ae1e6f92dc6532e21dd96df7` |
| Poppler `pdfinfo` | 25.03.0 | `a1a371340d7b76e7d501da9136cc9256dfdb9cbdf09300520d7a3b4465343e67` |
| Poppler `pdfimages` | 25.03.0 | `213eba4a36ef021f49a0abc94292a7566baba8dfafef5017467166d9f06074f5` |

The recorded final report is at
`/tmp/caj2pdf-layout-reference-final-v7-report.json`. To reproduce the
generation and save a report for the comparison command below, run:

```sh
mkdir -p /tmp/caj2pdf-layout-reference-final
python3 scripts/hnc8_layout_reference.py \
  --corpus-dir /tmp/caj2pdf-ranged-corpus \
  --reference-repo /tmp/caj2pdf-python-oracle \
  --python-bin /tmp/caj2pdf-oracle-venv/bin/python \
  --pydeps-dir /tmp/caj2pdf-oracle-pydeps \
  --jbig-lib /tmp/caj2pdf-jbig-oracle/libjbigdec.so \
  --artifact-dir /tmp/caj2pdf-layout-reference-final \
  --max-perturbations 12 --json \
  > /tmp/caj2pdf-layout-reference-final/report.json
```

The two conversions of each source were byte-identical. The HN-A and C8
artifact hashes also match the two earlier exploratory PDFs exactly:

| Case and pinned source | Source SHA-256 | Output pages / ordered draws | PDF SHA-256, both runs |
| --- | --- | ---: | --- |
| HN-A `issue-21/实时网络流量异常检测算法研究和系统实现_林尚朕.caj` | `33386f14fd75994c7c70c8b4578d25ee1c731d5d76bb6ad4db48be0a774495d4` | 68 / 91 | `833f0c40881f0baf7f76f3505b6679e2dfa0b16c0e469dfbf10f791e687dec40` |
| C8 `issue-33/test1.caj` | `35951c3775790c230c84e4312e8db7a57ca2980a04d35ff03d328806df7d622c` | 7 / 34 | `acbd822358c08713a795f8c0ad82c6f23c4b43d9e137d515c3210114ca1bc885` |
| HN-B `issue-65/伽利略的原子论思想_近代科学革命的形而上学基础.caj` | `e1b17805a87f62097987c41f2821836d6b774caaf035c9846be966c965f08a49` | 2 / 2 | `b058b38be3efc3915f69bb79d6160d7a54c4ef47ac529ebe963214eb697d2a51` |

## Metadata and independent PDF checks

The compact [oracle](../tests/conformance/hnc8_layout_oracle.json) contains
only source IDs, page/image numbers, checked descriptor type/offset/length,
image dimensions and payload SHA-256, text-span offset/length/SHA-256,
output-page-to-source-page mapping, PDF artifact digest, MediaBox, ordered XObject
identity/type/dimensions/raw-stream digest and affine draw matrices. It has no
source page-row unknown values, raw text, image or PDF bytes, absolute private
paths, or rendered pixels. Its three cases contain 81 source rows, 77 output
pages, and 127 ordered draws; the original two-PDF subset is 75/75 pages and
125/125 draws.

The [opt-in comparison](../scripts/hnc8_layout_parity.py) first verifies all
27 sources against the matrix, then measures their rows independently: 2,047
declared rows, 2,044 structurally valid rows (2,027 with images, 17 without),
405 multi-image and 402 mixed-type rows. It finds 1,400 type-0, six type-1,
1,085 type-2 and 546 type-3 descriptors. Three known malformed `issue-100`
rows remain separate: page 2/image 1 unsupported type at byte 12,886, page 3
negative image count at byte 264, and page 4 out-of-range text span at byte
276. No malformed discovery counts as a matching layout page. The earlier
#22/#102/#43 inventories remain separate image-metadata evidence; their
hash-only files contain no source bytes.

For each reference PDF, qpdf parses the page resources, boxes, filtered
content stream and `q/Q/cm/Do` draw order; MuPDF independently checks page
boxes, ordered draws, dimensions and transformed CTMs via its trace output;
Poppler independently checks image object identity, order, dimensions, color
and bit depth. qpdf and MuPDF independently hash every original encoded image
stream. Repeated XObject uses remain separate draws. The parser rejects
unknown graphics operations, Form XObjects and invalid geometry explicitly;
it is not a general PDF interpreter. The 53 selected JPEG source payload
hashes match their PDF `/DCTDecode` raw-stream hashes exactly. The type-0 PDF
stream is decoded output and is not claimed byte-identical to its coded DIB
payload.

Run the private comparison with the run-2 PDFs named in the generation report:

```sh
python3 - <<'PY'
import json
import subprocess
from pathlib import Path

report = json.loads(Path("/tmp/caj2pdf-layout-reference-final/report.json").read_text())
pdfs = {case["profile"]: case["runs"][1]["output_path"]
        for case in report["generations"]}
subprocess.run([
    "python3", "scripts/hnc8_layout_parity.py", "--json",
    "--corpus-dir", "/tmp/caj2pdf-ranged-corpus",
    "--hn-a-pdf", pdfs["hn_a"], "--c8-pdf", pdfs["c8"],
    "--hn-b-pdf", pdfs["hn_b"],
], check=True)
PY
```

The final comparison passed 75/75 original pages, 125/125 original draws and
the separate HN-B 2/2 pages and 2/2 draws: **77/77 output pages, 127/127
draws, zero mismatches, skips and unexpected unsupported PDFs**. All 27
source and three PDF SHA-256 values were identical before and after; the
tool-executable hashes were also unchanged. A clean clone reports `NOT_RUN`
and zero compared pages, draws and private compatibility matches; an
explicitly requested missing or changed input fails.

## Sizing, placement and unresolved fields

All 77 first images fill their MediaBox. The box dimensions exactly follow
`image pixels × 72 / 300` points within floating-point noise (largest
absolute difference `1.14e-13` points). For type-0 DIBs, page width uses the
32-bit row-stride width rather than the visible width: C8 page 1 has visible
width 2,573 pixels, padded width 2,592, and a 622.08-point page width.
The 50 additional HN-A/C8 image draws also scale their own pixel dimensions
by `72 / 300` (largest difference `1.14e-13` points). Every additional draw
has fractional x or y in the PDF. Their source-field placement rule remains
**unknown**.

The HN-B source has six index rows with image counts `[1, 0, 0, 0, 0, 1]`.
The four image-free rows have positive text spans of 8,084, 7,484, 7,488
and 7,120 bytes, yet the black-box output has only two pages. Its two JPEG
stream hashes prove that output pages 1 and 2 map to source rows 1 and 6,
respectively. This is one measured document, not a general omission rule.

The 20-byte row fields at `+10`, `+12`, and `+16` remain uninterpreted. In
the 27-source valid-row inventory, `+12` is zero in 2,043 of 2,044 rows;
`issue-100` page 1 is the one nonzero case. Each of 436 valid consecutive
row pairs with nonzero `+16` in the earlier row matches the next row's text
offset. Those observations do not identify image coordinates. All 3,037
valid image descriptors have zero gap before their payload; structural
descriptor fields are excluded from coordinate perturbations.

The black-box runner changes one field in one temporary source copy per
attempt: low- and higher-order byte flips in each of `+10`, `+12`, and `+16`
on C8 page 1 and HN-A page 16. All 12 requested attempts were observed; 11
generated byte-identical PDFs with unchanged page boxes and draw CTMs, and
one higher-order `+16` mutation on C8 page 1 failed conversion without a PDF.
There were zero surviving translation candidates, so no numeric x/y rule
was proposed and no held-out validation is claimed. These results describe
only the 12 stated byte changes; they do not prove that every bit or text
field is irrelevant. No guessed multi-image compositor is implemented.

Each temporary copy flips bit 0 (`xor 0x01`) of exactly the listed absolute
source byte. The original source and successful output PDF SHA-256 values are
in the case table above; `PDF_IDENTICAL` means the full output SHA-256 equals
that case's value. The final-source report records the field-span hashes and
the post-conversion mutated-input audit as well. The compact public attempt
record is:

| Source/page | Row field | Changed byte offset | Mutated source SHA-256 | Outcome |
| --- | --- | ---: | --- | --- |
| C8 p1 | `+10` | 90 | `bcec1d557ba67cbcd6c9ea06edcba9ca680e209d3aa594293d3a7e3ba0436ddc` | `PDF_IDENTICAL` |
| C8 p1 | `+12` | 92 | `1823b1c4d04f96547d943995f1ec85ad5c98e18332beefd968b3badc2e19801c` | `PDF_IDENTICAL` |
| C8 p1 | `+16` | 96 | `183663ded47c8cfaedaaa8c53b03c442208040c6bd06f6217043d588ab86bb1f` | `PDF_IDENTICAL` |
| C8 p1 | `+10` | 91 | `945a2223c3e88566b7cc7d9ea4b3da214921a3284eedb04d62bfb7fc905131ff` | `PDF_IDENTICAL` |
| C8 p1 | `+12` | 94 | `045dd351da7a2dd2a7831e40039e10b440de5f615b298e821411060753f7bb70` | `PDF_IDENTICAL` |
| C8 p1 | `+16` | 98 | `a8d048116ee9610de3338ec48e6a06812cfe7f3e810d1a440a5155d4a843b7d3` | exit 1; no PDF |
| HN-A p16 | `+10` | 16,674 | `7e12b97b4ae929e6ac88408e1e43221a167c56f3329c3d04510e246f4aa51711` | `PDF_IDENTICAL` |
| HN-A p16 | `+12` | 16,676 | `1e7ce3c6bb895933919ee241ac8ae0af47a67db5d70ea8250aaa5b64c454f8eb` | `PDF_IDENTICAL` |
| HN-A p16 | `+16` | 16,680 | `5b34aea9080308167383da848997a8785ed5146236dcb9ea2dbd2ebc118b1261` | `PDF_IDENTICAL` |
| HN-A p16 | `+10` | 16,675 | `c0dc3fbe8de1181dfba9cd5cb1d1cef9a2e70a548340af92de498c326690d48b` | `PDF_IDENTICAL` |
| HN-A p16 | `+12` | 16,678 | `0f8da18ab8d8948f657f83d62d0037b1445fe12b2e2810fc7f0dcc91eb497997` | `PDF_IDENTICAL` |
| HN-A p16 | `+16` | 16,682 | `4df97360b234077221c38c3875d238425af615942e13bf40dc79cb453a3d8864` | `PDF_IDENTICAL` |

## Resource and verification boundary

The final parity comparison used at most 65,536 bytes per ranged source
request and read 273,634,985 bytes through that reader, excluding separate
pre/post source hash passes. Its largest input PDF was 8,239,244 bytes; the
three reference PDFs totaled 11,825,498 bytes. The largest captured tool
stdout was 790,923 bytes. Linux `/proc/self/status` reported the Python
driver's peak `VmHWM` as 25,140 KiB, separately from the largest validator
child at 42,592 KiB. The PDF extractor created no temporary files of its
own; that zero does not measure hidden internal temporary storage of qpdf,
MuPDF or Poppler. The 12-probe generation run retained 137,159,801 bytes of
inspectable external artifacts outside Git. Its 20 ms disk sampler observed
at most 137,159,801 bytes in the session; the harness peak `VmHWM` was
26,264 KiB and the largest converter child was 41,356 KiB. Generation and
perturbation used at most 1 MiB per file-hash read. These are diagnostic
measurements, not conversion-runtime benchmarks.

The synthetic tests cover source truncation, short/zero/overreported ranged
reads, aliasing, cancellation and limits; PDF draw order, repeated XObjects,
nested CTMs, malformed page boxes and CTMs, tool disagreement, timeout and
changed files; clean-clone and altered metadata-oracle behavior. Private
source/PDF compatibility is counted only by the explicit opt-in runs above.
