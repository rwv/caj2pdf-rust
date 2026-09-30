# Additional platform candidates

The required release set is `platform-targets.json`. Candidate jobs are runtime
probes, not released support, until explicitly promoted after successful tests.
All use standard GitHub-hosted runners; public-repository standard runner time
is free. Larger runners are not part of this workflow.

## Observed blockers

- `i586-unknown-linux-gnu`: Ubuntu's i386 GNU sysroot executes instructions newer
  than the emulated original Pentium and fails with SIGILL before the test
  harness. The i586 musl build passes on that same CPU model and is included.
  A GNU artifact needs a verified Pentium-compatible libc/sysroot first.
- FreeBSD RISC-V64 / PowerPC64: 14.3 VM setup cannot bootstrap pkg from the
  official `FreeBSD:14:<arch>/latest` repository (404 for pkg.pkg and pkg.txz).
  Checks of the corresponding FreeBSD 15 pkg.pkg URLs also returned 404.
  Repeatable VM probes remain manual; a working package repository or cross
  toolchain/sysroot and validation tools are prerequisites for promotion.

## Active probes

NetBSD, OpenBSD, illumos, Android and Linux RISC-V32 use the separate candidate
workflow. Its failed jobs are failures, not compatibility passes. MIPS GNU
32/64 little/big endian passed both core and CLI tests and is promoted using
pinned nightly std builds. The main project continues using stable Rust.

Android and illumos tests run the core suite except two qpdf/MuPDF-dependent render tests,
which cannot launch host programs inside the device. Portable CLI regressions
run on the emulator; a device-produced PDF is pulled back and independently
checked/rendered on the host. Report these filtered tests explicitly. No
external-corpus tests are counted as passed when their inputs are absent.

Further Bootlin probes cover m68k GNU and MIPS32 big/little-endian, PowerPC32
and s390x musl. These use pinned std builds and hashed SDKs. Dynamic musl SDK
probes are separate from static Docker targets. Bootlin's current SPARCv8 SDKs
use uClibc, not the GNU libc expected by `sparc-unknown-linux-gnu`; that SDK is
not used as an unverified substitute.

- m68k GNU reaches an LLVM instruction-selection failure while compiling std,
  before project code runs (nightly-2026-09-29). See follow-up #214.
- RISC-V32 musl initially lacked a static unwinder. The dynamic SDK build links
  but crashes before the test harness under both the distro and pinned newer
  QEMU; it is not promoted. RISC-V32 GNU passes and is separately included.

- MIPS32 musl (both byte orders): core/CLI tests passed, but review found linker
  warnings proving an ABI mismatch: Rust defaults to soft-float while the
  Bootlin SDK libc uses hard-float. Those passes do not establish a sound ABI.
  These targets were removed from the release inventory. A matching soft-float
  SDK is required; Bootlin probes now make linker warnings fatal. MIPS GNU
  32/64 big/little-endian targets remain independently verified.
