#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Fail when a tracked Markdown file links to a repository path that is absent.

Only relative link targets are checked (inline `[text](target)` and reference
definitions `[label]: target`); URLs, mail links and same-page anchors are
skipped, and a `#fragment` is ignored. Fenced code blocks are not scanned.
"""
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
INLINE = re.compile(r"\]\(\s*<?([^)\s>]+)>?(?:\s+\"[^\"]*\")?\s*\)")
DEFINITION = re.compile(r"^\s{0,3}\[[^\]]+\]:\s*<?(\S+?)>?(?:\s+.*)?$")
EXTERNAL = re.compile(r"^(?:[a-zA-Z][a-zA-Z0-9+.-]*:|#|//)")


def targets(text):
    fenced = False
    for number, line in enumerate(text.splitlines(), 1):
        if line.lstrip().startswith(("```", "~~~")):
            fenced = not fenced
            continue
        if fenced:
            continue
        found = INLINE.findall(line)
        definition = DEFINITION.match(line)
        if definition:
            found.append(definition.group(1))
        for target in found:
            yield number, target


def broken(path):
    for number, target in targets(path.read_text(encoding="utf-8")):
        if EXTERNAL.match(target):
            continue
        relative = target.split("#", 1)[0]
        if relative and not (path.parent / relative).exists():
            yield number, target


def main():
    files = subprocess.run(["git", "ls-files", "-z", "*.md"], cwd=ROOT, check=True,
                           capture_output=True).stdout.decode().split("\0")
    failures = [f"{name}:{number}: {target}" for name in filter(None, files)
                for number, target in broken(ROOT / name)]
    print("\n".join(failures) or "All relative Markdown links resolve.")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
