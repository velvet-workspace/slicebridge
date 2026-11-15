#!/usr/bin/env python3
"""Archive a locally produced SliceBridge build with verifiable source provenance.

This explicit local opt-in path uploads only caller-named files; it does not build,
install, execute, publish a stable release, or interact with a printer.
"""
from __future__ import annotations

import argparse
import datetime as dt
import json
import re
import subprocess
import tempfile
from pathlib import Path

import publish_ci_artifacts as ci


def git(source: Path, *args: str) -> str:
    return subprocess.run(["git", "-C", str(source), *args], capture_output=True,
                          text=True, check=True).stdout.strip()


def source_provenance(repo: str, source: Path) -> dict:
    if not source.is_dir():
        raise ValueError("Source directory does not exist")
    root = Path(git(source, "rev-parse", "--show-toplevel")).resolve()
    if root != source.resolve():
        raise ValueError("Source directory must be the Git repository root")
    origin = git(source, "remote", "get-url", "origin")
    if origin not in {
        f"https://github.com/{repo}.git",
        f"https://github.com/{repo}",
        f"git@github.com:{repo}.git",
        f"ssh://git@github.com/{repo}.git",
    }:
        raise ValueError(f"Git remote origin is not the requested repository: {origin}")
    if git(source, "status", "--porcelain", "--untracked-files=normal"):
        raise ValueError("Dirty source worktree cannot prove an exact build snapshot")
    sha = ci.require_sha(git(source, "rev-parse", "HEAD"))
    if git(source, "cat-file", "-t", sha) != "commit":
        raise ValueError("HEAD is not a source commit")
    # Require the actual commit to exist remotely; an unpushed local SHA
    # cannot be independently fetched by a future Release consumer.
    remote = ci.github_json(f"repos/{repo}/commits/{sha}")
    if remote.get("sha") != sha:
        raise RuntimeError("Remote source commit readback did not match local HEAD")
    return {
        "source_commit_sha": sha,
        "source_branch": ci.safe_branch(git(source, "branch", "--show-current")),
        "source_origin": origin,
        "source_worktree_clean": True,
        "source_commit_verified_on_github": True,
    }


def validate_build_at(raw: str) -> str:
    value = dt.datetime.fromisoformat(raw.replace("Z", "+00:00"))
    if value.tzinfo is None:
        raise ValueError("--build-at must contain a UTC offset or Z")
    return value.astimezone(dt.timezone.utc).isoformat().replace("+00:00", "Z")


def local_tag(build_id: str, sha: str) -> str:
    if not re.fullmatch(r"[a-zA-Z0-9][a-zA-Z0-9._-]{0,63}", build_id):
        raise ValueError("Build ID must be 1-64 URL-safe characters")
    return f"local-build-{build_id}-{ci.require_sha(sha)[:12]}"


def validate_files(asset_names: list[str]) -> list[Path]:
    if not asset_names:
        raise ValueError("At least one --asset is required")
    files = [Path(s).resolve(strict=True) for s in asset_names]
    if len({p.name for p in files}) != len(files):
        raise ValueError("Asset basenames must be distinct in a Release")
    for user_path, path in zip(asset_names, files):
        if Path(user_path).is_symlink() or not path.is_file():
            raise ValueError(f"Asset must be an ordinary file, not a symlink: {user_path}")
        if path.stat().st_size == 0:
            raise ValueError(f"Empty build asset: {path}")
        if path.name.startswith("LOCAL_BUILD_MANIFEST"):
            raise ValueError("Asset name reserved for provenance manifest")
    return files


