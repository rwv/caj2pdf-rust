# SPDX-License-Identifier: MIT
"""Retain a bounded Wine relay stream; report every dropped byte."""
import json, re, sys, time
from pathlib import Path

meta = Path('/output/relay-filter.json')
cap = 16 * 1024 * 1024
seen = written = chunks = selected = selected_bytes = dropped_selected = 0
pattern = re.compile(
    rb":(?:Call|Ret  ?) gdi32\.|:trace:font:(?:NtGdiHfontCreate|NtGdiGetGlyphOutline)\b"
)
for line in iter(lambda: sys.stdin.buffer.readline(8192), b''):
    seen += len(line)
    chunks += 1
    if pattern.search(line):
        selected += 1
        selected_bytes += len(line)
        if written + len(line) <= cap:
            sys.stdout.buffer.write(line)
            sys.stdout.buffer.flush()
            written += len(line)
        else:
            dropped_selected += len(line)
    if chunks % 128 == 0:
        meta.write_text(json.dumps(dict(
            input_bytes=seen, retained_bytes=written,
            filtered_bytes=seen-selected_bytes, selected_lines=selected,
            selected_bytes=selected_bytes, dropped_selected_bytes=dropped_selected,
            limit_bytes=cap, chunks=chunks, updated_unix=time.time()
        )) + '\n')
meta.write_text(json.dumps(dict(
    input_bytes=seen, retained_bytes=written,
    filtered_bytes=seen-selected_bytes, selected_lines=selected,
    selected_bytes=selected_bytes, dropped_selected_bytes=dropped_selected,
    limit_bytes=cap, chunks=chunks, eof=True, updated_unix=time.time()
)) + '\n')
