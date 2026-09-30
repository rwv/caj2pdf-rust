#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
# Install a pinned SDK for the native and candidate Linux matrices.
set -euo pipefail
uname -a
rustup toolchain install nightly-2026-09-29 --profile minimal --component rust-src
sudo apt-get update
sudo apt-get install --no-install-recommends -y qemu-user qpdf mupdf-tools
if [[ "$QEMU" == riscv32 ]]; then
  curl --fail --location --retry 3 'https://deb.debian.org/debian/pool/main/q/qemu/qemu-user_10.0.13+ds-0+deb13u1_amd64.deb' -o "$RUNNER_TEMP/qemu.deb"
  echo "ca6ede739327a20ae5a3498230c8a3a9a072c586e131bc6bcc1201eb1725e336  $RUNNER_TEMP/qemu.deb" | sha256sum -c -
  dpkg-deb -x "$RUNNER_TEMP/qemu.deb" "$RUNNER_TEMP/qemu"
  echo "$RUNNER_TEMP/qemu/usr/bin" >> "$GITHUB_PATH"
fi
stem="$ARCH--$LIBC--stable-2025.08-1"
curl --fail --location --retry 3 "https://toolchains.bootlin.com/downloads/releases/toolchains/$ARCH/tarballs/$stem.tar.xz" -o "$RUNNER_TEMP/toolchain.tar.xz"
echo "$SHA  $RUNNER_TEMP/toolchain.tar.xz" | sha256sum -c -
tar -xJf "$RUNNER_TEMP/toolchain.tar.xz" -C "$RUNNER_TEMP"
toolchain="$RUNNER_TEMP/$stem"
"$toolchain/relocate-sdk.sh"
echo "$toolchain/bin" >> "$GITHUB_PATH"
"$toolchain/bin/$PREFIX-gcc" --version
sysroot="$("$toolchain/bin/$PREFIX-gcc" -print-sysroot)"
printf 'Target sysroot: %s\n' "$sysroot"
key="$(echo "$TARGET" | tr '[:lower:]-' '[:upper:]_')"
echo "CARGO_TARGET_${key}_LINKER=$PREFIX-gcc" >> "$GITHUB_ENV"
echo "CARGO_TARGET_${key}_RUNNER=qemu-$QEMU -L $sysroot" >> "$GITHUB_ENV"
echo "CAJ2PDF_TEST_RUNNER=qemu-$QEMU -L $sysroot" >> "$GITHUB_ENV"
echo "CC_${TARGET//-/_}=$PREFIX-gcc" >> "$GITHUB_ENV"
flags='-C link-arg=-Wl,--fatal-warnings'
if [[ "$ARCH" == m68k-* ]]; then flags+=' -C relocation-model=static -C link-arg=-no-pie'; fi
if [[ "$LIBC" == musl ]]; then flags+=' -C target-feature=-crt-static -C link-self-contained=no'; fi
echo "RUSTFLAGS=$flags" >> "$GITHUB_ENV"
