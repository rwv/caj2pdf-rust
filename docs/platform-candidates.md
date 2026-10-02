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
| i586 GNU | Ubuntu i386 libc SIGILLs on QEMU Pentium. Supply a Pentium-compatible GNU sysroot. The i586 musl artifact passes that CPU model and is released. |
| MIPS32 musl, big/little endian | Tests passed but linker warnings revealed Rust soft-float / SDK hard-float ABI mismatch. Withdrawn from release inventory. Supply a matching soft-float SDK; do not hide warnings or silently change ABI. MIPS GNU variants are independently verified. |
| RISC-V32 musl | Static std build lacks an unwinder; the dynamic SDK build links but SIGSEGVs before the harness with both distro QEMU and pinned QEMU 10.0.13. Diagnose runtime/ABI startup or provide a working static runtime. RISC-V32 GNU passes. |
| m68k GNU | LLVM instruction selection fails while compiling std, before project code runs. Static relocation / non-PIE retry hits the same failure; ineffective flags were removed. A working compiler/std build is required. |
| FreeBSD RISC-V64 / PowerPC64 | Official 14.3 pkg bootstrap URLs return 404 for pkg.pkg and pkg.txz; corresponding 15 pkg.pkg URLs were also unavailable. Supply a working package repository or cross sysroot/compiler and validators. |
| Android ARM64 / ARMv7 | CLI builds succeed. Linux x86_64 emulator refuses ARM64 images; standard macOS ARM64 fails with HVF_UNSUPPORTED even when requesting software acceleration. ARMv7 was also attempted via the ARM64 API 30 image. Supply a suitable ARM device or virtualization host. |

SPARC32 GNU is not configured: the current Bootlin SPARCv8 SDK uses uClibc,
which is not a verified substitute for the Rust GNU target's libc. Bare-metal,
GPU and microcontroller triples require a different execution/storage model;
they are not native CLI release assets.

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
