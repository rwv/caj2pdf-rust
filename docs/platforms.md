# Native platforms and release assets

The matrix in `platform-targets.json` is the required release set. Each target
must build and pass conversion tests before a release can be published; merely
cross-compiling a binary is not sufficient. v0.1.0 remains Linux x86_64-only.
The expanded matrix applies to v0.2.0 once all release jobs pass.

| Platform | Architectures | Runtime evidence / baseline |
| --- | --- | --- |
| Linux GNU | x86_64, ARM64 | Native Ubuntu 24.04; glibc 2.39 build baseline |
| Linux musl | x86_64, ARM64 | Native Ubuntu 24.04 execution of static binaries; no glibc dependency |
| macOS | Intel x86_64, Apple ARM64 | Native macOS 15 runners |
| Windows MSVC | x86_64, ARM64, x86 | Windows Server 2025 x64 / Windows 11 ARM64; x86 uses Windows compatibility execution |
| Linux GNU extended | i686, ARMv7 hard-float, RISC-V64 GC, ppc64le, s390x | QEMU user-mode on Ubuntu 24.04 with its cross sysroots (glibc 2.39 baseline) |
| FreeBSD | x86_64, ARM64 | FreeBSD 14.3 virtual machines; Rust from the FreeBSD package repository, version printed in CI |
| JavaScript/WASM | Browser and Node 22+ | Portable WASM package; real Node 22/24 and Chromium tests |

The pinned Rust toolchain is used wherever distributed for hosted runners.
FreeBSD uses its packaged compiler; the exact build log records the version.
Do not infer support for older OS/libc versions from compilation alone. No
claim is made for Android/iOS applications, embedded targets, other BSDs or
architectures outside this matrix. They need their own platform adapter and
runtime evidence. Windows builds statically link the MSVC CRT to avoid a separate VC runtime
installation. Windows binaries are not Authenticode-signed and macOS
binaries are not notarized; local OS trust prompts may apply.

## What each target checks

Core unit tests include independent qpdf/MuPDF rendering. Portable CLI tests
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
