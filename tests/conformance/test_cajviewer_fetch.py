# SPDX-License-Identifier: MIT
"""Network-free fetch failure tests using original synthetic bytes."""
import hashlib
import io
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
import cajviewer_canary as canary
import fetch_cajviewer as fetcher


class FetchTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.path = Path(self.directory.name) / "viewer.deb"
        self.payload = b"original synthetic installer stand-in"
        for module, name, value in (
            (canary, "INSTALLER_SIZE", len(self.payload)),
            (canary, "INSTALLER_SHA256", hashlib.sha256(self.payload).hexdigest()),
            (fetcher, "INSTALLER_SIZE", len(self.payload)),
        ):
            context = patch.object(module, name, value)
            context.start()
            self.addCleanup(context.stop)

    def test_download_then_cache_without_network(self):
        with patch.object(fetcher.urllib.request, "urlopen", return_value=io.BytesIO(self.payload)) as request:
            report = fetcher.fetch(self.path)
            request.assert_called_once_with(fetcher.MIRROR_URL, timeout=30)
        self.assertFalse(report["cached"])
        self.assertEqual(report["vendor_passes"], 0)
        self.assertEqual(self.path.read_bytes(), self.payload)
        with patch.object(fetcher.urllib.request, "urlopen", side_effect=AssertionError("network")):
            self.assertTrue(fetcher.fetch(self.path)["cached"])
        self.assertEqual(list(self.path.parent.iterdir()), [self.path])

    def test_corrupt_cache_is_not_overwritten(self):
        self.path.write_bytes(b"x" * len(self.payload))
        with patch.object(fetcher.urllib.request, "urlopen", side_effect=AssertionError("network")):
            with self.assertRaises(canary.CanaryError):
                fetcher.fetch(self.path)
        self.assertEqual(self.path.read_bytes(), b"x" * len(self.payload))

    def test_bad_downloads_leave_no_output_or_temporary_file(self):
        for payload in (b"short", b"x" * len(self.payload), self.payload + b"extra"):
            with self.subTest(payload=payload):
                with patch.object(fetcher.urllib.request, "urlopen", return_value=io.BytesIO(payload)):
                    with self.assertRaises(canary.CanaryError):
                        fetcher.fetch(self.path)
                self.assertEqual(list(self.path.parent.iterdir()), [])

    def test_network_failure_cleans_temporary_file(self):
        with patch.object(fetcher.urllib.request, "urlopen", side_effect=OSError("offline")):
            with self.assertRaises(OSError):
                fetcher.fetch(self.path)
        self.assertEqual(list(self.path.parent.iterdir()), [])

    def test_interrupted_body_cleans_temporary_file(self):
        class Interrupted(io.BytesIO):
            def read(self, size):
                if self.tell():
                    raise OSError("connection interrupted")
                return super().read(4)
        with patch.object(fetcher.urllib.request, "urlopen", return_value=Interrupted(self.payload)):
            with self.assertRaises(OSError):
                fetcher.fetch(self.path)
        self.assertEqual(list(self.path.parent.iterdir()), [])

    def test_repository_destination_rejected(self):
        with self.assertRaises(canary.CanaryError):
            fetcher.fetch(Path(fetcher.__file__).resolve().parents[1] / "viewer.deb")

    def test_concurrent_destination_not_overwritten(self):
        real_link = fetcher.os.link
        def concurrent(source, destination):
            destination.write_bytes(b"another writer")
            return real_link(source, destination)
        with patch.object(fetcher.urllib.request, "urlopen", return_value=io.BytesIO(self.payload)):
            with patch.object(fetcher.os, "link", side_effect=concurrent):
                with self.assertRaises(FileExistsError):
                    fetcher.fetch(self.path)
        self.assertEqual(self.path.read_bytes(), b"another writer")
        self.assertEqual(list(self.path.parent.iterdir()), [self.path])
