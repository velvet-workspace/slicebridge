//! Native GPUI pilot: never embeds a browser or calls Go/Wails.
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use serde_json::{json, Value};
use slicebridge_alpha::{protocol, Registry};
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

// Staged files, in queue order, as the Registry reports them.
fn staged_paths(status: &Value) -> Vec<String> {
    status["batch_selection"]["paths"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

// Mirrors app.js renderQueue: the staged files are the list. With nothing staged, the active
// source stands in for one file, unless the user just removed that source and the Registry
// has not cleared it yet.
fn queue_display_paths(status: &Value, suppressed_source: Option<&str>) -> Vec<String> {
    let staged = staged_paths(status);
    if !staged.is_empty() {
        return staged;
    }
    status["source"]
        .as_str()
        .filter(|source| Some(*source) != suppressed_source)
        .map(str::to_owned)
        .into_iter()
        .collect()
}

// A reselected folder contains file cache keys, not the folder path itself.
// Match path components (not string prefixes), so /parts does not match /parts-old.
fn selected_queue_file_needs_refresh(file: &Path, selections: &[PathBuf]) -> bool {
    selections.iter().any(|selected| file.starts_with(selected))
}

// Same format as Go native/app.go bytesLabel: size < 1KiB is B, then
// one decimal in KB or MB. The stable app captures this on input selection.
fn source_bytes_label(size: u64) -> String {
    if size < 1024 {
        format!("{size} B")
    } else if size < 1024 * 1024 {
        format!("{:.1} KB", size as f64 / 1024.)
    } else {
        format!("{:.1} MB", size as f64 / (1024. * 1024.))
    }
}

fn source_queue_subtitle(path: &str, size: Option<u64>) -> String {
    let folder = Path::new(path)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(|parent| parent.to_string_lossy().into_owned())
        .unwrap_or_else(|| ".".into());
    size.map_or_else(
        || folder.clone(),
        |bytes| format!("{folder} · {}", source_bytes_label(bytes)),
    )
}

// Cache metadata at queue changes, never during GPUI render. A large queue of
// up to 200 paths must not stat every file on each frame.
fn sync_source_subtitles(paths: &[String], labels: &mut BTreeMap<String, String>) {
    labels.retain(|path, _| paths.contains(path));
    for path in paths {
        labels.entry(path.clone()).or_insert_with(|| {
            let size = std::fs::metadata(path)
                .ok()
                .filter(|metadata| metadata.is_file())
                .map(|metadata| metadata.len());
            source_queue_subtitle(path, size)
        });
    }
}

// Same definition the layout uses for disabling queue controls during a conversion.
fn snapshot_running(status: &Value) -> bool {
    status["phase"] == "running" || status["batch"]["phase"] == "running"
}

// `batch.remove` answers {"removed":true,...} on success; anything else leaves the queue unchanged.
fn remove_reply_ok(reply: &Value) -> bool {
    reply["ok"] == true && reply["data"]["removed"] == true
}

// Go's remove and clear reset memoryStatus, so the ready/idle summary returns instead of the echo.
fn clears_activity_message(name: &str, reply: &Value) -> bool {
    match name {
        "batch.clear" => reply["ok"] == true,
        "batch.remove" => remove_reply_ok(reply),
        _ => false,
    }
}

// Stable per file: the same path keeps its button id when other rows are added or removed.
fn file_button_id(prefix: &str, path: &str) -> String {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    format!("{prefix}-{:016x}", hasher.finish())
}

// Same cap the Registry enforces for `makerworld.origins`.
const MAKERWORLD_ORIGIN_LIMIT: usize = 200;

// Mirrors app.js renderQueue: only an exact high-confidence MakerWorld origin earns the MW badge.
// CDN, unknown, medium-confidence and malformed evidence all stay generic.
fn is_verified_makerworld(origin: &Value) -> bool {
    origin["kind"] == "makerworld" && origin["confidence"] == "high"
}

// Drops cached verdicts for paths no longer queued, then returns only the queued paths still
// unknown. A removed or cleared file never keeps its badge, and a restaged path is inspected again.
fn origin_lookup_plan(display: &[String], cache: &mut BTreeMap<String, bool>) -> Vec<String> {
    cache.retain(|path, _| display.contains(path));
    let mut seen = BTreeSet::new();
    display
        .iter()
        .filter(|path| !cache.contains_key(path.as_str()) && seen.insert(path.as_str()))
        .cloned()
        .collect()
}

// Caches only the badge decision for each requested path. A path the Registry omits, or any
// failed call, is recorded as not verified: nothing is inferred from the file name or folder.
fn record_origin_reply(cache: &mut BTreeMap<String, bool>, paths: &[String], reply: &Value) {
    for path in paths {
        let verified = reply["ok"] == true && is_verified_makerworld(&reply["data"][path.as_str()]);
        cache.insert(path.clone(), verified);
    }
}

// OS metadata reads may finish after a source was removed and re-added under
// the same path. Accept only matching current request IDs to prevent a stale
// background result from lending a new file a MakerWorld badge.
fn apply_origin_reply(
    cache: &mut BTreeMap<String, bool>,
    pending: &mut BTreeMap<String, u64>,
    current_paths: &[String],
    requested: &[String],
    token: u64,
    reply: &Value,
) -> usize {
    let mut eligible = Vec::new();
    for path in requested {
        if pending.get(path) != Some(&token) {
            continue;
        }
        pending.remove(path);
        if current_paths.contains(path) {
            eligible.push(path.clone());
        }
    }
    record_origin_reply(cache, &eligible, reply);
    eligible.len()
}

fn output_folder(status: &Value) -> Option<PathBuf> {
    let batch_output = status["batch"]["items"].as_array().and_then(|items| {
        items.iter().rev().find_map(|entry| {
            // Multi-target files may have one successful printer even if
            // another failed. Only a completed target permits opening output.
            if let Some(targets) = entry["targets"].as_array() {
                targets
                    .iter()
                    .rev()
                    .find(|target| target["phase"] == "completed")
                    .and_then(|target| target["output"].as_str())
            } else if entry["phase"] == "completed" {
                entry["output"].as_str()
            } else {
                None
            }
        })
    });
    let single_output = if status["phase"] == "completed" {
        status["output"].as_str()
    } else {
        None
    };
    batch_output
        .or(single_output)
        .and_then(|path| Path::new(path).parent())
        .map(Path::to_path_buf)
}

#[path = "desktop/source_queue.rs"]
mod source_queue;
use source_queue::collect_source_files;

#[cfg(target_os = "macos")]
#[path = "desktop/macos_language.rs"]
mod desktop_macos_language;

#[path = "desktop/i18n.rs"]
mod desktop_i18n;
use desktop_i18n::Locale;

#[path = "desktop/printer_picker.rs"]
mod desktop_printer_picker;
use desktop_printer_picker::PrinterCard;

#[path = "desktop/printer_settings_ui.rs"]
mod desktop_printer_settings_ui;

// Read only validated persisted dimensions, otherwise use Wails' 1280x900 defaults.
fn initial_window_dimensions(settings_reply: &Value) -> (i64, i64) {
    let window = &settings_reply["data"]["window"];
    let width = window["width"].as_i64().unwrap_or(1280);
    let height = window["height"].as_i64().unwrap_or(900);
    (
        if (720..=2000).contains(&width) {
            width
        } else {
            1280
        },
        if (500..=1400).contains(&height) {
            height
        } else {
            900
        },
    )
}

fn observed_window_dimensions(size: Size<Pixels>) -> (i64, i64) {
    (
        size.width.as_f32().round() as i64,
        size.height.as_f32().round() as i64,
    )
}

// Idle last_message: layout's activity_detail_text maps it to Go's readyHint/idleHint.
const IDLE_MESSAGE: &str =
    "No UI action yet. Agent / CLI / MCP changes appear in the project fields above.";

// Source: stable native/frontend/dist/app.js document keydown handler (Ctrl/Cmd+O,
// Ctrl/Cmd+Shift+O, Ctrl/Cmd+,). Use the already-existing native picker/settings actions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorkspaceShortcut {
    AddFiles,
    AddFolder,
    Settings,
}

fn original_workspace_shortcut(
    stroke: &Keystroke,
    settings_or_recovery_open: bool,
    conversion_running: bool,
) -> Option<WorkspaceShortcut> {
    let modifiers = stroke.modifiers;
    if settings_or_recovery_open
        || conversion_running
        || !(modifiers.control || modifiers.platform)
        || modifiers.alt
        || modifiers.function
    {
        return None;
    }
    match (stroke.key.to_ascii_lowercase().as_str(), modifiers.shift) {
        ("o", false) => Some(WorkspaceShortcut::AddFiles),
        ("o", true) => Some(WorkspaceShortcut::AddFolder),
        (",", _) => Some(WorkspaceShortcut::Settings),
        _ => None,
    }
}

struct SliceBridgeWindow {
    focus_handle: FocusHandle,
    window_resize_revision: u64,
    commands: Registry,
    last_message: String,
    destination: Option<String>,
    destination_source: Option<String>,
    // Set by `batch.remove` so a removed source is not shown again while the Registry still reports it.
    suppressed_source: Option<String>,
    // Go source-row folder and size captured when each path enters the queue.
    source_subtitles: BTreeMap<String, String>,
    // Per queued path: true only for an exact high-confidence MakerWorld origin. Filled outside render.
    makerworld_origins: BTreeMap<String, bool>,
    makerworld_pending: BTreeMap<String, u64>,
    makerworld_token: u64,
    locale: Locale,
    printers_open: bool,
    printers_first_run: bool,
    printers_cards: Vec<PrinterCard>,
    // Catalogue-only cards; installed PrusaSlicer targets are merged in on each open.
    printers_catalogue_cards: Vec<PrinterCard>,
    ready_oem_targets: BTreeSet<String>,
    selected_batch_targets: BTreeSet<String>,
    ready_profiles: Value,
    source_nozzle: Option<String>,
    printers_saved: Vec<String>,
    printers_draft: Vec<String>,
    printers_maker: Option<String>,
    printers_maker_menu: bool,
    printers_show_all: bool,
    printers_error: String,
    printers_search: Entity<InputState>,
    _printer_subscription: Subscription,
    settings_open: bool,
    settings_loaded: Value,
    settings_draft: Value,
    settings_target: String,
    settings_material: String,
    settings_options: Value,
    settings_error: String,
    settings_recovery_pending: bool,
    settings_recovery_error: String,
    about_open: bool,
    about_developer_mode: bool,
    about_developer_error: bool,
    about_link_error: bool,
    about_document: Option<AboutDocument>,
    update: UpdateCheck,
    // MakerWorld setup modal. Reachable only while the Registry reports developer mode as an explicit On.
    integrations_open: bool,
    integrations_developer: bool,
    integrations_browser: BrowserChoice,
    integrations_mode: Option<LinkMode>,
    integrations_link: LinkState,
    integrations_page: Option<&'static str>,
    integrations_folder: Option<String>,
    integrations_install_opened: bool,
    integrations_install: Feedback,
    integrations_mode_feedback: Feedback,
    // True while a prepare, open or mode request is in flight, so one click cannot start a second.
    integrations_busy: bool,
}

// Matches the installed-version string the stable Go app shows (`appVersion` in native/updates_bridge.go).
const ABOUT_INSTALLED_VERSION: &str = slicebridge_alpha::updates::INSTALLED_VERSION;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AboutDocument {
    License,
    Credits,
}

impl AboutDocument {
    // The same files Go embeds from frontend/dist/legal, so the About text works offline.
    fn text(self) -> &'static str {
        match self {
            Self::License => include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../native/frontend/dist/legal/LICENSE.txt"
            )),
            Self::Credits => include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../native/frontend/dist/legal/THIRD_PARTY_NOTICES.md"
            )),
        }
    }
}

// Mirrors about.js: developer mode reads as On only for an explicit native `true`; anything else is Off.
fn developer_mode_enabled(reply: &Value) -> bool {
    reply["ok"] == true && reply["data"]["developerMode"] == true
}

// Mirrors native/about.go aboutPageURL: the only About URLs, fixed in code. The GPUI alpha build
// has no buildCommit stamp, so "source" is the base repository, never a /tree/ link.
fn about_page_url(kind: &str) -> Option<&'static str> {
    match kind {
        "source" => Some("https://github.com/nocturney/slicebridge"),
        "releases" => Some("https://github.com/nocturney/slicebridge/releases"),
        _ => None,
    }
}

