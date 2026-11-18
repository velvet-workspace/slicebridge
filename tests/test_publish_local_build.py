"""Local experimental build release provenance contract tests."""
import importlib.util
import json
import pathlib
import tempfile
import unittest
from unittest import mock

PATH = pathlib.Path(__file__).resolve().parents[1] / "tools" / "publish_local_build.py"
SPEC = importlib.util.spec_from_file_location("local_build", PATH)
import sys
sys.path.insert(0, str(PATH.parent))
module = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(module)

SHA = "9135136096b9473a5a2eff6b303c638cd7bbc54c"
REPO = "velvet-workspace/slicebridge"


class TestLocalBuildPublisher(unittest.TestCase):
    def test_tag_and_timestamp(self):
        self.assertEqual(module.local_tag("mac-01", SHA),
                         "local-build-mac-01-9135136096b9")
        with self.assertRaises(ValueError):
            module.local_tag("bad/path", SHA)
        self.assertEqual(module.validate_build_at("2026-11-15T08:00:00+02:00"),
                         "2026-11-15T06:00:00Z")
        with self.assertRaises(ValueError):
            module.validate_build_at("2026-11-15T08:00:00")

    def test_reject_empty_and_duplicated_files(self):
        with tempfile.TemporaryDirectory() as d:
            base = pathlib.Path(d)
            a = base / "native.zip"
            a.write_bytes(b"test")
            empty = base / "empty.zip"
            empty.touch()
            self.assertEqual(module.validate_files([str(a)]), [a.resolve()])
            with self.assertRaises(ValueError):
                module.validate_files([str(a), str(a)])
            with self.assertRaises(ValueError):
                module.validate_files([str(empty)])
            link = base / "linked.zip"
            link.symlink_to(a)
            with self.assertRaises(ValueError):
                module.validate_files([str(link)])

    def test_source_requires_clean_worktree_and_verified_remote(self):
        with tempfile.TemporaryDirectory() as folder:
            root = pathlib.Path(folder).resolve()
            def fake_git(_source, *args):
                key = args[0]
                if key == "rev-parse":
                    return str(root) if args[1] == "--show-toplevel" else SHA
                return {
                    "remote": f"https://github.com/{REPO}.git",
                    "status": "",
                    "cat-file": "commit",
                    "branch": "feature/safe",
                }[key]
            with mock.patch.object(module, "git", side_effect=fake_git), \
                    mock.patch.object(module.ci, "github_json",
                                      return_value={"sha": SHA}):
                result = module.source_provenance(REPO, root)
                self.assertTrue(result["source_worktree_clean"])
                self.assertTrue(result["source_commit_verified_on_github"])
            def dirty_git(_source, *args):
                if args[0] == "status":
                    return " M source.py"
                return fake_git(_source, *args)
            with mock.patch.object(module, "git", side_effect=dirty_git):
                with self.assertRaises(ValueError):
                    module.source_provenance(REPO, root)

    def test_dry_run_exposes_manifest_without_release_mutation(self):
        with tempfile.TemporaryDirectory() as folder:
            asset = pathlib.Path(folder) / "SliceBridge.exe"
            asset.write_bytes(b"verified fixture")
            src = pathlib.Path(folder)
            provenance = {
                "source_commit_sha": SHA, "source_branch": "feature/safe",
                "source_origin": f"https://github.com/{REPO}.git",
                "source_worktree_clean": True,
                "source_commit_verified_on_github": True,
            }
            with mock.patch.object(module, "source_provenance",
                                   return_value=provenance), \
                    mock.patch.object(module.ci, "_release_by_tag",
                                      return_value=None), \
                    mock.patch.object(module.ci, "exec_gh") as gh, \
                    mock.patch("builtins.print") as output:
                tag = module.publish(REPO, "local01", "2026-11-15T08:00:00+02:00",
                                     "Chris", "windows", src, [asset], True)
                gh.assert_not_called()
                data = json.loads(output.call_args[0][0])
                self.assertEqual(data["release_tag"], tag)
                self.assertEqual(data["artifacts"][0]["size"], len(b"verified fixture"))
                self.assertFalse(data["production_approved"])


if __name__ == "__main__":
    unittest.main()
