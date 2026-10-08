# Native platforms and release assets

The matrix in `platform-targets.json` is the required release set. Each target
must build and pass conversion tests before a release can be published; merely
cross-compiling a binary is not sufficient. v0.1.0 remains Linux x86_64-only.
v0.2.0 has 16 native targets. v0.3.0 expands the required inventory to 50 OS/CPU/ABI targets.
Current main adds i586 GNU and FreeBSD RISC-V64/PowerPC64 for 53 required targets; these
additions are not in historical v0.3.x assets. Unverified targets remain in [platform-candidates.md](platform-candidates.md).
Target counts include OS/libc/ABI combinations, not just CPU architectures.
The full matrix runs on `main`, release tags and on demand. Pull requests always
run the `native` job (its Linux entries are required checks) and run the other
jobs only when packaging or toolchain inputs change (see [CONTRIBUTING](../CONTRIBUTING.md#quality-gates)).

| Platform | Architectures | Runtime evidence / baseline |
| --- | --- | --- |
| Linux GNU | x86_64, ARM64 | Native Ubuntu 24.04; glibc 2.39 build baseline |
| Linux musl | x86_64, ARM64 | Native Ubuntu 24.04 execution of static binaries; no glibc dependency |
| macOS | Intel x86_64, Apple ARM64 | Native macOS 15 runners |
| Windows MSVC | x86_64, ARM64, x86 | Windows Server 2025 x64 / Windows 11 ARM64; x86 uses Windows compatibility execution |
| Linux GNU Pentium | i586 | Isolated pinned Debian Jessie glibc 2.19 sysroot; Clang/LLD and QEMU 10.0.13 `-cpu pentium`; all core and portable CLI tests execute |
| Linux GNU extended | i686, ARMv7 hard-float, RISC-V64 GC, ppc64le, s390x | QEMU user-mode on Ubuntu 24.04 with its cross sysroots (glibc 2.39 baseline) |
| Linux GNU additions | ARMv5TE, ARMv6 soft/hard-float, ARMv7 soft-float, PowerPC32, PowerPC64 big endian, SPARC64 | QEMU and Ubuntu 24.04 sysroots; ARMv6 hard-float uses the hashed Bootlin stable-2025.08-1 ARMv6 sysroot |
| Linux LoongArch64 | GNU and musl, LP64D + LSX | Pinned QEMU 10.0.2; GNU uses Loongson GCC 15.1.0 / binutils 2.45 / glibc 2.42; musl is static |
| Linux musl additions | i586, i686, ARMv5TE, ARMv6/ARMv7 soft/hard-float, PowerPC64 big/little endian, RISC-V64 GC | Static Rust-bundled musl runtime, tested with QEMU |
| Linux GNU MIPS | MIPS32 and MIPS64 n64, each big/little endian | Ubuntu cross sysroots and QEMU; std built with pinned nightly-2026-09-29 |
| FreeBSD | x86_64 | FreeBSD 14.3 virtual machine; Rust from the FreeBSD package repository, version printed in CI |
| NetBSD | x86_64 | NetBSD 11.0 VM, packaged Rust compiler |
| OpenBSD | x86_64 | OpenBSD 7.9 VM, packaged Rust compiler |
| BSD cross-built | FreeBSD ARM64, RISC-V64 GC, PowerPC64 big endian; NetBSD ARM64; OpenBSD ARM64, RISC-V64 GC | Pinned official release sets, nightly-2026-09-29 std, Clang/LLD; matching VMs execute tests, host validates target PDF ([details](#bsd-cross-builds)) |
| illumos | x86_64 | OmniOS r151054 VM; 499 core tests and 4 CLI tests, host qpdf/MuPDF validation |
| Linux Bootlin GNU | RISC-V32 GC ILP32D | Bootlin stable-2025.08-1 glibc sysroot; pinned std build and QEMU |
| Linux Bootlin musl (dynamic) | PowerPC32 (e300c3), s390x (z13) | Bootlin stable-2025.08-1 SDK runtime; pinned std build and QEMU; not static Docker artifacts |
| Android CLI | x86_64, x86 | NDK 28.2.13676358 (API 24 build), API 30 emulators; 499 core + 4 CLI tests and host PDF validation |
| JavaScript/WASM | Browser and Node 22+ | Portable WASM package; real Node 22/24 and Chromium tests |

The pinned Rust toolchain is used wherever distributed for hosted runners.
MIPS and cross-built BSD targets build std with a separate pinned nightly.
Other BSD targets and illumos use their packaged compilers; the exact build log
records the version.
Do not infer support for older OS/libc versions from compilation alone. No
claim is made for Android/iOS applications, embedded targets or platforms
outside this matrix. They need their own platform adapter and
runtime evidence. Windows builds statically link the MSVC CRT to avoid a separate VC runtime
installation. Windows binaries are not Authenticode-signed and macOS
binaries are not notarized; local OS trust prompts may apply.

## What each target checks

Core unit tests include independent qpdf/MuPDF rendering. On illumos, Android
and cross-built FreeBSD RISC-V64/PowerPC64,
the four tests requiring local PDF validators are explicitly filtered because
those packages are unavailable; target-produced PDFs are instead checked and
rendered on the host. The mixed-image codec/draw-order test and HN-B type-3
conversion, draw-order and malformed-payload checks run separately without
validators. Run reports distinguish executed and filtered tests; the
499-test count in the published baseline predates these additions. Portable CLI tests
execute Unicode paths, inspect/page counts, stdin/stdout conversion equality,
existing-output refusal, hard-link/input protection and failed-output cleanup.
Cross-platform tests do not use Unix-only `/dev` test fixtures. QEMU validates
instruction/ABI behavior but does not reproduce every physical CPU or kernel.
Only Linux-compiled instrumented Rust is covered by the exact 100% line gate;
Windows/other-target execution is separate evidence.

Archives contain the tested CLI, README, project MIT license and selected MIT
Cargo dependency notices. Windows uses ZIP; other systems use tar.gz. CI
collects every required target, refuses missing/unexpected artifacts and writes
one SHA256SUMS before publication. Independent PDF validators, toolchains and
external document corpora are never included in the archives.

## CPU and runtime limits

ARMv5TE tests use QEMU arm926, ARMv6 uses arm1176, and ARMv7 uses cortex-a9.
Both i586 GNU and musl binaries pass on the original Pentium CPU model.
The GNU build uses a pinned glibc 2.19 sysroot and explicit guest library search
paths to avoid the incompatible Ubuntu i386 runtime. This upcoming GNU artifact
is not part of historical v0.3.x downloads; no old-kernel guarantee is inferred. PowerPC
endian variants retain their target ELF ABIs. LoongArch is distinct from MIPS;
these artifacts do not imply compatibility with all old Loongson vendor ABIs.
LoongArch musl disables linker relaxation (`--no-relax`) for both tested and
packaged binaries to avoid [LLVM's layout oscillation issue](https://github.com/llvm/llvm-project/issues/226712)
with the pinned Rust toolchain. Static linking and the full runtime checks remain
in place; the workaround does not change other targets.

Exact sysroot and emulator hashes are recorded in the workflows. No old-kernel
compatibility is inferred from user-mode emulation on a newer host kernel.

The container inventory is separate: `container-platforms.json`. Every listed
image must execute from the exported OCI archive before publication.

The dynamic PowerPC32 SDK targets e300c3 and the s390x SDK targets z13; the
SDK may require newer CPU features than Rust's generic target baseline. MIPS
GNU archives use their standard hard-float target ABI. MIPS32 musl candidates
are withheld because the available SDK disagrees with Rust's soft-float ABI.

Android artifacts are command-line executables, not APKs or a JNI API. Emulator
tests enable adb root so the hard-link protection fixture can be created under
Android's filesystem policy. API 30 is the tested runtime; compiling with an
API 24 NDK setting is not an execution claim for every older Android release.

## BSD cross-builds

BSD targets whose VMs run under full-system emulation are cross-built on the
x86_64 host by `scripts/build-bsd-cross.py`, because compiling Rust inside
those VMs took up to 79 minutes. The script downloads the official release
sets, checks each against the digest pinned from the project's published
checksum file, extracts only libraries and headers into a temporary sysroot,
and builds with Clang/LLD and the pinned `nightly-2026-09-29` std. One
`bsd-cross` job covers these targets:

| OS | Targets | Sysroot and test VM | Minimum runtime |
| --- | --- | --- | --- |
| FreeBSD | ARM64 | 14.3 `base.txz` from the archive mirror | 14.3 |
| FreeBSD | RISC-V64 GC, PowerPC64 | 15.1 `base.txz` | 15.1 |
| NetBSD | ARM64 | 11.0 `base` and `comp` sets | 11.0 |
| OpenBSD | ARM64, RISC-V64 GC | 7.9 `base79` and `comp79` sets | 7.9 |

Alongside the checkout, the VM receives the staged core test executable,
portable CLI tests, CLI and original PDF fixture; temporary sysroot/build
files are removed before transfer. The tested executable is packaged on the
host after qpdf/page-count/MuPDF checks. Guests with PDF validator packages
run every core test. FreeBSD RISC-V64 and PowerPC64 have none, so four
validator-dependent core tests are explicitly filtered there and never
counted as passes. The RISC-V64/PowerPC64 route was first verified in run
[37164189357](https://github.com/rwv/caj2pdf-rust/actions/runs/37164189357).

FreeBSD 14.3 has moved to the archive mirror, which serves only plain HTTP;
its download is checked against the SHA-256 from its official `MANIFEST`.
OpenBSD ships only versioned shared libraries such as `libc.so.103.0`, which
its own linker resolves; the script adds unversioned links in the sysroot so
upstream LLD links them dynamically instead of falling back to static
archives. x86_64 FreeBSD, NetBSD and OpenBSD keep their KVM-accelerated
in-VM builds with packaged Rust. Every cross-built triple stays in
`platform-targets.json`; the release aggregator requires its `native-*`
archive and includes it in the existing checksums and provenance flow.
