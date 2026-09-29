<!-- SPDX-License-Identifier: MIT -->

# Bounded inventory-helper diagnostics

## Scope and status

This is the original MIT source and control contract for
[#148](https://github.com/rwv/caj2pdf-rust/issues/148), completed in
[PR #150](https://github.com/rwv/caj2pdf-rust/pull/150), and the completed
[#151](https://github.com/rwv/caj2pdf-rust/issues/151) fixed-cache source child
([PR #152](https://github.com/rwv/caj2pdf-rust/pull/152))
and blocker of [capability issue #124](https://github.com/rwv/caj2pdf-rust/issues/124).
The prerequisites #125, #133 and #143 are complete. The capability-protocol
source in #146 is a separate prerequisite; this child does not depend on it.

The implementation lives in [run.py](../tools/cajviewer/run.py), with mandatory
original controls in
[test_cajviewer_inventory_diagnostics.py](../tests/conformance/test_cajviewer_inventory_diagnostics.py).
It retains a bounded diagnostic when an inventory metadata helper fails.
The #148 diagnostic slice supplies no runtime profile and performs no image
build, module installation, cache-policy change, viewer launch or vendor comparison.

Parent #124's later [V14 inventory](cajviewer-runtime-view-v14.md) completed
successfully with both actual closing reviews. The separate metadata consumer
passed one actual call and both closing reviews, and GUI capability remains
unproved. The historical failures below are preserved; only parent AC7 is complete.
The new source-loading child [#153](https://github.com/rwv/caj2pdf-rust/issues/153)
covers capability execution origin before separate fresh delivery/load checks
and the GUI profile; the upstream inventory source checks cannot fill that role.

The sole historical V12 inventory remains `CLOSED_FAIL`. Both original modules
loaded; `dpkg-query` passed, and `fc-list` returned helper status PASS and typed
exit 0 with 48 complete stderr bytes. Required empty-stderr validation then
failed with `ValueError`. The actual stderr message and cause are unavailable.
This implementation cannot recover them. The inventory did not complete;
all 2,731 comparisons remain `NOT_RUN`, with zero application/vendor actions
in that phase. Successful closing reviews preserve that failure.

The fresh V13 phase subsequently retained a complete 48-byte stderr diagnostic
reporting that Fontconfig has no writable cache directory. Raw bytes remain
external; SHA-256 is
`e0a46db3e7d086b6ed55477f9c8cc5ec4dbe4b6ef804e1372e9a4a6dc5a6b01a`.
`fc-list` returned helper PASS and integer exit 0, but strict
`HELPER_STDERR_NOT_EMPTY` validation kept the operation FAIL. The receipt is
129,972 bytes, SHA-256
`eb38465d59de93b45ca114768b425357ccb4fced4250b4f0e317f3c7aa26bbf3`.
Five Docker clients, one admitted but incomplete inventory and zero app/vendor
actions leave all 2,731 comparisons NOT_RUN. Both closing reviews passed:
381 public audits, four dynamic/environment checks, three output audits,
owned-ID removal, final absence and caps/ENV/UID checks. The specific filesystem
or configuration details are not inferred from the diagnostic. V12's message
and cause remain UNKNOWN; the twelve launcher outcomes stay unchanged.
Parent #124 remains open with only AC7 complete.

## Fixed host cache profile (#151)

`inventory_cache_profile(environment)` in [run.py](../tools/cajviewer/run.py)
is a pure host-only API outside all canonical-entry fragments. It accepts the
already declared image ENV with the phase HOSTNAME. Exact dictionaries and
string keys/values are required: at most 64 entries, 128 characters per key
and 4,096 per value. These narrow profile limits match the canonical entry's
limits; empty keys, `=` in keys and NUL characters are refused. Applicability
requires `HOME=/home/canary`, `XDG_CONFIG_HOME=/home/canary/.config` and
`XDG_CACHE_HOME=/home/canary/.cache` for those three keys.

The returned pair contains a new expected-ENV dictionary changing only
`XDG_CACHE_HOME` to `/tmp` and a new token list containing exactly
`["--env", "XDG_CACHE_HOME=/tmp"]`. Input is unchanged. No ambient ENV,
filesystem, clock, tool or process is consulted, and no cache-path option is
exposed. This adds no installed module or inline statement.

Parent #124's external caller must consume both values: insert those
tokens exactly once into Docker create and pass the returned ENV to the
canonical entry and independent host observation. Preserve every other
image/resource/user/mount argument. Its existing `future_exact_argvs` equality
gate must reject missing, duplicate or foreign overrides before Docker.
There is no public P2 argv builder. A distinct original V14 caller and its
argument/refusal controls passed, followed by one successful inventory with
unchanged raw font/package observations. The consumed V13 caller, plans and
evidence remain immutable.

Fontconfig's documented XDG user-cache default makes `/tmp/fontconfig` a
candidate inside the existing 8 MiB `/tmp` tmpfs. See the
[Debian Fontconfig 2.14.1 manual](https://manpages.debian.org/bookworm/fontconfig-config/fonts-conf.5.en.html),
[upstream Fontconfig configuration](https://fontconfig.pages.freedesktop.org/fontconfig/fontconfig-user.html)
and [XDG base-directory specification](https://specifications.freedesktop.org/basedir/latest/).
The fixed V14 inventory profile passed with identical raw font output and empty
helper stderr. Actual cache creation, contents and occupied bytes remain
unmeasured. `/tmp` is outside the six inventory roots; no
membership exception, cache warmup, custom font configuration, writable HOME,
image rebuild, baseline refresh, output sorting/normalization or cap increase
is introduced.

Original [profile controls](../tests/conformance/test_cajviewer_inventory_cache.py)
exercise the actual API and unchanged full canonical entry with invented ENV
and helper results. Wrong initial cache ENV stops before modules/helpers;
changed closing ENV fails after helpers; stderr failure remains primary.
The existing entry test helper only gains explicit injected ENV observations;
all production entry/finally statement nodes still run. One original focused
run passed all nine methods with zero skips and six synthetic helper callbacks;
forbidden effects, additional candidate children and vendor passes were zero.
Root and independent closing reviews passed. Its actual assembly assertion
verified the unchanged 16,328-byte entry, SHA-256
`8fd9503d1e4d7d8692fd3589f2dc0e6044e212c03adf438f82c5ed489e955f88`.
The whole source is 65,260/65,536 bytes (276 bytes of headroom); assembly remains
bounded by 16,384 bytes. These controls prove source behavior only: cache
creation and runtime compatibility remain unproved and NOT_RUN.

Source closure needs final-head Root/independent correctness, MIT provenance
and simplification reviews, meaningful original controls and all four hosted
gates. It supplies no runtime acceptance. A future finite caller/profile must
retain image/config/layers, the sole read-only public-module bind, user
1000:1000, read-only root, offline network, memory 512 MiB, memory-plus-swap
512 MiB (effective swap zero), two CPUs, 64 PIDs, 8 MiB tmpfs and all existing
read/output/deadline bounds. Complete empty stderr and unchanged file/tool/
font/package comparisons remain mandatory. The new phase needs actual caller
controls, two frozen reviews, fresh same-PID pending reviews/token and actual
closing. Only complete successful unchanged inventory permits a separate
reviewed GUI profile. Complete-page images and fresh ordinary-copy fixtures
remain NOT_RUN.

## Exact terminal grammar

An action is attempted before the pinned process helper is called. Only the
first terminal helper failure is retained, and the ordered helper chain stops.
Successful actions have no terminal diagnostic. The envelope's
`terminal_helper` is null when no helper has failed.

The diagnostic is an object with exactly these keys. Unknown keys and
bool-for-integer substitutions are refused.

| Key | Required value |
| --- | --- |
| `schema` | `cajviewer-inventory-helper-diagnostic/1` |
| `action_ordinal` | Integer 1 or 2, matching the final failed action |
| `stage` | `metadata-helper-run` or `metadata-helper-validation` |
| `reason` | One of the seven reasons below |
| `error_type` | A fixed public exception type or `OTHER_ERROR_TYPE` |
| `helper_status` | Observed `PASS`, `FAIL`, `TIMEOUT`, `OUTPUT_LIMIT`, or null |
| `exit_code` | Observed integer from -255 through 255, or null |
| `spawned` | True for a well-framed returned pinned-helper result; otherwise null |
| `captures` | Exact stdout/stderr capture identities, or null |
| `bytes_read` | Exact stdout/stderr read counts, or null |
| `stderr` | The exact retained-byte object below, or null |

Missing, raised and unframed results leave the result metadata unavailable.
A helper may raise after spawning: null never proves that no process started.
Host aggregates keep `observed_spawned` as the known lower bound and
`spawn_unknown` as the unknown count. Aggregate `spawned` is null whenever
any attempted action has unknown spawn status.

The public error-type whitelist is the shared original source-loading
whitelist: `FileNotFoundError`, `PermissionError`, `IsADirectoryError`,
`NotADirectoryError`, `OSError`, `ValueError`, `TypeError`, `KeyError`,
`AttributeError`, `SyntaxError`, `UnicodeError`, `UnicodeDecodeError`,
`ImportError`, `ModuleNotFoundError`, `MemoryError`, `RuntimeError`,
`OverflowError`, `KeyboardInterrupt`, `SystemExit`, and `OTHER_ERROR_TYPE`.
Exception messages and tracebacks are never included.

### Failure precedence

The producer applies these guards in order. A frame must be valid before its
status, exit code and capture identities become observed result metadata.

| Order | Reason | Stage |
| --- | --- | --- |
| 1 | `HELPER_RESULT_UNAVAILABLE`: missing result or raised helper | run |
| 2 | `HELPER_RESULT_MALFORMED`: invalid result framing | run |
| 3 | `HELPER_TIMEOUT` | run |
| 4 | `HELPER_OUTPUT_LIMIT` | run |
| 5 | `HELPER_NONZERO_EXIT` | run |
| 6 | `HELPER_RESULT_MALFORMED`: a remaining non-PASS status | run |
| 7 | `HELPER_CAPTURE_INCOMPLETE` | validation |
| 8 | `HELPER_STDERR_NOT_EMPTY` | validation |

Here `run` and `validation` abbreviate the two full stage strings in the
grammar. A returned helper PASS with exit 0 and complete nonempty stderr is a
validation failure. Its observed helper PASS is retained; acceptance stays FAIL.

### Capture and excerpt identities

Each `captures` object has exactly `stdout` and `stderr`. Each identity has
exactly `size_bytes`, `sha256`, `complete`, and `hash_scope`. `bytes_read` has
exactly the two stream names and nonnegative integer counts.

| Stream | Maximum retained capture | Maximum sentinel-inclusive read |
| --- | ---: | ---: |
| stdout | 262,144 bytes | 262,145 bytes |
| stderr | 65,536 bytes | 65,537 bytes |

An identity hashes retained capture bytes only. `complete` requires a complete
non-timeout, non-limit result whose capture length equals its actual read count.
Its scope is `retained-stream` when complete and `captured-prefix` otherwise.
TIMEOUT and OUTPUT_LIMIT always produce incomplete capture identities.
The diagnostic embeds no stdout bytes.

When available, `stderr` has exactly these keys:

| Key | Required value |
| --- | --- |
| `encoding` | `base64` |
| `data` | Canonical ASCII base64, at most 5,464 characters |
| `retained_bytes` | Integer equal to the decoded length, at most 4,096 |
| `sha256` | Lowercase SHA-256 of those decoded retained bytes only |
| `complete` | True only when the stream capture is complete and entirely fits |
| `truncated` | Exact inverse of `complete` |
| `hash_scope` | `retained-stream` when complete; otherwise `captured-prefix` |

The excerpt is the first min(captured stderr length, 4,096) bytes. A 65,537-byte
read may yield a 65,536-byte stream capture and a 4,096-byte excerpt; all three
counts remain distinct. A timeout excerpt is incomplete even when it fits
without slicing. Canonical base64 requires strict decoding and exact re-encoding
equality; whitespace, extra padding and noncanonical pad bits are refused.

For complete stderr of at most 4,096 bytes, the host also binds the excerpt hash
to the complete capture hash. For a larger or incomplete stream, a full-stream
hash cannot prove excerpt membership. The excerpt is an observation from the
pinned producer; the host checks its own identity, length and action bindings.

Raw public-helper stderr remains verbatim in the external envelope/receipt.
It may contain public-tool paths. Do not classify, sanitize, normalize or
publish the actual bytes. No vendor/document bytes, raw stdout, environment
snapshots or arbitrary path fields are added to the diagnostic.

## Producer, host and closing behavior

The two planned commands remain the pinned inventory module's ordered
`dpkg-query -W` and `fc-list --format` actions, with deadline 10 seconds and
stdout limit 256 KiB. The original process-helper and inventory modules keep
their existing byte identities. No third installed module is introduced.

Small named owned fragments separate the producer from independent host
validation. The complete canonical entry combines the unchanged public
source loader, the diagnostic producer and the actual environment/user/cgroup
closing logic. A private whitespace compaction applies only to owned assembly
pieces; the public loader bytes remain exact. Operational callers must pin the
whole runner and the exact assembled entry separately.

`parse_inventory_envelope` accepts only complete bounded UTF-8 JSON, rejects
duplicate decoded keys, excessive depth and non-JSON numeric constants, and
checks the inventory-accounting envelope header. Independent validation checks
the ordered action records, diagnostic keys and reason/stage precedence,
strict types, base64, hashes, lengths, completeness and capture bindings.

`observe_inventory_result` adopts complete valid helper and source-loading
accounting before required outer status/exit/stderr checks. A valid complete
FAIL therefore survives outer rejection. Prefix or malformed accounting stays
`UNKNOWN`, with unavailable counts and no invented whole-output identity.
`COMPLETE_ORDERED_RECORDS` describes validated accounting, not overall PASS.

PASS still requires two ordered spawned helpers, typed exit 0, complete stdout,
empty complete stderr, both pinned public source loads, inventory identity,
matching protected environment/user facts and all cgroup closing checks.
The enclosing host still owns verified-ID removal, final container absence,
source/output audits and persistence. A later closing or serialization refusal
does not replace the primary terminal-helper diagnostic.

## Bounds and mandatory controls

The whole runner remains at most 65,536 bytes. Complete inline assembly remains
at most 16,384 bytes and refuses a larger result. The JSON envelope remains at
most 4 MiB. Oversized inventory content can be explicitly omitted only while
marking FAIL and retaining all diagnostic/action/source ledgers; if those ledgers
still do not fit, serialization explicitly refuses. The inherited external
inventory receipt cap remains 256 KiB; its save path explicitly refuses a
larger receipt and its incomplete FAIL path preserves primary/nested accounting.
No truncation or omitted-ledger path can produce success.

Mandatory original controls exercise live owned fragments, independent host
validation, the actual complete assembled entry and its actual finally, and
the actual observer before outer refusal. They cover invented exit-0/nonempty
stderr, missing/malformed/nonzero results, timeouts, limits, sentinel counts,
4,095/4,096/4,097-byte boundaries, strict mutations, primary-plus-closing failure,
complete FAIL retention, prefix UNKNOWN, no-input zero work, token preservation
and bounded serialization refusal. Existing source-loading controls remain
mandatory. Original control processes and synthetic events are counted
separately from actual vendor/private work, which remains NOT_RUN with zero
compatibility passes.

Child closure additionally requires final-head Root and independent
correctness/provenance/simplification reviews and all four hosted native,
WASM, MIT/provenance and coverage gates. Actual LCOV DA records must be
recounted for every recorded source file, without exclusions; raw LF/LH totals
are reported separately. Rust line coverage does not establish Python or
whole-repository coverage.

## Next runtime phase

This child establishes source diagnostic readiness only. Parent #124 and the
[#123 fixture epic](https://github.com/rwv/caj2pdf-rust/issues/123) remain open.
No actual complete-page image or ordinary-copy fixture is established here.

A later phase needs its own preserved-history plan, exact code/module/image/
action/environment/resource pins, source/model/source-loading controls, two
independent frozen reviews, fresh same-PID preflight/token and actual closing
review. All old failures, snapshots, plans and receipts remain unchanged.
Closing this issue authorizes no replay or application/runtime/vendor/private
action. Successful capability proof then precedes image #126 and text #127;
comparators #128 and compatibility rollout #129 follow their declared blockers.
