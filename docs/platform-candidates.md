# Additional platform candidates

`platform-targets.json` is the required release set. A target is promoted only
after runtime tests and ABI review. The experimental `Platform candidates`
workflow is manual and offers `all`, `linux`, `bsd` and `android` groups. Its
failures and skipped groups are not compatibility passes. It is not a release
gate. All jobs use standard public-repository runners, with no paid larger
runners. Remaining work is tracked in [#214](https://github.com/rwv/caj2pdf-rust/issues/214).

## Concrete blockers

| Candidate | Observed blocker / next prerequisite |
| --- | --- |
| i586 GNU | An isolated Jessie GNU sysroot passes local Pentium tests; the `i586` candidate job must verify the installer on hosted CI before promotion. The i586 musl artifact is already released. |
| MIPS32 musl, big/little endian | Tests passed but linker warnings revealed Rust soft-float / SDK hard-float ABI mismatch. Withdrawn from release inventory. Supply a matching soft-float SDK; do not hide warnings or silently change ABI. MIPS GNU variants are independently verified. |
| RISC-V32 musl | Static std build lacks an unwinder; the dynamic SDK build links but SIGSEGVs before the harness with both distro QEMU and pinned QEMU 10.0.13. Diagnose runtime/ABI startup or provide a working static runtime. RISC-V32 GNU passes. |
| m68k GNU | LLVM instruction selection fails while compiling std, before project code runs. Static relocation / non-PIE retry hits the same failure; ineffective flags were removed. A working compiler/std build is required. |
| FreeBSD RISC-V64 / PowerPC64 | Local FreeBSD 15.1 VM tests pass with host cross-builds against pinned official sysroots. The candidate workflow verifies this route without guest Rust packages; hosted verification and release promotion remain separate. |
| Android ARM64 / ARMv7 | CLI builds succeed. Linux x86_64 emulator refuses ARM64 images; standard macOS ARM64 fails with HVF_UNSUPPORTED even when requesting software acceleration. ARMv7 was also attempted via the ARM64 API 30 image. Supply a suitable ARM device or virtualization host. |

SPARC32 GNU is not configured: the current Bootlin SPARCv8 SDK uses uClibc,
which is not a verified substitute for the Rust GNU target's libc. Bare-metal,
GPU and microcontroller triples require a different execution/storage model;
they are not native CLI release assets.

## FreeBSD cross-build candidates

The previous 14.3 guest-package bootstrap failed before project tests. The
candidate workflow now builds on Linux using `nightly-2026-09-29` with
`rust-src`, Clang and LLD. `scripts/build-freebsd-cross.py` downloads the
official FreeBSD 15.1 base archive for the selected architecture, verifies its
pinned SHA256 before extracting headers/libraries, and stages the core tests,
portable CLI tests and executable in `target/freebsd-cross`. Build intermediates
stay in a temporary directory outside the VM source transfer. No sysroot or
compiler is included in the candidate archive.

The matching FreeBSD 15.1 VM executes the tests and produces an original-fixture
PDF. Three independent-render tests require guest qpdf/MuPDF and are explicitly
filtered, as on existing validator-less targets. They are not counted as
passes. The host separately validates and renders the actual target output
before packaging the tested executable with the existing MIT notices helper.

Local preflight at `60c0671` passed 674 core tests and six portable CLI tests
on each architecture, with three filtered tests each. Original atomic/thread/
file-I/O probes also passed. RISC-V64 used FreeBSD 15.1-RELEASE-p3; PowerPC64
used 15.1-RELEASE. Both used QEMU 10.0.13, builder 2.2.8, Clang/LLD 19.1.7 and
the pinned nightly. Target PDFs passed host qpdf and MuPDF. These local results
do not replace hosted candidate verification or promote release support; this
route's minimum FreeBSD version is 15.1.

The sysroot hashes come from the official
[RISC-V64 MANIFEST](https://download.freebsd.org/releases/riscv/riscv64/15.1-RELEASE/MANIFEST)
and [PowerPC64 MANIFEST](https://download.freebsd.org/releases/powerpc/powerpc64/15.1-RELEASE/MANIFEST).
External receipts and original probes remain under
`caj2pdf-freebsd-cross-20261003`; no platform/vendor source is copied into the
project. Follow #214 for hosted results and any subsequent promotion.

## Test boundaries

Required BSD targets (including NetBSD ARM64 and OpenBSD ARM64/RISC-V64) have
passed their full core and portable CLI suites in VMs. Android x86/x86_64 and
illumos ran 499 core tests in the published baseline, with the two then-existing local-validator tests explicitly
filtered, plus all four portable CLI tests; target-produced PDFs are separately
checked/rendered on the host. This is not represented as 501 core passes.
The current workflow also filters the new native-content mixed-image raster
check on those targets; its codec, draw-order and cleanup test runs separately
without external tools. Current test counts come from each run, not the
historical baseline above.
Absent external corpora remain NOT_RUN. Emulation does not certify every
physical CPU or older kernel/libc release.

## Pentium GNU candidate (#214)

The `i586` dispatch group isolates `i586-unknown-linux-gnu` from the other
Linux candidates. `scripts/install-i586-gnu.py` downloads five SHA256-pinned
Debian Jessie i386 development/runtime packages into runner scratch space.
It relocates absolute library symlinks inside that sysroot without changing
library bytes. Clang/LLD link the official Rust target against glibc 2.19;
QEMU 10.0.13 executes it with `-cpu pentium`. The archived sysroot is a build
and test input, not a host installation or a bundled CLI dependency.

This addresses the earlier Ubuntu i386 runtime's unsupported CPU instructions.
Local core tests (677, no filters), portable CLI tests (six), and independently
validated target-produced PDF output pass with Rust 1.98.1. The candidate job
repeats these checks using the installer, then packages the tested executable.
Hosted success and subsequent required-matrix integration remain necessary
before release support is claimed. A glibc 2.19 runtime test does not establish
compatibility with every historical kernel. No global compiler downgrade or
production converter change is required.
