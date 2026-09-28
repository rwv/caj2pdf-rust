<!-- SPDX-License-Identifier: MIT -->

# Original PDF controls and Linux startup canary

This is work toward [#124](https://github.com/rwv/caj2pdf-rust/issues/124).
The offline image is prepared. The first two original-PDF startup attempts
failed; detailed capture was unavailable because the Docker cp transport
omitted tmpfs files. **Full-page and text compatibility remain NOT_RUN, zero
passes.** The capability issue remains open.
The [fixture plan](cajviewer-fixtures.md) and issue acceptance criteria govern
the later image/text capability probes and private acquisition.

## Verified public preparation

The official installer at
`https://download.cnki.net/cajviewer_9.0_amd64.deb` was streamed and verified:
235,087,704 bytes, SHA-256
`3142c633d74dcf34ebaca9b7653f88ad3619f0b7a6cb689487b6cc583ec926d3`.
Its static control metadata declares `9.0.0-24093`, amd64. Packaged `VERSION`
declares Ubuntu 18.04, Qt 5.15.10 and application 9.0.0. Those labels do not
measure the loaded Qt runtime, About build, rendering backend or device
pixel ratio. The desktop entry's documented launcher is
`/opt/cajviewer/bin/start.sh %f`; it has no `Path=` field.

The packaged help PDF was extracted as documentation only, 4,276,992 bytes,
SHA-256 `7f700c9a3dc7a30631e086f896dffe0e6f997b1a0bbb63c19821f93c730fa77c`.
It documents opening, region capture, ordinary/enhanced copy, OCR and printing.
It does not establish a supported Linux render/text CLI or complete-page image
export. Its Ctrl+A description itself contains a draft verification note.
Treat menu/shortcut capabilities as questions for the observed build.

The first bounded package-metadata producer failed before its first entry;
the exact cause is unknown. That FAIL receipt is retained externally. An
explicit single-thread amendment succeeded: 903 archive entries and
967,298,772 declared member bytes. No vendor implementation was inspected.

A public X11-only preparation control initially failed readiness. Its preserved
diagnostic amendment showed that `xdpyinfo` exceeded the original 32 KiB output
cap. The session now permits bounded 256 KiB display metadata, stores the full
record separately and keeps a hash-bound summary in the session receipt. A
fresh public X11 control then passed with an exact 1600 × 1200 RGB capture.
This was a public helper failure; no viewer had been launched.

The runtime is an **extraction-based experimental Bookworm profile**. Vendor
maintainer hooks are omitted. All `/opt/cajviewer/doc` entries, including the
bundled `example.caj`, are omitted without opening those documents. No CAJ
corpus is accessible. The package's proprietary runtime files are opaque
hashed. This is not a verified vendor-supported Debian installation.

## Ownership and external storage

The [AUR packaging metadata](https://github.com/archlinux/aur/blob/04001d051c1f8bf7fc82c283b8b9bae4412ea1ed/PKGBUILD)
declares a custom license. No separately named license/EULA file was found in
the package member inventory. The
[vendor usage agreement](https://cajviewer.cnki.net/protocol/UseAgreement.html)
is a public registered-service reference; applicability to this exact local
package remains unverified. No distribution grant or MIT eligibility is
inferred. An actual agreement dialog requires separate handling; this driver
never accepts one.

Keep the installer, extracted files, manuals, fonts, vendor-containing image,
screenshots, raw logs and receipts outside Git and release artifacts. Do not
publish the development image. Committed recipes, drivers, generators and
tests are original MIT code. The recipe uses public Debian packages only as
external development dependencies; their licenses are not relabeled MIT.

## Reproducible preparation boundary

[Dockerfile](../tools/cajviewer/Dockerfile) selects `linux/amd64` Debian at
platform digest
`sha256:f3034a6ec3c1205360777c4aae76234998866ad18806ae62b63a3f84ccad782b`.
The public registry response bytes were verified against that digest. Public
package installation uses the `20250901T000000Z` Debian snapshot and exact
direct versions written in the recipe. The corresponding Bookworm InRelease
is 151,074 bytes, SHA-256
`43aa39b1719427e47f5b0d3e9cc9575520fc5b18f9967498b5e2d924c0535827`;
APT still verifies repository signatures. Retain full installed package,
opaque library/plugin, font/fontconfig and tool inventories externally.

Create a dedicated external context containing only the verified extracted
runtime tree and original recipe/session sources. Do not use a workspace,
home or corpus directory as the context. The build performs public package
preparation; it never invokes the vendor application or installer hooks.
[prepare.py](../tools/cajviewer/prepare.py) refuses existing output/receipts,
uses a single-thread archive producer, 64 KiB reads, 2 GiB archive/member-byte
ceilings, 10,000 members and a 120-second producer deadline, followed by a
separate five-second diagnostic drain and five-second process reap allowances. Failed receipts and
partial directories are retained. Its final source audit also runs on failure.

[inventory.py](../tools/cajviewer/inventory.py) hashes declared runtime roots
opaquely, at most 8,192 entries / 4 GiB, and runs only public metadata helpers.
It does not load vendor libraries or establish that a complete capture works.
The first v3 inventory helper invocation failed because its mount placed it
outside the directory containing its original MIT import. A preserved v4
mount amendment succeeded without changing the image or app configuration.
Freeze the actual image ID, this inventory's exact identity and all source
file hashes together before the first application launch.

## Original controls

[cajviewer_canary_fixtures.py](../scripts/cajviewer_canary_fixtures.py) authors
PDF objects, Type 3 glyph rectangles and ToUnicode mappings directly. No
installed font, converter output or external document is read. Controls are
generated at runtime into a new directory:

```sh
python3 scripts/cajviewer_canary_fixtures.py --output-dir /absolute/external/controls-v1
```

| File | Original condition |
| --- | --- |
| `digital.pdf` | Four ordered pages: fractional 256.5 × 192.25 pt, known blank, 90° rotation, and an eight-times larger box. Latin/CJK glyphs, two columns and asymmetric colored edges/corners. |
| `alternate-unicode.pdf` | Identical visible operators; only the `R` ToUnicode mapping changes to U+E000. Independent MuPDF renders agree exactly. This can characterize existing-text behavior without assuming every copy is native text. |
| `image-only.pdf` | A tightly bounded 1026 × 769 RGB array depicts the control. It has no font, ToUnicode or text-showing operators. |
| `second-text.pdf` | Different ordered text exposes stale-copy reuse. |

qpdf validates all four PDFs, Poppler validates expected Unicode and the empty
image-only text layer, and MuPDF validates the alternate mapping's identical
pixels. These are original controls, not CAJViewer compatibility passes.
`original_control_edges` checks the original raster's size and four midpoint
markers only. It is not complete-edge, page-extent or arbitrary viewer-crop
proof. The later acquisition protocol must demonstrate all actual page bounds.

## Frozen startup-only protocol

[run.py](../tools/cajviewer/run.py) takes an explicitly supplied, immutable
`original-pdf-startup-v1` protocol and a fresh external output directory. A
no-input invocation returns **NOT_RUN**, zero app launches/passes, without
Docker calls or environment-variable auto-discovery. Explicit missing inputs,
changed hashes, wrong image or unexpected control files fail.

The protocol records two distinct predeclared container names, exact source
pins, the five original control-file pins, the prepared image ID and runtime
inventory identity. The reader also compares each PDF against the original
recipe and binds the image's session/core file identities to the host code.
No build/pull/profile/flag/backend retry occurs during this phase.

| Boundary | Enforced startup limit |
| --- | --- |
| Sessions | Exactly two planned fresh attempts; stop further scheduling if cleanup or the global deadline fails. |
| Vendor launches | One official desktop launcher attempt per session, `/input/digital.pdf`, working directory `/home/canary`. No UI click, dialog acceptance or other document opening. |
| Container | UID/GID 1000, read-only root/inputs, offline network, init, dropped capabilities, no new privileges, no host display/home/socket. |
| Memory / swap / CPU / PID | 1536 MiB entire cgroup; memory+swap also 1536 MiB; 2 CPUs; 256 concurrent tasks/threads. These are not Rust RSS or cumulative descendant-start counts. |
| Bounded writable storage | Home 64 MiB, `/tmp` 64 MiB, runtime 8 MiB, output 32 MiB, shared memory 64 MiB. |
| Display | Dedicated Xvfb `:99`, 1600 × 1200, depth 24, requested 96 DPI; X11 RGB masks/stride are measured. Vendor DPR/Qt/backend stay UNKNOWN. |
| Persistent helpers | One Xvfb and one openbox attempt; at most 400 controlled metadata helper attempts. |
| Stage time | X11 readiness 10 s; title search 30 s; helper primary deadlines 2 s, plus bounded reap allowances. Query sleeps do not prove readiness. |
| Host time / calls | Collection readiness 60 s / 60 polls; primary Docker calls ≤80, with three independent closing slots. Global scheduling deadline 360 s plus closing allowances. |
| Artifact extraction | Original in-container Python tar stream at most 40 MiB; exactly seven eligible flat diagnostic filenames, ≤32 archive members, 32 MiB aggregate content, ≤6 MiB per file; unknown names, symlinks/special/traversal entries fail. |

The session matches one visible `digital.pdf` window and records its title and
geometry. This is a startup observation only. The diagnostic whole-screen P6
is explicitly `viewport-diagnostic-only`, `complete_page: false`. The host
checks the exact P6 grid, full payload length/hash and absence of tail before
accepting artifact integrity. No image/text comparisons or vendor passes are
performed.

The scheduling deadline is checked before primary operations. Existing helper
deadlines/reap allowances finish in bounded time. Closing actions always retain
their independent budgets and cannot be interrupted by a scheduling alarm.

Before/final cgroup metrics include `memory.current`, `memory.peak`,
`memory.events` and PID observations. A rise in `oom_kill` fails startup even
if the supervisor survives. Process snapshots are sampled current argv lists,
not a complete descendant launch inventory. Do not infer peak application
memory from the Docker client or subtract cache without declaring the method.

Required-stage errors force FAIL even after observing a window. The session
terminates its started process groups, closes the receipt, then writes a
separate READY marker. A 60-second collection lease keeps bounded tmpfs data
mounted for the host; it does not establish application readiness. The host
always removes its owned container by known name and audits absence, including
when the create client fails or log collection raises. A pre-existing container
is never removed. Final source/control/protocol/inventory audits are mandatory.

## Preserved first startup failure

The independently reviewed first protocol used source commit
`56f928b52b531af0694c56bac42786748a569add`, protocol SHA-256
`6df29f3e990152fea101d5fdcfcb4ee60639f8d9a8cde37d53ed2b5f73220229`
(8,317 bytes), and prepared image
`sha256:e2bb737b662f863a5a99f959bc411ce5b4e248d18b0c6a5dff191e25bcf5cf2c`.
Its 517,977-byte runtime inventory has SHA-256
`717022a8c0a1a8dafbcc5dc6ffd06acfc71081d6dadc9a669199586b52bfbaa0`.

Both supervisor stdout records report FAIL with one launcher attempt and zero
vendor passes. Host receipts honestly retain `UNKNOWN_AFTER_START`: Docker cp
returned only the empty `output` directory while the container was running,
so the detailed session receipt, application log and diagnostic pixels were
unavailable. No application failure cause or successful document opening is
inferred. Both owned containers were removed, absence was audited, and final
source/control/protocol/inventory audits passed.

[Docker's documented tmpfs limitation](https://docs.docker.com/reference/cli/docker/container/cp/#corner-cases)
requires collection inside the container's mount namespace. The narrow
transport amendment uses its pinned Python tool to stream seven known regular
diagnostic files through a held directory descriptor. It refuses extras and
unstable, oversized or special files. Streaming, extraction and independent
cleanup limits remain unchanged. Original process tests and a public tmpfs-only
canary must verify this transport before a separately frozen/reviewed pair of
app attempts. The first failure remains immutable; no app flag, profile, image
or rendering-backend retry is part of the amendment.

## Public tests and remaining acceptance work

```sh
python3 -m unittest discover -s tests/fixtures -p 'test_cajviewer_canary_fixtures.py' -v
python3 -m unittest discover -s tests/conformance -p 'test_cajviewer_*.py' -v
python3 tools/cajviewer/run.py
```

These required original tests cover structure/Unicode/pixels, missing/bad
installer pins, process failure/output limits, child termination, partial
capture, stale clipboard metadata, trailing producer diagnostics, immutable
failure receipts and independent daemon cleanup. They do not execute Docker
or the vendor app in CI.

After independent protocol review, record actual startup attempts and preserve
their first failures. Full-page capture, physical page navigation, ordinary
copy freshness, enhanced copy, print-PDF and local OCR remain unattempted.
Run separate finite original-PDF capability protocols before any private pilot.
If a complete-page method is blocked, publish a precise fallback issue; no
viewport screenshot or relaxed pixel tolerance satisfies #124's capture gate.