// Shell-free launcher for one allowlisted URL, passed as a single argument. Callers pass only
// values returned by `about_page_url` or URLs that passed `allowed_update_url`.
fn about_open_command(url: &str) -> Command {
    if cfg!(windows) {
        let mut command = Command::new("rundll32.exe");
        command.arg("url.dll,FileProtocolHandler").arg(url);
        command
    } else if cfg!(target_os = "macos") {
        let mut command = Command::new("/usr/bin/open");
        command.arg(url);
        command
    } else {
        let mut command = Command::new("xdg-open");
        command.arg(url);
        command
    }
}

// Update links the release notice may open, mirroring updates.rs: the repository's Releases page,
// its preview-build workflow page, and one release tag page. Any other host, path, query or
// fragment is refused, so no UI or GitHub JSON value reaches the OS launcher unchecked.
fn allowed_update_url(url: &str) -> bool {
    const REPOSITORY: &str = "https://github.com/nocturney/slicebridge";
    let Some(path) = url.strip_prefix(REPOSITORY) else {
        return false;
    };
    match path {
        "/releases" | "/actions/workflows/native.yml" => true,
        _ => path
            .strip_prefix("/releases/tag/")
            .is_some_and(allowed_release_tag),
    }
}

// Same bounds as updates.rs safe_tag: short, and only ASCII letters, digits, dots, dashes and plus.
fn allowed_release_tag(tag: &str) -> bool {
    let version = tag.strip_prefix('v').unwrap_or(tag);
    !version.is_empty()
        && tag.len() <= 90
        && !tag.contains("..")
        && tag
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+'))
}

// The only links the update notice offers, as app.js renderUpdates does. The Registry checks them again.
#[derive(Clone, Debug, PartialEq, Eq)]
enum UpdateAction {
    Release(String),
    Releases,
    Builds,
}

impl UpdateAction {
    fn label_key(&self) -> &'static str {
        match self {
            Self::Release(_) => "openRelease",
            Self::Releases => "openReleases",
            Self::Builds => "openPreviewBuilds",
        }
    }

    fn page_params(&self) -> Value {
        match self {
            Self::Release(tag) => json!({"kind":"release","tag":tag}),
            Self::Releases => json!({"kind":"releases"}),
            Self::Builds => json!({"kind":"builds"}),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct UpdateNotice {
    message_key: &'static str,
    // Fills `{v}` in the message. Empty when the status carries no version.
    version: String,
    action: Option<UpdateAction>,
}

// Mirrors app.js renderUpdates, status by status. A release tag is offered only when it is safe.
fn update_notice_for(data: &Value) -> UpdateNotice {
    let (message_key, action) = match data["status"].as_str().unwrap_or("") {
        "update_available" => (
            "updateAvailable",
            data["releaseTag"]
                .as_str()
                .filter(|&tag| allowed_release_tag(tag))
                .map(|tag| UpdateAction::Release(tag.to_owned())),
        ),
        "up_to_date" => ("latestInstalled", None),
        "no_releases" => ("noPublishedReleases", Some(UpdateAction::Builds)),
        "platform_unavailable" => ("platformUnavailable", Some(UpdateAction::Releases)),
        "authentication_required" => ("authRequired", Some(UpdateAction::Releases)),
        _ => ("unsupportedPlatform", None),
    };
    UpdateNotice {
        message_key,
        version: data["latestVersion"].as_str().unwrap_or("").to_owned(),
        action,
    }
}

// The Registry reply as the notice reads it: the check data, or the error it reports. Never a guess.
fn update_check_outcome(reply: &Value) -> Result<Value, String> {
    if reply["ok"] != true {
        return Err(reply["error"]
            .as_str()
            .unwrap_or("unknown error")
            .to_owned());
    }
    if reply["data"]["status"].as_str().is_none() {
        return Err("invalid update check reply".to_owned());
    }
    Ok(reply["data"].clone())
}

// Update notice state, kept apart from GPUI so the request rules can be unit-tested.
#[derive(Default)]
struct UpdateCheck {
    checking: bool,
    // Bumped by each check and by closing About, so only the request the user still sees may show.
    request: u64,
    data: Option<Value>,
    error: String,
    link_error: bool,
}

impl UpdateCheck {
    // Returns the request ID for a new check, or None while one is already in flight.
    fn begin(&mut self) -> Option<u64> {
        if self.checking {
            return None;
        }
        self.checking = true;
        self.request = self.request.wrapping_add(1);
        self.data = None;
        self.error.clear();
        self.link_error = false;
        Some(self.request)
    }

    // A reply for an older request still ends the wait, but it is never shown.
    fn finish(&mut self, request: u64, reply: &Value) {
        self.checking = false;
        if request == self.request {
            match update_check_outcome(reply) {
                Ok(data) => self.data = Some(data),
                Err(error) => self.error = error,
            }
        }
    }

    fn dismiss(&mut self) {
        self.data = None;
        self.error.clear();
        self.link_error = false;
    }

    // Closing About abandons what was shown. A check still running can no longer show its result.
    fn close(&mut self) {
        self.request = self.request.wrapping_add(1);
        self.dismiss();
    }
}

// MakerWorld setup modal, from native/frontend/dist/integrations.js and index.html. The modal opens only
// while the Registry reports developer mode as an explicit On. Browser actions run only on a click, with
// the fixed browser.setup.* arguments below. browser.setup.store is not called: store listings are
// unpublished, so no store install is offered or claimed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum BrowserChoice {
    #[default]
    Edge,
    Chrome,
    Brave,
    Firefox,
}

impl BrowserChoice {
    const ALL: [Self; 4] = [Self::Edge, Self::Chrome, Self::Brave, Self::Firefox];

    // The only browser keys sent to the Registry, the same set native/browser_setup.go accepts.
    fn key(self) -> &'static str {
        match self {
            Self::Edge => "edge",
            Self::Chrome => "chrome",
            Self::Brave => "brave",
            Self::Firefox => "firefox",
        }
    }

    #[cfg(test)]
    fn from_key(key: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|browser| browser.key() == key)
    }

    // Names as index.html shows them; the "(test)" marking is the source's own wording.
    fn display_name(self) -> &'static str {
        match self {
            Self::Edge => "Microsoft Edge",
            Self::Chrome => "Google Chrome",
            Self::Brave => "Brave",
            Self::Firefox => "Firefox (test)",
        }
    }

    // Extension package the Registry prepares for this browser.
    fn kind(self) -> &'static str {
        match self {
            Self::Firefox => "firefox",
            _ => "chromium",
        }
    }

    // The fixed extensions page for this browser, as integrations.js lists it.
    fn settings_page(self) -> &'static str {
        match self {
            Self::Edge => "edge://extensions",
            Self::Chrome => "chrome://extensions",
            Self::Brave => "brave://extensions",
            Self::Firefox => "about:debugging#/runtime/this-firefox",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LinkMode {
    Off,
    Ask,
    Always,
}

impl LinkMode {
    const ALL: [Self; 3] = [Self::Off, Self::Ask, Self::Always];

    // The only values accepted for `browser.setup.open` with kind "mode".
    fn key(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Ask => "ask",
            Self::Always => "always",
        }
    }

    #[cfg(test)]
    fn from_key(key: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|mode| mode.key() == key)
    }

    fn title_key(self) -> &'static str {
        match self {
            Self::Off => "interceptOff",
            Self::Ask => "interceptAsk",
            Self::Always => "interceptAlways",
        }
    }

    fn detail_key(self) -> &'static str {
        match self {
            Self::Off => "interceptOffDetail",
            Self::Ask => "interceptAskDetail",
            Self::Always => "interceptAlwaysDetail",
        }
    }
}

// Step 1 status, as integrations.js refreshStatus shows it. Windows is not offered a fix button yet:
// protocol registration is not ported, so an unregistered Windows install only reports Missing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum LinkState {
    #[default]
    Unknown,
    Ready,
    Missing,
    Mac,
    Error,
}

impl LinkState {
    fn message_key(self) -> &'static str {
        match self {
            Self::Unknown => "browserLinkUnknown",
            Self::Ready => "browserLinkReady",
            Self::Missing => "browserLinkMissing",
            Self::Mac => "browserLinkMac",
            Self::Error => "browserLinkError",
        }
    }

    fn is_ready(self) -> bool {
        matches!(self, Self::Ready | Self::Mac)
    }
}

// A line of status under one setup step. `error` picks the red copy.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Feedback {
    text: String,
    error: bool,
}

impl Feedback {
    fn ok(text: String) -> Self {
        Self { text, error: false }
    }

    fn failed(text: String) -> Self {
        Self { text, error: true }
    }
}

// Only the fixed page for a browser is shown; any other URL the Registry returns is ignored.
fn allowed_browser_page(url: &str) -> Option<&'static str> {
    BrowserChoice::ALL
        .iter()
        .map(|browser| browser.settings_page())
        .find(|page| *page == url)
}

// Windows and macOS only, as integrations.js reads them. Missing data or any other platform is an error.
fn link_state_from(reply: &Value) -> LinkState {
    if reply["ok"] != true {
        return LinkState::Error;
    }
    match reply["data"]["platform"].as_str() {
        Some("darwin") => LinkState::Mac,
        Some("windows") if reply["data"]["registered"] == true => LinkState::Ready,
        Some("windows") => LinkState::Missing,
        _ => LinkState::Error,
    }
}

// Only an explicit `opened: true` counts as a launched browser; the modal never assumes success.
fn browser_opened(reply: &Value) -> bool {
    reply["ok"] == true && reply["data"]["opened"] == true
}

// The prepared folder must be an absolute path and match the browser's package kind.
fn prepared_folder(reply: &Value, browser: BrowserChoice) -> Option<String> {
    if reply["ok"] != true || reply["data"]["kind"].as_str() != Some(browser.kind()) {
        return None;
    }
    reply["data"]["folder"]
        .as_str()
        .filter(|folder| Path::new(folder).is_absolute())
        .map(str::to_owned)
}

fn reply_error(reply: &Value) -> String {
    reply["error"]
        .as_str()
        .unwrap_or("unknown error")
        .to_owned()
}

// Connect links first, then install in the browser, then choose the mode.
fn active_setup_step(link: LinkState, install_opened: bool) -> u8 {
    if !link.is_ready() {
        1
    } else if install_opened {
        3
    } else {
        2
    }
}

// The modal stays open only while developer mode is an explicit On; an explicit Off or any error closes it.
fn integrations_still_open(open: bool, developer_on: bool) -> bool {
    open && developer_on
}

