# Native platforms and release assets

The matrix in `platform-targets.json` is the required release set. Each target
must build and pass conversion tests before a release can be published; merely
cross-compiling a binary is not sufficient. v0.1.0 remains Linux x86_64-only.
v0.2.0 has 16 native targets. This branch expands the next release inventory;
unverified targets remain in [platform-candidates.md](platform-candidates.md).
Target counts include OS/libc/ABI combinations, not just CPU architectures.

| Platform | Architectures | Runtime evidence / baseline |
| --- | --- | --- |
| Linux GNU | x86_64, ARM64 | Native Ubuntu 24.04; glibc 2.39 build baseline |
| Linux musl | x86_64, ARM64 | Native Ubuntu 24.04 execution of static binaries; no glibc dependency |
| macOS | Intel x86_64, Apple ARM64 | Native macOS 15 runners |
| Windows MSVC | x86_64, ARM64, x86 | Windows Server 2025 x64 / Windows 11 ARM64; x86 uses Windows compatibility execution |
| Linux GNU extended | i686, ARMv7 hard-float, RISC-V64 GC, ppc64le, s390x | QEMU user-mode on Ubuntu 24.04 with its cross sysroots (glibc 2.39 baseline) |
| Linux GNU additions | ARMv5TE, ARMv6 soft/hard-float, ARMv7 soft-float, PowerPC32, PowerPC64 big endian, SPARC64 | QEMU and Ubuntu 24.04 sysroots; ARMv6 hard-float uses the hashed Bootlin stable-2025.08-1 ARMv6 sysroot |
| Linux LoongArch64 | GNU and musl, LP64D + LSX | Pinned QEMU 10.0.2; GNU uses Loongson GCC 15.1.0 / binutils 2.45 / glibc 2.42; musl is static |
| Linux musl additions | i586, i686, ARMv5TE, ARMv6/ARMv7 soft/hard-float, PowerPC64 big/little endian, RISC-V64 GC | Static Rust-bundled musl runtime, tested with QEMU |
| Linux GNU MIPS | MIPS32 and MIPS64 n64, each big/little endian | Ubuntu cross sysroots and QEMU; std built with pinned nightly-2026-09-29 |
| FreeBSD | x86_64, ARM64 | FreeBSD 14.3 virtual machines; Rust from the FreeBSD package repository, version printed in CI |
| NetBSD | x86_64, ARM64 | NetBSD 11.0 VMs, packaged Rust compiler |
| OpenBSD | x86_64, ARM64 | OpenBSD 7.9 VMs, packaged Rust compiler |
| illumos | x86_64 | OmniOS r151054 VM; 499 core tests and 4 CLI tests, host qpdf/MuPDF validation |
| Linux Bootlin GNU | RISC-V32 GC ILP32D | Bootlin stable-2025.08-1 glibc sysroot; pinned std build and QEMU |
| Linux Bootlin musl (dynamic) | PowerPC32 (e300c3), s390x (z13) | Bootlin stable-2025.08-1 SDK runtime; pinned std build and QEMU; not static Docker artifacts |
| Android CLI | x86_64, x86 | NDK 28.2.13676358 (API 24 build), API 30 emulators; 499 core + 4 CLI tests and host PDF validation |
| JavaScript/WASM | Browser and Node 22+ | Portable WASM package; real Node 22/24 and Chromium tests |

The pinned Rust toolchain is used wherever distributed for hosted runners.
MIPS builds std with a separate pinned nightly. BSD and illumos use their packaged compilers; the exact build log records the version.
Do not infer support for older OS/libc versions from compilation alone. No
claim is made for Android/iOS applications, embedded targets or platforms
outside this matrix. They need their own platform adapter and
runtime evidence. Windows builds statically link the MSVC CRT to avoid a separate VC runtime
installation. Windows binaries are not Authenticode-signed and macOS
binaries are not notarized; local OS trust prompts may apply.

## What each target checks

Core unit tests include independent qpdf/MuPDF rendering. On illumos and Android, the two
core tests requiring local PDF validators are explicitly filtered because those
packages are unavailable; the VM-produced PDF is instead checked and rendered
on the host. This is 499 core tests plus separate validation, not 501 passes. Portable CLI tests
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
The i586 musl binary passes on the original Pentium CPU model. Ubuntu's i386
GNU libc fails on that model, so no i586 GNU artifact is advertised. PowerPC
endian variants retain their target ELF ABIs. LoongArch is distinct from MIPS;
these artifacts do not imply compatibility with all old Loongson vendor ABIs.
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
