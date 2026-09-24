# KDH PDF wrapper observations

This note records independent byte measurements for [issue #11](https://github.com/rwv/caj2pdf-rust/issues/11). The only KDH inputs used were three files from the external [CAJSamples](https://github.com/caj2pdf/CAJSamples) corpus at commit `7e1c35e7b6de34e21972fcd1752c2a7e99b4ad07`. Their paths and reference output fingerprints are in the [conformance matrix](../tests/conformance/matrix.json). No CAJSamples document, decrypted PDF, or rendered page is stored in this repository.

| Corpus row | Input SHA-256 | Input bytes | `%%EOF` absolute offset | PDF end (exclusive) | Opaque trailing bytes | PDF bytes |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| `issue-21` | `556541114226584fe9e4a08ad6600f9d38bc27ae307fdd17e4246831bcfb1054` | 480,486 | 480,469 | 480,475 | 11 | 480,221 |
| `issue-34` | `d19a294f78a6ec8dfe27993288a88a0560aa369fe5c4ebb22ca5f57c85209fb3` | 1,877,728 | 1,836,885 | 1,836,891 | 40,837 | 1,836,637 |
| `issue-48` | `5f6f1af5b148af2b6756ff8878124c09797882505d0aca12ca983d9073d94507` | 37,743 | 37,574 | 37,581 | 162 | 37,327 |

## Wrapper and byte transformation

Each input begins with the 32-byte ASCII signature `KDH 2.00 Copyright(C) 2000 CAJCD`. The four-byte little-endian field at offset `0x28` is `0x00020000` in all three files. The PDF payload begins at absolute offset 254 (`0xFE`). The other observed header fields vary; their meanings are not established, so the decoder does not infer a PDF length from them.

For PDF-relative byte position `i`, the observed transformation is:

```text
pdf[i] = kdh[254 + i] XOR "FZHMEI"[i mod 6]
```

The six key bytes follow by XORing the first six encoded payload bytes with the public PDF header prefix `%PDF-1`. Applying this phase produces valid `%PDF-1.7`, `%PDF-1.6`, and `%PDF-1.2` headers respectively, with the expected PDF binary-comment line. The same cycle restores indirect object delimiters, `startxref`, and `%%EOF` throughout each file. As a negative control, none of the encoded payloads contains a raw `%PDF-`, `startxref`, or `%%EOF` marker; each decoded payload contains exactly one of each. The isolated author of `kdh.rs` derived and verified this rule only from the pinned corpus bytes and independent PDF readers, without consulting converter source.

The last `%%EOF` is followed by LF for `issue-21` and `issue-34`, and CRLF for `issue-48`. The remaining bytes are outside the PDF and are treated as opaque KDH data. A scanner must keep its XOR phase across chunk boundaries, recognize a line-delimited `startxref` number and `%%EOF`, check that the referenced offset begins with a plausible xref table or indirect xref object, and avoid taking a later false marker in trailing data as the PDF end. The PDF-relative `startxref` values are 479,678, 1,834,967, and 24,259 in row order. The first two point to Flate-compressed xref-stream objects with `/W [1 4 1]`; their 82 and 445 entries are only free or ordinary uncompressed-object entries. The third points to a classic xref table. No type-2 compressed-object entry was observed.

The decoder first checks the KDH signature, version field, and decoded PDF start. It scans in at most 64 KiB requests, retains only a fixed 160-byte marker history, then exposes a positioned XOR view limited to the measured PDF end. A second plausible xref/EOF pair pointing beyond the first PDF boundary is rejected as ambiguous; no incremental KDH PDF was observed in this corpus. The shared PDF reader still owns object validation and output normalization. The complete KDH source size, including trailing bytes, is checked against `Limits.max_input_bytes` before scanning. A caller with forward-only input must use a platform spool; the core never stores the decrypted document as a whole.

## Independent PDF checks and present limits

Temporary decoded PDFs cut at those ends had 6, 67, and 1 pages. Every page dimension and all 74 MuPDF PAM RGB render SHA-256 values matched the matrix. MuPDF 1.25.1 rendered all pages. `qpdf --check` 12.2.0 accepted `issue-48` cleanly. For `issue-21` and `issue-34`, qpdf returned warning status 3 because object 1 is a `/Pages` dictionary with two identical direct `/MediaBox[0 0 612 792]` values. `issue-21` also has six stream keywords followed by a lone CR, which qpdf warns about. Their page renders still matched; a clean validator pass requires the shared PDF repair path. The temporary, merely trimmed PDFs had SHA-256 values `86011857255adcfe313f72e4dac86a0f8b1562921060adcb6808cc4508d81d52`, `d6aef22832ad11d2e86f3b9033eaa339c659001efe177662c15a1c6348d29f88`, and `448d785710c5c2195d326e1a104894efc7ead4c215b1aa3e0a105e32e0c604b5` respectively.

At the initial PDF-layer baseline, `convert_kdh` rejected the first two files as unsupported xref streams at KDH absolute offsets 479,932 and 1,835,221. The third exposed a stale `/Parent 606 0 R` reference in object 184. The bounded shared PDF normalization in [issue #36](https://github.com/rwv/caj2pdf-rust/issues/36) addresses these observed structures. The synthetic KDH tests cover valid classic xref conversion, short ranged reads, cancellation, sink failure, and typed wrapper, missing-EOF, false-trailer-marker, full-input-limit, and corrupt-object failures. Synthetic tests are not counted as external corpus compatibility passes.

## Reproducing the scan and memory observation

Verify each input SHA-256 against the table first. Build the release probe, then run it on the three matching local corpus paths. The probe opens the KDH virtual source and reports PDF length and trailing bytes; it does **not** index or write a PDF:

```sh
cargo build --release -p caj2pdf-core --example native_kdh_probe
python3 - <<'PY'
import json, os, subprocess
from pathlib import Path
root = Path(os.environ['CAJ2PDF_CORPUS_DIR'])
rows = json.loads(Path('tests/conformance/matrix.json').read_text())['samples']
for row in rows:
    if row['detected_type'] != 'KDH':
        continue
    path = root / row['path']
    process = subprocess.Popen(['target/release/examples/native_kdh_probe', str(path)], stdout=subprocess.PIPE, text=True)
    _, status, usage = os.wait4(process.pid, 0)
    print(row['id'], os.waitstatus_to_exitcode(status), usage.ru_maxrss, process.stdout.read().strip())
PY
```

On Linux 6.17.4-2-pve x86_64 with rustc 1.98.1, the release probe's per-process peak RSS from `os.wait4` was 10,760 KiB (`issue-48`, 37,743 input bytes), 10,900 KiB (`issue-21`, 480,486 bytes), and 10,900 KiB (`issue-34`, 1,877,728 bytes). A sparse temporary copy of `issue-48` with a 64 MiB zero trailer measured 11,596 KiB RSS for a 67,146,607-byte input and still reported a 37,327-byte PDF. This is evidence for bounded decoder buffers only. Full converter measurements, including the PDF object/page index and repair writer, remain required after the shared PDF path accepts all three inputs.

## End-to-end KDH conversion evidence

In a temporary integration checkout containing the #36 PDF changes and the #11
KDH converter, all three pinned raw KDH inputs converted to PDFs outside Git:

```sh
cargo build --locked --release -p caj2pdf-core --example native_kdh_to_pdf
target/release/examples/native_kdh_to_pdf INPUT.caj OUTPUT.pdf
qpdf --check OUTPUT.pdf
python3 scripts/conformance.py --corpus-dir /path/to/CAJSamples \
  --pdf-dir /path/to/converted-pdfs --only-format KDH --json
```

`qpdf --check` exited 0 without warnings for 3/3 outputs. The format-scoped
matrix reported inventory `PASS` 3/3 and PDF `PASS` 3/3: page counts,
dimensions, outline hierarchy/destinations, and all 74 rendered-page hashes
matched. The same Rust `convert_kdh` future was also called from the real
WASM runtime through both Blob/Web Writable and positioned Node file/Writable
adapters in synthetic tests; `node --test js/test/*.test.mjs` passed 21/21.
The production JavaScript package API remains issue #13 work.

For the full release converter, including PDF indexing and repair writing,
Linux `os.wait4(...).ru_maxrss` on the same host measured:

| Input | Input bytes | Peak RSS |
| --- | ---: | ---: |
| `issue-48` | 37,743 | 7,896 KiB |
| `issue-21` | 480,486 | 7,896 KiB |
| `issue-34` | 1,877,728 | 7,908 KiB |
| `issue-48` plus a sparse 64 MiB zero trailer | 67,146,607 | 8,848 KiB |

The large-trailer output had the same SHA-256 as the original `issue-48`
output. These measurements show a bounded path for the observed corpus and
trailer experiment; they do not establish a global memory ceiling for every
possible PDF object count or configured limit. The corpus and all generated
PDFs remained outside the repository.