impl SliceBridgeWindow {
    fn new(registry: Registry, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        // app.js remembers a user resize after 900ms, not on every repaint.
        // GPUI's observer fires on native resize. Persist content size, not
        // the outer frame: macOS titlebar adds 32px and would grow on restart.
        cx.observe_window_bounds(window, |view, window, cx| {
            let (width, height) = observed_window_dimensions(window.viewport_size());
            view.window_resize_revision = view.window_resize_revision.wrapping_add(1);
            let revision = view.window_resize_revision;
            let commands = view.commands.clone();
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(900))
                    .await;
                let latest = this
                    .update(cx, |view, _| view.window_resize_revision == revision)
                    .unwrap_or(false);
                if latest {
                    // Persist outside the UI executor, only in GPUI-Alpha.
                    cx.background_executor()
                        .spawn(async move {
                            commands.execute(
                                "settings.window.remember",
                                &json!({"width":width,"height":height}),
                            )
                        })
                        .await;
                }
            })
            .detach();
        })
        .detach();
        let locale = Locale::load();
        let placeholder = if locale.rtl() {
            "שם יצרן או דגם"
        } else {
            "Manufacturer or model"
        };
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let subscription = cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        let catalog = registry.execute("printers.catalog", &json!({}));
        let catalogue_cards = if catalog["ok"] == true {
            desktop_printer_picker::original_catalogue_cards(&catalog["data"])
        } else {
            Vec::new()
        };
        let installed_orca = registry.execute("printers.installed_orca", &json!({}));
        let installed_prusa = registry.execute("printers.installed_prusa", &json!({}));
        let cards = desktop_printer_picker::cards_with_installed_printers(
            &catalogue_cards,
            &installed_orca["data"],
            &installed_prusa["data"],
        );
        let discovered = registry.execute("profiles.discover", &json!({}));
        let read_settings = registry.execute("settings.get", &json!({}));
        let settings_value = if read_settings["ok"] == true {
            read_settings["data"].clone()
        } else {
            Value::Null
        };
        let ready_oem_targets: BTreeSet<String> =
            desktop_printer_settings_ui::ready_from_preferences(
                &discovered["data"],
                &settings_value,
                None,
            );
        let prefs = registry.execute("printers.get", &json!({}));
        let saved: Vec<String> = prefs["data"]["selectedIds"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        // Original app checks newly visible My Printers targets by default.
        let selected_batch_targets =
            desktop_printer_picker::selected_ready_legacy_rows(&saved, &ready_oem_targets)
                .into_iter()
                .map(|(id, _)| id.to_owned())
                .collect();
        Self {
            focus_handle,
            window_resize_revision: 0,
            commands: registry,
            last_message: IDLE_MESSAGE.into(),
            destination: None,
            destination_source: None,
            suppressed_source: None,
            source_subtitles: BTreeMap::new(),
            makerworld_origins: BTreeMap::new(),
            makerworld_pending: BTreeMap::new(),
            makerworld_token: 0,
            locale,
            printers_open: false,
            printers_first_run: prefs["data"]["onboardingPending"] == true,
            printers_cards: cards,
            printers_catalogue_cards: catalogue_cards,
            ready_oem_targets,
            selected_batch_targets,
            ready_profiles: discovered["data"].clone(),
            source_nozzle: None,
            printers_saved: saved,
            printers_draft: Vec::new(),
            printers_maker: None,
            printers_maker_menu: false,
            printers_show_all: false,
            printers_error: String::new(),
            printers_search: search,
            _printer_subscription: subscription,
            settings_open: false,
            settings_loaded: settings_value,
            settings_draft: Value::Null,
            settings_target: "u1".into(),
            settings_material: "PLA".into(),
            settings_options: Value::Null,
            settings_error: String::new(),
            settings_recovery_pending: false,
            settings_recovery_error: String::new(),
            about_open: false,
            about_developer_mode: false,
            about_developer_error: false,
            about_link_error: false,
            about_document: None,
            update: UpdateCheck::default(),
            integrations_open: false,
            integrations_developer: false,
            integrations_browser: BrowserChoice::default(),
            integrations_mode: None,
            integrations_link: LinkState::default(),
            integrations_page: None,
            integrations_folder: None,
            integrations_install_opened: false,
            integrations_install: Feedback::default(),
            integrations_mode_feedback: Feedback::default(),
            integrations_busy: false,
        }
    }

    // Repaint when CLI, MCP or attach changes the same Registry outside GPUI.
    // This is UI-only: no conversion polling command, slicer or printer action.
    fn start_live_refresh(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let mut previous = Value::Null;
            let mut tick: u32 = 0;
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
                tick = tick.wrapping_add(1);
                // Developer mode can also change from CLI or MCP, so the MakerWorld gate is re-read every
                // 2 seconds on the background executor. Only an explicit On keeps the setup modal usable.
                if tick % 4 == 1 {
                    let Ok(registry) = this.update(cx, |view, _| view.commands.clone()) else {
                        break;
                    };
                    let reply = cx
                        .background_executor()
                        .spawn(async move { registry.execute("settings.get", &json!({})) })
                        .await;
                    let gate = this.update(cx, |view, cx| {
                        if view.set_integrations_gate(developer_mode_enabled(&reply)) {
                            cx.notify();
                        }
                    });
                    if gate.is_err() {
                        break;
                    }
                }
                let updated = this.update(cx, |view, cx| {
                    let current = view.commands.snapshot();
                    if current != previous {
                        previous = current;
                        // Queue changes from CLI or MCP also reach the badges here, never from render.
                        view.sync_makerworld_origins(cx);
                        cx.notify();
                    }
                });
                if updated.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn run(&mut self, name: &str, args: Value, cx: &mut Context<Self>) {
        let source_before = self.commands.snapshot()["source"].clone();
        let result = self.commands.execute(name, &args);
        if name == "source.select" && result["ok"] == true {
            // Explicit re-selection may point to a replaced 3MF at the same
            // path: never inherit provenance from an old copy.
            if let Some(path) = args["path"].as_str() {
                self.makerworld_origins.remove(path);
                self.makerworld_pending.remove(path);
                self.source_subtitles.remove(path);
            }
            self.destination = None;
            self.destination_source = None;
            self.suppressed_source = None;
            self.source_nozzle = result["data"]["source_nozzle"].as_str().map(str::to_owned);
            self.ready_oem_targets = desktop_printer_settings_ui::ready_from_preferences(
                &self.ready_profiles,
                &self.settings_loaded,
                self.source_nozzle.as_deref(),
            );
        } else if name == "batch.clear" && result["ok"] == true {
            self.source_nozzle = None;
            self.ready_oem_targets = desktop_printer_settings_ui::ready_from_preferences(
                &self.ready_profiles,
                &self.settings_loaded,
                None,
            );
        } else if name == "batch.remove" && remove_reply_ok(&result) {
            self.suppressed_source = args["path"].as_str().map(str::to_owned);
            // Removing the active file makes the Registry clear the source; drop the source-tied state too.
            if self.commands.snapshot()["source"] != source_before {
                self.source_nozzle = None;
                self.ready_oem_targets = desktop_printer_settings_ui::ready_from_preferences(
                    &self.ready_profiles,
                    &self.settings_loaded,
                    None,
                );
            }
        }
        self.last_message = if clears_activity_message(name, &result) {
            IDLE_MESSAGE.into()
        } else if result["ok"] == true {
            format!("{name}: {}", result["data"])
        } else {
            format!("Error: {}", result["error"].as_str().unwrap_or("unknown"))
        };
        self.sync_makerworld_origins(cx);
        cx.notify();
    }

    // Unlike a synchronous Registry call on the GPUI thread, this preserves
    // Go's best-effort *asynchronous* provenance check. A macOS mdls lookup can
    // take up to 1.5s per file; never freeze the main window for a large queue.
    // Paths and request IDs are all local, bounded, and never contain URLs.
    fn sync_makerworld_origins(&mut self, cx: &mut Context<Self>) {
        let snap = self.commands.snapshot();
        let display = queue_display_paths(&snap, self.suppressed_source.as_deref());
        sync_source_subtitles(&display, &mut self.source_subtitles);
        let unknown = origin_lookup_plan(&display, &mut self.makerworld_origins);
        self.makerworld_pending
            .retain(|path, _| display.contains(path));
        let to_query: Vec<String> = unknown
            .into_iter()
            .filter(|path| !self.makerworld_pending.contains_key(path))
            .collect();
        for chunk in to_query.chunks(MAKERWORLD_ORIGIN_LIMIT) {
            let paths = chunk.to_vec();
            self.makerworld_token = self.makerworld_token.wrapping_add(1);
            let token = self.makerworld_token;
            for path in &paths {
                self.makerworld_pending.insert(path.clone(), token);
            }
            let registry = self.commands.clone();
            cx.spawn(async move |this, cx| {
                let for_query = paths.clone();
                let reply = cx
                    .background_executor()
                    .spawn(async move {
                        registry.execute("makerworld.origins", &json!({"paths":for_query}))
                    })
                    .await;
                let _ = this.update(cx, |view, cx| {
                    let snapshot = view.commands.snapshot();
                    let display = queue_display_paths(&snapshot, view.suppressed_source.as_deref());
                    if apply_origin_reply(
                        &mut view.makerworld_origins,
                        &mut view.makerworld_pending,
                        &display,
                        &paths,
                        token,
                        &reply,
                    ) > 0
                    {
                        cx.notify();
                    }
                });
            })
            .detach();
        }
    }

    fn makerworld_verified(&self, path: &str) -> bool {
        self.makerworld_origins.get(path).copied().unwrap_or(false)
    }

    fn open_about(&mut self, cx: &mut Context<Self>) {
        // Read the preference on each open: CLI or MCP may have changed it while the window was running.
        let reply = self.commands.execute("settings.get", &json!({}));
        self.about_developer_mode = developer_mode_enabled(&reply);
        self.set_integrations_gate(self.about_developer_mode);
        self.about_developer_error = reply["ok"] != true;
        self.about_link_error = false;
        self.about_document = None;
        self.about_open = true;
        cx.notify();
    }

    // Launch only on a real click. The launcher's exit status is awaited off the UI thread, so a
    // spawn failure or a non-zero launcher exit shows the link error instead of a false success.
    fn open_about_link(&mut self, kind: &'static str, cx: &mut Context<Self>) {
        let Some(url) = about_page_url(kind) else {
            self.about_link_error = true;
            cx.notify();
            return;
        };
        self.about_link_error = false;
        let mut command = about_open_command(url);
        cx.spawn(async move |this, cx| {
            let opened = cx
                .background_executor()
                .spawn(async move { command.status().is_ok_and(|status| status.success()) })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.about_link_error = !opened;
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn toggle_about_developer_mode(&mut self, cx: &mut Context<Self>) {
        let requested = !self.about_developer_mode;
        let reply = self
            .commands
            .execute("settings.developer.set", &json!({"enabled":requested}));
        if reply["ok"] == true && reply["data"]["enabled"] == requested {
            self.about_developer_mode = requested;
            self.about_developer_error = false;
        } else {
            // As in stable Go, an error never reports Developer Mode enabled.
            self.about_developer_mode = false;
            self.about_developer_error = true;
        }
        self.set_integrations_gate(self.about_developer_mode);
        cx.notify();
    }

    fn close_about(&mut self, cx: &mut Context<Self>) {
        self.about_open = false;
        self.about_link_error = false;
        self.about_document = None;
        self.update.close();
        cx.notify();
    }

    // Runs only on a deliberate click. The Registry read can take up to 13 seconds, so it runs on
    // the background executor, and a second click cannot start an overlapping request.
    fn check_for_updates(&mut self, cx: &mut Context<Self>) {
        let Some(request) = self.update.begin() else {
            return;
        };
        let registry = self.commands.clone();
        cx.spawn(async move |this, cx| {
            let reply = cx
                .background_executor()
                .spawn(async move { registry.execute("updates.check", &json!({})) })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.update.finish(request, &reply);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn dismiss_update_notice(&mut self, cx: &mut Context<Self>) {
        self.update.dismiss();
        cx.notify();
    }

    // Only a page reply the Registry produced and the allowlist accepts reaches the launcher.
    // Launch happens off the UI thread, and a failed launch shows the link error.
    fn open_update_action(&mut self, action: UpdateAction, cx: &mut Context<Self>) {
        let reply = self.commands.execute("updates.page", &action.page_params());
        let url = reply["data"]["url"]
            .as_str()
            .filter(|url| reply["ok"] == true && allowed_update_url(url))
            .map(str::to_owned);
        let Some(url) = url else {
            self.update.error = reply["error"]
                .as_str()
                .unwrap_or("invalid update page")
                .to_owned();
            cx.notify();
            return;
        };
        self.update.error.clear();
        self.update.link_error = false;
        let request = self.update.request;
        let mut command = about_open_command(&url);
        cx.spawn(async move |this, cx| {
            let opened = cx
                .background_executor()
                .spawn(async move { command.status().is_ok_and(|status| status.success()) })
                .await;
            let _ = this.update(cx, |view, cx| {
                // A stale browser-launch reply must not rewrite the notice
                // after the user closed About or started another check.
                if view.about_open && view.update.request == request {
                    view.update.link_error = !opened;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn toggle_about_document(&mut self, document: AboutDocument, cx: &mut Context<Self>) {
        self.about_document = if self.about_document == Some(document) {
            None
        } else {
            Some(document)
        };
        cx.notify();
    }

    // Setup-modal Registry calls run off the UI thread; the reply is applied back on it, as updates are.
    fn ask_setup(
        &mut self,
        name: &'static str,
        args: Value,
        cx: &mut Context<Self>,
        apply: impl FnOnce(&mut Self, Value) + 'static,
    ) {
        let registry = self.commands.clone();
        cx.spawn(async move |this, cx| {
            let reply = cx
                .background_executor()
                .spawn(async move { registry.execute(name, &args) })
                .await;
            let _ = this.update(cx, |view, cx| {
                apply(view, reply);
                cx.notify();
            });
        })
        .detach();
    }

    // Read-only status and page lookups. Nothing is written or opened here.
    fn load_setup_state(&mut self, cx: &mut Context<Self>) {
        self.ask_setup("browser.setup.status", json!({}), cx, |view, reply| {
            view.integrations_link = link_state_from(&reply);
        });
        self.load_browser_page(cx);
    }

    fn load_browser_page(&mut self, cx: &mut Context<Self>) {
        let browser = self.integrations_browser;
        self.integrations_page = None;
        self.ask_setup(
            "browser.setup.page",
            json!({"browser": browser.key()}),
            cx,
            move |view, reply| {
                // A reply for a browser the user has since switched away from is dropped.
                if view.integrations_browser == browser && reply["ok"] == true {
                    view.integrations_page =
                        reply["data"]["url"].as_str().and_then(allowed_browser_page);
                }
            },
        );
    }

    // Topbar click only. The preference is read again so the modal never opens on stale state.
    fn open_integrations(&mut self, cx: &mut Context<Self>) {
        let reply = self.commands.execute("settings.get", &json!({}));
        self.set_integrations_gate(developer_mode_enabled(&reply));
        if self.integrations_developer {
            self.integrations_open = true;
            self.integrations_link = LinkState::Unknown;
            self.load_setup_state(cx);
        }
        cx.notify();
    }

    fn close_integrations(&mut self, cx: &mut Context<Self>) {
        self.integrations_open = false;
        cx.notify();
    }

    // Applies a developer-mode reading. Anything other than an explicit On locks the modal and closes it.
    // Returns true when the state changed, so callers repaint only then.
    fn set_integrations_gate(&mut self, enabled: bool) -> bool {
        let before = (self.integrations_developer, self.integrations_open);
        self.integrations_developer = enabled;
        self.integrations_open = integrations_still_open(self.integrations_open, enabled);
        before != (self.integrations_developer, self.integrations_open)
    }

    fn select_integrations_browser(&mut self, browser: BrowserChoice, cx: &mut Context<Self>) {
        if self.integrations_busy {
            return;
        }
        self.integrations_browser = browser;
        self.integrations_folder = None;
        self.integrations_install_opened = false;
        self.integrations_install = Feedback::default();
        self.load_browser_page(cx);
        cx.notify();
    }

    // Explicit click only: writes the extension folder for the selected browser and shows its path.
    fn prepare_integrations_extension(&mut self, cx: &mut Context<Self>) {
        if self.integrations_busy {
            return;
        }
        let browser = self.integrations_browser;
        self.integrations_busy = true;
        self.ask_setup(
            "browser.setup.prepare",
            json!({"browser": browser.key()}),
            cx,
            move |view, reply| {
                view.integrations_busy = false;
                if view.integrations_browser != browser {
                    return;
                }
                match prepared_folder(&reply, browser) {
                    Some(folder) => {
                        view.integrations_folder = Some(folder);
                        view.integrations_install = Feedback::default();
                    }
                    None => {
                        view.integrations_folder = None;
                        view.integrations_install = Feedback::failed(format!(
                            "{}{}",
                            view.locale.text("folderError"),
                            reply_error(&reply)
                        ));
                    }
                }
            },
        );
        cx.notify();
    }

    // Explicit click only: opens the selected browser's fixed extensions page and nothing else.
    fn open_integrations_manager(&mut self, cx: &mut Context<Self>) {
        if self.integrations_busy {
            return;
        }
        let browser = self.integrations_browser;
        self.integrations_busy = true;
        self.ask_setup(
            "browser.setup.open",
            json!({"browser": browser.key(), "kind": "manager"}),
            cx,
            move |view, reply| {
                view.integrations_busy = false;
                if view.integrations_browser != browser {
                    return;
                }
                if browser_opened(&reply) {
                    view.integrations_install_opened = true;
                    let key = if browser == BrowserChoice::Firefox {
                        "betaOpenedFirefox"
                    } else {
                        "betaOpened"
                    };
                    view.integrations_install = Feedback::ok(view.locale.text(key));
                } else {
                    view.integrations_install = Feedback::failed(format!(
                        "{}{}",
                        view.locale.text("modeError"),
                        reply_error(&reply)
                    ));
                }
            },
        );
        cx.notify();
    }

    fn select_integrations_mode(&mut self, mode: LinkMode, cx: &mut Context<Self>) {
        self.integrations_mode = Some(mode);
        self.integrations_mode_feedback = Feedback::default();
        cx.notify();
    }

    // Explicit click only, and only after a mode is chosen. The browser opens; the extension must confirm.
    fn open_integrations_mode(&mut self, cx: &mut Context<Self>) {
        if self.integrations_busy {
            return;
        }
        let Some(mode) = self.integrations_mode else {
            self.integrations_mode_feedback = Feedback::failed(self.locale.text("selectModeFirst"));
            cx.notify();
            return;
        };
        let browser = self.integrations_browser;
        self.integrations_busy = true;
        self.ask_setup(
            "browser.setup.open",
            json!({"browser": browser.key(), "kind": "mode", "mode": mode.key()}),
            cx,
            move |view, reply| {
                view.integrations_busy = false;
                view.integrations_mode_feedback = if browser_opened(&reply) {
                    Feedback::ok(format!(
                        "{} {}",
                        view.locale.text("modeLaunched"),
                        view.locale.text("modeNoExtension")
                    ))
                } else {
                    Feedback::failed(format!(
                        "{}{}",
                        view.locale.text("modeError"),
                        reply_error(&reply)
                    ))
                };
            },
        );
        cx.notify();
    }

    fn choose_file(&mut self, command: &'static str, cx: &mut Context<Self>) {
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Select SliceBridge input".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picker.await {
                if let Some(path) = paths.first() {
                    let path = path.to_string_lossy().to_string();
                    let _ = this.update(cx, |view, cx| {
                        view.run(command, json!({"path":path}), cx);
                    });
                }
            }
        })
        .detach();
    }

    fn choose_batch_sources(&mut self, cx: &mut Context<Self>) {
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Add 3MF source files".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picker.await {
                let _ = this.update(cx, |view, cx| {
                    view.queue_add(paths, cx);
                });
            }
        })
        .detach();
    }

    fn choose_source_folder(&mut self, cx: &mut Context<Self>) {
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Add folder containing 3MF projects".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picker.await {
                let _ = this.update(cx, |view, cx| view.queue_add(paths, cx));
            }
        })
        .detach();
    }

    fn queue_add(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let snap = self.commands.snapshot();
        let selected = paths.clone();
        let mut existing: Vec<PathBuf> =
            queue_display_paths(&snap, self.suppressed_source.as_deref())
                .into_iter()
                .map(PathBuf::from)
                .collect();
        existing.extend(paths);
        match collect_source_files(existing) {
            Ok(files) if !files.is_empty() => {
                let encoded: Vec<String> = files
                    .iter()
                    .map(|path| path.to_string_lossy().into_owned())
                    .collect();
                let reply = self
                    .commands
                    .execute("batch.stage", &json!({"paths":encoded}));
                if reply["ok"] == true {
                    self.suppressed_source = None;
                    // Folder picks expand to files. Invalidate only selected
                    // file keys, including descendants of a reselected folder.
                    // Unrelated staged files keep their cached provenance.
                    for file in &files {
                        if selected_queue_file_needs_refresh(file, &selected) {
                            let key = file.to_string_lossy();
                            self.makerworld_origins.remove(key.as_ref());
                            self.makerworld_pending.remove(key.as_ref());
                            self.source_subtitles.remove(key.as_ref());
                        }
                    }
                    if snap["source"].is_null() {
                        self.run(
                            "source.select",
                            json!({"path":files[0].to_string_lossy()}),
                            cx,
                        );
                    }
                    // Go's file staging resets memoryStatus; no "Queued files" echo is shown.
                    self.last_message = IDLE_MESSAGE.into();
                } else {
                    self.last_message = reply["error"]
                        .as_str()
                        .unwrap_or("Could not stage sources")
                        .to_owned();
                }
            }
            Ok(_) => {
                self.last_message = self.locale.text("queueEmpty");
            }
            Err(error) => {
                self.last_message = error;
            }
        }
        self.sync_makerworld_origins(cx);
        cx.notify();
    }

    fn choose_batch_directory(&mut self, cx: &mut Context<Self>) {
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Select existing folder for converted batch outputs".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picker.await {
                if let Some(path) = paths.first() {
                    let path = path.to_string_lossy().to_string();
                    let _ = this.update(cx, |view, cx| {
                        view.run("batch.directory", json!({"path":path}), cx);
                    });
                }
            }
        })
        .detach();
    }

    fn set_locale(&mut self, locale: Locale, window: &mut Window, cx: &mut Context<Self>) {
        self.locale = locale;
        let placeholder = if locale.rtl() {
            "שם יצרן או דגם"
        } else {
            "Manufacturer or model"
        };
        self.printers_search.update(cx, |input, cx| {
            input.set_placeholder(placeholder, window, cx)
        });
        if let Err(error) = locale.save() {
            self.last_message = format!("Could not remember language selection: {error}");
        }
        cx.notify();
    }

    fn toggle_batch_target(&mut self, target: &'static str, cx: &mut Context<Self>) {
        if !self.selected_batch_targets.remove(target) {
            self.selected_batch_targets.insert(target.to_owned());
        }
        // Go target checkbox changes clear memoryStatus before renderTargets.
        self.last_message = IDLE_MESSAGE.into();
        cx.notify();
    }

    fn select_all_batch_targets(&mut self, cx: &mut Context<Self>) {
        let visible: BTreeSet<String> = desktop_printer_picker::selected_ready_legacy_rows(
            &self.printers_saved,
            &self.ready_oem_targets,
        )
        .into_iter()
        .map(|(id, _)| id.to_owned())
        .collect();
        if visible.is_empty() {
            // Go's selectAll returns without clearing memoryStatus when no
            // selectable printer rows exist.
            return;
        }
        if visible
            .iter()
            .all(|id| self.selected_batch_targets.contains(id))
        {
            // Go's button toggles between selecting and clearing shown targets.
            self.selected_batch_targets.clear();
        } else {
            self.selected_batch_targets.extend(visible);
        }
        self.last_message = IDLE_MESSAGE.into();
        cx.notify();
    }

    fn start_batch(&mut self, cx: &mut Context<Self>) {
        let snapshot = self.commands.snapshot();
        let paths = queue_display_paths(&snapshot, self.suppressed_source.as_deref());
        let targets = desktop_printer_picker::selected_ready_legacy_rows(
            &self.printers_saved,
            &self.ready_oem_targets,
        )
        .into_iter()
        .filter(|(id, _)| self.selected_batch_targets.contains(*id))
        .map(|(id, _)| id.to_owned())
        .collect::<Vec<_>>();
        if paths.is_empty() || targets.is_empty() {
            self.last_message = self.locale.text("noSource");
            cx.notify();
            return;
        }
        self.run("batch.start", json!({"paths":paths,"targets":targets}), cx);
    }

    fn open_output_folder(&mut self, cx: &mut Context<Self>) {
        let snapshot = self.commands.snapshot();
        let Some(folder) = output_folder(&snapshot) else {
            self.last_message = self.locale.text("failedHint");
            cx.notify();
            return;
        };
        if !folder.is_dir() {
            self.last_message = "Output folder no longer exists".into();
            cx.notify();
            return;
        }
        let executable = if cfg!(windows) {
            "explorer.exe"
        } else if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        match Command::new(executable).arg(&folder).spawn() {
            Ok(_) => {
                self.last_message = folder.to_string_lossy().into_owned();
            }
            Err(error) => {
                self.last_message = format!("Could not open output folder: {error}");
            }
        }
        cx.notify();
    }

    fn handle_dropped_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        // Same source queue path for file selection, folder selection and native OS drops.
        self.queue_add(paths, cx);
    }

    fn choose_destination(&mut self, cx: &mut Context<Self>) {
        let current = self.commands.snapshot();
        let source = current["source"].as_str().unwrap_or("");
        let directory = Path::new(source)
            .parent()
            .unwrap_or(Path::new("."))
            .to_owned();
        let picker = cx.prompt_for_new_path(&directory, Some("converted.3mf"));
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(path))) = picker.await {
                let _ = this.update(cx, |view, cx| {
                    view.destination = Some(path.to_string_lossy().to_string());
                    view.destination_source = view.commands.snapshot()["source"]
                        .as_str()
                        .map(str::to_owned);
                    view.last_message =
                        "Destination selected; original will never be overwritten.".into();
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn choose_slicer(&mut self, cx: &mut Context<Self>) {
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Select installed slicer executable".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picker.await {
                if let Some(path) = paths.first() {
                    let executable = path.to_string_lossy().to_string();
                    let _ = this.update(cx, |view, cx| {
                        view.run("slicer.open", json!({"executable": executable}), cx);
                    });
                }
            }
        })
        .detach();
    }

    fn start_conversion(&mut self, cx: &mut Context<Self>) {
        let source = self.commands.snapshot()["source"]
            .as_str()
            .map(str::to_owned);
        if source.is_none() || self.destination_source != source {
            self.last_message = "Choose a new destination for the selected source.".into();
            cx.notify();
            return;
        }
        if let Some(output) = &self.destination {
            self.run("conversion.start", json!({"output": output}), cx);
        } else {
            self.last_message = "Choose a destination before converting.".into();
            cx.notify();
        }
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        self.run("conversion.cancel", json!({}), cx);
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let snap = self.commands.snapshot();
        self.last_message = format!(
            "{} — {}",
            snap["phase"].as_str().unwrap_or("idle"),
            snap["result"]
        );
        cx.notify();
    }

    fn select_oem(&mut self, target: &'static str, nozzle: &'static str, cx: &mut Context<Self>) {
        self.run(
            "profiles.select",
            json!({"target":target,"nozzle":nozzle}),
            cx,
        );
    }

    fn next_material(&mut self, material: &'static str, cx: &mut Context<Self>) {
        let reply = self
            .commands
            .execute("profiles.materials", &json!({"material":material}));
        if reply["ok"] != true {
            self.last_message = format!("OEM material options: {}", reply["error"]);
            cx.notify();
            return;
        }
        let options = reply["data"]["presets"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if options.is_empty() {
            self.last_message = format!("No compatible installed {material} presets");
            cx.notify();
            return;
        }
        let current = reply["data"]["selected"].as_str().unwrap_or("");
        let next = options
            .iter()
            .position(|v| v.as_str() == Some(current))
            .map(|idx| (idx + 1) % options.len())
            .unwrap_or(0);
        if let Some(preset) = options[next].as_str() {
            self.run(
                "material.select",
                json!({"material":material,"preset":preset}),
                cx,
            );
        }
    }
}

