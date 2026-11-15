//! Explicit, read-only update checks matching native/updates/*.go.
//! No updater downloads, no executable installations, no background polling,
//! no browser launch, and no arbitrary URLs from callers or release metadata.
use semver::{BuildMetadata, Version};
use serde_json::{json, Value};
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub const INSTALLED_VERSION: &str = "0.4.0-beta.6.dev.3";
pub const API_URL: &str = "https://api.github.com/repos/nocturney/slicebridge/releases?per_page=50";
pub const RELEASES_URL: &str = "https://github.com/nocturney/slicebridge/releases";
pub const BUILDS_URL: &str =
    "https://github.com/nocturney/slicebridge/actions/workflows/native.yml";
const API_MAX_BYTES: usize = 2 << 20;

const WINDOWS: &[&str] = &[
    "slicebridge.exe",
    "slicebridge-windows.zip",
    "slicebridge-windows-native.zip",
    "3mf-bridge-windows-native.zip",
    "3mf-bridge-windows-x64.zip",
];
const MAC_ARM64: &[&str] = &[
    "slicebridge-macos-apple-silicon.zip",
    "slicebridge-macos-arm64.zip",
    "3mf-bridge-macos-apple-silicon.zip",
    "3mf-bridge-macos-arm64.zip",
];
const MAC_X64: &[&str] = &[
    "slicebridge-macos-intel.zip",
    "3mf-bridge-macos-intel.zip",
    "3mf-bridge-macos-amd64.zip",
];

pub fn current_platform() -> (&'static str, &'static str) {
    let os = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "unsupported"
    };
    let arch = if cfg!(target_arch = "x86_64") {
        "amd64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "unsupported"
    };
    (os, arch)
}

fn expected_assets(os: &str, arch: &str) -> &'static [&'static str] {
    match (os, arch) {
        ("windows", "amd64") => WINDOWS,
        ("darwin", "arm64") => MAC_ARM64,
        ("darwin", "amd64") => MAC_X64,
        _ => &[],
    }
}

fn normalized_version(raw: &str) -> Option<Version> {
    let trimmed = raw.trim();
    let value = trimmed.strip_prefix('v').unwrap_or(trimmed);
    let core = value.split(['-', '+']).next()?;
    if core.matches('.').count() != 2 {
        return None;
    }
    let mut version = Version::parse(value).ok()?;
    // golang.org/x/mod/semver.Canonical strips build metadata, which does
    // not change ordering; keep the same canonical result as native Go.
    version.build = BuildMetadata::EMPTY;
    Some(version)
}

fn safe_tag(raw: &str) -> bool {
    !raw.is_empty() && raw.len() <= 90 && normalized_version(raw).is_some()
}

/// The ONLY supported release URL constructor; never open an arbitrary
/// browser URL returned by GitHub JSON or supplied by an agent.
pub fn release_url(tag: &str) -> Result<String, String> {
    if !safe_tag(tag) {
        return Err("invalid release tag".into());
    }
    Ok(format!("{RELEASES_URL}/tag/{tag}"))
}

fn result(status: &str, installed: &str, os: &str, arch: &str) -> Value {
    json!({
        "status": status,
        "installedVersion": installed,
        "platform": format!("{os}-{arch}")
    })
}

fn matching_asset(release: &Value, names: &[&str]) -> Option<String> {
    let assets = release["assets"].as_array()?;
    // Go prioritizes the release's asset ordering, not the wish-list order.
    for asset in assets {
        let Some(name) = asset["name"].as_str() else {
            continue;
        };
        if names.iter().any(|want| name.eq_ignore_ascii_case(want)) {
            return Some(name.to_owned());
        }
    }
    None
}

