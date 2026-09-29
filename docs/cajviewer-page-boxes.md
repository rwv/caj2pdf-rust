# CAJViewer page-box fallback

## Observed behavior

A headerless CAJ fragment may lose the page-tree node that supplied inherited
page dimensions. For a **synthesized root only**, the converter supplies
`/MediaBox [0 0 612 792]`. Existing descendant MediaBox entries take precedence.
This is a compatibility fallback matching observed CAJViewer behavior, not a
claim that the lost source geometry can be recovered. Existing PDF validation
still rejects invalid explicit boxes; ordinary PDF input is not repaired here.

On 2026-09-29, CAJViewer 9.0.0 rendered two original PDF controls identically:
one omitted MediaBox, and one explicitly used Letter (612 × 792 points).
An explicit A4 control rendered differently. The missing-box and Letter page
captures had **zero changed pixels** over the complete 651 × 843 RGB grid;
both RGB SHA-256 values were
`36cda9116272f089671cd3f6491eae3930f94950d660df4a0edb1f5abe6af16f`.

## Reproduce the controls

Use a four-object PDF: catalog 1, page tree 2, page 3, content stream 4.
The page tree has one kid, `3 0 R`; the page has parent `2 0 R`, empty
Resources and Contents `4 0 R`. Keep the same content for all controls:

```text
q 1 0 0 RG 2 w 2 2 608 788 re S 0 0 1 RG 10 10 m 600 780 l S 0 0 0 rg 100 100 40 60 re f Q
```

Generate missing, Letter and A4 variants by omitting the page MediaBox or
setting it to `[0 0 612 792]` or `[0 0 595.276 841.89]`. Write correct stream
lengths and xref offsets. These controls contain only original vector marks.

Use the pinned image, font and display settings from the
[fixture snapshot](cajviewer-fixture-snapshot.md): maximize, close the sidebar,
select single-page mode and enter 80% zoom. Verify the settings after every
open; document opens reset the view. Capture the full desktop, then extract
the physical page at `(499, 245, 651, 843)` for missing/Letter. Do not resize or
align by content. The initial scripted zoom attempts for Letter/A4 failed to
apply; those captures remain failed attempts and were excluded. The corrected
captures visibly show 80% and single-page mode.

## Real CAJ result

External CAJSamples issue 77, source SHA-256
`5d988d74a6e6a0c392eb58297e70d91ff2e1ad2c2887374a04adc67a253ac2ab`,
now converts to a 536,962-byte PDF with 75 Letter pages. `qpdf --check` and
`pdfinfo` pass. All 75 PDF page object IDs match the source page-table order.
Output SHA-256:
`17af66b3201925945c16cbfbda3b587cc3cd2369eafe9ddfcd2f71f15f72c1cf`.

Opened in the same pinned CAJViewer environment, output pages 1 and 75 each
match the existing source captures with **zero changed pixels** over their
complete 651 × 843 grids:

| Page | Source and output RGB SHA-256 |
| --- | --- |
| 1 | `9fa77ab3248d9916f399d5a42b743a1b0a68a8bec2e6043de292837ff03b8fda` |
| 75 | `4e4c15f79afbb172db8fb769e096723e0a59a3d1f26d00bcf6d74db0fbdd2b35` |

This establishes selected-page equality, not rendered coverage of all 75
pages, text parity or other formats. The clipped top content visible in the
source viewer remains visible in the output; the fallback does not invent a
larger page to restore content outside the viewer's displayed bounds. The
Python reference still fails on this input and supplies no usable baseline.

External evidence is retained under `caj2pdf-page-size-20260929`: controls,
full screenshots, comparison JSON, page-order JSON, output and runtime logs.
The offline container used the snapshot's resource limits and was stopped and
removed. Its observed memory peak was 614,481,920 bytes; that is viewer/display
memory, not converter memory. No external document, output or capture is
committed. Prior acquisition receipt/review states remain unchanged.
