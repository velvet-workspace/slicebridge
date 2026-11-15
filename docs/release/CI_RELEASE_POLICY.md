# Experimental GitHub Releases for every internal CI build

Owner decision: **2026-11-15**. GitHub Release publication is approved as a normal documentation and archiving operation; **it is not permission to merge PRs, install builds, deploy, print, or mark binaries production-ready**.

## Scope

Every completed **internal artifact-producing build** in these workflows must be reflected in a published, explicitly labeled **Pre-release** with an immutable commit SHA, original workflow/run links, and the original Actions artifact ZIP byte stream preserved in GitHub Release assets:

- Native WebView Windows and macOS
- Windows portable beta
- Printer profile registry QA

Source review and security checks retain their original authority. CI failure is disclosed in the Release log; a failed workflow can still produce an archive-worthy experimental artifact. Never represent an unknown or failed QA state as success. Fork-originated workflows are excluded from privileged publication because they are untrusted.

## Design

The trusted default-branch `.github/workflows/auto-release-builds.yml` subscribes to completed build workflows via `workflow_run`. It checks out **main only**, with persisted Git credentials disabled, and does **not** execute source code, install software or extract artifacts from the triggering branch. The script `tools/publish_ci_artifacts.py` verifies an internal head repository, downloads the original ZIP by artifact ID, verifies its size, ZIP CRC, and SHA-256, then uploads that exact ZIP and a matching checksum sidecar.

One pre-release is created per source SHA (tag `ci-build-<sha16>`) unless an existing pre-release already targets that SHA. The release body contains the original run ID, result, artifact ID and checksum. Repeated builds for the same commit are appended as distinct immutable artifact IDs. Every Release write is checked by GitHub readback; mismatched assets fail closed instead of being overwritten. The GitHub Actions artifact expiry is not used as permanent storage.

This workflow intentionally has `contents: write` / `actions: read` only; it never uses arbitrary fork artifacts or checks out PR code. Releases are always `--prerelease --latest=false`.

## Retroactive inventory

On 2026-11-15, 317 Actions artifacts on 139 distinct source SHAs were audited. The repo already contained 29 historical Releases; one additional manually validated RC was published as `v0.4.0-beta.6-rc.1`; 131 other previously undocumented build commits were backfilled as `v0.4.0-ci.<UTC-timestamp>.<sha10>` Releases. Source-commit and prerelease status were verified live on all 131 backfills. Release asset migration is separate and must not be called complete for any tag until GitHub digests, sizes, metadata and notes have been independently verified.

## Operations and recovery

For a single run, inspect without writing:

```bash
python3 tools/publish_ci_artifacts.py --repo velvet-workspace/slicebridge --run-id <RUN_ID> --dry-run
```

After the workflow is merged into protected `main`, the automatic publisher becomes active for **new completed runs**. Existing releases are never deleted, main is not changed by the publisher, and missed or expired assets are recorded as gaps rather than fabricated. GitHub tag and Release publication history remains publicly visible. Run `python3 -m unittest discover -s tests -p 'test_publish_ci_artifacts.py' -v` for offline contract tests.

**Activation state:** A PR/branch containing these files is *not* enough; only a reviewed merge into `main` activates the GitHub workflow. Do not claim it was installed while pending review.
