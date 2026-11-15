"""Offline unit checks for the default-branch CI archival publisher."""
import importlib.util
import pathlib
import unittest

PATH = pathlib.Path(__file__).resolve().parents[1] / "tools" / "publish_ci_artifacts.py"
SPEC = importlib.util.spec_from_file_location("ci_archive", PATH)
module = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(module)


class TestCIReleaseArchive(unittest.TestCase):
    def test_deterministic_tag_per_immutable_sha(self):
        sha = "9135136096b9473a5a2eff6b303c638cd7bbc54c"
        self.assertEqual(module.tag_for_sha(sha), "ci-build-9135136096b9473a")
        with self.assertRaises(ValueError):
            module.tag_for_sha("not-a-sha")
        with self.assertRaises(ValueError):
            module.tag_for_sha("A" * 40)

    def test_reject_external_or_unapproved_workflow(self):
        run = {"name": "Native WebView Windows and macOS", "head_repository": {"full_name": "velvet-workspace/slicebridge"}, "status": "completed"}
        self.assertTrue(module.eligible(run, "velvet-workspace/slicebridge"))
        run["head_repository"]["full_name"] = "attacker/fork"
        self.assertFalse(module.eligible(run, "velvet-workspace/slicebridge"))
        run["head_repository"]["full_name"] = "velvet-workspace/slicebridge"
        run["name"] = "CodeQL"
        self.assertFalse(module.eligible(run, "velvet-workspace/slicebridge"))
        run["name"] = "Windows portable beta"
        run["status"] = "in_progress"
        self.assertFalse(module.eligible(run, "velvet-workspace/slicebridge"))

    def test_remote_asset_mismatch_fails_closed(self):
        self.assertFalse(module.verify_asset(None, "test.zip", "a" * 64, 10))
        good = {"size": 10, "digest": "sha256:" + "a" * 64}
        self.assertTrue(module.verify_asset(good, "test.zip", "a" * 64, 10))
        with self.assertRaises(RuntimeError):
            module.verify_asset({"size": 9, "digest": good["digest"]}, "test.zip", "a" * 64, 10)
        with self.assertRaises(RuntimeError):
            module.verify_asset({"size": 10, "digest": "sha256:" + "b" * 64}, "test.zip", "a" * 64, 10)

    def test_sanitize_untrusted_branch_metadata(self):
        self.assertEqual(module.safe_branch("feature/test\nmalicious"), "feature/test-malicious")
        self.assertEqual(module.safe_branch(""), "unknown")


if __name__ == "__main__":
    unittest.main()
