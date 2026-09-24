#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
set -euo pipefail

report_dir=target/coverage
report="$report_dir/lcov.info"
summary="$report_dir/summary.txt"
minimum=90
mkdir -p "$report_dir"

cargo llvm-cov --workspace --all-features --locked --lcov --output-path "$report"

if [[ ! -f "$report" ]]; then
  printf 'Coverage tool did not produce an LCOV report.\n' | tee "$summary" >&2
  exit 1
fi

# LLVM emits LF/LH once per source file.
read -r lines hits < <(awk -F: '
  /^LF:/ { lines += $2 }
  /^LH:/ { hits += $2 }
  END { printf "%d %d\n", lines, hits }
' "$report")

if (( lines == 0 )); then
  printf 'Line coverage: no coverable lines reported; the coverage gate cannot pass.\n' | tee "$summary" >&2
  exit 1
fi

awk -v lines="$lines" -v hits="$hits" -v minimum="$minimum" 'BEGIN {
  percent = 100 * hits / lines
  printf "Line coverage: %.2f%% (%d/%d); minimum %d%%; target 100%%.\n", percent, hits, lines, minimum
  if (percent + 0.000001 < minimum) exit 1
}' | tee "$summary"
