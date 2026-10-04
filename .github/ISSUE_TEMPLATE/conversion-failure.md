---
name: Conversion failure
about: A document fails to convert or converts incorrectly
title: "conversion failure: "
labels: ""
assignees: ""
---

<!--
Please do not attach the document. CNKI and similar documents usually come
without a redistribution grant, so we cannot accept, store or share them.
The structure report below is enough to locate most parser gaps.
-->

## Environment

- caj2pdf version (`caj2pdf --version`):
- Operating system and architecture:
- Interface (command line, Node.js, browser):

## Input

- SHA-256 of the input file (`sha256sum INPUT`, `shasum -a 256 INPUT`, or
  `Get-FileHash INPUT` on Windows):
- File size in bytes:

## Command and error

The exact command and its complete output, including every
`caj2pdf: error:` and `caj2pdf: warning:` line:

```text

```

## Structure report

Output of `caj2pdf inspect INPUT --json --pages`. It contains offsets,
lengths, counts and reader errors only, never document text, titles or
pixels. Do not add `--bookmarks`, which would include titles.

```json

```

## Expected behavior

What you expected instead, for example what CAJViewer shows for the failing
page (describe it; please do not attach screenshots of the document's
content).
