#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
# Test each manifest from the same OCI archive that will be published.
set -euo pipefail
archive="$(realpath "$1")"
stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
cp tests/fixtures/valid_nested_outline.pdf "$stage/input.pdf"
chmod 777 "$stage"
for arch in amd64 arm64; do
  image="caj2pdf-test:$arch"
  skopeo copy --override-os linux --override-arch "$arch" "oci-archive:$archive" "docker-daemon:$image"
  docker run --rm --platform "linux/$arch" "$image" --version
  docker run --rm --platform "linux/$arch" --read-only \
    --tmpfs /tmp:rw,noexec,nosuid,mode=1777,size=64m \
    --mount "type=bind,src=$stage,dst=/data" \
    "$image" input.pdf -o "$arch.pdf"
  qpdf --check "$stage/$arch.pdf"
  docker run --rm -i --platform "linux/$arch" --read-only \
    --user "$(id -u):$(id -g)" --tmpfs /tmp:rw,noexec,nosuid,mode=1777,size=64m \
    "$image" - < "$stage/input.pdf" > "$stage/$arch-stdout.pdf"
  cmp "$stage/$arch.pdf" "$stage/$arch-stdout.pdf"
  docker image rm "$image"
done
