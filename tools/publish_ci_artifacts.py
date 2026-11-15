#!/usr/bin/env python3
"""Publish every internal SliceBridge build artifact as an experimental GitHub Release asset.

Invoked only by a trusted default-branch workflow_run. Never checks out or executes
code/artifacts from the triggering run; archives the exact original Actions ZIP bytes.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
import re
import subprocess
import tempfile
import zipfile
from pathlib import Path

ALLOWED_WORKFLOWS = {
    "Native WebView Windows and macOS",
    "Windows portable beta",
    "Printer profile registry QA",
}

def github_json(endpoint: str) -> dict | list:
    proc = subprocess.run(["gh", "api", endpoint], capture_output=True, check=True)
    return json.loads(proc.stdout)

def exec_gh(*args: str) -> str:
    proc = subprocess.run(["gh", *args], text=True, capture_output=True, check=True)
    return proc.stdout.strip()

def tag_for_sha(sha: str) -> str:
    if not re.fullmatch(r"[0-9a-f]{40}", sha):
        raise ValueError("Expected immutable 40-character commit SHA")
    return "ci-build-" + sha[:16]

def safe_branch(raw: str) -> str:
    return re.sub(r"[^A-Za-z0-9._/-]", "-", raw or "unknown")[:72]

def eligible(run: dict, repo: str) -> bool:
    head = run.get("head_repository") or {}
    return (run.get("name") in ALLOWED_WORKFLOWS
            and head.get("full_name") == repo
            and run.get("status") == "completed")

def all_artifacts(repo: str, run_id: int) -> list[dict]:
    items = []
    for page in range(1, 30):
        obj = github_json(f"repos/{repo}/actions/runs/{run_id}/artifacts?per_page=100&page={page}")
        items.extend(obj.get("artifacts", []))
        if len(obj.get("artifacts", [])) < 100:
            return items
    raise RuntimeError("Artifact listing exceeded 29 pages")

def find_release(repo: str, sha: str) -> dict | None:
    # Prefer a dedicated CI build tag, then reuse historical or manually created
    # pre-releases for this exact commit; never modify stable/production releases.
    tag = tag_for_sha(sha)
    try:
        return github_json(f"repos/{repo}/releases/tags/{tag}")
    except subprocess.CalledProcessError as exc:
        if b"404" not in (exc.stderr or b"") and b"Not Found" not in (exc.stderr or b""):
            raise
    for page in range(1, 40):
        rows = github_json(f"repos/{repo}/releases?per_page=100&page={page}")
        for release in rows:
            if (release.get("target_commitish") == sha
                    and release.get("prerelease") and not release.get("draft")):
                return release
        if len(rows) < 100:
            return None
    raise RuntimeError("Release search exceeded 39 pages; explicit reconciliation required")

def create_release(repo: str, run: dict) -> dict:
    sha = run["head_sha"]
    tag = tag_for_sha(sha)
    notes = (
        "# SliceBridge automatic CI build archive\n\n"
        "**EXPERIMENTAL PRE-RELEASE. Not production approved.** "
        "Published under the owner's 2026-11-15 standing authorization "
        "to publish all build outputs, including experiments.\n\n"
        f"- Source commit: `{sha}`\n"
        f"- Original branch: `{safe_branch(run.get('head_branch') or '')}`\n"
        f"- Original workflow: {run.get('name')} (conclusion: {run.get('conclusion')})\n"
        f"- Evidence: https://github.com/{repo}/actions/runs/{run['id']}\n"
        "- Source commit provenance is derived from the workflow-run metadata; "
        "packaged binaries are not independently certified.\n"
        "- Files are original Actions ZIP archives, with ZIP CRC and SHA-256 verification.\n"
        "- No code signing, OEM GUI approval, printing validation, or installation is implied.\n"
    )
    with tempfile.TemporaryDirectory(prefix="slicebridge-release-notes-") as temp:
        file = Path(temp) / "notes.md"
        file.write_text(notes)
        exec_gh("release", "create", tag, "-R", repo, "--target", sha,
                "--prerelease", "--latest=false",
                "--title", f"SliceBridge CI experimental | {safe_branch(run.get('head_branch') or '')} | {sha[:12]}",
                "--notes-file", str(file))
    r = github_json(f"repos/{repo}/releases/tags/{tag}")
    if r.get("target_commitish") != sha or not r.get("prerelease"):
        raise RuntimeError("Created GitHub Release fails SHA/prerelease readback")
    return r

def get_sha(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(4 * 1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()

def verify_asset(existing: dict | None, name: str, digest: str, size: int) -> bool:
    if existing is None:
        return False
    if existing.get("size") != size or existing.get("digest") != f"sha256:{digest}":
        raise RuntimeError(f"Remote asset content mismatch for {name}; no overwrite permitted")
    return True

def upload_artifact(repo: str, tag: str, artifact: dict, existing: dict) -> tuple[str, str]:
    aid = int(artifact["id"])
    name = f"CI-artifact-{aid}.zip"
    with tempfile.TemporaryDirectory(prefix="slicebridge-ci-artifact-") as temp:
        path = Path(temp) / name
        # Download the original GitHub Actions ZIP without extraction or execution.
        with path.open("wb") as f:
            subprocess.run(["gh", "api", f"repos/{repo}/actions/artifacts/{aid}/zip"],
                           stdout=f, stderr=subprocess.PIPE, check=True)
        size = path.stat().st_size
        if size != artifact["size_in_bytes"]:
            raise RuntimeError(f"Artifact {aid} size differs from Actions metadata")
        if not zipfile.is_zipfile(path):
            raise RuntimeError(f"Artifact {aid} is not a ZIP archive")
        with zipfile.ZipFile(path) as z:
            bad = z.testzip()
        if bad:
            raise RuntimeError(f"Artifact {aid} ZIP CRC failed: {bad}")
        digest = get_sha(path)
        if not verify_asset(existing.get(name), name, digest, size):
            exec_gh("release", "upload", tag, "-R", repo, str(path))
        checksum = Path(temp) / (name + ".sha256")
        checksum.write_text(f"{digest}  {name}\n")
        checksum_digest = get_sha(checksum)
        checksum_size = checksum.stat().st_size
        if not verify_asset(existing.get(checksum.name), checksum.name,
                            checksum_digest, checksum_size):
            exec_gh("release", "upload", tag, "-R", repo, str(checksum))
    # Server-side readback is required before adding the asset to release notes.
    server = github_json(f"repos/{repo}/releases/tags/{tag}")
    present = {a["name"]: a for a in server.get("assets", [])}
    if not verify_asset(present.get(name), name, digest, size):
        raise RuntimeError(f"GitHub Release asset {name} missing after upload")
    if not verify_asset(present.get(name + ".sha256"), name + ".sha256", checksum_digest, checksum_size):
        raise RuntimeError(f"GitHub Release checksum {name}.sha256 missing after upload")
    return name, digest

def append_evidence(repo: str, tag: str, run: dict, completed: list[tuple[dict, str, str]]) -> None:
    release = github_json(f"repos/{repo}/releases/tags/{tag}")
    body = release.get("body") or ""
    lines = []
    for art, name, digest in completed:
        marker = f"CI-artifact-{art['id']}.zip"
        if marker in body:
            continue
        lines.append(f"- Original [run #{run['id']}](https://github.com/{repo}/actions/runs/{run['id']}) "
                     f"({run.get('conclusion') or 'unknown'}); {art['name']} / id {art['id']} "
                     f"archived as `{name}` (SHA-256 `{digest}`).")
    if not lines:
        return
    updated = body + "\n\n## Verified CI artifact archive\n\n" + "\n".join(lines) + "\n"
    with tempfile.TemporaryDirectory(prefix="slicebridge-note-update-") as temp:
        file = Path(temp) / "notes.md"
        file.write_text(updated)
        exec_gh("release", "edit", tag, "-R", repo, "--notes-file", str(file))
    final = github_json(f"repos/{repo}/releases/tags/{tag}")
    if not all(f"CI-artifact-{x['id']}.zip" in (final.get("body") or "")
               for x, _, _ in completed):
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
    artifacts = all_artifacts(args.repo, args.run_id)
    if not artifacts:
        print("SKIP: workflow completed without distributable artifacts")
        return
    sha = run["head_sha"]
    tag = tag_for_sha(sha)
    if args.dry_run:
        print(json.dumps({"repo": args.repo, "sha": sha, "run_id": args.run_id,
                          "new_tag_if_needed": tag,
                          "artifacts": [a["id"] for a in artifacts]}))
        return
    release = find_release(args.repo, sha) or create_release(args.repo, run)
    tag = release["tag_name"]
    old = {a["name"]: a for a in release.get("assets", [])}
    completed = []
    for art in artifacts:
        if art.get("expired"):
            print(f"EXPIRED: {art['id']} - cannot archive original bytes")
            continue
        name, digest = upload_artifact(args.repo, tag, art, old)
        completed.append((art, name, digest))
        print(f"ARCHIVED {art['id']} -> {tag}: {digest}", flush=True)
    if completed:
        append_evidence(args.repo, tag, run, completed)
    print(f"VERIFIED_RELEASE https://github.com/{args.repo}/releases/tag/{tag}")

if __name__ == "__main__":
    main()
