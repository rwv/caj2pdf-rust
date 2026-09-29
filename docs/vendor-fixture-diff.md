<!-- SPDX-License-Identifier: MIT -->

# Compare decoded vendor fixtures

`scripts/vendor_fixture_diff.py` compares two bundles using the existing
[#125 manifest contract](vendor-fixture-manifest.md). It is original MIT
Python tooling with no new dependencies. It does not launch CAJViewer or the
converter. Acquisition must supply each page's decoded pixel payload; the
comparator does not decode arbitrary PNG/TIFF files or prove that an external
decoder produced the supplied payload. The acquisition recipe establishes
that relationship. Encoded artifacts are still required and hash-checked.

## Try original controls

Run from the repository root, using new external directories:

```sh
python3 scripts/vendor_fixtures.py example --output /tmp/fixture-left
python3 scripts/vendor_fixtures.py example --output /tmp/fixture-right
python3 scripts/vendor_fixture_diff.py \
  --reference /tmp/fixture-left/catalog.json \
  --reference-bundle /tmp/fixture-left/bundle \
  --reference-source /tmp/fixture-left/source \
  --reference-runtime /tmp/fixture-left/runtime \
  --candidate /tmp/fixture-right/catalog.json \
  --candidate-bundle /tmp/fixture-right/bundle \
  --candidate-source /tmp/fixture-right/source \
  --candidate-runtime /tmp/fixture-right/runtime
```

The generated controls contain two distinct nonblank RGB pages and Unicode
text. The result is `EQUAL`: two image observations and one nonempty text
observation match. The second page's `NO_TEXT` remains `UNAVAILABLE` for text
comparison and contributes no text equality. These are synthetic test results,
not vendor compatibility results.

For external fixtures, use the same command with their catalog and roots.
Both sides refer to the same original source IDs/hashes and ordered physical
page mapping. Capture the source and candidate PDF under the same viewer and
settings; keep the original source identity in both manifests. Differences in
bundle paths, compression and code/fixture version are permitted; differences
in runtime content/settings or capture interpretation are not silently ignored.
The acquisition report must independently identify the candidate PDF/converter;
this tool's result does not establish that provenance or advertised format parity.

## Output and exits

After argument parsing, the command emits a JSON summary. No arguments means `NOT_RUN` with
zero comparison counts; a partially specified or missing requested bundle is
an error. There is no implicit optional corpus lookup.

| Status | Exit | Meaning |
| --- | --- | --- |
| `EQUAL` | 0 | All images and available comparable text observations match |
| `NOT_RUN` | 0 | No bundles requested; no compatibility credit |
| `DIFFERENT` | 1 | Content, source identity, coverage, page mapping or dimensions differ |
| `ERROR` | 2 | Invalid/missing/corrupt/oversized input, exhausted limits or failed final integrity check |
| `INCOMPARABLE` | 3 | Runtime, origin, pixel interpretation, geometry or text acquisition differs |
| `UNAVAILABLE` | 3 | A required image is unavailable, or no content can be compared |

Known content differences take precedence over incomparable/unavailable
observations. Inspect per-page `image` and `text` fields for the full outcome.
Counts count image/text observations, not pages, documents or vendor passes.
Partial observations can be retained on error; only the final top-level status
is authoritative. Manifest integrity counters are separate from diff counts.
The `basis`, `review` and manifest identities describe what was compared;
`EQUAL` does not promote a `REVIEW_REQUIRED` fixture to a reviewed reference.

Images report changed-pixel count and the first changed `[x, y]` position
(zero-based, top-left origin). A pixel with several changed channels is counted
once. Bilevel, 8-bit and 16-bit manifest layouts are supported; nonzero bilevel
row padding is invalid. Equal image dimensions are required, and row order,
alpha, ICC identity and background must agree. There is no content alignment,
cropping, resizing, tolerance or automatic renderer-noise exemption.

Text compares raw bytes without normalization. A mismatch reports the first
byte and Unicode code-point positions. `--text-context` additionally includes
up to 33 code points per side; this can expose document content, so keep that
output external. Normalized files are integrity-checked by #125 but never
substitute for raw equality. OCR/reflow/different encodings are not silently
compared as if they were ordinary-copy observations. `NO_TEXT` and unavailable
text remain classifications and do not block otherwise equal images; they do
not count as text matches.

## Bounds and verification

Both complete manifests and all declared hashes are validated before content
comparison. Source identity/count/coverage/mapping checks precede page diffs.
A final audit rejects changed files or bundle membership. The existing limits
apply: 1 MiB manifests/text, 64 MiB artifacts/pages, 4,096 selected pages,
64 KiB read requests and a shared 60-second wall-time deadline. Each bundle
has a 4 GiB requested-read allowance covering validation, comparison and audit.

Image processing retains at most one reference page; candidate pixels are
compared in bounded chunks. The existing collected-read helper temporarily
holds chunks plus their joined page bytes, so peak page storage can approach
twice the reference payload limit. Text is page-bounded. Metadata and compact
results scale with the bounded manifest/page count, not full document pixels.
There is no full-document decoded buffer or temporary pixel spool.

The existing conformance discovery CI runs the original tests automatically.
They exercise real files and CLI exits, changed edge pixels, channel/chunk
boundaries, bilevel padding, dimensions/page order, Unicode/order/length,
missing images, corrupt/oversized files, budget failure and mutation before
the final audit. A larger two-page control verifies one reference-page
collection followed by chunked candidate reads. These tests require neither
the proprietary viewer nor CAJSamples.

The [Viewer pilot](cajviewer-capture-pilot.md) already observed unequal reopen
pixels. Such inputs must report differences, even if they look similar.
#126/#127 still own external acquisition; #129 owns actual converter validation.
