#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
set -euo pipefail

# This is a review tripwire, not a substitute for checking source provenance.
# Committed vendor trees or build products require a deliberate policy update
# with a per-file license review before they can enter this repository.
found=0
while IFS= read -r -d '' file; do
  case "/$file/" in
    */vendor/*|*/third_party/*|*/node_modules/*|*/target/*|*/pkg/*|*/dist/*|*/generated/*)
      printf 'Review required for vendored/generated path: %s\n' "$file" >&2
      found=1
      ;;
    *.wasm/*|*.node/*|*.so/*|*.dll/*|*.dylib/*|*.rlib/*|*.rmeta/*)
      printf 'Review required for compiled artifact: %s\n' "$file" >&2
      found=1
      ;;
  esac
done < <(git ls-files -z --cached --others --exclude-standard)

if (( found )); then
  printf 'Committed vendor trees and build outputs need an explicit MIT provenance review.\n' >&2
  exit 1
fi

printf 'No vendored source trees or build outputs found in the checkout.\n'