#[cfg(test)]
mod source_locked_keyboard_shortcut_tests {
    use super::{original_workspace_shortcut, WorkspaceShortcut};
    use gpui_kit::{Keystroke, Modifiers};

    #[test]
    fn original_wails_keyboard_shortcuts_and_guards() {
        let stroke = |key: &str, modifiers: Modifiers| Keystroke {
            key: key.into(),
            modifiers,
            key_char: None,
        };
        let ctrl = Modifiers {
            control: true,
            ..Default::default()
        };
        let cmd = Modifiers {
            platform: true,
            ..Default::default()
        };
        let shift_cmd = Modifiers { shift: true, ..cmd };
        assert_eq!(
            original_workspace_shortcut(&stroke("o", ctrl), false, false),
            Some(WorkspaceShortcut::AddFiles)
        );
        assert_eq!(
            original_workspace_shortcut(&stroke("O", cmd), false, false),
            Some(WorkspaceShortcut::AddFiles)
        );
        assert_eq!(
            original_workspace_shortcut(&stroke("o", shift_cmd), false, false),
            Some(WorkspaceShortcut::AddFolder)
        );
        assert_eq!(
            original_workspace_shortcut(&stroke(",", ctrl), false, false),
            Some(WorkspaceShortcut::Settings)
        );
        assert_eq!(
            original_workspace_shortcut(&stroke(",", cmd), false, false),
            Some(WorkspaceShortcut::Settings)
        );
        assert_eq!(
            original_workspace_shortcut(&stroke("o", ctrl), true, false),
            None
        );
        assert_eq!(
            original_workspace_shortcut(&stroke("o", ctrl), false, true),
            None
        );
        assert_eq!(
            original_workspace_shortcut(&stroke("o", Modifiers::default()), false, false),
            None
        );
        assert_eq!(
            original_workspace_shortcut(
                &stroke("o", Modifiers { alt: true, ..ctrl }),
                false,
                false
            ),
            None
        );
        assert_eq!(
            original_workspace_shortcut(&stroke("x", ctrl), false, false),
            None
        );
    }
}

