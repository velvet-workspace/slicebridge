"""Validate the version/changelog contract without a release or internet access."""
from __future__ import annotations
import json
import re
import tempfile
import unittest
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
from check_versioning import validate

ROOT = Path(__file__).resolve().parents[1]

class VersionContract(unittest.TestCase):
    def test_current_commit_milestone_consistent(self):
        self.assertEqual(validate(ROOT), "0.4.0-beta.6.dev.3")

    def test_native_app_version_matches_root_version(self):
        source = (ROOT / "native" / "updates_bridge.go").read_text(encoding="utf-8")
        match = re.search(r'(?m)^var appVersion\s*=\s*"([^"]+)"', source)
        self.assertIsNotNone(match, "native appVersion must be discoverable")
        self.assertEqual(match.group(1), validate(ROOT), "About/GetVersion must match VERSION")

    def test_wails_bundle_metadata_matches_version_family(self):
        config = json.loads((ROOT / "native" / "wails.json").read_text(encoding="utf-8"))
        info = config["info"]
        self.assertEqual(info["productVersion"], validate(ROOT).split("-")[0])
        self.assertNotRegex(info.get("comments", ""), r"(?i)\bbeta\s*\d+\b",
                            "Wails bundle description must not advertise an obsolete beta")

    def test_invalid_versions_and_changelog_mismatches_fail(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "CHANGELOG.md").write_text(
                "# Changelog\n\n## [0.4.0-beta.4.dev.0] — development not released\n\n- Added tracking.\n",
                encoding="utf-8")
            for bad in ("1.0", "1.02.0", "1.0.0-beta.01", "not-a-version", "1.0.0\n2.0.0"):
                with self.subTest(bad=bad):
                    (root / "VERSION").write_text(bad + "\n", encoding="utf-8")
                    with self.assertRaises(ValueError):
                        validate(root)
            (root / "VERSION").write_text("0.4.0-beta.4.dev.0\n", encoding="utf-8")
            self.assertEqual(validate(root), "0.4.0-beta.4.dev.0")
            (root / "CHANGELOG.md").write_text("# Changelog\n\n## Unreleased\n", encoding="utf-8")
            with self.assertRaises(ValueError):
                validate(root)

if __name__ == "__main__":
    unittest.main()
