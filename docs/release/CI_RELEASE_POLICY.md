# Experimental CI and local build Release policy

Owner decision (2026-11-15): every distributable internal SliceBridge build, including failed-QA or experimental builds, is archived in GitHub Releases. Release publication is authorized and is not production approval, permission to install, a merge to main, or permission to print.

## Coverage

The current artifact-producing distribution workflows are:

- Native WebView Windows and macOS — native Windows/macOS packages
- Windows portable beta — Windows portable package
- Printer profile registry QA — printer catalog package

The only other workflow currently uploading an Actions artifact, release-preflight.yml, creates a source/evidence package, not a distribution build. The publisher's contract test inventories every workflow YAML with actions/upload-artifact, so a new build producer must be registered or classified as evidence-only. Build code added by another mechanism still requires review.

## Security and integrity

The workflow_run subscriber on protected main checks out only trusted main; it does not execute triggering-branch code or unpack uploaded artifacts. It rejects forks and needs only actions:read plus contents:write. Artifacts are archived as the exact Actions ZIP byte stream after size, ZIP CRC and SHA-256 verification. GitHub Release asset digests, sizes and names are independently read back. All Releases are Pre-release / Experimental with latest=false.

Each new CI run attempt has a unique tag of the form ci-run-RUNID-aATTEMPT-SHA12, targeted at the full source commit SHA. An individual manifest, CI-run-RUNID-aATTEMPT-manifest.json, records the source branch/SHA, run/attempt, build start time, CI conclusion, original artifact IDs/names, sizes, hashes, final archive names and unavailable assets; it has a SHA-256 sidecar. Failed runs that produced artifacts are archived with the failure disclosed. A missing or expired original is a recorded gap and a non-successful publication outcome, never an invented archive.

Idempotency: exact duplicate artifact IDs are recognized, legacy releases containing those IDs are reused, and mismatched remote SHA, prerelease state or asset hashes abort without overwrite. All 2026-11-15 historical backfill records are already archived and are not replayed.

## Locally built binary pathway

Use tools/publish_local_build.py. Example dry run:

    python3 tools/publish_local_build.py --source-dir /path/to/clean/slicebridge --asset /path/to/SliceBridge-macOS.zip --platform macos --build-id macmini-20261115-01 --build-at 2026-11-15T08:00:00+02:00 --builder MacMiniOffice --dry-run

After validating the proposed manifest, remove --dry-run to publish. The tool requires a clean Git repository of the expected origin, with the exact commit independently found on GitHub. Every explicitly supplied asset is verified by SHA-256, file size and GitHub digest readback. The unique local-build-BUILDID-SHA12 prerelease stores original files, a LOCAL_BUILD_MANIFEST JSON and a checksum. A changed asset with an existing name is rejected. Uncommitted source cannot be misrepresented as a clean source SHA. Exact reproducibility and native execution are NOT certified by this archive process.

## Checks and activation

    python3 -m unittest discover -s tests -p 'test_publish_*.py' -v
    python3 tools/publish_ci_artifacts.py --repo velvet-workspace/slicebridge --run-id RUN_ID --dry-run

Separate read-only Actions contract tests run on related pull requests. PR #122 is not active on main until owner-approved merge and the first genuine automatic workflow_run, Release publication and readback succeeds. Merge permission is separate from the standing Release publication approval.