#[path = "desktop/layout.rs"]
mod desktop_layout;

impl Render for SliceBridgeWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        desktop_layout::render_window(self, window, cx)
    }
}

fn main() {
    let registry = Registry::new();
    let saved_settings = registry.execute("settings.get", &json!({}));
    let (initial_width, initial_height) = initial_window_dimensions(&saved_settings);
    let attach_status = protocol::start_attach(registry.clone(), "127.0.0.1:48637");
    if let Err(error) = attach_status {
        eprintln!("local attach not available: {error}");
    }
    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        let commands = registry.clone();
        let bounds = Bounds::centered(
            None,
            size(px(initial_width as f32), px(initial_height as f32)),
            cx,
        );
        // Exact stable Wails dimensions and minimum; never recenter on redraw.
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(720.), px(500.))),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, move |window, cx| {
            let on_close = commands.clone();
            window.on_window_should_close(cx, move |window, _| {
                // Wails WindowGetSize is the drawable size, not macOS frame.
                let (width, height) = observed_window_dimensions(window.viewport_size());
                let result = on_close.execute(
                    "settings.window.remember",
                    &json!({"width":width,"height":height}),
                );
                if result["ok"] != true {
                    eprintln!(
                        "SliceBridge could not remember window size: {}",
                        reply_error(&result)
                    );
                }
                true // Never block closing, same as Wails OnBeforeClose.
            });
            cx.new(|cx| {
                let mut view = SliceBridgeWindow::new(commands, window, cx);
                view.start_live_refresh(cx);
                view
            })
        })
        .expect("SliceBridge GPUI window failed to open");
    });
}

#[cfg(test)]
mod original_window_geometry_tests {
    use super::{initial_window_dimensions, observed_window_dimensions};
    use gpui_kit::{px, size};
    use serde_json::json;

    #[test]
    fn saved_wails_size_is_used_only_when_valid() {
        assert_eq!(
            initial_window_dimensions(&json!({"ok":true,"data":{"window":{
                "width":1100,"height":750
            }}})),
            (1100, 750)
        );
        assert_eq!(
            initial_window_dimensions(&json!({"ok":false,"error":"corrupt"})),
            (1280, 900)
        );
        assert_eq!(
            initial_window_dimensions(&json!({"data":{"window":{
                "width":700,"height":1500
            }}})),
            (1280, 900)
        );
        assert_eq!(
            observed_window_dimensions(size(px(1150.4), px(790.7))),
            (1150, 791)
        );
    }
}

#[cfg(test)]
mod original_output_folder_tests {
    use super::output_folder;
    use serde_json::json;
    use std::path::PathBuf;

    #[test]
    fn only_successful_conversion_exposes_output_folder() {
        let status = json!({
            "phase":"idle",
            "batch":{"items":[
                {"phase":"failed","output":"failed/part.3mf"},
                {"phase":"completed","output":"outputs/part.3mf"}
            ]}
        });
        assert_eq!(output_folder(&status), Some(PathBuf::from("outputs")));
        assert!(output_folder(&json!({
            "phase":"failed","output":"outputs/failed.3mf","batch":null
        }))
        .is_none());
        assert_eq!(
            output_folder(&json!({
                "phase":"completed","output":"converted/file.3mf","batch":null
            })),
            Some(PathBuf::from("converted"))
        );
        let mixed = json!({
            "batch":{"items":[
                {"phase":"failed","targets":[
                    {"phase":"completed","output":"source-folder/U1-part.3mf"},
                    {"phase":"failed","output":"source-folder/XPlus5-part.3mf"}
                ]}
            ]}
        });
        assert_eq!(output_folder(&mixed), Some(PathBuf::from("source-folder")));
        assert!(output_folder(&json!({
            "batch":{"items":[
                {"phase":"failed","targets":[
                    {"phase":"failed","output":"source-folder/never-created.3mf"}
                ]}
            ]}
        }))
        .is_none());
    }
}

#[cfg(test)]
mod layout_helper_tests {
    use super::desktop_i18n::Locale;
    use super::desktop_layout::{
        activity_detail_text, converting_detail_text, empty_target_notice_text,
        empty_target_notice_visible, receipt_error, result_counts, selected_ready_count,
    };
    use serde_json::json;
    use serde_json::Value;
    use std::collections::BTreeSet;

    #[test]
    fn converting_detail_names_the_active_file_from_registry_batch() {
        let batch = json!({
            "phase":"running","total":3,"active_index":1,
            "items":[{"source":"/jobs/a.3mf"},{"source":"/jobs/b.3mf"},{"source":"/jobs/c.3mf"}]
        });
        // Literal Go strings in native/frontend/dist/app.js (convertingFile), 1-based index.
        assert_eq!(
            converting_detail_text(Locale::En, &batch),
            "Project 2 of 3: b.3mf"
        );
        assert_eq!(
            converting_detail_text(Locale::He, &batch),
            "קובץ 2 מתוך 3: b.3mf"
        );
        // Between files, a single conversion with no batch, or a stale index: no invented file.
        let between = json!({"phase":"running","total":3,"active_index":null,"items":[]});
        assert_eq!(converting_detail_text(Locale::En, &between), "…");
        assert_eq!(converting_detail_text(Locale::En, &json!(null)), "…");
        let stale = json!({"total":1,"active_index":4,"items":[{"source":"/a.3mf"}]});
        assert_eq!(converting_detail_text(Locale::En, &stale), "…");
        // A stale active index must not display an impossible "Project 5 of 1".
        let contradictory = json!({"total":1,"active_index":4,
            "items":[{"source":"0.3mf"},{"source":"1.3mf"},{"source":"2.3mf"},
                     {"source":"3.3mf"},{"source":"4.3mf"}]});
        assert_eq!(converting_detail_text(Locale::He, &contradictory), "…");
    }

    #[test]
    fn idle_detail_becomes_go_ready_summary_only_with_files_and_printers() {
        let idle =
            "No UI action yet. Agent / CLI / MCP changes appear in the project fields above.";
        // Literal Go strings in native/frontend/dist/app.js (readyHint, idleHint).
        assert_eq!(
            activity_detail_text(Locale::En, idle, 2, 1),
            "2 projects × 1 printers"
        );
        assert_eq!(
            activity_detail_text(Locale::He, idle, 2, 1),
            "2 קבצים × 1 מדפסות"
        );
        // Zero files or zero selected printers is work=0 in Go, so the idle hint stays.
        assert_eq!(
            activity_detail_text(Locale::En, idle, 0, 1),
            "Add projects and select printers to start"
        );
        assert_eq!(
            activity_detail_text(Locale::En, idle, 2, 0),
            "Add projects and select printers to start"
        );
        assert_eq!(
            activity_detail_text(Locale::He, idle, 0, 0),
            "בחר קבצים ומדפסות כדי להתחיל"
        );
    }

    #[test]
    fn transient_message_keeps_precedence_over_ready_summary() {
        assert_eq!(
            activity_detail_text(Locale::En, "Output folder no longer exists", 2, 1),
            "Output folder no longer exists"
        );
        assert_eq!(
            activity_detail_text(Locale::He, "Choose a destination before converting.", 0, 0),
            "Choose a destination before converting."
        );
    }

    #[test]
    fn selected_ready_count_ignores_stale_or_unavailable_printers() {
        let rows = [("u1", "Ultimaker"), ("qidi", "Qidi")];
        let mut selected = BTreeSet::new();
        assert_eq!(selected_ready_count(&rows, &selected), 0);
        // A selection for a printer that is no longer ready must not inflate the count.
        selected.insert("gone".to_string());
        assert_eq!(selected_ready_count(&rows, &selected), 0);
        selected.insert("qidi".to_string());
        assert_eq!(selected_ready_count(&rows, &selected), 1);
        assert_eq!(selected_ready_count(&[], &selected), 0);
    }

    #[test]
    fn empty_target_notice_matches_go_copy() {
        // Literal Go strings in native/frontend/dist/app.js (emptyTargetNotice).
        assert_eq!(
            empty_target_notice_text(true),
            "לא נבחרו מדפסות יעד. אפשר להוסיף דרך סמל המדפסת בראש החלון."
        );
        assert_eq!(
            empty_target_notice_text(false),
            "No target printers selected. Use the printer icon at the top to add them."
        );
    }

    #[test]
    fn receipt_error_keeps_code_and_message() {
        assert_eq!(
            receipt_error(&json!({"code":"SOURCE_INVALID","error":"bad 3MF"})),
            "SOURCE_INVALID: bad 3MF"
        );
        assert_eq!(receipt_error(&json!({"error":"bad 3MF"})), "bad 3MF");
        assert_eq!(
            receipt_error(&json!({"code":"OUTPUT_COLLISIONS"})),
            "OUTPUT_COLLISIONS"
        );
        assert_eq!(
            receipt_error(&json!({"code":"X","data":{"error":"nested"}})),
            "X: nested"
        );
        assert_eq!(receipt_error(&Value::Null), "");
    }

    #[test]
    fn result_counts_use_results_and_failures_arrays() {
        let item = json!({
            "results": [{"target":"u1","receipt":null}],
            "failures": [{"target":"qidi","receipt":{"code":"X"}}, {"target":"k2","receipt":null}]
        });
        assert_eq!(result_counts(&item), (1, 3));
        assert_eq!(result_counts(&json!({"phase":"queued"})), (0, 0));
        assert_eq!(result_counts(&json!({"phase":"completed"})), (1, 1));
        assert_eq!(result_counts(&json!({"phase":"failed"})), (0, 1));
    }

