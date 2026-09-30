#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
# Ubuntu 24.04's QEMU 8.2 misexecutes this target's optimized arithmetic.
set -euo pipefail
install_dir="$RUNNER_TEMP/caj2pdf-qemu"
mkdir -p "$install_dir"
curl --fail --location --retry 3 https://github.com/loongson/build-tools/releases/download/2025.06.06/qemu-loongarch64 -o "$install_dir/qemu-loongarch64"
echo "a0a022d8543f7188d357f880b05557bbdc75038a4953d0b0e3e0496f20004bb1  $install_dir/qemu-loongarch64" | sha256sum -c -
chmod +x "$install_dir/qemu-loongarch64"
"$install_dir/qemu-loongarch64" --version
echo "$install_dir" >> "$GITHUB_PATH"
