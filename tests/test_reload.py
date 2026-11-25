"""A failed live reload must leave the previous selection and rules reachable."""
from pathlib import Path
import os
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
import runtime
from kokoro_i18n import KokoroError


class ReloadTest(unittest.TestCase):
    def test_failed_reload_restores_live_and_saved_profile(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            store = root / "yamls/.kokoro"
            old = store / "profile-old"
            old.mkdir(parents=True)
            (old / "config.yaml").write_bytes(b"rules: [MATCH,DIRECT]")
            (store / "current").symlink_to(old.name)
            raw = root / "yamls/config.yaml"
            raw.symlink_to(".kokoro/current/config.yaml")
            runtime_file = root / "running.yaml"
            runtime_file.write_bytes(b"previous runtime")
            stage = store / "profile-new"
            stage.mkdir()
            (stage / "config.yaml").write_bytes(b"rules: []")
            with (patch.object(runtime, "ROOT", root), patch.object(runtime, "RUN", root),
                  patch.object(runtime, "CONFIG", runtime_file),
                  patch.object(runtime, "settings", return_value={"port": 7890, "controller": 9999,
                                                                "secret": "test", "transparent": False}),
                  patch.object(runtime, "validate"), patch.object(runtime, "running", return_value=True),
                  patch.object(runtime, "reload_config", side_effect=[OSError(), None]) as reload):
                with self.assertRaises(KokoroError):
                    runtime.apply_profile(stage)
            self.assertEqual(reload.call_args_list[-1].args, (b"previous runtime",))
            self.assertEqual(os.readlink(store / "current"), old.name)
            self.assertEqual(raw.read_bytes(), b"rules: [MATCH,DIRECT]")
            self.assertEqual(runtime_file.read_bytes(), b"previous runtime")

    def test_failed_link_publication_can_reuse_staging_link_for_rollback(self):
        with tempfile.TemporaryDirectory() as tmp:
            stage = Path(tmp)
            current = stage / "current"
            current.symlink_to("old")
            occupied = stage / "directory"
            occupied.mkdir()
            runtime.replace_link(current, "new", stage)
            with self.assertRaises(OSError):
                runtime.replace_link(occupied, "current/config.yaml", stage)
            runtime.replace_link(current, "old", stage)
            self.assertEqual(os.readlink(current), "old")


if __name__ == "__main__":
    unittest.main()