    #[test]
    fn empty_target_notice_requires_no_selected_ready_printer() {
        let rows = [("u1", "Ultimaker"), ("qidi", "Qidi")];
        let mut selected = BTreeSet::new();
        assert!(empty_target_notice_visible(&rows, &selected));
        // A stale selection that matches no ready printer still leaves nothing to convert to.
        selected.insert("gone".to_string());
        assert!(empty_target_notice_visible(&rows, &selected));
        selected.insert("qidi".to_string());
        assert!(!empty_target_notice_visible(&rows, &selected));
        assert!(empty_target_notice_visible(&[], &BTreeSet::new()));
    }
}

#[cfg(test)]
mod queue_subtitle_tests {
    use super::{
        selected_queue_file_needs_refresh, source_bytes_label, source_queue_subtitle,
        sync_source_subtitles,
    };
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    #[test]
    fn reselecting_a_directory_refreshes_only_files_inside_it() {
        let picked = PathBuf::from("/work/models");
        let added_file = picked.join("sub/updated.3mf");
        assert!(selected_queue_file_needs_refresh(
            &added_file,
            &[picked.clone()]
        ));
        assert!(selected_queue_file_needs_refresh(
            &picked.join("single.3mf"),
            &[picked.clone()]
        ));
        assert!(!selected_queue_file_needs_refresh(
            &PathBuf::from("/work/models-other/unchanged.3mf"),
            &[picked.clone()]
        ));
        assert!(!selected_queue_file_needs_refresh(
            &PathBuf::from("/work/unrelated.3mf"),
            &[picked]
        ));
        let direct = PathBuf::from("/work/direct.3mf");
        assert!(selected_queue_file_needs_refresh(
            &direct,
            &[direct.clone()]
        ));
        assert!(!selected_queue_file_needs_refresh(
            &PathBuf::from("/work/direct-other.3mf"),
            &[direct]
        ));
    }

    #[test]
    fn source_size_uses_exact_go_bytes_label_thresholds() {
        assert_eq!(source_bytes_label(0), "0 B");
        assert_eq!(source_bytes_label(1023), "1023 B");
        assert_eq!(source_bytes_label(1024), "1.0 KB");
        assert_eq!(source_bytes_label(1536), "1.5 KB");
        assert_eq!(source_bytes_label(1_048_575), "1024.0 KB");
        assert_eq!(source_bytes_label(1_048_576), "1.0 MB");
        assert_eq!(source_bytes_label(250 * 1_048_576), "250.0 MB");
    }

    #[test]
    fn source_subtitle_uses_folder_and_size_with_safe_missing_file_fallback() {
        let parent = std::env::temp_dir().join("slicebridge-queue-parity");
        let path = parent.join("example.3mf").to_string_lossy().into_owned();
        let folder = parent.to_string_lossy().into_owned();
        assert_eq!(
            source_queue_subtitle(&path, Some(1536)),
            format!("{folder} · 1.5 KB")
        );
        assert_eq!(source_queue_subtitle(&path, None), folder);
    }

    #[test]
    fn subtitle_cache_keeps_known_rows_and_drops_removed_paths() {
        let path = std::env::temp_dir()
            .join("slicebridge-queue-parity-nonexistent.3mf")
            .to_string_lossy()
            .into_owned();
        let mut labels = BTreeMap::from([("old.3mf".into(), "old".into())]);
        sync_source_subtitles(std::slice::from_ref(&path), &mut labels);
        assert!(!labels.contains_key("old.3mf"));
        assert_eq!(labels.get(&path), Some(&source_queue_subtitle(&path, None)));
        labels.insert(path.clone(), "cached selection".into());
        sync_source_subtitles(std::slice::from_ref(&path), &mut labels);
        assert_eq!(
            labels.get(&path).map(String::as_str),
            Some("cached selection")
        );
        sync_source_subtitles(&[], &mut labels);
        assert!(labels.is_empty());
    }
}

#[cfg(test)]
mod about_tests {
    use super::desktop_i18n::Locale;
    use super::{
        about_open_command, about_page_url, developer_mode_enabled, AboutDocument,
        ABOUT_INSTALLED_VERSION,
    };
    use serde_json::json;

    const ABOUT_KEYS: &[&str] = &[
        "aboutOpen",
        "aboutTitle",
        "aboutSubtitle",
        "aboutVersion",
        "aboutDevTitle",
        "aboutDevWarning",
        "aboutDevButton",
        "aboutDevOn",
        "aboutDevOff",
        "aboutDevError",
        "aboutLicense",
        "aboutLicenseSummary",
        "aboutCredits",
        "aboutCreditBl",
        "aboutCreditWails",
        "aboutCreditVendor",
        "aboutFullLicense",
        "aboutFullCredits",
        "aboutHideDocument",
        "aboutSource",
        "aboutReleases",
        "aboutLinkError",
        "aboutOffline",
        "aboutClose",
    ];

    #[test]
    fn developer_mode_is_on_only_for_explicit_native_true() {
        assert!(developer_mode_enabled(
            &json!({"ok":true,"data":{"developerMode":true}})
        ));
        assert!(!developer_mode_enabled(
            &json!({"ok":true,"data":{"developerMode":"true"}})
        ));
        assert!(!developer_mode_enabled(&json!({"ok":true,"data":{}})));
        assert!(!developer_mode_enabled(
            &json!({"ok":false,"data":{"developerMode":true}})
        ));
        assert!(!developer_mode_enabled(&json!({"ok":true,"data":null})));
    }

    #[test]
    fn installed_version_matches_stable_go_app_version() {
        let go = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../native/updates_bridge.go"
        ));
        assert!(go.contains(&format!("var appVersion = \"{ABOUT_INSTALLED_VERSION}\"")));
    }

    #[test]
    fn bundled_documents_are_the_embedded_license_and_notices() {
        assert!(AboutDocument::License
            .text()
            .contains("GNU GENERAL PUBLIC LICENSE"));
        assert!(AboutDocument::Credits.text().contains("bl2std"));
        assert!(AboutDocument::Credits.text().contains("Wails"));
    }

    #[test]
    fn about_links_resolve_only_to_the_pinned_repository_endpoints() {
        assert_eq!(
            about_page_url("source"),
            Some("https://github.com/nocturney/slicebridge")
        );
        assert_eq!(
            about_page_url("releases"),
            Some("https://github.com/nocturney/slicebridge/releases")
        );
    }

    #[test]
    fn about_links_reject_any_kind_outside_the_allowlist() {
        for invalid in [
            "",
            "Source",
            "RELEASES",
            "release",
            " source",
            "source/../releases",
            "https://github.com/nocturney/slicebridge",
            "file:///etc/passwd",
            "--help",
        ] {
            assert_eq!(about_page_url(invalid), None, "{invalid:?}");
        }
    }

    #[test]
    fn about_open_command_passes_only_the_pinned_url_as_one_argument() {
        let url = "https://github.com/nocturney/slicebridge/releases";
        let command = about_open_command(url);
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        let program = command.get_program().to_string_lossy().into_owned();
        if cfg!(windows) {
            assert_eq!(program, "rundll32.exe");
            assert_eq!(args, ["url.dll,FileProtocolHandler", url]);
        } else if cfg!(target_os = "macos") {
            assert_eq!(program, "/usr/bin/open");
            assert_eq!(args, [url]);
        } else {
            assert_eq!(program, "xdg-open");
            assert_eq!(args, [url]);
        }
    }

    #[test]
    fn about_copy_exists_in_both_locales() {
        for key in ABOUT_KEYS {
            assert_ne!(Locale::He.text(key), *key, "he {key}");
            assert_ne!(Locale::En.text(key), *key, "en {key}");
        }
        assert_eq!(Locale::He.text("aboutOpen"), "אודות");
        assert_eq!(Locale::En.text("aboutTitle"), "About SliceBridge");
        assert_eq!(Locale::En.text("aboutDevOff"), "Off");
    }
}

#[cfg(test)]
mod queue_remove_tests {
    use super::desktop_i18n::Locale;
    use super::desktop_layout::activity_detail_text;
    use super::{
        clears_activity_message, file_button_id, queue_display_paths, remove_reply_ok,
        snapshot_running, IDLE_MESSAGE,
    };
    use serde_json::json;

    #[test]
    fn queue_remove_and_clear_return_to_go_ready_summary_only_on_success() {
        assert!(clears_activity_message(
            "batch.remove",
            &json!({"ok":true,"data":{"removed":true,"staged":0,"paths":[]}})
        ));
        assert!(clears_activity_message(
            "batch.clear",
            &json!({"ok":true,"data":{}})
        ));
        // Failed or no-op replies keep their status line.
        assert!(!clears_activity_message(
            "batch.remove",
            &json!({"ok":false,"error":"busy"})
        ));
        assert!(!clears_activity_message(
            "batch.remove",
            &json!({"ok":true,"data":{"removed":false}})
        ));
        assert!(!clears_activity_message(
            "batch.clear",
            &json!({"ok":false,"error":"busy"})
        ));
        // Other commands, including running or failed conversions, keep their message.
        assert!(!clears_activity_message(
            "conversion.start",
            &json!({"ok":true,"data":{}})
        ));
        assert!(!clears_activity_message(
            "conversion.cancel",
            &json!({"ok":false,"error":"no conversion"})
        ));
    }

    #[test]
    fn idle_message_resolves_to_go_ready_summary_in_both_languages() {
        // Literal Go strings in native/frontend/dist/app.js (readyHint); same as the layout test.
        assert_eq!(
            activity_detail_text(Locale::En, IDLE_MESSAGE, 2, 1),
            "2 projects × 1 printers"
        );
        assert_eq!(
            activity_detail_text(Locale::He, IDLE_MESSAGE, 2, 1),
            "2 קבצים × 1 מדפסות"
        );
    }

    #[test]
    fn staged_files_are_the_list_and_source_stands_in_only_when_nothing_is_staged() {
        let staged = json!({"source":"a.3mf","batch_selection":{"paths":["a.3mf","b.3mf"]}});
        assert_eq!(queue_display_paths(&staged, None), vec!["a.3mf", "b.3mf"]);
        let source_only = json!({"source":"a.3mf","batch_selection":{"paths":[]}});
        assert_eq!(queue_display_paths(&source_only, None), vec!["a.3mf"]);
        assert!(queue_display_paths(&json!({"source":null}), None).is_empty());
    }

    #[test]
    fn removed_source_is_not_reinserted_while_registry_still_reports_it() {
        let lagging = json!({"source":"a.3mf","batch_selection":{"paths":[]}});
        assert!(queue_display_paths(&lagging, Some("a.3mf")).is_empty());
        // A different active source is not hidden by an earlier removal.
        assert_eq!(
            queue_display_paths(&json!({"source":"c.3mf"}), Some("a.3mf")),
            vec!["c.3mf"]
        );
        // Staged files still show even when the removed path is still the source.
        let staged = json!({"source":"a.3mf","batch_selection":{"paths":["b.3mf"]}});
        assert_eq!(queue_display_paths(&staged, Some("a.3mf")), vec!["b.3mf"]);
    }

    #[test]
    fn only_a_successful_remove_reply_counts() {
        assert!(remove_reply_ok(
            &json!({"ok":true,"data":{"removed":true,"staged":1,"paths":["b.3mf"]}})
        ));
        assert!(!remove_reply_ok(&json!({"ok":false,"error":"busy"})));
        assert!(!remove_reply_ok(
            &json!({"ok":true,"data":{"removed":false}})
        ));
        assert!(!remove_reply_ok(&json!({"ok":true,"data":{}})));
    }

    #[test]
    fn file_button_ids_are_stable_unique_and_prefixed() {
        assert_eq!(
            file_button_id("source-remove", "a.3mf"),
            file_button_id("source-remove", "a.3mf")
        );
        assert_ne!(
            file_button_id("source-remove", "a.3mf"),
            file_button_id("source-remove", "b.3mf")
        );
        assert_ne!(
            file_button_id("source-remove", "a.3mf"),
            file_button_id("source-open", "a.3mf")
        );
        assert!(file_button_id("source-remove", "a.3mf").starts_with("source-remove-"));
    }

    #[test]
    fn conversion_phase_blocks_queue_removal() {
        assert!(snapshot_running(&json!({"phase":"running"})));
        assert!(snapshot_running(
            &json!({"phase":"idle","batch":{"phase":"running"}})
        ));
        assert!(!snapshot_running(
            &json!({"phase":"completed","batch":{"phase":"completed"}})
        ));
        assert!(!snapshot_running(&json!({"batch":null})));
    }
}

#[cfg(test)]
mod makerworld_badge_tests {
    use super::{
        apply_origin_reply, is_verified_makerworld, origin_lookup_plan, record_origin_reply,
        MAKERWORLD_ORIGIN_LIMIT,
    };
    use serde_json::json;
    use std::collections::BTreeMap;

