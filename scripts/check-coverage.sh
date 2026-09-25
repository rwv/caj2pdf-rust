#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
set -euo pipefail

# Line-coverage quality gate. The total floor and the per-file floor are
# ratchets: raise them when coverage improves and never lower them to make a
# change pass. The project target remains 100%.
report_dir=target/coverage
report="$report_dir/lcov.info"
summary="$report_dir/summary.txt"
minimum=98.8
file_minimum=97
mkdir -p "$report_dir"

cargo llvm-cov --workspace --all-features --locked --lcov --output-path "$report"

if [[ ! -f "$report" ]]; then
  printf 'Coverage tool did not produce an LCOV report.\n' | tee "$summary" >&2
  exit 1
fi

# LLVM emits SF, then LF/LH, once per source file.
awk -F: -v root="$PWD/" -v minimum="$minimum" -v file_minimum="$file_minimum" '
  /^SF:/ {
    file = substr($0, 4)
    if (index(file, root) == 1) file = substr(file, length(root) + 1)
  }
  /^LF:/ { found[file] += $2; lines += $2 }
  /^LH:/ { hit[file] += $2; hits += $2 }
  END {
    if (lines == 0) {
      print "Line coverage: no coverable lines reported; the coverage gate cannot pass."
      exit 1
    }
    failed = 0
    for (file in found) {
      if (found[file] == 0) continue
      percent = 100 * hit[file] / found[file]
      if (percent + 0.000001 < file_minimum) {
        printf "File below %g%%: %s %.2f%% (%d/%d)\n", file_minimum, file, percent, hit[file], found[file]
        failed = 1
      }
    }
    percent = 100 * hits / lines
    printf "Line coverage: %.2f%% (%d/%d); minimum %g%% total and %g%% per file; target 100%%.\n", \
      percent, hits, lines, minimum, file_minimum
    if (percent + 0.000001 < minimum) failed = 1
    exit failed
  }
' "$report" | tee "$summary"