def publish(repo: str, build_id: str, build_at: str, builder: str, platform: str,
            source: Path, paths: list[Path], dry_run: bool) -> str:
    provenance = source_provenance(repo, source)
    tag = local_tag(build_id, provenance["source_commit_sha"])
    existing = ci._release_by_tag(repo, tag)
    if existing:
        ci._validate_release(existing, provenance["source_commit_sha"])
    asset_proofs = [{
        "name": p.name, "size": p.stat().st_size, "sha256": ci.get_sha(p)
    } for p in paths]
    manifest = {
        "schema": "slicebridge-local-build-v1",
        "archive_class": "experimental",
        "production_approved": False,
        "source": provenance,
        "repository": repo,
        "release_tag": tag,
        "build_id": build_id,
        "built_at_utc": validate_build_at(build_at),
        "builder": ci.safe_label(builder),
        "platform": platform,
        "qa_status": "not independently verified by the publishing tool",
        "ci_run_id": None,
        "artifacts": asset_proofs,
    }
    if dry_run:
        print(json.dumps(manifest, indent=2, sort_keys=True))
        return tag
    if not existing:
        notes = (
            "# SliceBridge locally built binary archive\n\n"
            "**EXPERIMENTAL PRE-RELEASE — NOT PRODUCTION APPROVED.**\n"
            f"- Source commit: \x60{provenance['source_commit_sha']}\x60\n"
            f"- Source branch: \x60{provenance['source_branch']}\x60\n"
            f"- Verified GitHub origin: \x60{repo}\x60\n"
            f"- Build identifier: \x60{build_id}\x60\n"
            f"- Build time UTC: \x60{manifest['built_at_utc']}\x60\n"
            f"- Platform: \x60{platform}\x60; Builder: \x60{manifest['builder']}\x60\n"
            "- Build provenance: clean source tree, exact remote commit verified; "
            "binary reproducibility, execution and OEM/printing safety NOT certified.\n"
            "- No installation, deployment or printing was performed.\n"
        )
        with tempfile.TemporaryDirectory(prefix="slicebridge-local-notes-") as temp:
            path = Path(temp) / "notes.md"
            path.write_text(notes, encoding="utf-8")
            ci.exec_gh("release", "create", tag, "-R", repo,
                       "--target", provenance["source_commit_sha"],
                       "--prerelease", "--latest=false",
                       "--title", f"SliceBridge experimental local {platform} {build_id}",
                       "--notes-file", str(path))
    reread = ci._release_by_tag(repo, tag)
    if reread is None:
        raise RuntimeError("Local build Release not visible on GitHub")
    ci._validate_release(reread, provenance["source_commit_sha"])
    for path in paths:
        receipt = ci.upload_and_verify(repo, tag, path)
        if (receipt["size"] != next(a["size"] for a in asset_proofs if a["name"] == path.name)
                or receipt["sha256"] != next(a["sha256"] for a in asset_proofs
                                              if a["name"] == path.name)):
            raise RuntimeError("Local build remote asset readback differs from input")
    with tempfile.TemporaryDirectory(prefix="slicebridge-local-manifest-") as temp:
        path = Path(temp) / f"LOCAL_BUILD_MANIFEST-{build_id}.json"
        path.write_text(json.dumps(manifest, sort_keys=True, indent=2) + "\n",
                        encoding="utf-8")
        receipt = ci.upload_and_verify(repo, tag, path)
        sums = Path(temp) / (path.name + ".sha256")
        sums.write_text(f"{receipt['sha256']}  {path.name}\n", encoding="utf-8")
        ci.upload_and_verify(repo, tag, sums)
    reread = ci._release_by_tag(repo, tag)
    if reread is None:
        raise RuntimeError("Release disappeared after local build upload")
    names = {a["name"] for a in reread.get("assets", [])}
    if not all(p.name in names for p in paths):
        raise RuntimeError("Missing original local build asset after remote upload")
    if path.name not in names:
        raise RuntimeError("Missing local build manifest after remote upload")
    print(f"VERIFIED_LOCAL_RELEASE https://github.com/{repo}/releases/tag/{tag}")
    return tag


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", default="velvet-workspace/slicebridge")
    parser.add_argument("--source-dir", type=Path, required=True)
    parser.add_argument("--asset", action="append", required=True)
    parser.add_argument("--platform", choices=["windows", "macos", "linux"], required=True)
    parser.add_argument("--build-id", required=True)
    parser.add_argument("--build-at", required=True,
                        help="Actual build timestamp with UTC offset, e.g. 2026-11-15T08:00:00+02:00")
    parser.add_argument("--builder", required=True)
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    files = validate_files(args.asset)
    publish(args.repo, args.build_id, args.build_at, args.builder,
            args.platform, args.source_dir, files, args.dry_run)


if __name__ == "__main__":
    main()