/// Pure source-locked release selector, testable with offline JSON fixtures.
/// Input is bounded and must be an array. Rejects drafts/non-SemVer tags.
pub fn evaluate_releases(
    raw: &[u8],
    installed: &str,
    os: &str,
    arch: &str,
) -> Result<Value, String> {
    let current = normalized_version(installed).ok_or("invalid installed application version")?;
    let names = expected_assets(os, arch);
    if names.is_empty() {
        return Ok(result("unsupported_platform", installed, os, arch));
    }
    if raw.len() > API_MAX_BYTES {
        return Err("GitHub response is unexpectedly large".into());
    }
    let parsed: Value =
        serde_json::from_slice(raw).map_err(|_| "invalid GitHub Releases response")?;
    let rows = parsed
        .as_array()
        .ok_or("invalid GitHub Releases response: expected array")?;

    let mut latest_any: Option<Version> = None;
    let mut latest_applicable: Option<(Version, String, String)> = None;
    for row in rows {
        if row["draft"] == true {
            continue;
        }
        let Some(tag) = row["tag_name"].as_str() else {
            continue;
        };
        if !safe_tag(tag) {
            continue;
        }
        let version = normalized_version(tag).expect("checked safe tag");
        if latest_any.as_ref().is_none_or(|latest| version > *latest) {
            latest_any = Some(version.clone());
        }
        let Some(asset_name) = matching_asset(row, names) else {
            continue;
        };
        if latest_applicable
            .as_ref()
            .is_none_or(|(latest, _, _)| version > *latest)
        {
            latest_applicable = Some((version, tag.to_owned(), asset_name));
        }
    }
    let Some(any) = latest_any else {
        return Ok(result("no_releases", installed, os, arch));
    };
    if let Some((version, tag, asset_name)) = latest_applicable {
        if version > current {
            let mut reply = result("update_available", installed, os, arch);
            reply["latestVersion"] = json!(version.to_string());
            reply["releaseTag"] = json!(tag);
            reply["assetName"] = json!(asset_name);
            return Ok(reply);
        }
    }
    let mut reply = result(
        if any > current {
            "platform_unavailable"
        } else {
            "up_to_date"
        },
        installed,
        os,
        arch,
    );
    reply["latestVersion"] = json!(any.to_string());
    Ok(reply)
}

#[cfg(target_os = "macos")]
fn curl_path() -> PathBuf {
    PathBuf::from("/usr/bin/curl")
}
#[cfg(windows)]
fn curl_path() -> PathBuf {
    std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
        .join("System32")
        .join("curl.exe")
}
#[cfg(not(any(target_os = "macos", windows)))]
fn curl_path() -> PathBuf {
    PathBuf::from("/usr/bin/curl")
}

/// Supervise the OS HTTP client and authenticated fallback with a real
/// wall-clock deadline and a hard pipe read cap; no child process may block
/// the caller indefinitely or allocate an unbounded remote JSON payload.
fn bounded_output(
    command: &mut Command,
    limit: usize,
    deadline: Duration,
) -> Result<(ExitStatus, Vec<u8>), String> {
    let mut child = command
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|_| "GitHub client process unavailable")?;
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("GitHub client output unavailable".into());
    };
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut output = Vec::new();
        let reply = stdout
            .take((limit + 1) as u64)
            .read_to_end(&mut output)
            .map(|_| output);
        let _ = sender.send(reply);
    });

    let started = Instant::now();
    let mut captured: Option<Vec<u8>> = None;
    loop {
        if captured.is_none() {
            match receiver.try_recv() {
                Ok(Ok(bytes)) => captured = Some(bytes),
                Ok(Err(_)) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("GitHub client read failed".into());
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("GitHub client output unavailable".into());
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if captured.as_ref().is_some_and(|bytes| bytes.len() > limit) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("GitHub response is unexpectedly large".into());
        }
        match child.try_wait() {
            Ok(Some(status)) if captured.is_some() => {
                return Ok((status, captured.expect("confirmed captured")));
            }
            Ok(Some(_)) | Ok(None) => {}
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("GitHub client process failed".into());
            }
        }
        if started.elapsed() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("GitHub request timed out".into());
        }
        std::thread::sleep(Duration::from_millis(15));
    }
}

/// Source-locked HTTP request. curl is part of supported desktop OSes; arguments
/// do not invoke a shell. Only the fixed official HTTPS API endpoint is used.
/// --max-time and --max-filesize bound time and response; no fallback to an
/// arbitrary URL or download from the JSON payload.
fn fetch_public_releases() -> Result<(u16, Vec<u8>), String> {
    let mut command = Command::new(curl_path());
    command.args([
        "--silent",
        "--show-error",
        "--max-time",
        "12",
        "--max-filesize",
        "2097152",
        "--retry",
        "0",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--header",
        "Accept: application/vnd.github+json",
        "--header",
        "X-GitHub-Api-Version: 2022-11-28",
        "--user-agent",
        "SliceBridge/0.4.0-beta.6.dev.3",
        "--write-out",
        "\n%{http_code}",
        "--url",
        API_URL,
    ]);
    let (status, raw) = bounded_output(&mut command, API_MAX_BYTES + 32, Duration::from_secs(13))?;
    if !status.success() {
        return Err("GitHub Releases HTTPS request failed or exceeded limits".into());
    }
    let split = raw
        .iter()
        .rposition(|b| *b == b'\n')
        .ok_or("GitHub Releases HTTP status missing")?;
    let body = &raw[..split];
    if body.len() > API_MAX_BYTES {
        return Err("GitHub response is unexpectedly large".into());
    }
    let code = std::str::from_utf8(&raw[split + 1..])
        .map_err(|_| "invalid GitHub HTTP status")?
        .parse::<u16>()
        .map_err(|_| "invalid GitHub HTTP status")?;
    Ok((code, body.to_vec()))
}

