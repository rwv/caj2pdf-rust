<!-- SPDX-License-Identifier: MIT -->

# Original PDF controls and Linux startup canary

This is work toward [#124](https://github.com/rwv/caj2pdf-rust/issues/124).
The first startup pair lost its tmpfs diagnostics. The transport amendment
collected two loader failures with missing `libxslt.so.1`. After adding the
measured provider, two more attempts exited 153 with a file-size-limit
diagnostic; the affected file/child is unknown. A separately reviewed
application-limit pair then lost its Xvfb display to signal 25 and reported
an additional QtWebEngine sandbox error. An original public XCB shared-memory
control isolates the Xvfb limit failure without launching the viewer.
The fifth pair kept its display and launcher alive but failed an unverified
filename-based window predicate. Its preserved, identical viewport captures
show the original four-page PDF in the viewer. That manual opening observation
does not satisfy complete-page capture or text acquisition.
**Full-page and text compatibility remain NOT_RUN, zero passes.** The
capability issue remains open; all ten reported launcher attempts and
failures are retained.
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
| Current file-limit investigation | Supervisor/query/window manager soft 1 MiB, supervisor hard 64 MiB; child-only application and Xvfb soft/hard 64 MiB; capture soft 6 MiB. Core dumps disabled. The Xvfb amendment requires a separately frozen startup phase; the fourth pair still gave Xvfb soft 1 MiB. |
| Current QtWebEngine investigation | The fifth image sets documented `QTWEBENGINE_DISABLE_SANDBOX=1`; both sessions record the supervisor's observed value. Outer Docker isolation stays unchanged. Vendor consumption and the loaded Qt version remain unknown. |
| Display | Dedicated Xvfb `:99`, 1600 × 1200, depth 24, requested 96 DPI; X11 RGB masks/stride are measured. Vendor DPR/Qt/backend stay UNKNOWN. |
| Persistent helpers | One Xvfb and one openbox attempt; at most 400 controlled metadata helper attempts. |
| Stage time | X11 readiness 10 s; owned-window observation shares one 30 s deadline; helper primary deadlines at most 2 s, plus bounded reap allowances. Query sleeps do not prove readiness. |
| Host time / calls | Collection readiness 60 s / 60 polls; primary Docker calls ≤80, with three independent closing slots. Global scheduling deadline 360 s plus closing allowances. |
| Artifact extraction | Original in-container Python tar stream at most 40 MiB; exactly seven eligible flat diagnostic filenames, ≤32 archive members, 32 MiB aggregate content, ≤6 MiB per file; unknown names, symlinks/special/traversal entries fail. |

The prepared session enumerates at most 16 visible X11 window candidates
within the declared depth-2 search, using a seventeenth result as an overflow
refusal. It
checks each window's `_NET_WM_PID` and Linux process group against the launched
application before and after measuring its title and geometry. Missing,
foreign, disappeared or changed owners are refused. All queries share the
original deadline and bounded helper accounting. No filename or localized
title content is assumed. The first owned window can be a dialog; the receipt
marks `document_identity: UNVERIFIED` and
`scope: startup-owned-visible-window-only`. This source amendment has passed
original controls but requires its own frozen image/protocol before app use.
The diagnostic whole-screen P6
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
cleanup limits remain unchanged. Original process tests pass. A fresh public
tmpfs-only canary copied and verified all seven original stand-in files,
including an exact 1600 × 1200 RGB array, in 0.446 seconds. Its final source
audit and container cleanup passed; app launches and vendor passes were zero.
The first failure remains immutable; no app flag, profile, image or
rendering-backend retry was part of that transport amendment.

## Transport amendment result and runtime preparation

The second reviewed protocol used source commit
`ed6f9d59f42ba10dbb4743c48b4341c557bdb3fb`, protocol SHA-256
`926e3a4ef080858c31a4117951dc67607f9e4e811d49bfd13f456b31b6ba1f90`
(10,252 bytes), and the same prepared image. Both sessions report one launcher
attempt, exit 127, no matching window, a 30-second title deadline and the same
unavailable `libxslt.so.1` loader diagnostic. This establishes initial loading
failure in that exact extraction profile; document opening and later feature
capabilities remain unverified.

The closed 61,995-byte run receipt has SHA-256
`20a52f3134cbde5972760e09c43c5c88c3de1b99d02a91f5d5126809b12fe89d`.
Each attempt has 38 Docker clients and 293 controlled query helpers; the outer
phase has 77 Docker clients including image preflight. Both whole-screen P6
artifacts are 5,760,017 bytes with exact full-payload/EOF integrity, encoded
SHA-256 `e97e86645d70c039981b1e17f36773fb30d16373f91a0467d7006b51f7bae78f`
and pixel SHA-256 `c0e5fc1ce8c727d3e75fa229cdb40a4f971cf6a8dea9ba552ec8f3d3b81d8082`.
They do not contain an observed document window and establish no page parity.
Whole-cgroup memory peaks are 55,889,920 and 55,291,904 bytes, concurrent task
peaks are 12, and OOM-kill deltas are zero. These include the supervisor/display/
window manager/query tree; they are not Rust or loaded application RSS.
Both groups/container cleanups and all final declared file audits passed.
Independent review checked the closed records, historical pins and complete
viewport payload integrity; vendor/image/text comparison passes remain zero.

The pinned [AUR static metadata](https://github.com/archlinux/aur/blob/04001d051c1f8bf7fc82c283b8b9bae4412ea1ed/.SRCINFO)
declares glibc, gcc-libs, bash, hicolor-icon-theme and libxml2-legacy. It does
not list libxslt and is not a complete dynamic-loader contract. Debian
[libxslt1.1 supplies the named library](https://packages.debian.org/bookworm/amd64/libxslt1.1/filelist).
Public signed snapshot preparation resolved `libxslt1.1=1.1.35-1+deb12u1`.
The recipe adds that measured missing-library provider and the separately
declared icon dependency `hicolor-icon-theme=0.17-2`.
The public build and opaque inventory succeeded with zero app launches. The
new image is
`sha256:cd6c06786977df1dbb1556309b43e713349eed52ced1e702b7b027cacf1f1c21`
(1,424,924,931 bytes). Its 518,665-byte runtime inventory has SHA-256
`527427bd2538308053a10a1486582503fe9555330e805b652bf73b73596762e4`.
Independent review verified that these are the only added installed packages;
all 832 opaque vendor entries, original installed modules and public tool
identities remained unchanged.

## Preserved file-size-limit failure and original process controls

The third startup protocol froze source
`50880751c75e6e3c7a0cedc7e2e24b97f572f228`, the new image and inventory,
12,758 protocol bytes with SHA-256
`1f2a85d784467635d38da8da784c0a39754072a7c1b5357751777164b022bcc0`,
and exactly two fresh names. Its explicit cumulative reported-attempt ceiling
was six; no UI, document, backend or other environment action was added.
An app-zero protocol-preparation failure is also retained: no-follow host
hashing correctly refused the Python symlink. The amendment recorded its
resolved binary path while verifying the previous binary bytes unchanged.

Both app attempts exited 153 with a 25-byte `File size limit exceeded` log,
SHA-256 `912456ad93f80e531567561305bf88e56c769ddeb2dbcde00410659837121374`.
This is consistent with Linux SIGXFSZ; the diagnostic does not identify its
target file or child. No matching window was observed. Both 30-second title
queries used 293 helpers and each host attempt used 38 Docker clients. The
61,995-byte run receipt has SHA-256
`789f80e8900ab338bda70eb269ae9bdf9b8f9db5d6acdb9ba1048fbb03b6fd37`.
Whole-cgroup memory peaks were 146,976,768 and 65,839,104 bytes, with zero
OOM-kill deltas. All group/container cleanups, final declared source audits and
11 additional historical metadata pins passed independent verification.
Both viewport payloads still matched the previous diagnostic desktop hash;
this proves artifact integrity only, with no document-page or text result.

The inherited application soft file-size limit was 1 MiB. The application
amendment prepared a 64 MiB parent hard allowance and set application
soft/hard 64 MiB in its single-threaded fork child before exec. The official
launcher argv stayed unchanged. Supervisor/query/window-manager/Xvfb children
retained soft 1 MiB and the capture retained soft 6 MiB. Core dumps were
disabled; no host/system ulimit was changed.
Before-launch supervisor limits and bounded `/proc` process-limit snapshots
are recorded separately from the declared policy. A file can still exhaust
its original bounded tmpfs. Application logging now has the application
ceiling: an oversized log still fails the unchanged 6 MiB-per-file/32 MiB
aggregate diagnostic collection. Do not claim its log remains capped at 1 MiB.

Three original Python child controls demonstrate the previous 1 MiB refusal,
successful bounded writing beyond 1 MiB under the child allowance, and refusal
past 64 MiB without allocating a 65 MiB fixture. Parent limits remain unchanged
and the amendment's mock integration verified only the application received
the pre-exec action. These controls do not prove that CAJViewer can initialize.

## Preserved display failure and public shared-memory control

The fourth startup protocol froze source
`7506f5b4ec2bc97ad7bf64946003a012f65849ee`, 13,278 protocol bytes with SHA-256
`140b3f1f665e7b5fbbf4bc1d5a0e5990bcd09994addfd81b239d216cb4714d4a`,
and prepared image
`sha256:0b1287f4b0e720cf21295bacf5a9009320348fc9443cfafa00f7a4cb18c4bd35`.
Its 518,665-byte runtime inventory has SHA-256
`f3fdaeabb8fdeebc1584b3b95e3687e1e670131d0d340c4288cfa5539ba3f07b`.
Only the original installed session module changed from the third image;
vendor files, packages, fonts, libraries and tools stayed unchanged. The
reviewed cumulative reported-attempt ceiling was eight.

Both attempts failed: Xvfb logged `ftruncate`, caught signal 25 (file-size
limit exceeded), and aborted with signal 6. No matching document window or
diagnostic P6 was produced. Session capture failed after the display died;
host validation also retained the missing-capture failure. No raster-integrity
pass is claimed. The app logs separately reported `No usable sandbox!` from
QtWebEngine. Its startup requirements and the loaded Qt version remain
unverified; the driver did not disable that sandbox or alter the host kernel.
Do not assign this fourth-phase diagnosis to the third phase's unknown child.

The closed 61,473-byte run receipt has SHA-256
`374591ebe4560ae3a7dbca005fc91b6e4c44a64ad6d768999c9ef3c3f6b74f15`.
There were 296/297 controlled metadata helper attempts and 38 Docker clients
per session. Whole-cgroup memory peaks were 296,407,040 and 208,154,624 bytes,
concurrent task peaks 111, and OOM-kill deltas zero. All process-group/container
cleanups and final declared file audits passed. Historical preparation
metadata and the exact committed test source also passed their closing
audit; later tests do not replace that immutable historical identity.

A separately frozen original public control used the public XCB MIT-SHM ABI,
the exact fourth image, cleared loader overrides and absolute hash-pinned
Debian `libxcb`/`libxcb-shm` libraries. It requested one 2 MiB shared-memory
segment per fresh Xvfb session. Under soft 1 MiB/hard 64 MiB, the request
failed and Xvfb again logged `ftruncate` and caught signal 25 before aborting.
Under soft/hard 64 MiB, the request returned a descriptor of exactly
2,097,152 bytes and Xvfb remained alive. Both worker/display/container
cleanups and final source/inventory audits passed. Eleven Docker clients
were recorded; an exact inner readiness-helper count is not available.
Application launches and vendor passes were zero.

This 4,126-byte public-control plan has SHA-256
`de51817cae755103f841cb9a30dc09bbc974f5962fee7a05acc076e43226b982`;
its 7,985-byte closed receipt has SHA-256
`b567fac19754312a1596ab9770b6a6c195ed0be5a1177dc98b54147e38fb4be0`.
The original external probe source is 7,330 bytes, SHA-256
`e5d6a729c80f4a2a78449f23d28dc2b08f6ee2c86d1a0b9ca2965209dcd48103`.
It contains no vendor library loading or implementation inspection. This
isolates the public display limit; it establishes no viewer capability.

The current source reuses the same bounded child action for Xvfb and the
application. The window manager and query children retain soft 1 MiB, the
supervisor starts at soft 1 MiB/hard 64 MiB, and capture retains soft 6 MiB.
Core, mount, memory, swap, PID, process, collection and cleanup caps remain
unchanged. Both Xvfb and application logs can exceed 6 MiB under their child
allowance; the unchanged collector then fails instead of truncating them.
Original writer controls and mock integration verify the narrow dispatch.
The fifth experimental image sets `QTWEBENGINE_DISABLE_SANDBOX=1`,
as described by the [Qt 5.15 platform documentation](https://github.com/qt/qtwebengine/blob/v5.15.2/src/webengine/doc/src/qtwebengine-platform-notes.qdoc).
This intentionally disables QtWebEngine's inner sandbox for the original-PDF
canary. It does not establish that this vendor build honors the setting.
The offline/non-root/read-only/container capability, seccomp, memory/swap,
shared-memory and storage restrictions remain in place. No host user-namespace
setting, seccomp override, viewer flag or rendering-backend change is made.
The session records its supervisor environment value, not whether the opaque
launcher preserves it or the vendor runtime consumes it, while keeping the
loaded Qt version unknown. Original mock integration checks that record and
the unchanged official launcher argv. Treat these two changes as one explicit
diagnostic profile; no single-variable viewer-causality claim follows.
The fifth phase used these two changes as one explicit diagnostic profile.
Preserve every earlier phase and its original ceiling; do not retry
automatically.

## Preserved fifth failure and original-PDF opening observation

The fifth protocol froze source
`b2eab06d876a2468e8396f378df6e106eb0e4196`, 15,012 protocol bytes with SHA-256
`7864462fd77a4052f6b6e901ec898c505758f950008e2b854452062827e8cddd`,
and image
`sha256:f3cec80cfef700d525fe141d8e02dd963fc2deb1406dbbaf516a7683c6563083`.
Its 518,665-byte runtime inventory has SHA-256
`ad500f678d227e8dffe2df43404937ebe443be2f357de41a3730df1dcd7b1c46`.
Only the original installed session changed from the previous inventory;
vendor files, packages, fonts, libraries and tools stayed unchanged. The
reviewed cumulative reported-attempt ceiling was ten and is now exhausted.

Both sessions retained **FAIL: matching-window-deadline**. The launcher stayed
alive through the 30-second predicate deadline. The predicate had assumed a
visible window name containing `digital.pdf`; that title convention was never
verified. The preserved UI has a `digital` tab and an `/input/digital.pdf`
status path. A screenshot does not establish which X11 title property existed,
so the revised observer records the actual title instead of inventing one.
Both supervisor receipts observe `QTWEBENGINE_DISABLE_SANDBOX=1`. Their
bounded logs do not contain the previously observed loader, file-limit,
unusable-sandbox or OpenGL error classes. These observations do not establish
the loaded Qt build or the opaque launcher's handling of the environment.

The closed 62,090-byte run receipt has SHA-256
`8c066c7d88a04b3bc149e33c99592383fe1aa0f3434376af326697a815b48167`.
There were 282 controlled metadata helpers and 38 Docker clients per session.
Whole-cgroup memory peaks were 336,359,424 and 313,155,584 bytes, concurrent
task peaks 122/120, and OOM-kill deltas zero. All declared cleanup, source,
control, protocol, inventory and additional historical-metadata closing
audits passed. These are external startup measurements, not Rust performance.

Both captures have an exact P6 header for 1600 × 1200 RGB, all 5,760,000 pixel
bytes, and no trailing bytes. Each encoded file is 5,760,017 bytes with SHA-256
`07cc94f8c18980addb3cca9012712c458d2924bdd66cfdd4ff87b29313eea9b2`;
each full pixel payload has SHA-256
`827aa6bbc25137ef52af689029f0643254dd535b6d2a29867b43a0497e10c094`.
Root and independent review checked both complete payloads. The exact repeat
is diagnostic viewport repeatability only: `complete_page: false` remains.

An external lossless P6-to-PNG format conversion preserves that pixel hash.
Its 46,771-byte preview has SHA-256
`0f7ee4ddaa2de678a953d04dac72d16fbb2107a31cb99c2bce44c6e32efaffad`.
Manual review by root and the independent reviewer sees the original four
PDF pages in continuous layout, the known blank page, rotation, large page
and colored controls, with an application page indicator of 1/4. This
establishes an original-PDF opening observation only. The UI's zoom label
does not measure capture DPI, page navigation, DPR or complete page bounds.
No selection, copy, page export, print, OCR or private document was attempted.

The corrected external summary is 4,387 bytes, SHA-256
`0acf7dff83869e7a51bd8d4afcdcb3c1ed992cb666191d86aaf5aa19e4125dc6`.
It explicitly supersedes an incorrect postprocessing summary that used the
wrong log filename/keys; both summaries and all original receipts are retained.
No original outcome or artifact was replaced.

## Prepared owned-window observation amendment

The independent source review and 33 original focused tests verify the
owner-based observer, including two real original process groups. Controls
cover foreign/missing/malformed PIDs, disappearing or inaccessible owners,
changed PID/group after measurement, candidate overflow, malformed title and
geometry, helper failures/output limits, and the single shared deadline.
The public
[xdotool 3.20160805.1 manual](https://github.com/jordansissel/xdotool/blob/v3.20160805.1/xdotool.pod)
is a command usage reference only; no external implementation is copied.
`--maxdepth 2` is the declared decorated-window profile, not a claim about all
possible application window trees. An owned dialog cannot prove document
identity or rendering readiness.

Build and inventory this source without launching the app, then freeze exact
source/image/runtime/control/environment pins and a new finite cumulative
budget for independent review before further activity. The previous ten
reported FAIL attempts remain immutable. Full-page and standard-copy probes
require separate original-control protocols even if this startup observer
succeeds.

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