    fn owned(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|path| (*path).to_owned()).collect()
    }

    #[test]
    fn badge_requires_exact_makerworld_kind_and_high_confidence() {
        assert!(is_verified_makerworld(
            &json!({"kind":"makerworld","confidence":"high","originUrl":"https://makerworld.com/m/1"})
        ));
        assert!(!is_verified_makerworld(
            &json!({"kind":"makerworld","confidence":"medium"})
        ));
        // A CDN host is never shown as MakerWorld, even if its confidence is claimed to be high.
        assert!(!is_verified_makerworld(
            &json!({"kind":"bambu_cdn","confidence":"medium"})
        ));
        assert!(!is_verified_makerworld(
            &json!({"kind":"bambu_cdn","confidence":"high"})
        ));
        assert!(!is_verified_makerworld(
            &json!({"kind":"MakerWorld","confidence":"high"})
        ));
        assert!(!is_verified_makerworld(
            &json!({"kind":"makerworld","confidence":true})
        ));
        assert!(!is_verified_makerworld(&json!({"kind":"makerworld"})));
        assert!(!is_verified_makerworld(
            &json!({"kind":"unknown","confidence":"high"})
        ));
        assert!(!is_verified_makerworld(&serde_json::Value::Null));
    }

    #[test]
    fn origin_lookup_requests_only_new_queue_paths() {
        let mut cache = BTreeMap::new();
        assert_eq!(
            origin_lookup_plan(&owned(&["a.3mf"]), &mut cache),
            owned(&["a.3mf"])
        );
        cache.insert("a.3mf".to_owned(), true);
        // Cached paths are not sent again on later snapshots or renders; only the new one is.
        assert_eq!(
            origin_lookup_plan(&owned(&["a.3mf", "b.3mf"]), &mut cache),
            owned(&["b.3mf"])
        );
        cache.insert("b.3mf".to_owned(), false);
        assert!(origin_lookup_plan(&owned(&["a.3mf", "b.3mf"]), &mut cache).is_empty());
    }

    #[test]
    fn removed_or_cleared_paths_lose_cached_provenance() {
        let mut cache = BTreeMap::from([("a.3mf".to_owned(), true), ("b.3mf".to_owned(), true)]);
        assert!(origin_lookup_plan(&owned(&["b.3mf"]), &mut cache).is_empty());
        assert!(!cache.contains_key("a.3mf"));
        assert_eq!(cache.get("b.3mf"), Some(&true));
        origin_lookup_plan(&[], &mut cache);
        assert!(cache.is_empty());
    }

    #[test]
    fn restaged_path_is_inspected_again_after_removal() {
        let mut cache = BTreeMap::from([("a.3mf".to_owned(), true)]);
        origin_lookup_plan(&[], &mut cache);
        // The same path back in the queue may be a replaced file, so it is inspected again.
        assert_eq!(
            origin_lookup_plan(&owned(&["a.3mf"]), &mut cache),
            owned(&["a.3mf"])
        );
    }

    #[test]
    fn failed_or_partial_reply_never_fabricates_provenance() {
        let paths = owned(&["a.3mf", "b.3mf", "c.3mf"]);
        let mut cache = BTreeMap::new();
        record_origin_reply(
            &mut cache,
            &paths,
            &json!({"ok":true,"data":{
                "a.3mf":{"kind":"makerworld","confidence":"high","originUrl":"https://makerworld.com/m/1"},
                "b.3mf":{"kind":"bambu_cdn","confidence":"medium"},
                "z.3mf":{"kind":"makerworld","confidence":"high"}
            }}),
        );
        assert_eq!(cache.get("a.3mf"), Some(&true));
        assert_eq!(cache.get("b.3mf"), Some(&false));
        // Omitted by the Registry means no provenance evidence, so no badge.
        assert_eq!(cache.get("c.3mf"), Some(&false));
        // A reply for a path that was not requested is ignored.
        assert!(!cache.contains_key("z.3mf"));
        record_origin_reply(
            &mut cache,
            &paths,
            &json!({"ok":false,"error":"too many paths"}),
        );
        assert!(paths.iter().all(|path| cache.get(path) == Some(&false)));
    }

    #[test]
    fn stale_background_results_cannot_restore_removed_or_replaced_badges() {
        let mut cache = BTreeMap::new();
        let path = owned(&["m.3mf"]);
        let mut inflight = BTreeMap::from([("m.3mf".to_owned(), 1_u64)]);
        let original = json!({"ok":true,"data":{"m.3mf":{
            "kind":"makerworld","confidence":"high"
        }}});
        // User removed the file after request 1; its reply must be discarded.
        assert_eq!(
            apply_origin_reply(&mut cache, &mut inflight, &[], &path, 1, &original),
            0
        );
        assert!(cache.is_empty());
        assert!(inflight.is_empty());

        // The same path is re-staged, now with request 2. The previous
        // signed-origin result cannot lend this newly selected file a badge.
        inflight.insert("m.3mf".to_owned(), 2);
        assert_eq!(
            apply_origin_reply(&mut cache, &mut inflight, &path, &path, 1, &original),
            0
        );
        assert!(cache.is_empty());
        assert_eq!(inflight.get("m.3mf"), Some(&2));

        let new_origin = json!({"ok":true,"data":{"m.3mf":{
            "kind":"bambu_cdn","confidence":"medium"
        }}});
        assert_eq!(
            apply_origin_reply(&mut cache, &mut inflight, &path, &path, 2, &new_origin),
            1
        );
        assert_eq!(cache.get("m.3mf"), Some(&false));
        assert!(inflight.is_empty());
    }

    #[test]
    fn origin_batches_match_the_registry_cap() {
        assert_eq!(MAKERWORLD_ORIGIN_LIMIT, 200);
    }
}

#[cfg(test)]
mod update_check_tests {
    use super::desktop_i18n::Locale;
    use super::{
        allowed_update_url, update_check_outcome, update_notice_for, UpdateAction, UpdateCheck,
        UpdateNotice,
    };
    use serde_json::json;

    #[test]
    fn statuses_map_to_the_go_copy_and_one_action() {
        let available = update_notice_for(&json!({
            "status":"update_available",
            "latestVersion":"0.4.0-beta.10",
            "releaseTag":"v0.4.0-beta.10"
        }));
        assert_eq!(
            available,
            UpdateNotice {
                message_key: "updateAvailable",
                version: "0.4.0-beta.10".into(),
                action: Some(UpdateAction::Release("v0.4.0-beta.10".into())),
            }
        );
        assert_eq!(
            Locale::En
                .text(available.message_key)
                .replace("{v}", &available.version),
            "New version available: 0.4.0-beta.10"
        );

        let up_to_date = update_notice_for(&json!({"status":"up_to_date"}));
        assert_eq!(up_to_date.message_key, "latestInstalled");
        assert_eq!(up_to_date.action, None);
        assert_eq!(
            Locale::En.text("latestInstalled"),
            "You have the latest published version."
        );

        let no_releases = update_notice_for(&json!({"status":"no_releases"}));
        assert_eq!(no_releases.message_key, "noPublishedReleases");
        assert_eq!(no_releases.action, Some(UpdateAction::Builds));
        assert_eq!(
            Locale::En.text(UpdateAction::Builds.label_key()),
            "View beta builds"
        );

        let platform = update_notice_for(&json!({
            "status":"platform_unavailable",
            "latestVersion":"0.4.9"
        }));
        assert_eq!(platform.message_key, "platformUnavailable");
        assert_eq!(platform.version, "0.4.9");
        assert_eq!(platform.action, Some(UpdateAction::Releases));
        assert_eq!(
            Locale::En
                .text(platform.message_key)
                .replace("{v}", &platform.version),
            "Version 0.4.9 exists, but not for this platform."
        );
        assert_eq!(
            Locale::En.text(UpdateAction::Releases.label_key()),
            "View releases"
        );

        let auth = update_notice_for(&json!({"status":"authentication_required"}));
        assert_eq!(auth.message_key, "authRequired");
        assert_eq!(auth.action, Some(UpdateAction::Releases));
    }

    #[test]
    fn unknown_or_unsupported_status_shows_unsupported_copy_without_action() {
        for data in [
            json!({"status":"unsupported_platform"}),
            json!({"status":"something_new"}),
            json!({"status":7}),
            json!({}),
        ] {
            assert_eq!(
                update_notice_for(&data),
                UpdateNotice {
                    message_key: "unsupportedPlatform",
                    version: String::new(),
                    action: None,
                },
                "{data}"
            );
        }
        assert_eq!(
            Locale::En.text("unsupportedPlatform"),
            "No update package exists for this platform."
        );
    }

    #[test]
    fn update_available_offers_the_release_only_for_a_safe_tag() {
        for tag in [
            json!(null),
            json!(""),
            json!("../../evil"),
            json!("https://evil.example"),
            json!("v1..2"),
        ] {
            let notice = update_notice_for(&json!({
                "status":"update_available",
                "latestVersion":"0.4.0",
                "releaseTag":tag
            }));
            assert_eq!(notice.message_key, "updateAvailable");
            assert_eq!(notice.action, None, "{tag}");
        }
    }

    #[test]
    fn actions_send_only_the_registry_page_kinds_and_labels() {
        assert_eq!(
            UpdateAction::Release("v0.4.0".into()).page_params(),
            json!({"kind":"release","tag":"v0.4.0"})
        );
        assert_eq!(
            UpdateAction::Releases.page_params(),
            json!({"kind":"releases"})
        );
        assert_eq!(UpdateAction::Builds.page_params(), json!({"kind":"builds"}));
        assert_eq!(
            UpdateAction::Release("v0.4.0".into()).label_key(),
            "openRelease"
        );
        assert_eq!(UpdateAction::Releases.label_key(), "openReleases");
        assert_eq!(UpdateAction::Builds.label_key(), "openPreviewBuilds");
    }

    #[test]
    fn update_links_accept_only_the_fixed_repository_pages() {
        for url in [
            "https://github.com/nocturney/slicebridge/releases",
            "https://github.com/nocturney/slicebridge/releases/tag/v0.4.0-beta.10",
            "https://github.com/nocturney/slicebridge/actions/workflows/native.yml",
        ] {
            assert!(allowed_update_url(url), "{url}");
        }
    }

    #[test]
    fn update_links_refuse_host_path_and_scheme_tricks() {
        for url in [
            "",
            "https://github.com/nocturney/slicebridge",
            "https://github.com/nocturney/slicebridge/",
            "https://github.com/nocturney/slicebridge.evil/releases",
            "https://github.com/nocturney/slicebridgeX/releases",
            "https://github.com/nocturney/slicebridge/releases.evil",
            "https://github.com/nocturney/slicebridge@evil.example/releases",
            "https://github.com/nocturney/slicebridge/releases/tag/",
            "https://github.com/nocturney/slicebridge/releases/tag/v1..2",
            "https://github.com/nocturney/slicebridge/releases/tag/v1.2/../../x",
            "https://github.com/nocturney/slicebridge/releases/tag/v1.2?x=1",
            "https://github.com/nocturney/slicebridge/releases/tag/v1.2#x",
            "https://github.com/nocturney/slicebridge/releases/tag/v1.2 ",
            "https://github.com/nocturney/slicebridge/releases/tag/v1.2\n",
            "https://evil.example/https://github.com/nocturney/slicebridge/releases",
            "http://github.com/nocturney/slicebridge/releases",
            "file:///etc/passwd",
            "--help",
        ] {
            assert!(!allowed_update_url(url), "{url:?}");
        }
    }

    #[test]
    fn check_reply_is_its_data_or_its_error_never_a_guess() {
        assert_eq!(
            update_check_outcome(&json!({"ok":true,"data":{"status":"up_to_date"}})),
            Ok(json!({"status":"up_to_date"}))
        );
        assert_eq!(
            update_check_outcome(&json!({"ok":false,"error":"GitHub API access limited"})),
            Err("GitHub API access limited".to_owned())
        );
        assert_eq!(
            update_check_outcome(&json!({"ok":false})),
            Err("unknown error".to_owned())
        );
        assert!(update_check_outcome(&json!({"ok":true,"data":{}})).is_err());
        assert!(update_check_outcome(&json!({"ok":true,"data":{"status":7}})).is_err());
    }

    #[test]
    fn only_one_check_is_in_flight_at_a_time() {
        let mut check = UpdateCheck::default();
        let first = check.begin().expect("first check starts");
        assert!(check.checking);
        assert_eq!(check.begin(), None);
        check.finish(first, &json!({"ok":true,"data":{"status":"up_to_date"}}));
        assert!(!check.checking);
        assert!(check.begin().is_some());
    }

