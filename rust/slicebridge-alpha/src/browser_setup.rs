//! Source-locked MakerWorld extension setup, from native/browser_setup.go.
//! All OS/browser actions require explicit GPUI user gestures AND Developer Mode.
//! Never replaces Bambu Studio handlers or associates a URI without consent.
use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const FILES: [&str; 9] = [
    "manifest.json",
    "content.js",
    "popup.html",
    "popup.css",
    "popup.js",
    "icon-16.png",
    "icon-32.png",
    "icon-48.png",
    "icon-128.png",
];
type Resource = (&'static str, &'static [u8]);

pub fn browser_kind(browser: &str) -> Result<&'static str, String> {
    match browser {
        "edge" | "chrome" | "brave" => Ok("chromium"),
        "firefox" => Ok("firefox"),
        _ => Err("unsupported browser".into()),
    }
}
pub fn extension_page(browser: &str) -> Result<&'static str, String> {
    match browser {
        "edge" => Ok("edge://extensions"),
        "chrome" => Ok("chrome://extensions"),
        "brave" => Ok("brave://extensions"),
        "firefox" => Ok("about:debugging#/runtime/this-firefox"),
        _ => Err("unsupported browser".into()),
    }
}
pub fn makerworld_mode_url(mode: &str) -> Result<&'static str, String> {
    match mode {
        "off" => Ok("https://makerworld.com/#slicebridge-setup=off"),
        "ask" => Ok("https://makerworld.com/#slicebridge-setup=ask"),
        "always" => Ok("https://makerworld.com/#slicebridge-setup=always"),
        _ => Err("unsupported MakerWorld mode".into()),
    }
}

// Source Go's publisher-approved store registry is EMPTY. Do not point users
// to a store search, unverified listing, or silently install an extension.
pub fn store_url(browser: &str) -> Result<&'static str, String> {
    browser_kind(browser)?;
    Ok("")
}

fn resources(kind: &str) -> Result<[Resource; 9], String> {
    match kind {
        "chromium" => Ok([
            (
                FILES[0],
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../native/frontend/dist/extension-packages/chromium/manifest.json"
                )),
            ),
            (
                FILES[1],
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../native/frontend/dist/extension-packages/chromium/content.js"
                )),
            ),
            (
                FILES[2],
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../native/frontend/dist/extension-packages/chromium/popup.html"
                )),
            ),
            (
                FILES[3],
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../native/frontend/dist/extension-packages/chromium/popup.css"
                )),
            ),
            (
                FILES[4],
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../native/frontend/dist/extension-packages/chromium/popup.js"
                )),
            ),
            (
                FILES[5],
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../native/frontend/dist/extension-packages/chromium/icon-16.png"
                )),
            ),
            (
                FILES[6],
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../native/frontend/dist/extension-packages/chromium/icon-32.png"
                )),
            ),
            (
                FILES[7],
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../native/frontend/dist/extension-packages/chromium/icon-48.png"
                )),
            ),
            (
                FILES[8],
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../native/frontend/dist/extension-packages/chromium/icon-128.png"
                )),
            ),
        ]),
        "firefox" => Ok([
            (
                FILES[0],
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../native/frontend/dist/extension-packages/firefox/manifest.json"
                )),
            ),
            (
                FILES[1],
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../native/frontend/dist/extension-packages/firefox/content.js"
                )),
            ),
            (
                FILES[2],
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../native/frontend/dist/extension-packages/firefox/popup.html"
                )),
            ),
            (
                FILES[3],
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../native/frontend/dist/extension-packages/firefox/popup.css"
                )),
            ),
            (
                FILES[4],
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../native/frontend/dist/extension-packages/firefox/popup.js"
                )),
            ),
            (
                FILES[5],
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../native/frontend/dist/extension-packages/firefox/icon-16.png"
                )),
            ),
            (
                FILES[6],
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../native/frontend/dist/extension-packages/firefox/icon-32.png"
                )),
            ),
            (
                FILES[7],
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../native/frontend/dist/extension-packages/firefox/icon-48.png"
                )),
            ),
            (
                FILES[8],
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../native/frontend/dist/extension-packages/firefox/icon-128.png"
                )),
            ),
        ]),
        _ => Err("invalid browser extension source".into()),
    }
}

fn developer_mode_enabled() -> Result<(), String> {
    let saved = crate::printer_settings::current()?;
    if saved["developerMode"] == true {
        Ok(())
    } else {
        Err("Developer Mode must be enabled in About before MakerWorld setup".into())
    }
}
pub fn config_base() -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Application Support"));
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(not(any(target_os = "macos", windows)))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
    let base = base.ok_or("no user configuration directory")?;
    if !base.is_absolute() {
        return Err("user configuration path must be absolute".into());
    }
    Ok(base)
}

