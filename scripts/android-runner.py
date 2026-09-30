#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Cargo test runner for an already booted, adb-connected Android emulator."""
from pathlib import Path
import shlex
import subprocess
import sys

binary = Path(sys.argv[1])
remote = '/data/local/tmp/caj2pdf-tests/' + binary.name
subprocess.run(['adb', 'shell', 'mkdir', '-p', '/data/local/tmp/caj2pdf-tests'], check=True)
subprocess.run(['adb', 'push', str(binary), remote], check=True)
subprocess.run(['adb', 'shell', 'chmod', '755', remote], check=True)
command = 'TMPDIR=/data/local/tmp CAJ2PDF_TEST_BINARY=/data/local/tmp/caj2pdf ' + shlex.join([remote, *sys.argv[2:]])
sys.exit(subprocess.run(['adb', 'shell', command]).returncode)
