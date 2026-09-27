#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
set -euo pipefail

# Require every instrumented Rust source line to be covered. Compare integer
# counts so a displayed percentage rounded to 100% cannot hide a missed line.
report_dir=target/coverage
report="$report_dir/lcov.info"
summary="$report_dir/summary.txt"
mkdir -p "$report_dir"
rm -f -- "$report" "$summary"

cargo llvm-cov --workspace --all-features --locked --lcov --output-path "$report"

if [[ ! -s "$report" ]]; then
  printf 'Coverage tool did not produce a nonempty LCOV report.\n' | tee "$summary" >&2
  exit 1
fi

# Count the actual source-line DA records. With generic async Rust functions,
# LLVM's LF/LH summary can include extra uncovered instantiation lines that
# have no DA record or uncovered line in the annotated source report. Dedup
# repeated source-line records before applying the exact coverage gate.
awk -F: -v root="$PWD/" '
  /^SF:/ {
    file = substr($0, 4)
    if (index(file, root) == 1) file = substr(file, length(root) + 1)
  }
  /^DA:/ {
    parts = split(substr($0, 4), data, ",")
    if (file == "" || parts < 2 || data[1] !~ /^[0-9]+$/ || data[2] !~ /^[0-9]+$/) {
      malformed = 1
      next
    }
    key = file SUBSEP data[1]
    if (!(key in seen)) {
      seen[key] = 1
      found[file]++
      lines++
    }
    if (data[2] > 0 && !(key in covered)) {
      covered[key] = 1
      hit[file]++
      hits++
    }
  }
  END {
    if (malformed || lines == 0) {
      print "Line coverage: malformed or empty DA records; the coverage gate cannot pass."
      exit 1
    }
    failed = 0
    for (file in found) {
      if (found[file] == 0) continue
      if (hit[file] != found[file]) {
        percent = 100 * hit[file] / found[file]
        printf "File below 100%%: %s %.2f%% (%d/%d)\n", file, percent, hit[file], found[file]
        failed = 1
      }
    }
    percent = 100 * hits / lines
    printf "Line coverage: %.2f%% (%d/%d); required 100%% total and per file.\n", \
      percent, hits, lines
    if (hits != lines) failed = 1
    exit failed
  }
' "$report" | tee "$summary"
