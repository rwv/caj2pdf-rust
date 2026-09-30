#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
# Test each manifest from the same OCI archive that will be published.
set -euo pipefail
archive="$(realpath "$1")"
stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
cp tests/fixtures/valid_nested_outline.pdf "$stage/input.pdf"
chmod 777 "$stage"
while IFS= read -r platform; do
  arch="${platform#linux/}"
  tag="${arch//\//-}"
  options=(--override-os linux --override-arch "${arch%%/*}")
  if [[ "$arch" == */* ]]; then options+=(--override-variant "${arch#*/}"); fi
  image="caj2pdf-test:$tag"
  skopeo copy "${options[@]}" "oci-archive:$archive" "docker-daemon:$image"
  docker run --rm --platform "$platform" "$image" --version
  docker run --rm --platform "$platform" --read-only \
    --tmpfs /tmp:rw,noexec,nosuid,mode=1777,size=64m \
    --mount "type=bind,src=$stage,dst=/data" \
    "$image" input.pdf -o "$tag.pdf"
  qpdf --check "$stage/$tag.pdf"
  docker run --rm -i --platform "$platform" --read-only \
    --user "$(id -u):$(id -g)" --tmpfs /tmp:rw,noexec,nosuid,mode=1777,size=64m \
    "$image" - < "$stage/input.pdf" > "$stage/$tag-stdout.pdf"
  cmp "$stage/$tag.pdf" "$stage/$tag-stdout.pdf"
  docker image rm "$image"
done < <(python3 -c 'import json; print("\n".join(json.load(open("docs/container-platforms.json"))))')