// Reject symlinked components all the way down from the OS config directory;
// never write to the installed stable SliceBridge extension folder.
fn ensure_owned_dir(base: &Path, kind: &str) -> Result<PathBuf, String> {
    if !base.is_absolute() {
        return Err("extension output base must be absolute".into());
    }
    let mut dir = base.to_path_buf();
    for component in ["SliceBridge", "GPUI-Alpha", "BrowserExtensions", kind] {
        dir.push(component);
        match fs::symlink_metadata(&dir) {
            Ok(meta) => {
                if meta.file_type().is_symlink() || !meta.is_dir() {
                    return Err("extension output folder cannot be a symlink or file".into());
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&dir).map_err(|_| "cannot create isolated extension folder")?;
            }
            Err(_) => return Err("extension output folder unavailable".into()),
        }
    }
    Ok(dir)
}

/// Purely prepare bundled local files: no browser opened, extension installed,
/// registry modified, or protocol associated. User click required by Registry.
pub fn prepare_in(base: &Path, kind: &str) -> Result<PathBuf, String> {
    let bundle = resources(kind)?;
    let folder = ensure_owned_dir(base, kind)?;
    for (filename, bytes) in bundle {
        if bytes.is_empty() {
            return Err("bundled browser extension resource missing".into());
        }
        let destination = folder.join(filename);
        match fs::symlink_metadata(&destination) {
            Ok(meta) => {
                if meta.file_type().is_symlink() || !meta.is_file() {
                    return Err("browser extension destination is not a regular file".into());
                }
                if fs::read(&destination).map_err(|_| "cannot read existing extension file")?
                    != bytes
                {
                    // A modified file might be the owner's local experiment.
                    // Fail closed instead of overwriting it.
                    return Err("existing alpha extension file differs from bundled version".into());
                }
                continue;
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("browser extension destination unavailable".into()),
        }
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&destination)
            .map_err(|_| "cannot create extension file")?;
        output
            .write_all(bytes)
            .map_err(|_| "cannot write extension file")?;
        output
            .sync_all()
            .map_err(|_| "cannot verify extension file sync")?;
    }
    Ok(folder)
}

pub fn prepare(browser: &str) -> Result<Value, String> {
    developer_mode_enabled()?;
    let kind = browser_kind(browser)?;
    let folder = prepare_in(&config_base()?, kind)?;
    Ok(json!({"kind":kind, "folder":folder.to_string_lossy()}))
}

/// Read-only local association check. Do not claim GPUI Alpha URI support is
/// registered simply because stable Go (or Bambu Studio) is installed.
pub fn status() -> Result<Value, String> {
    #[cfg(windows)]
    {
        let key = r"HKCU\Software\Classes\slicebridge\shell\open\command";
        let exe = std::env::current_exe().map_err(|_| "cannot locate alpha executable")?;
        let output = Command::new("reg.exe")
            .args(["query", key, "/ve"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .map_err(|_| "could not inspect Windows app-link association")?;
        let raw = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
        let exe = exe.to_string_lossy().to_ascii_lowercase();
        // Standard "C:\..." "%1" format. Exact alpha executable required.
        let registered = output.status.success() && raw.contains(&format!("\"{exe}\""));
        return Ok(json!({"platform":"windows","registered":registered}));
    }
    #[cfg(target_os = "macos")]
    {
        // Unlike the packaged stable Wails .app, the isolated GPUI executable
        // has not registered a CFBundleURLTypes handler. Never fabricate one.
        Ok(json!({"platform":"darwin","registered":false,"pendingInstaller":true}))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        Ok(json!({"platform":"unsupported","registered":false}))
    }
}
pub fn manager_page(browser: &str) -> Result<Value, String> {
    developer_mode_enabled()?;
    Ok(json!({"url":extension_page(browser)?}))
}
pub fn unpublished_store(browser: &str) -> Result<Value, String> {
    developer_mode_enabled()?;
    Ok(json!({"url":store_url(browser)?}))
}

/// Selected app only. No system browser defaults, URI registration, or Bambu
/// Studio handlers are changed. This method is called ONLY on an explicit GPUI
/// click; no browser opens as part of rendering or preparation.
pub fn open(browser: &str, kind: &str, mode: Option<&str>) -> Result<Value, String> {
    developer_mode_enabled()?;
    browser_kind(browser)?;
    let url = match kind {
        "manager" => {
            if mode.is_some() {
                return Err("mode is not valid for extensions manager".into());
            }
            extension_page(browser)?
        }
        "mode" => makerworld_mode_url(mode.ok_or("MakerWorld mode is required")?)?,
        _ => return Err("unsupported browser action".into()),
    };
    #[cfg(windows)]
    {
        let executable = installed_browser(browser)?;
        Command::new(executable)
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| "selected browser could not open")?;
    }
    #[cfg(target_os = "macos")]
    {
        let app = match browser {
            "edge" => "Microsoft Edge",
            "chrome" => "Google Chrome",
            "brave" => "Brave Browser",
            "firefox" => "Firefox",
            _ => unreachable!(),
        };
        let status = Command::new("/usr/bin/open")
            .args(["-a", app, url])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|_| "selected browser could not open")?;
        if !status.success() {
            return Err("selected browser could not open".into());
        }
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = url;
        return Err("selected browser launch is unsupported on this OS".into());
    }
    Ok(json!({"opened":true}))
}

