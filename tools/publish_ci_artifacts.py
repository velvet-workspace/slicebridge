#!/usr/bin/env python3
"""Archive each trusted internal CI build attempt in an immutable experimental Release.

Run from a trusted default-branch workflow_run context. Do not execute or extract
triggering artifacts. Retain the original Actions ZIP bytes in GitHub Releases.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import tempfile
import zipfile
from datetime import datetime, timezone
from pathlib import Path

ALLOWED_WORKFLOWS = {
    "Native WebView Windows and macOS",
    "Windows portable beta",
    "Printer profile registry QA",
}
# This is evidence, not a built distribution. Any NEW artifact-producing
# workflow must be classified by the coverage test before CI can pass.
EVIDENCE_ONLY_WORKFLOW_FILES = {"release-preflight.yml"}


def github_json(endpoint: str) -> dict | list:
    result = subprocess.run(["gh", "api", endpoint], capture_output=True, check=True)
    return json.loads(result.stdout)


def exec_gh(*args: str) -> str:
    result = subprocess.run(["gh", *args], text=True, capture_output=True, check=True)
    return result.stdout.strip()


def require_sha(sha: str) -> str:
    if not isinstance(sha, str) or not re.fullmatch(r"[0-9a-f]{40}", sha):
        raise ValueError("Expected immutable lowercase 40-character source commit SHA")
    return sha


def tag_for_sha(sha: str) -> str:
    """Historical per-commit tag, read-only compatibility with first publisher."""
    return "ci-build-" + require_sha(sha)[:16]


def tag_for_run(run: dict) -> str:
    """Distinct tags for separate runs AND attempts of a source commit."""
    sha = require_sha(run["head_sha"])
    run_id = int(run["id"])
    attempt = int(run.get("run_attempt") or 1)
    if run_id < 1 or attempt < 1:
        raise ValueError("Invalid workflow run ID or attempt")
    return f"ci-run-{run_id}-a{attempt}-{sha[:12]}"


def safe_branch(raw: str) -> str:
    return re.sub(r"[^A-Za-z0-9._/-]", "-", raw or "unknown")[:72]


def safe_label(raw: str) -> str:
    # Untrusted artifact names can contain newlines or Markdown.
    return re.sub(r"[^A-Za-z0-9._/-]", "-", str(raw or "unknown"))[:100]


def eligible(run: dict, repo: str) -> bool:
    head = run.get("head_repository") or {}
    return (run.get("name") in ALLOWED_WORKFLOWS
            and head.get("full_name") == repo
            and run.get("status") == "completed"
            and isinstance(run.get("head_sha"), str)
            and bool(re.fullmatch(r"[0-9a-f]{40}", run["head_sha"])))


def all_artifacts(repo: str, run_id: int) -> list[dict]:
    items = []
    for page in range(1, 30):
        obj = github_json(
            f"repos/{repo}/actions/runs/{run_id}/artifacts?per_page=100&page={page}")
        batch = obj.get("artifacts", [])
        items.extend(batch)
        if len(batch) < 100:
            return items
    raise RuntimeError("Artifact listing exceeded 29 pages")


def artifacts_for_attempt(run: dict, artifacts: list[dict]) -> list[dict]:
    """On reruns, exclude artifacts produced by previous attempts of this run.

    GitHub's run-wide artifact listing has no attempt filter, so use the
    current attempt's run_started_at using GitHub UTC timestamps without a backward grace period.
    An artifact without a creation timestamp is unsafe to attribute to a
    rerun and is excluded, not silently assigned to the latest attempt.
    """
    attempt = int(run.get("run_attempt") or 1)
    if attempt <= 1:
        return artifacts
    started = run.get("run_started_at")
    if not started:
        raise RuntimeError("Rerun lacks run_started_at; cannot distinguish artifacts")
    start = datetime.fromisoformat(started.replace("Z", "+00:00"))
    if start.tzinfo is None:
        raise RuntimeError("Rerun timestamp is not timezone-aware")
    floor = start.astimezone(timezone.utc)
    result = []
    for artifact in artifacts:
        created = artifact.get("created_at")
        if not created:
            continue
        stamp = datetime.fromisoformat(created.replace("Z", "+00:00"))
        if stamp.tzinfo is None:
            continue
        if stamp.astimezone(timezone.utc) >= floor:
            result.append(artifact)
    return result


def _validate_release(release: dict, sha: str) -> dict:
    if (release.get("target_commitish") != sha or not release.get("prerelease")
            or release.get("draft")):
        raise RuntimeError(
            f"Unsafe existing Release {release.get('tag_name')}: "
            "source SHA/prerelease/draft mismatch")
    return release


def _release_by_tag(repo: str, tag: str) -> dict | None:
    try:
        return github_json(f"repos/{repo}/releases/tags/{tag}")
    except subprocess.CalledProcessError as exc:
        stderr = exc.stderr or b""
        if isinstance(stderr, str):
            stderr = stderr.encode()
        if b"404" not in stderr and b"Not Found" not in stderr:
            raise
        return None


def find_release(repo: str, run: dict, artifacts: list[dict]) -> dict | None:
    """New builds: per-run Release. Old already-archived IDs: reuse, never duplicate.

    Legacy source-commit Releases are consulted only when they already contain
    at least one of this exact run's immutable artifact IDs.
    """
    sha = require_sha(run["head_sha"])
    current = _release_by_tag(repo, tag_for_run(run))
    if current:
        return _validate_release(current, sha)
    names = {f"CI-artifact-{int(a['id'])}.zip" for a in artifacts}
    for page in range(1, 40):
        rows = github_json(f"repos/{repo}/releases?per_page=100&page={page}")
        for release in rows:
            if any(a.get("name") in names for a in release.get("assets", [])):
                return _validate_release(release, sha)
        if len(rows) < 100:
            return None
    raise RuntimeError("Legacy Release search exceeded 39 pages")


def _run_details(run: dict, repo: str) -> str:
    return (
        f"- Source SHA: \x60{require_sha(run['head_sha'])}\x60\n"
        f"- Build branch: \x60{safe_branch(run.get('head_branch') or '')}\x60\n"
        f"- Workflow: {safe_label(run.get('name'))}\n"
        f"- Run ID/attempt: \x60{run['id']}\x60 / \x60{run.get('run_attempt') or 1}\x60\n"
        f"- Build start UTC: \x60{run.get('run_started_at') or run.get('created_at') or 'unknown'}\x60\n"
        f"- Workflow conclusion: \x60{safe_label(run.get('conclusion') or 'unknown')}\x60\n"
        f"- Workflow evidence: https://github.com/{repo}/actions/runs/{run['id']}\n"
    )


def create_release(repo: str, run: dict) -> dict:
    sha = require_sha(run["head_sha"])
    tag = tag_for_run(run)
    notes = (
        "# SliceBridge automatic CI build archive\n\n"
        "**EXPERIMENTAL PRE-RELEASE — NOT PRODUCTION APPROVED.**\n"
        "A failed CI run can contain build artifacts: consult its conclusion. "
        "This archive does not authorize installation, deployment or printing.\n\n"
        + _run_details(run, repo) +
        "\nOriginal GitHub Actions artifact ZIP bytes are retained, not executed. "
        "Each is verified for download size, ZIP CRC and SHA-256 before upload. "
        "GitHub Release assets and a run-specific manifest are checked by readback. "
        "No code signing, OEM approval or printing validation is implied.\n"
    )
    with tempfile.TemporaryDirectory(prefix="slicebridge-release-notes-") as temp:
        filename = Path(temp) / "notes.md"
        filename.write_text(notes, encoding="utf-8")
        try:
            exec_gh(
                "release", "create", tag, "-R", repo, "--target", sha,
                "--prerelease", "--latest=false",
                "--title", f"SliceBridge experimental CI run {run['id']} "
                           f"attempt {run.get('run_attempt') or 1} | {sha[:12]}",
                "--notes-file", str(filename))
        except subprocess.CalledProcessError:
            # A concurrent writer may have created the exact tag; fail closed
            # on unexpected remote state rather than overwriting.
            existing = _release_by_tag(repo, tag)
            if existing is None:
                raise
            return _validate_release(existing, sha)
    created = _release_by_tag(repo, tag)
    if created is None:
        raise RuntimeError("Created Release missing from GitHub readback")
    return _validate_release(created, sha)


def get_sha(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(4 * 1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def verify_asset(existing: dict | None, name: str, digest: str, size: int) -> bool:
    if existing is None:
        return False
    if existing.get("name") != name and existing.get("name") is not None:
        raise RuntimeError("Mismatched GitHub Release asset name")
    if existing.get("size") != size or existing.get("digest") != f"sha256:{digest}":
        raise RuntimeError(f"Remote asset content mismatch for {name}; no overwrite")
    return True


def upload_and_verify(repo: str, tag: str, path: Path) -> dict:
    digest = get_sha(path)
    size = path.stat().st_size
    before = _release_by_tag(repo, tag)
    if before is None:
        raise RuntimeError("Release disappeared before asset upload")
    old = {a["name"]: a for a in before.get("assets", [])}
    if not verify_asset(old.get(path.name), path.name, digest, size):
        exec_gh("release", "upload", tag, "-R", repo, str(path))
    after = _release_by_tag(repo, tag)
    if after is None:
        raise RuntimeError("Release missing after asset upload")
    assets = {a["name"]: a for a in after.get("assets", [])}
    if not verify_asset(assets.get(path.name), path.name, digest, size):
        raise RuntimeError(f"Asset {path.name} missing after upload")
    return {"name": path.name, "size": size, "sha256": digest}


def upload_artifact(repo: str, tag: str, artifact: dict) -> dict:
    aid = int(artifact["id"])
    name = f"CI-artifact-{aid}.zip"
    with tempfile.TemporaryDirectory(prefix="slicebridge-ci-artifact-") as temp:
        path = Path(temp) / name
        # Do not extract or execute untrusted artifacts.
        with path.open("wb") as f:
            subprocess.run(
                ["gh", "api", f"repos/{repo}/actions/artifacts/{aid}/zip"],
                stdout=f, stderr=subprocess.PIPE, check=True)
        if path.stat().st_size != int(artifact["size_in_bytes"]):
            raise RuntimeError(f"Artifact {aid} ZIP size mismatch")
        if not zipfile.is_zipfile(path):
            raise RuntimeError(f"Artifact {aid} is not a ZIP")
        with zipfile.ZipFile(path) as zipped:
            bad = zipped.testzip()
            if bad is not None:
                raise RuntimeError(f"Artifact {aid} ZIP CRC failed: {bad}")
        uploaded = upload_and_verify(repo, tag, path)
        checksum = Path(temp) / (name + ".sha256")
        checksum.write_text(f"{uploaded['sha256']}  {name}\n", encoding="utf-8")
        sidecar = upload_and_verify(repo, tag, checksum)
    return {
        "id": aid,
        "original_name": safe_label(artifact.get("name")),
        "archive": uploaded,
        "checksum": sidecar,
    }


def archive_run_manifest(
    repo: str, tag: str, run: dict, completed: list[dict], unavailable: list[dict]
) -> dict:
    manifest = {
        "schema": "slicebridge-ci-release-v2",
        "archive_class": "experimental",
        "production_approved": False,
        "repository": repo,
        "source_commit_sha": require_sha(run["head_sha"]),
        "branch": safe_branch(run.get("head_branch") or ""),
        "workflow": run.get("name"),
        "run_id": int(run["id"]),
        "run_attempt": int(run.get("run_attempt") or 1),
        "run_started_at_utc": run.get("run_started_at") or run.get("created_at"),
        "run_conclusion": run.get("conclusion"),
        "run_url": f"https://github.com/{repo}/actions/runs/{run['id']}",
        "pull_requests": [
            f"https://github.com/{repo}/pull/{int(pr['number'])}"
            for pr in (run.get("pull_requests") or [])
            if isinstance(pr.get("number"), int) and pr["number"] > 0
        ],
        "release_tag": tag,
        "verified_original_artifacts": completed,
        "unavailable_original_artifacts": unavailable,
    }
    basename = f"CI-run-{run['id']}-a{run.get('run_attempt') or 1}-manifest.json"
    with tempfile.TemporaryDirectory(prefix="slicebridge-ci-manifest-") as temp:
        path = Path(temp) / basename
        path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n",
                        encoding="utf-8")
        receipt = upload_and_verify(repo, tag, path)
        sums = Path(temp) / (basename + ".sha256")
        sums.write_text(f"{receipt['sha256']}  {basename}\n", encoding="utf-8")
        sidecar = upload_and_verify(repo, tag, sums)
    return {"manifest": receipt, "checksum": sidecar}


def append_evidence(
    repo: str, tag: str, run: dict, completed: list[dict],
    unavailable: list[dict], manifest: dict
) -> None:
    release = _release_by_tag(repo, tag)
    if release is None:
        raise RuntimeError("Release missing before note update")
    body = release.get("body") or ""
    marker = f"CI-run-{run['id']}-a{run.get('run_attempt') or 1}-manifest.json"
    if marker in body:
        return
    lines = ["", f"## Run {run['id']}, attempt {run.get('run_attempt') or 1}",
             _run_details(run, repo).rstrip(),
             f"- Verified run manifest: \x60{manifest['manifest']['name']}\x60",
             f"- Manifest checksum: \x60{manifest['manifest']['sha256']}\x60"]
    for item in completed:
        lines.append(
            f"- Archived \x60{item['original_name']}\x60, immutable ID "
            f"\x60{item['id']}\x60: \x60{item['archive']['name']}\x60 "
            f"({item['archive']['size']} bytes; SHA-256 "
            f"\x60{item['archive']['sha256']}\x60)")
    for item in unavailable:
        lines.append(f"- NOT ARCHIVED: artifact ID {item['id']} "
                     f"({safe_label(item['reason'])})")
    with tempfile.TemporaryDirectory(prefix="slicebridge-ci-notes-") as temp:
        path = Path(temp) / "notes.md"
        path.write_text(body + "\n" + "\n".join(lines) + "\n", encoding="utf-8")
        exec_gh("release", "edit", tag, "-R", repo, "--notes-file", str(path))
    final = _release_by_tag(repo, tag)
    if final is None or marker not in (final.get("body") or ""):
        raise RuntimeError("Release notes update readback failed")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", required=True)
    parser.add_argument("--run-id", type=int, required=True)
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    run = github_json(f"repos/{args.repo}/actions/runs/{args.run_id}")
    if not eligible(run, args.repo):
        print("SKIP: not a completed internal build workflow")
        return
    artifacts = artifacts_for_attempt(run, all_artifacts(args.repo, args.run_id))
    if not artifacts:
        print("NO DISTRIBUTABLE ARTIFACTS: CI outcome is not a build archive")
        return
    if args.dry_run:
        print(json.dumps({
            "repo": args.repo, "sha": run["head_sha"],
            "run_id": run["id"], "run_attempt": run.get("run_attempt") or 1,
            "new_tag_if_needed": tag_for_run(run),
            "artifacts": [a["id"] for a in artifacts],
        }, sort_keys=True))
        return
    release = find_release(args.repo, run, artifacts) or create_release(args.repo, run)
    tag = release["tag_name"]
    completed, unavailable = [], []
    for art in artifacts:
        if art.get("expired"):
            unavailable.append({"id": int(art["id"]), "reason": "Actions artifact expired"})
            print(f"UNAVAILABLE artifact {art['id']}: expired", flush=True)
            continue
        receipt = upload_artifact(args.repo, tag, art)
        completed.append(receipt)
        print(f"VERIFIED {art['id']} -> {tag}: {receipt['archive']['sha256']}",
              flush=True)
    receipt = archive_run_manifest(args.repo, tag, run, completed, unavailable)
    append_evidence(args.repo, tag, run, completed, unavailable, receipt)
    if unavailable:
        raise RuntimeError(f"Release {tag} has {len(unavailable)} unavailable artifact(s)")
    print(f"VERIFIED_RELEASE https://github.com/{args.repo}/releases/tag/{tag}")


if __name__ == "__main__":
    main()