/// Optional authenticated fallback for the same hardcoded GitHub endpoint.
/// Any credentials are handled by the user's existing gh CLI login; this app
/// neither reads nor persists tokens, and ignores arbitrary CLI stderr.
fn github_cli() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    #[cfg(windows)]
    {
        if let Some(pf) = std::env::var_os("ProgramFiles") {
            candidates.push(PathBuf::from(pf).join("GitHub CLI/gh.exe"));
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            candidates.push(PathBuf::from(local).join("Programs/GitHub CLI/gh.exe"));
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            candidates.push(PathBuf::from(home).join(".local/bin/gh"));
        }
        candidates.push(PathBuf::from("/opt/homebrew/bin/gh"));
        candidates.push(PathBuf::from("/usr/local/bin/gh"));
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        candidates.push(PathBuf::from("/usr/bin/gh"));
    }
    candidates.into_iter().find(|path| path.is_file())
}

fn fetch_authenticated() -> Result<Vec<u8>, String> {
    let path = github_cli().ok_or("GitHub CLI is not installed")?;
    // No shell, no query from the user, no token in args or output.
    let mut command = Command::new(path);
    command.args([
        "api",
        "repos/nocturney/slicebridge/releases?per_page=50",
        "--method",
        "GET",
    ]);
    let (status, bytes) = bounded_output(&mut command, API_MAX_BYTES, Duration::from_secs(12))?;
    if !status.success() {
        return Err("GitHub CLI cannot access the repository".into());
    }
    Ok(bytes)
}

