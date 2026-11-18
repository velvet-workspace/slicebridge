"""Offline contract tests for the privileged CI publisher."""
import importlib.util
import json
import pathlib
import re
import unittest
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parents[1]
PATH = ROOT / "tools" / "publish_ci_artifacts.py"
SPEC = importlib.util.spec_from_file_location("ci_archive", PATH)
module = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(module)

SHA = "9135136096b9473a5a2eff6b303c638cd7bbc54c"
REPO = "velvet-workspace/slicebridge"


def sample_run(**kwargs):
    run = {"id": 43676877894, "run_attempt": 1,
           "name": "Native WebView Windows and macOS",
           "head_repository": {"full_name": REPO},
           "status": "completed", "conclusion": "failure", "head_sha": SHA,
           "head_branch": "feat/rust-3mf",
           "created_at": "2026-11-15T04:00:00Z"}
    run.update(kwargs)
    return run


class TestCIReleaseArchive(unittest.TestCase):
    def test_per_attempt_tag_and_invalid_source(self):
        r = sample_run()
        self.assertEqual(module.tag_for_run(r), "ci-run-43676877894-a1-9135136096b9")
        self.assertEqual(module.tag_for_sha(SHA), "ci-build-9135136096b9473a")
        self.assertNotEqual(module.tag_for_run(r),
                            module.tag_for_run(sample_run(run_attempt=2)))
        self.assertNotEqual(module.tag_for_run(r),
                            module.tag_for_run(sample_run(id=43676877895)))
        with self.assertRaises(ValueError):
            module.tag_for_run(sample_run(head_sha="bad"))
        with self.assertRaises(ValueError):
            module.tag_for_run(sample_run(id=0))

    def test_reject_forks_but_preserve_failed_build_artifacts(self):
        self.assertTrue(module.eligible(sample_run(), REPO))
        self.assertFalse(module.eligible(sample_run(
            head_repository={"full_name": "attacker/fork"}), REPO))
        self.assertFalse(module.eligible(sample_run(name="CodeQL"), REPO))
        self.assertFalse(module.eligible(sample_run(status="in_progress"), REPO))

    def test_all_artifact_build_workflows_subscribed(self):
        """Fail automatically if a new upload-artifact builder is not covered."""
        builders = set()
        for path in sorted((ROOT / ".github" / "workflows").glob("*.yml")) + sorted((ROOT / ".github" / "workflows").glob("*.yaml")):
            content = path.read_text(encoding="utf-8")
            if not re.search(r"uses:\s*actions/upload-artifact@", content):
                continue
            if path.name in module.EVIDENCE_ONLY_WORKFLOW_FILES:
                self.assertIn("evidence", content.lower())
                continue
            match = re.search(r"(?m)^name:\s*(.*)$", content)
            self.assertIsNotNone(match, path.name)
            builders.add(match.group(1).strip())
        self.assertEqual(builders, module.ALLOWED_WORKFLOWS)
        subscriber = (ROOT / ".github/workflows/auto-release-builds.yml"
                      ).read_text(encoding="utf-8")
        for name in builders:
            self.assertIn("      - " + name, subscriber)

    def test_remote_asset_mismatch_fails_closed(self):
        self.assertFalse(module.verify_asset(None, "test.zip", "a" * 64, 10))
        good = {"name": "test.zip", "size": 10, "digest": "sha256:" + "a" * 64}
        self.assertTrue(module.verify_asset(good, "test.zip", "a" * 64, 10))
        for bad in [
            {"name": "test.zip", "size": 9, "digest": good["digest"]},
            {"name": "test.zip", "size": 10, "digest": "sha256:" + "b" * 64},
            {"name": "other.zip", "size": 10, "digest": good["digest"]},
        ]:
            with self.assertRaises(RuntimeError):
                module.verify_asset(bad, "test.zip", "a" * 64, 10)

    def test_reuse_legacy_archive_only_with_same_artifact_id(self):
        art = {"id": 14147881360, "size_in_bytes": 33}
        legacy = {
            "tag_name": "ci-build-9135136096b9473a",
            "target_commitish": SHA, "draft": False, "prerelease": True,
            "assets": [{"name": "CI-artifact-14147881360.zip"}],
        }
        with mock.patch.object(module, "_release_by_tag", return_value=None), \
                mock.patch.object(module, "github_json", return_value=[legacy]):
            self.assertEqual(module.find_release(REPO, sample_run(), [art]), legacy)
            self.assertIsNone(module.find_release(
                REPO, sample_run(), [{"id": 999}]))
        bad = dict(legacy, target_commitish="a" * 40)
        with mock.patch.object(module, "_release_by_tag", return_value=None), \
                mock.patch.object(module, "github_json", return_value=[bad]):
            with self.assertRaises(RuntimeError):
                module.find_release(REPO, sample_run(), [art])

    def test_manifest_contains_failure_and_verified_asset_receipts(self):
        captured = {}
        def fake_upload(repo, tag, path):
            captured[path.name] = path.read_bytes()
            return {"name": path.name,
                    "sha256": module.hashlib.sha256(captured[path.name]).hexdigest(),
                    "size": len(captured[path.name])}
        with mock.patch.object(module, "upload_and_verify", side_effect=fake_upload):
            receipt = module.archive_run_manifest(REPO, "testtag", sample_run(), [
                {"id": 77, "original_name": "Windows",
                 "archive": {"name": "CI-artifact-77.zip", "size": 5,
                             "sha256": "f" * 64}}],
                [{"id": 78, "reason": "expired"}])
        manifest = json.loads(captured[receipt["manifest"]["name"]])
        self.assertEqual(manifest["run_conclusion"], "failure")
        self.assertEqual(manifest["source_commit_sha"], SHA)
        self.assertEqual(manifest["verified_original_artifacts"][0]["id"], 77)
        self.assertEqual(manifest["unavailable_original_artifacts"][0]["id"], 78)
        self.assertFalse(manifest["production_approved"])
        self.assertIn(receipt["manifest"]["sha256"].encode(),
                      captured[receipt["checksum"]["name"]])

    def test_rerun_never_rearchives_previous_attempt_artifacts(self):
        rerun = sample_run(run_attempt=2, run_started_at="2026-11-15T07:20:00Z")
        original = {"id": 1, "created_at": "2026-11-15T07:19:59Z"}
        retry = {"id": 2, "created_at": "2026-11-15T07:20:01Z"}
        unstamped = {"id": 3}
        self.assertEqual(module.artifacts_for_attempt(rerun,
                         [original, retry, unstamped]), [retry])
        self.assertEqual(module.artifacts_for_attempt(sample_run(),
                         [original, retry]), [original, retry])
        with self.assertRaises(RuntimeError):
            module.artifacts_for_attempt(sample_run(run_attempt=2), [retry])

    def test_safe_branch_and_label(self):
        self.assertEqual(module.safe_branch("feature/test\nmalicious"),
                         "feature/test-malicious")
        self.assertEqual(module.safe_label("foo\nmalicious"), "foo-malicious")

    def test_expired_artifacts_not_reported_as_archived(self):
        with mock.patch.object(module, "github_json",
                               side_effect=[sample_run(),
                                            {"artifacts": [{"id": 90, "expired": True}]}]), \
                mock.patch.object(module, "find_release",
                                  return_value={"tag_name": "testtag"}), \
                mock.patch.object(module, "archive_run_manifest",
                                  return_value={"manifest": {"name": "manifest"}}), \
                mock.patch.object(module, "append_evidence"), \
                mock.patch("sys.argv", ["ci", "--repo", REPO,
                                        "--run-id", "43676877894"]):
            with self.assertRaisesRegex(RuntimeError, "unavailable"):
                module.main()


if __name__ == "__main__":
    unittest.main()
