#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Wait for a rooted test emulator, allowing bounded adbd restart races."""
import subprocess
import time

# The Android shell domain cannot create the hard links used by portable tests.
# A root request may disconnect before replying; verify the resulting identity.
for attempt in range(3):
    try:
        subprocess.run(['adb', 'wait-for-device'], check=True, timeout=30)
        subprocess.run(['adb', 'root'], check=False, timeout=15)
        subprocess.run(['adb', 'wait-for-device'], check=True, timeout=30)
        identity = subprocess.run(
            ['adb', 'shell', 'id', '-u'], check=True, timeout=15,
            capture_output=True, text=True,
        )
        if identity.stdout.strip() == '0':
            subprocess.run(['adb', 'shell', 'id'], check=True, timeout=15)
            break
        print('Android test emulator is not root yet', flush=True)
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as error:
        print(f'Android readiness attempt {attempt + 1}/3 failed: {error}', flush=True)
    if attempt == 2:
        raise SystemExit('Android test emulator did not become ready as root')
    time.sleep(2)