#[cfg(windows)]
fn installed_browser(browser: &str) -> Result<PathBuf, String> {
    let relative = match browser {
        "edge" => "Microsoft/Edge/Application/msedge.exe",
        "chrome" => "Google/Chrome/Application/chrome.exe",
        "brave" => "BraveSoftware/Brave-Browser/Application/brave.exe",
        "firefox" => "Mozilla Firefox/firefox.exe",
        _ => return Err("unsupported browser".into()),
    };
    for base in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
        let Some(parent) = std::env::var_os(base) else {
            continue;
        };
        let candidate = PathBuf::from(parent).join(relative);
        if fs::symlink_metadata(&candidate)
            .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
        {
            return Ok(candidate);
        }
    }
    Err("selected browser is not installed in a standard location".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn browser_and_mode_allowlists_source_locked_and_store_unpublished() {
        for browser in ["edge", "chrome", "brave", "firefox"] {
            assert!(!browser_kind(browser).unwrap().is_empty());
            assert!(
                extension_page(browser).unwrap().contains("extensions") || browser == "firefox"
            );
            assert_eq!(store_url(browser).unwrap(), "");
        }
        for invalid in [
            "",
            "../edge",
            "safari",
            "edge;rm",
            "EDGE",
            "https://evil.test",
        ] {
            assert!(browser_kind(invalid).is_err());
            assert!(extension_page(invalid).is_err());
            assert!(store_url(invalid).is_err());
        }
        for mode in ["off", "ask", "always"] {
            assert_eq!(
                makerworld_mode_url(mode).unwrap(),
                format!("https://makerworld.com/#slicebridge-setup={mode}")
            );
        }
        for mode in ["", "on", "ask?url=file://", "always#x", "https://evil.test"] {
            assert!(makerworld_mode_url(mode).is_err());
        }
    }
    #[test]
    fn source_locked_bundled_extension_bytes_are_complete() {
        for kind in ["chromium", "firefox"] {
            let bundle = resources(kind).unwrap();
            assert_eq!(bundle.len(), FILES.len());
            for (idx, (name, content)) in bundle.iter().enumerate() {
                assert_eq!(name, &FILES[idx]);
                assert!(!content.is_empty());
            }
            let manifest = std::str::from_utf8(bundle[0].1).unwrap();
            let parsed: Value = serde_json::from_str(manifest).unwrap();
            assert!(parsed["manifest_version"].is_number());
        }
        assert!(resources("../chromium").is_err());
    }
    #[test]
    fn preparation_is_confined_to_isolated_alpha_and_preserves_original_files() {
        let base = std::env::temp_dir().join(format!("sb-extension-alpha-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let stable = base.join("SliceBridge/BrowserExtensions/chromium");
        fs::create_dir_all(&stable).unwrap();
        let stable_file = stable.join("manifest.json");
        fs::write(&stable_file, b"stable Go original never touched").unwrap();
        for kind in ["chromium", "firefox"] {
            let folder = prepare_in(&base, kind).unwrap();
            assert!(folder.starts_with(base.join("SliceBridge/GPUI-Alpha/BrowserExtensions")));
            for (name, bytes) in resources(kind).unwrap() {
                assert_eq!(fs::read(folder.join(name)).unwrap(), bytes);
            }
            assert_eq!(prepare_in(&base, kind).unwrap(), folder);
        }
        assert_eq!(
            fs::read(&stable_file).unwrap(),
            b"stable Go original never touched"
        );
        assert!(prepare_in(&base, "../bad").is_err());
        let candidate =
            base.join("SliceBridge/GPUI-Alpha/BrowserExtensions/chromium/manifest.json");
        fs::write(&candidate, b"owner modified").unwrap();
        assert!(prepare_in(&base, "chromium").is_err());
        assert_eq!(fs::read(&candidate).unwrap(), b"owner modified");
        fs::remove_dir_all(base).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn symlinked_alpha_extension_folder_is_refused_without_side_effects() {
        use std::os::unix::fs::symlink;
        let base =
            std::env::temp_dir().join(format!("sb-extension-symlink-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("SliceBridge/GPUI-Alpha/BrowserExtensions")).unwrap();
        let target = base.join("outside");
        fs::create_dir_all(&target).unwrap();
        symlink(
            &target,
            base.join("SliceBridge/GPUI-Alpha/BrowserExtensions/chromium"),
        )
        .unwrap();
        assert!(prepare_in(&base, "chromium").is_err());
        assert!(fs::read_dir(&target).unwrap().next().is_none());
        fs::remove_dir_all(base).unwrap();
    }
}
