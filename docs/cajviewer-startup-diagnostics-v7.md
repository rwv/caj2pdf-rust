<!-- SPDX-License-Identifier: MIT -->
# CAJViewer Linux startup diagnostics: seventh proposed profile

**Status: P1 CLOSED — PUBLIC OVERLAY PREPARATION PASS.
P2 AND STARTUP: DRAFT — EXECUTION REFUSED.** This archival document records
preparation for [#124](https://github.com/rwv/caj2pdf-rust/issues/124).
It does not authorize another build, runtime inventory, X11 query or
application launch. The
external startup draft deliberately uses a protocol value rejected by the existing
driver and has no approved new startup image, inventory or environment
bindings. P1's observed derived image and public environment identities
belong to that closed preparation phase, not a startup authorization.
P1's reviewed frozen document and plan were consumed by one authorized
public preflight and nine Docker clients. Both exact phase reviews and the
same-process actual pending-receipt/token review preceded Docker. Root and
the independent reviewer approved the closed result. The consumed frozen
document, plans, wrapper, context, token and receipts stay immutable.
Root and independent review must approve each later exact phase before it
runs. P2 and startup cannot reuse the consumed P1 approval.
The frozen original runtime/helper source basis is the verified merged main
commit `bac80ae65e3b423808ae5639be9d997c67847e25`, not the current main or
publication basis. Later publication preserves those exact consumed source
bytes and historical receipts.

The only proposed startup change is installing the original MIT terminal
helper diagnostics already reviewed in
[#133](https://github.com/rwv/caj2pdf-rust/issues/133) and
[PR #136](https://github.com/rwv/caj2pdf-rust/pull/136). No helper behavior,
search-error classification, vendor environment, numeric resource limit,
document, font or opaque vendor byte changes are proposed here. The merged
host driver already preserves a failed session's primary failure when no
diagnostic capture exists.

## Historical evidence stays immutable

All twelve reported launcher attempts remain counted. The first ten failed;
the sixth pair also failed, with one helper refusal and one owned-window
startup observation. The first detailed launcher count remains
`UNKNOWN_AFTER_START`; the cumulative count is the preserved supervisor
reported count, not a reconstructed descendant execution count.

The sixth pair's first search returned exit 1, zero stdout bytes and 254
stderr bytes. Its cause remains **UNKNOWN** because neither stderr content
nor its digest was retained. New diagnostics cannot recover that evidence.
The second session proved only an owned visible window and diagnostic
viewport integrity; it did not prove which document was displayed, a
complete physical page, ordinary copy or compatibility. Manual observation
of an original PDF in the earlier fifth viewport remains separately scoped
and does not turn a failed automatic observation into success.

| Preserved metadata | Bytes | SHA-256 |
| --- | ---: | --- |
| Sixth startup protocol | 18,429 | `07e6ac29470a36035a84b8d409a7ea7c983d042aad59ee2d98428f9985ee6057` |
| Sixth run receipt | 35,700 | `ba7761a2359d3f25730431c51aee15b767495c3a3931a4df63afe39cff0ce183` |
| Sixth safe summary | 3,108 | `630ac9225f548c3d8f056476d6a437d43f0a1e95f6ed13b1a35f620fb6620e74` |
| Sixth independent closing audit | 8,236 | `49f6979bd26dc476ac3db6592f4c9c0f0c22857e2395217971bb3090f3ed188a` |
| Previous opaque runtime inventory | 518,665 | `60c1a9e84ce8e566f60e8ef913f3b0f2e7247cf44da5fe0ed21c675d12d37372` |

Retain every earlier protocol, preparation failure, attempt, receipt and raw
external artifact. Later receipts append evidence; they cannot replace a
FAIL, fill an unobserved value or reclassify the unknown stderr. All paths
and full receipts remain external. This draft was derived from authorized
metadata and public source only, without opening or stating private inputs,
vendor implementation, PDF contents or raster payloads.

## Minimal original build-context amendment

The original Dockerfile and core process helper are byte-identical to the
sixth image's declared sources. Both Python files retain installed mode
0444 through the existing `COPY --chmod=0444` instructions.

| Original file | Bytes | SHA-256 | Proposed role |
| --- | ---: | --- | --- |
| `tools/cajviewer/Dockerfile` | 2,236 | `4221e4d90bd3a7c8cd5c6d84f0763a23ace6aca00284ab2def4a452ba5e606c6` | Unchanged build recipe |
| `scripts/cajviewer_canary.py` | 10,992 | `ba9abc3be6285cfb7d0af3840b88197fd5aa4ec22d7277ecee109ea0eae60c77` | Unchanged installed process helper |
| `tools/cajviewer/cajviewer_session.py` | 27,047 | `e674308b225fa301fe0fc8e6c35802a14613366c1538fe3f51d14bad24187bf3` | Reviewed diagnostics replace the 18,187-byte `b05b2ec1…` session |
| `tools/cajviewer/run.py` | 20,507 | `a87cd951dc9f8deefdd304a0287d6284710a1c98b934e9f1d4db4c254edf3ee1` | Reviewed host failure/capture handling; not installed in the image |

A fresh external overlay context contains only one original MIT Dockerfile
and the reviewed session helper. Its Dockerfile inherits a uniquely named
local alias, verified against the exact previous image, and copies that one
file with mode 0444. It has no `RUN`, package command, remote syntax directive,
vendor subtree, input PDF or application command. The repository's canonical
Dockerfile and historical build context remain unchanged. The official
Docker documentation defines [FROM image/tag references](https://docs.docker.com/reference/dockerfile/#from),
[local image tagging](https://docs.docker.com/reference/cli/docker/image/tag/)
and [COPY permissions](https://docs.docker.com/reference/dockerfile/#copy---chmod).
An image configuration ID is not assumed to be a registry digest suitable
for `FROM ...@sha256`; the alias is verified explicitly instead.

The official installer remains an **inherited prior verified pin**:
235,087,704 bytes, SHA-256
`3142c633d74dcf34ebaca9b7653f88ad3619f0b7a6cb689487b6cc583ec926d3`.
This draft has not rehashed it. Keep the AUR recipe commit
`04001d051c1f8bf7fc82c283b8b9bae4412ea1ed` and its packaging version `9.0-3`
separate from the still unobserved loaded/About build.

## Preparation phase: freeze before each new command

Preparation and application execution are separate phases. No application
authorization follows from a successful build or inventory. Review and
freeze exact public source, context, plan and known host executable pins
before public preparation. The proposed P1 wrapper then writes an immutable
same-process pending receipt containing actual parent/effective-child
environment and public code/context/history/tool identities. The bounded
actual parent environment stays only in the mode-0400 external receipt;
console output contains identities/counts, not its values. It waits at
most 600 seconds for a root-and-independent-review token bound to that
receipt, plan and PID. It rechecks those identities before the first Docker
client; no absent or stale token can start a tag or build. The shell's
canonical identity may be observed in this bounded public preflight because
no approved old shell pin is available; both reviewers must approve its
actual receipt pin before Docker. This is a declared review attestation,
not cryptographic signer authentication. DRAFT refuses before this preflight.

Reserve new immutable outputs and retain first failures. Record attempted
launches before creating each child,
successful spawns separately, argv, output size/digest/completeness,
elapsed time, cleanup and final audits. Never describe a retained TIMEOUT or
OUTPUT_LIMIT prefix as a complete stream.

### P1. Local alias and two-file overlay build, zero app launches

1. Freeze the original two-file context, plan, known Docker/Python identities,
   exact old image and a unique local alias. Review the actual shell and
   parent/effective-child environment in the same-process pending receipt
   before approving the first Docker client. Context
   membership must be exactly `Dockerfile` and `cajviewer_session.py`, both
   regular no-follow files, at most 64 KiB combined. The old image is
   `sha256:445831e2940c2e6eda320039680b8b3c507d9b9369cfcc6e07c573cbe53e8d7c`.
   Preserve old installer/runtime/context pins without copying their bytes.
2. Before assigning the alias, query the old image and prove that both the
   unique alias and result tag are absent. Never overwrite an existing alias
   or result tag, or remove a pre-existing tag. Make exactly one local
   `docker image tag` call, then query the alias
   and require its `Id` equals the old image. The inspected parent must have
   no `OnBuild`/healthcheck action and the inherited config must match the
   preserved image metadata. No registry pull is requested.
3. Build exactly once using the pinned overlay, `--pull=false --network=none
   --platform=linux/amd64 --progress=plain`, a fresh iid file and a fresh v9
   tag. Do not introduce `RUN`, remote frontend, apt, vendor copy or any
   alternate base/flag/environment/retry. Docker's build network setting
   applies to build instructions. The local alias and final layer/config
   checks require the declared local base; daemon registry networking is
   neither measured nor disabled by this flag.
4. Use the prior build limits: 180-second primary deadline; streamed build
   output at most 4 MiB, stderr at most 64 KiB, and 5-second helper reap.
   A declared `sh -c 'exec "$@" 2>&1'` wrapper joins build diagnostics into
   the streamed log; its hash describes that joined stream, not separately
   measured Docker stdout/stderr. No image entrypoint is executed.
5. After the build, query the new image, alias and old image. Require the
   alias and old image still identify the exact parent; require the new
   image's entire inherited `Config`, platform and prior rootfs layer prefix
   unchanged, with only the one original session-copy layer added. Newly
   created image ID/creation/history are bookkeeping, not permission for
   environment, rootfs membership or startup behavior changes. Each metadata
   query is bounded to 10 seconds, 64 KiB and 5-second reap.
6. Record at most nine Docker client attempts for P1: old-image inspection,
   alias-absence query, result-tag-absence query, tag, alias-after-tag query,
   one build, and new-image/alias-final/old-image-final queries. Preserve
   attempted name reservations and final identity even if build fails.
   Retain the uniquely owned alias for
   provenance; this phase does not delete aliases or historical images.
   Freeze actual image/config/size and complete source/context/tool/plan
   before/final audits. No app-zero preparation failure authorizes a retry.

The external P1 wrapper was executed **once in the closed public P1 phase**.
Its exact
36,967-byte source, SHA-256
`4e08fd2b4ffe1ee6bb8d47c5f4ad54cdc55e0878a1d4807a04c9be222fa39510`,
was statically reviewed by root and an independent reviewer. The two
separately frozen original-control runs below do not constitute a P1
environment probe, tag, build or image result. Its no-input branch declares
NOT_RUN/zero calls; its DRAFT gate and refused approval flags stop before
preflight. Both exact P1 plan/document reviews and the later actual pending
review/token were completed for this one consumed phase. It limits public requested
hash/read I/O to 512 MiB, metadata nesting to 24, context to 64 KiB,
retained output to 16 MiB, receipts to 256 KiB, and each child to the stated
deadline/stream/reap bounds. It records attempts before helper execution;
unavailable spawn outcomes stay unknown. The `alias_owned`/`result_tag_owned`
flags mean that reservation was attempted after an absence observation,
not that tag creation succeeded. An ambiguous failed API call leaves
creation UNKNOWN, never NOT_CREATED. Public preflight and the post-token
pin audit each have cooperative 60-second checks before and after bounded
reads; the post-token audit includes the exact plan bytes. The preserved
pending receipt is rehashed against the approved token before any Docker
client. Parent image metadata is a mandatory before/final history pin. Docker
scheduling has 360 seconds after token approval;
the complete measured run including token wait and report persistence has a
1,200-second final refusal. These are cooperative source guards, not a hard
OS watchdog or a daemon memory/disk guarantee. Kernel file operations cannot
be interrupted by these cooperative checks.

Closing source/history/context/tool and actual environment audits run even
on failure, with an independent 60-second file-audit deadline and the same
shared byte budget. Overall exhaustion still forces FAIL. Alias and parent
image inspections retain their independent
budgets, with no prune/delete. A receipt-size/storage refusal remains FAIL
with explicit ledger omission. Persist provisional receipt bytes first;
exclusive canonical publication requires the deadline check. Expiration
after publication independently removes or renames the canonical receipt
before attempting an incomplete marker. Mark publication as attempted
before the link operation, and invalidate only a regular canonical inode
proven identical to the retained provisional inode; a refused link cannot
authorize removal of an unrelated file. A marker-write failure cannot leave
an accepted canonical PASS; any inability to invalidate is explicitly
UNVERIFIED/non-consumable and overall FAIL. Root must review these exact
failure/cleanup boundaries before any phase. The existing `prepare.py` is
not rerun. No installer extraction, opaque vendor-tree copy or download
belongs to this overlay phase.

Finalize owned regular `build.log` and iid outputs with mode 0400, including
on failure when they exist. A Docker-client timeout/output-limit kills and
reaps the controlled client group; shared-daemon build completion and
cancellation remain UNKNOWN unless successful image lineage was verified.
No daemon resource or cancellation guarantee follows from client cleanup.

### Public P1 controls and freeze contract

Root invoked exactly two separately frozen original-control phases. Each
ran eight original cases in one Python runner, with zero real child
launches, Docker calls, environment/tool probes, application launches,
private-input observations and vendor passes. Each recorded nine synthetic
helper events; those eighteen events across both invocations are invented
control events, not actual child executions. Author and reviewer preparation
did not run the operational P1 wrapper.

| Preserved original-control metadata | Outcome | Bytes | SHA-256 |
| --- | --- | ---: | --- |
| First frozen control plan, v2 | One original runner | 4,079 | `1c075bd11045a398951828bb4ff1e3acdfe8d663cd443bd92eaa4da6a5cf737f` |
| First sealed control report | FAIL: 7 of 8 cases passed | 1,335 | `c56efb0c121e62c5e74df01eeb506201d0af42b368b3a1e8961938bbd5bd23de` |
| Second frozen control plan, v3 | One separately reviewed original runner | 6,074 | `dc7e10a1d3dbd9f1f4235d761e483f155ec9db4aa1e63aa362c5522e3008b27b` |
| Second sealed control report | PASS: all 8 cases passed | 1,296 | `a522144fcb6b9c2dfd340004c0fc4e39c567a7f70e6f5fa33c5fe454ab14729a` |

The first report remains FAIL. Its changed-pending-receipt case failed with
`AssertionError`; the retained invented P1 receipt recorded `KeyError` and
an unclassified public-preparation failure before any Docker call. The
dynamic missing-key cause is **UNKNOWN**. Static review found that the
invented plan omitted the wrapper declaration needed to construct the
pending receipt. Only that original fixture declaration and its explicit
null parent `Healthcheck` were added before the second run. The P1 wrapper,
context, source declarations, argv and numeric limits were unchanged. The
second PASS does not rewrite or reclassify the first failure. First-failure
fixtures stay retained; successful second-run fixtures were removed.

The current original-control source is 29,516 bytes, SHA-256
`7cdd8277eb3f466ed2df7609b5ac2ed1513ec5c077fe9e43183075e65a5ead75`.
It is a consumed provenance declaration for those completed controls, not a
P1 import or input hash: P1 neither loads nor audits that Python file. The
second frozen plan binds those exact consumed bytes. Both actual control
reports and both frozen control plans are mandatory historical JSON pins
in the new P1 plan, each appearing once in its history audit list. Together
with the sixteen previously declared historical inputs, all twenty are
required before Docker and in the final shared-budget file audit.

The previous exact DRAFT document, false-approval review candidate and final
attested P1 plan remain immutable external artifacts. The consumed final
plan bound the 28,362-byte frozen document, not this later archival update;
that exact snapshot is preserved separately. The dependency remains acyclic.
After both phase reviews, one authorized public preflight captured actual
parent/effective environment, shell and file identities in the immutable
mode-0400 pending receipt. Both reviewers then inspected those actual
identities, and root supplied a token bound to that active PID, exact plan
and pending receipt. Pending/plan reaudit preceded Docker. No P2 inventory
or startup attempt was authorized by either review step.

### Closed P1 result

The freeze document was committed before execution at
`4453eb4ed8d4574807063f354de51a3fcd5c888a`. Root invoked the reviewed wrapper
once. All nine ordered Docker clients completed successfully; application,
runtime-inventory, X11, converter, native and vendor-compatibility counts
were zero. Both root and the independent reviewer approved the closed
metadata result. No new launcher slot was consumed: the twelve historical
launcher attempts and all FAIL/UNKNOWN outcomes remain unchanged.

| Consumed or closed P1 metadata | Bytes | SHA-256 |
| --- | ---: | --- |
| Consumed frozen document snapshot | 28,362 | `87210e1d648f2bc58d3386b4557d18f736ca80dbca1f61c73d7a471cddac14ec` |
| Final attested plan | 19,057 | `7bdca1dcf2db7d70ce24c07e53a8f0bb5b8652d4188fb2e59207928ee07fa2af` |
| Actual pending preflight receipt | 10,729 | `f9226fec56a029a3a1fef7b6a8265b2baa8cc55402dff8e4a26e3c3ccaa119ad` |
| Exact approval token | 585 | `9b465613332507138af2f14841930245b440b98523eceb95c5ca82dfb83d8c2c` |
| Closed preparation receipt | 39,995 | `97f3d164a1985b30a5cb091ee686e25edd6d54f61a7c5930745a3460c6626674` |
| Root closing review | 1,785 | `1f92752427cdf98db7becab67095918238db3062337551f2c583d138fedcb1c7` |

The external root review was written before independent closing review;
its preserved `independent_review: PENDING` field is unchanged. The later
independent PASS message verified the exact closed receipt, all nine
clients and all 108 file-audit rows. That later review is not retroactively
inserted into the original root JSON.

The new **local image configuration ID** is
`sha256:cb5049d3448b6d5bcfd371cf195d522d637869dce075cb1650d74198875171de`.
Its entire inherited `Config` and platform match the parent. Its rootfs
contains the exact six old DiffIDs plus one new original COPY DiffID,
`sha256:4ac4b68364c537fb7d166404327ef7ece509ca88a5a3b129317c504e9661196a`.
Logical image size is 1,424,957,414 bytes, an increase of 27,047 bytes.
This does not measure shared-daemon physical allocation or memory. The
expected complete installed-file/runtime equality is still P2's unexecuted
acceptance check; image lineage alone does not count as that inventory.

All 108 exact public file-audit rows passed: 36 declared identities in each
of the preflight, post-token and final audits. Their shared reader recorded
130,207,498 requested bytes, 116,322,096 returned bytes and 1,991 calls.
The receipt's measured total was 200.631656 seconds, including the approval
wait. The build child took 1.219956 seconds and produced 883 bytes in its
joined diagnostic stream. Owned outputs were mode 0400; canonical and
provisional receipts retained the same inode. No container phase occurred.
Shared-daemon memory and physical disk remain unavailable.

### P2. Later separately frozen opaque inventory, zero app launches

1. P1 performs no inventory. After P1 closes, separately review and freeze
   the actual image/config and a new inventory plan.
   Pin unchanged `inventory.py`, its mount/read permissions, Docker/Python
   identities and the exact child environment. Use a fresh inventory
   container name and register cleanup only after proving it absent.
2. Override the entrypoint with the unchanged original Python inventory
   helper. Keep `--pull=never`, `--network=none`, read-only root, UID/GID
   1000, dropped capabilities/no-new-privileges, memory and memory+swap
   512 MiB, two CPUs, PID limit 64 and `/tmp` tmpfs 8 MiB. No X server,
   viewer entrypoint, source/PDF input mount or application launcher runs.
3. Limit one inventory start/attach to 90 seconds and 4 MiB JSON, 64 KiB
   stderr and 5-second reap. The helper caps inventory membership at 8,192
   and regular runtime bytes at 4 GiB, with 64-KiB opaque hash reads, eight
   tool identity hashes and two public package/font commands at 10 seconds
   and 256 KiB each. Record these two nested child attempts separately from
   Docker clients.
4. Bound exact-name container audit/create/start-attach/remove/final-absence
   clients to five, with primary deadlines 5/10/90/15/5 seconds and each
   independent 5-second reap. Cleanup runs on failed create/start, timeout
   and audit failure; a pre-existing container is never removed.
5. Compare complete sorted old/new inventory declarations. Require the
   same 2,731 members, all 832 opaque vendor entries, package/font/tool
   records, directory/symlink modes/targets and unchanged core helper.
   The sole permitted installed-file byte delta is the reviewed session
   at `/opt/canary/cajviewer_session.py`, still mode 0444. Reject any other
   change or added/missing member; do not permit a partial comparison.
6. Freeze exact inventory bytes/hash, its full receipt, comparison and
   cleanup/final audits. They are not inferred from a successful Docker
   command, the previous inventory, or the code's intended change.

Preparation disk use must be separately monitored across the tiny context,
new layer and logs. The previous image occupied 1,424,930,367 logical bytes;
image size is not daemon physical disk usage or a prediction for the new
image. Reusing the parent avoids the proposed 951,465,388-byte opaque vendor
copy declared by prior metadata. The exact daemon incremental storage method,
new-layer/storage interpretation and public process tree measurement/caps
remain review inputs. The P1 wrapper can refuse an unexpected new logical
image size but cannot infer daemon physical allocation, enforce shared-daemon
RSS, or subtract shared layers/cache. Do not claim old measurements are current.

## Proposed startup phase: at most two additional attempts

Only after successful reviewed P1/P2 receipts, actual runtime/host/environment
pins and an independently reviewed committed FROZEN protocol may root
authorize a single startup invocation. The proposed new budget is **at most
two** official launcher attempts, one per fresh session. The cumulative
reported maximum is **fourteen**, including all twelve old attempts.
An unknown-after-start result consumes its slot. No third launch, alternate
profile, extra document, automatic retry or use of an old protocol is allowed.
If the new phase does not run, its actual counts remain zero/NOT_RUN.

The final protocol will bind all seven public source files, both relevant
original test files, all five original control-file pins, official installer
and opaque context/runtime provenance, actual image/config/inventory/host
tool/environment identities and every historical metadata input used for
this amendment. Actual clean child settings are separate from the parent's
environment; inherited v6 host values are expectations, not a fresh audit.
Root's supplemental before/final audit must cover the exact external
historical/preparation/tool identities; `run.py` itself checks only its
seven source files, five controls, protocol and inventory.

Two predeclared new container names and one fresh external output directory
are bound before execution. Each launcher uses only the unchanged official
desktop argv `/opt/cajviewer/bin/start.sh /input/digital.pdf`. The five
original controls remain read-only; only `digital.pdf` is passed to the
launcher. No UI activation, dialog acceptance, page navigation, extra open,
selection, clipboard operation, copy, OCR or print/export action is proposed.

| Startup boundary | Unchanged requirement |
| --- | --- |
| Isolation | `linux/amd64`, `--pull=never`, offline, read-only root/inputs, non-root 1000:1000, private cgroup, init, all caps dropped/no-new-privileges; no host display/home/socket |
| Whole cgroup | 1,536 MiB memory and memory+swap, 2 CPUs, 256 concurrent tasks/threads, 64 MiB shared memory; OOM delta must be zero |
| Writable mounts | Home 64 MiB, `/tmp` 64 MiB, runtime 8 MiB, output 32 MiB |
| File size | Supervisor/query/window manager soft 1 MiB, hard 64 MiB; application and Xvfb soft/hard 64 MiB; capture soft 6 MiB; core dumps zero |
| Image environment | The reviewed image's `QTWEBENGINE_DISABLE_SANDBOX=1` stays explicit; outer isolation remains. Loaded Qt, vendor consumption, DPR/backend stay UNKNOWN until separately observed |
| Display | Dedicated Xvfb `:99`, 1600 × 1200 × 24, requested 96 DPI; measure display/masks/stride, never infer physical document DPI |
| Observation | Readiness 10 s; owned-window observation shares 30 s; query primary at most 2 s plus 5 s reap; depth 2, at most 16 visible candidates and 17th-result overflow refusal, query output 4 KiB |
| Helpers | At most 400 controlled query attempts plus one Xvfb, one openbox and one official launcher, hence at most 403 action entries; snapshots are not a complete cumulative descendant-launch count |
| Host scheduling | 360 s primary scheduling deadline; collection readiness 60 s/60 polls; at most 80 primary Docker calls per session plus three independent closing slots; image inspection is separately counted |
| Collection | Seven eligible flat diagnostic filenames; tar stream at most 40 MiB; at most 32 archive members/32 MiB content/6 MiB per file; session JSON at most 256 KiB |
| Cleanup | Kill/reap owned process groups; collect logs 5 s, force-remove 15 s, exact-name absence audit 5 s, helper reap 5 s; closing budgets survive scheduling failure |

The frozen scheduling gate counts the root driver invocation separately,
all actual Docker/helper attempted calls, successful spawns and incomplete
attempts. No declared ledger is confused with process-tree PID peaks or
Rust RSS. Before/final source/control/protocol/inventory and supplemental
frozen-input audits remain mandatory, including on failure.

## Diagnostic acceptance and refusal

An owned visible window requires `_NET_WM_PID` and Linux process-group
membership matching the launcher before and after title/geometry queries.
Missing, foreign, changed or disappeared owners are not a startup proof.
The first eligible window may be a dialog: `document_identity: UNVERIFIED`
and `scope: startup-owned-visible-window-only` stay mandatory.

An stderr-bearing search exit 1, unexpected exit, malformed/oversized output,
timeout, output limit, unknown helper fault, incomplete receipt, OOM event,
capture failure, cleanup failure or final audit mismatch remains FAIL. The
existing narrow display-readiness/missing-owner outcomes stay unchanged; no
new transient X11 class is added.

Retain the fixed terminal stage/reason/action index and helper status/exit,
attempted read counts, retained stdout/stderr byte counts/SHA-256 and
completion/truncation tags. Only terminal public-helper stderr up to 4 KiB
is embedded in the external session receipt. TIMEOUT/OUTPUT_LIMIT hashes
are captured-prefix hashes, even when a legacy truncation flag is false.
No arbitrary exception/traceback or helper stdout is embedded in that
diagnostic. Raw app/helper logs, window observations and raster bytes stay
external. Classify a new failure only from its retained exact receipt after
the phase closes; unknown or unobserved bytes stay unknown.

The complete compact session receipt must fit 256 KiB. Otherwise retain the
explicit bounded FAIL refusal with ledger-omission metadata and primary
failure/cleanup/OOM/final audit fields; never raise the limit, silently
truncate, claim a complete ledger or satisfy startup collection. If no
capture exists, host integrity is NOT_RUN and the primary failure survives.

Only two collected STARTUP_OBSERVED results, each with viewport integrity,
zero OOM delta, successful cleanup and all closing audits, satisfy this
startup-only phase. Any required failure keeps the pair FAIL. Stop scheduling
the second session after cleanup failure or scheduling expiry. Preserve
every attempted/unknown slot even if no session JSON was collected.

## Remaining #124 criteria and freeze inputs

- **AC1 remains partial:** official bytes and package/runtime pins were
  observed previously; installed/About/loaded build and vendor Qt/DPR/backend
  remain unverified. The applicable custom license and external-only image
  policy are unchanged.
- **AC2 remains unmet:** this proposed phase could show reproducible owned
  window startup/cleanup, but not two independently verified document
  sessions. A dialog cannot satisfy document opening.
- **AC3–AC6 remain unmet:** complete physical pages, navigation, ordinary
  copy freshness, born-digital/image-only behavior, enhanced copy, export,
  print, local OCR, physical page extents and image/text repeatability need
  separately frozen original-control capability phases. Startup diagnostic
  repeats contribute zero image/text compatibility comparisons.
- **AC7 is complete:** merged mandatory public diagnostics/fault/process
  controls are reviewed. The separately frozen original P1 controls above
  retain their first FAIL and second PASS; they contribute zero vendor or
  compatibility comparisons. The proposed operational P1 phase runs no tests.
- **AC8 remains partial:** English preparation/limits/provenance are present;
  an actual capability report awaits observations. #126/#127 remain blocked.

The P1 public overlay plan is consumed and closed. Its authorization applied
only to one public preflight and nine Docker clients; it cannot authorize
another build, inventory or application. P2 receives its own later freeze,
using the closed P1 identities while freshly binding its actual execution
environment and public source/tool inputs.

Before changing the **startup protocol** from DRAFT to FROZEN, record
actual complete P1/P2 receipts,
new image/config/inventory/comparison identities, exact observed host/effective
environment and public tools/libraries, fresh output/name reservation,
bounded execution/closing ledgers and source/doc/external-plan hashes.
Resolve and review preparation resource enforcement and supplemental audit
ownership. Then commit the exact frozen protocol and obtain explicit root
authorization. No native/Rust/CLI/JavaScript API or production format support
change is proposed here; all new source and protocol text is original MIT.