/// CALL ONLY on a deliberate Check for updates gesture. Never at startup,
/// from a render loop, or from a timer. No installers are downloaded/run.
pub fn check_installed() -> Result<Value, String> {
    let (os, arch) = current_platform();
    let installed = INSTALLED_VERSION;
    if normalized_version(installed).is_none() {
        return Err("invalid installed application version".into());
    }
    if expected_assets(os, arch).is_empty() {
        return Ok(result("unsupported_platform", installed, os, arch));
    }
    match fetch_public_releases() {
        Ok((200, body)) => evaluate_releases(&body, installed, os, arch),
        Ok((404, _)) => match fetch_authenticated() {
            Ok(body) => evaluate_releases(&body, installed, os, arch),
            Err(_) => Ok(result("authentication_required", installed, os, arch)),
        },
        Ok((403 | 429, _)) => Err("GitHub API access limited".into()),
        Ok((status, _)) => Err(format!("GitHub Releases returned HTTP {status}")),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_version_and_repo_endpoints_are_source_locked() {
        let native = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../native/updates_bridge.go"
        ));
        assert!(native.contains(&format!("var appVersion = \"{INSTALLED_VERSION}\"")));
        assert_eq!(
            API_URL,
            "https://api.github.com/repos/nocturney/slicebridge/releases?per_page=50"
        );
        assert_eq!(
            RELEASES_URL,
            "https://github.com/nocturney/slicebridge/releases"
        );
        assert_eq!(
            BUILDS_URL,
            "https://github.com/nocturney/slicebridge/actions/workflows/native.yml"
        );
        assert_eq!(
            release_url("v0.4.0-beta.10").unwrap(),
            format!("{RELEASES_URL}/tag/v0.4.0-beta.10")
        );
        for invalid in [
            "",
            "v1.2",
            "unknown",
            "v1.2.3/../",
            "https://evil.example",
            "v1.2.3?go=evil",
            "x".repeat(100).as_str(),
        ] {
            assert!(release_url(invalid).is_err(), "invalid accepted");
        }
    }

    #[test]
    fn prerelease_ordering_and_installed_dev_snapshot_exact() {
        let data = br#"[
          {"tag_name":"v0.4.0-beta.1","assets":[{"name":"3MF-Bridge-Windows-native.zip"}]},
          {"tag_name":"v0.4.0-beta.2","assets":[{"name":"3MF-Bridge-Windows-native.zip"}]},
          {"tag_name":"v0.4.0-beta.10","assets":[{"name":"3MF-Bridge-Windows-native.zip"}]}
        ]"#;
        let actual = evaluate_releases(data, "0.4.0-beta.1", "windows", "amd64").unwrap();
        assert_eq!(actual["status"], "update_available");
        assert_eq!(actual["latestVersion"], "0.4.0-beta.10");
        assert_eq!(actual["assetName"], "3MF-Bridge-Windows-native.zip");

        for (tag, expected) in [
            ("v0.4.0-beta.4", "up_to_date"),
            ("v0.4.0-beta.4.dev.29", "update_available"),
            ("v0.4.0-beta.5", "update_available"),
            ("v0.4.0-rc.1", "update_available"),
            ("v0.4.0", "update_available"),
            ("v0.3.9", "up_to_date"),
        ] {
            let data = json!([{"tag_name":tag, "assets":[{"name":"SliceBridge.exe"}]}]);
            let result = evaluate_releases(
                data.to_string().as_bytes(),
                "0.4.0-beta.4.dev.28",
                "windows",
                "amd64",
            )
            .unwrap();
            assert_eq!(result["status"], expected, "tag={tag}");
        }
    }

    #[test]
    fn drafts_invalid_tags_and_nonapplicable_assets_are_not_offered() {
        let rows = json!([
            {"tag_name":"v0.5.0", "draft":true,
              "assets":[{"name":"SliceBridge.exe"}]},
            {"tag_name":"evil/../../", "assets":[{"name":"SliceBridge.exe"}]},
            {"tag_name":"v0.4.9", "assets":[{"name":"SliceBridge-macOS-Apple-Silicon.zip"}]},
            {"tag_name":"v0.4.1", "assets":[{"name":"SliceBridge-Windows.zip"}]},
        ]);
        let data = rows.to_string();
        let result =
            evaluate_releases(data.as_bytes(), "0.4.0-beta.1", "windows", "amd64").unwrap();
        assert_eq!(result["status"], "update_available");
        assert_eq!(result["latestVersion"], "0.4.1");
        assert_eq!(result["assetName"], "SliceBridge-Windows.zip");
        assert_eq!(result["releaseTag"], "v0.4.1");
        let latest = evaluate_releases(
            br#"[{"tag_name":"v0.4.9","assets":[{"name":"SliceBridge-macOS-Apple-Silicon.zip"}]}]"#,
            "0.4.0-beta.1",
            "windows",
            "amd64",
        )
        .unwrap();
        assert_eq!(latest["status"], "platform_unavailable");
        assert_eq!(latest["latestVersion"], "0.4.9");
    }

    #[test]
    fn no_releases_unsupported_platform_invalid_json_and_limits() {
        let empty = evaluate_releases(b"[]", INSTALLED_VERSION, "darwin", "arm64").unwrap();
        assert_eq!(empty["status"], "no_releases");
        assert_eq!(empty["platform"], "darwin-arm64");
        let unsupported = evaluate_releases(b"[]", INSTALLED_VERSION, "linux", "arm64").unwrap();
        assert_eq!(unsupported["status"], "unsupported_platform");
        assert_eq!(unsupported["platform"], "linux-arm64");
        for invalid in [b"{}".as_slice(), b"not json", b"[{} garbage]"] {
            assert!(evaluate_releases(invalid, INSTALLED_VERSION, "windows", "amd64").is_err());
        }
        assert!(evaluate_releases(b"[]", "bad-local-version", "windows", "amd64").is_err());
        assert!(evaluate_releases(
            &vec![b'x'; API_MAX_BYTES + 1],
            INSTALLED_VERSION,
            "windows",
            "amd64"
        )
        .is_err());
    }

    #[test]
    fn stable_macos_and_legacy_windows_names_match_exactly() {
        let data = br#"[{"tag_name":"v0.4.0-beta.5","assets":[
          {"name":"SliceBridge.exe"},
          {"name":"SliceBridge-Windows.zip"},
          {"name":"SliceBridge-macOS-Apple-Silicon.zip"}]}]"#;
        assert_eq!(
            evaluate_releases(data, "0.4.0-beta.4.dev.28", "windows", "amd64").unwrap()
                ["assetName"],
            "SliceBridge.exe"
        );
        assert_eq!(
            evaluate_releases(data, "0.4.0-beta.4.dev.28", "darwin", "arm64").unwrap()["assetName"],
            "SliceBridge-macOS-Apple-Silicon.zip"
        );
        let old = json!([{"tag_name":"v0.4.0-beta.5",
            "assets":[{"name":"SliceBridge-Windows.zip"}]}]);
        assert_eq!(
            evaluate_releases(
                old.to_string().as_bytes(),
                "0.4.0-beta.4.dev.8",
                "windows",
                "amd64"
            )
            .unwrap()["status"],
            "update_available"
        );
    }

    #[test]
    fn isolated_normalization_ignores_build_metadata_not_prereleases() {
        assert_eq!(
            normalized_version("v0.4.0+test").unwrap().to_string(),
            "0.4.0"
        );
        assert_eq!(
            normalized_version(" 0.4.0-beta.5 ").unwrap().to_string(),
            "0.4.0-beta.5"
        );
        assert!(normalized_version("v1.2").is_none());
        assert!(normalized_version("v1.2.3/..").is_none());
    }
}