    #[test]
    fn finished_check_shows_its_data_or_error_and_dismiss_resets_it() {
        let mut check = UpdateCheck::default();
        let request = check.begin().unwrap();
        check.finish(request, &json!({"ok":true,"data":{"status":"no_releases"}}));
        assert_eq!(check.data, Some(json!({"status":"no_releases"})));
        check.dismiss();
        assert_eq!(check.data, None);
        assert!(check.error.is_empty());

        let request = check.begin().unwrap();
        check.finish(
            request,
            &json!({"ok":false,"error":"GitHub API access limited"}),
        );
        assert_eq!(check.data, None);
        assert_eq!(check.error, "GitHub API access limited");
        check.dismiss();
        assert!(check.error.is_empty());
        assert!(!check.link_error);
    }

    #[test]
    fn dismiss_during_a_check_keeps_the_wait_until_its_reply() {
        let mut check = UpdateCheck::default();
        check.begin().unwrap();
        check.dismiss();
        assert!(check.checking);
        assert_eq!(check.begin(), None);
    }

    #[test]
    fn reply_after_about_closed_is_never_shown_as_success() {
        let mut check = UpdateCheck::default();
        let request = check.begin().unwrap();
        check.close();
        check.finish(
            request,
            &json!({"ok":true,"data":{
                "status":"update_available","latestVersion":"9.9.9","releaseTag":"v9.9.9"
            }}),
        );
        assert!(!check.checking);
        assert_eq!(check.data, None);
        assert!(check.error.is_empty());

        // Reopening About and checking again shows that new request's own result.
        let request = check.begin().unwrap();
        check.finish(request, &json!({"ok":true,"data":{"status":"up_to_date"}}));
        assert_eq!(check.data, Some(json!({"status":"up_to_date"})));
    }
}

#[cfg(test)]
mod integrations_tests {
    use super::desktop_i18n::Locale;
    use super::{
        active_setup_step, allowed_browser_page, browser_opened, integrations_still_open,
        link_state_from, prepared_folder, BrowserChoice, LinkMode, LinkState,
    };
    use serde_json::json;

    // Every key integrations.js uses, embedded from its he and en maps.
    const INTEGRATION_KEYS: &[&str] = &[
        "integrationTitle",
        "integrationHint",
        "stepOneTitle",
        "stepOneHint",
        "browserLinkUnknown",
        "browserLinkReady",
        "browserLinkMissing",
        "browserLinkMac",
        "browserLinkError",
        "enableBrowserLinks",
        "stepTwoTitle",
        "stepTwoHint",
        "guidedBetaInstall",
        "installFromStore",
        "storeChecking",
        "storeReady",
        "storeNotPublished",
        "betaInstallGuide",
        "betaInstallInfo",
        "openExtensionsPage",
        "betaOpened",
        "betaOpenedFirefox",
        "storeOpened",
        "browserLabel",
        "prepareExtension",
        "installInstructionOne",
        "installInstructionTwo",
        "installInstructionThree",
        "installInstructionTwoFirefox",
        "installInstructionThreeFirefox",
        "firefoxInstallWarning",
        "folderPrepared",
        "folderError",
        "stepThreeTitle",
        "stepThreeHint",
        "interceptOff",
        "interceptOffDetail",
        "interceptAsk",
        "interceptAskDetail",
        "interceptAlways",
        "interceptAlwaysDetail",
        "confirmInBrowser",
        "selectModeFirst",
        "modeLaunched",
        "modeNoExtension",
        "modeError",
        "modeScopeNote",
        "done",
        "makerworldLocked",
    ];

    #[test]
    fn browser_keys_are_the_four_allowlisted_values() {
        let keys: Vec<&str> = BrowserChoice::ALL
            .iter()
            .map(|browser| browser.key())
            .collect();
        assert_eq!(keys, ["edge", "chrome", "brave", "firefox"]);
        for browser in BrowserChoice::ALL {
            assert_eq!(BrowserChoice::from_key(browser.key()), Some(browser));
        }
        for invalid in [
            "",
            "Edge",
            "chromium",
            "about:debugging",
            "edge://extensions",
            " brave",
        ] {
            assert_eq!(BrowserChoice::from_key(invalid), None, "{invalid:?}");
        }
        assert_eq!(BrowserChoice::default(), BrowserChoice::Edge);
    }

    #[test]
    fn mode_values_are_the_three_allowlisted_values() {
        let keys: Vec<&str> = LinkMode::ALL.iter().map(|mode| mode.key()).collect();
        assert_eq!(keys, ["off", "ask", "always"]);
        for mode in LinkMode::ALL {
            assert_eq!(LinkMode::from_key(mode.key()), Some(mode));
        }
        for invalid in ["", "Off", "on", "always ", "true", "bambu"] {
            assert_eq!(LinkMode::from_key(invalid), None, "{invalid:?}");
        }
    }

    #[test]
    fn browser_pages_come_only_from_the_fixed_list() {
        assert_eq!(
            allowed_browser_page("edge://extensions"),
            Some("edge://extensions")
        );
        assert_eq!(
            allowed_browser_page("about:debugging#/runtime/this-firefox"),
            Some("about:debugging#/runtime/this-firefox")
        );
        for invalid in [
            "",
            "edge://extensions/",
            "https://makerworld.com/",
            "file:///etc/passwd",
            "chrome://settings",
        ] {
            assert_eq!(allowed_browser_page(invalid), None, "{invalid:?}");
        }
    }

    #[test]
    fn only_firefox_uses_the_temporary_package() {
        assert_eq!(BrowserChoice::Firefox.kind(), "firefox");
        for browser in [
            BrowserChoice::Edge,
            BrowserChoice::Chrome,
            BrowserChoice::Brave,
        ] {
            assert_eq!(browser.kind(), "chromium");
        }
    }

    #[test]
    fn link_state_reads_only_windows_and_macos() {
        assert_eq!(
            link_state_from(&json!({"ok":true,"data":{"platform":"darwin"}})),
            LinkState::Mac
        );
        assert_eq!(
            link_state_from(&json!({"ok":true,"data":{"platform":"windows","registered":true}})),
            LinkState::Ready
        );
        assert_eq!(
            link_state_from(&json!({"ok":true,"data":{"platform":"windows","registered":false}})),
            LinkState::Missing
        );
        // A string "true" is not the explicit boolean the Registry contract requires.
        assert_eq!(
            link_state_from(&json!({"ok":true,"data":{"platform":"windows","registered":"true"}})),
            LinkState::Missing
        );
        assert_eq!(
            link_state_from(&json!({"ok":true,"data":{"platform":"unsupported"}})),
            LinkState::Error
        );
        assert_eq!(
            link_state_from(&json!({"ok":false,"data":{"platform":"darwin"}})),
            LinkState::Error
        );
        assert_eq!(
            link_state_from(&json!({"ok":true,"data":null})),
            LinkState::Error
        );
    }

    #[test]
    fn opened_is_true_only_for_an_explicit_true() {
        assert!(browser_opened(&json!({"ok":true,"data":{"opened":true}})));
        assert!(!browser_opened(
            &json!({"ok":true,"data":{"opened":"true"}})
        ));
        assert!(!browser_opened(&json!({"ok":true,"data":{}})));
        assert!(!browser_opened(&json!({"ok":false,"data":{"opened":true}})));
    }

    #[test]
    fn prepared_folder_needs_an_absolute_path_of_the_matching_kind() {
        let folder = std::env::temp_dir().join("SliceBridge");
        let folder = folder.to_string_lossy().into_owned();
        assert_eq!(
            prepared_folder(
                &json!({"ok":true,"data":{"folder":folder,"kind":"chromium"}}),
                BrowserChoice::Edge
            ),
            Some(folder.clone())
        );
        // Firefox needs the Firefox package kind; a chromium package is refused.
        assert_eq!(
            prepared_folder(
                &json!({"ok":true,"data":{"folder":folder,"kind":"chromium"}}),
                BrowserChoice::Firefox
            ),
            None
        );
        assert_eq!(
            prepared_folder(
                &json!({"ok":true,"data":{"folder":"relative/SliceBridge","kind":"chromium"}}),
                BrowserChoice::Edge
            ),
            None
        );
        assert_eq!(
            prepared_folder(
                &json!({"ok":false,"error":"no","data":{"folder":folder,"kind":"chromium"}}),
                BrowserChoice::Edge
            ),
            None
        );
    }

    #[test]
    fn active_step_follows_connect_then_install_then_mode() {
        assert_eq!(active_setup_step(LinkState::Unknown, false), 1);
        assert_eq!(active_setup_step(LinkState::Missing, true), 1);
        assert_eq!(active_setup_step(LinkState::Error, false), 1);
        assert_eq!(active_setup_step(LinkState::Ready, false), 2);
        assert_eq!(active_setup_step(LinkState::Mac, false), 2);
        assert_eq!(active_setup_step(LinkState::Ready, true), 3);
    }

    #[test]
    fn modal_stays_open_only_while_developer_mode_is_on() {
        assert!(integrations_still_open(true, true));
        // Developer mode turned off while the modal is open closes it.
        assert!(!integrations_still_open(true, false));
        // A closed modal never reopens by itself when developer mode turns on.
        assert!(!integrations_still_open(false, true));
    }

    #[test]
    fn every_setup_key_has_copy_in_both_locales() {
        for key in INTEGRATION_KEYS {
            assert_ne!(Locale::He.text(key), *key, "he {key}");
            assert_ne!(Locale::En.text(key), *key, "en {key}");
        }
        assert_eq!(Locale::En.text("done"), "Done");
        assert_eq!(
            Locale::En.text("makerworldLocked"),
            "MakerWorld setup is locked. Open About → Developer mode to unlock it."
        );
        assert_eq!(Locale::He.text("done"), "סיום");
        assert_eq!(Locale::En.text("browserLabel"), "Browser");
    }
}

#[cfg(test)]
mod batch_result_tests {
    use super::desktop_layout::{
        batch_totals, completed_output_name, completed_target_detail, source_nozzle_facts,
        target_display_name,
    };
    use serde_json::{json, Value};

    #[test]
    fn output_name_appears_only_after_completion() {
        let planned = json!("out/planned.3mf");
        let receipt = json!({"output":"out/done.3mf"});
        assert_eq!(completed_output_name("running", &receipt, &planned), None);
        assert_eq!(
            completed_output_name("failed", &Value::Null, &planned),
            None
        );
        assert_eq!(
            completed_output_name("completed", &receipt, &planned).as_deref(),
            Some("done.3mf")
        );
        assert_eq!(
            completed_output_name("completed", &Value::Null, &planned).as_deref(),
            Some("planned.3mf")
        );
    }

    #[test]
    fn source_nozzle_prefers_item_then_first_target_and_ignores_empty() {
        let single = json!({"source_nozzle":"0.6","source_nozzle_assumed":false});
        assert_eq!(source_nozzle_facts(&single), Some(("0.6".into(), false)));
        let multi = json!({"targets":[{"source_nozzle":"0.2","source_nozzle_assumed":true}]});
        assert_eq!(source_nozzle_facts(&multi), Some(("0.2".into(), true)));
        assert_eq!(source_nozzle_facts(&json!({"targets":[]})), None);
        assert_eq!(source_nozzle_facts(&json!({"source_nozzle":""})), None);
    }

    #[test]
    fn target_names_survive_temporary_profile_unavailability() {
        let rows = [("u1", "Snapmaker U1")];
        assert_eq!(target_display_name(&rows, "u1"), "Snapmaker U1");
        assert_eq!(target_display_name(&[], "c5"), "Flashforge Creator 5");
        assert_eq!(target_display_name(&[], "cc2"), "ELEGOO CC2");
        assert_eq!(target_display_name(&[], "qidi"), "QIDI Plus5");
        assert_eq!(target_display_name(&[], "unknown"), "unknown");
    }

    #[test]
    fn batch_totals_sum_every_source() {
        let items = vec![
            json!({"results":[{},{}],"failures":[{}]}),
            json!({"phase":"failed"}),
            json!({"phase":"completed"}),
        ];
        assert_eq!(batch_totals(&items), (3, 2));
    }

    #[test]
    fn completed_detail_reports_only_grounded_receipt_values() {
        let receipt = json!({"target_nozzle":"0.4","bytes":2_097_152});
        assert_eq!(
            completed_target_detail(&receipt, "Target Nozzle").as_deref(),
            Some("Target Nozzle 0.4 mm · 2.0 MB")
        );
        assert_eq!(completed_target_detail(&Value::Null, "Target Nozzle"), None);
    }
}
