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

// Same definition the layout uses for disabling queue controls during a conversion.
fn snapshot_running(status: &Value) -> bool {
    status["phase"] == "running" || status["batch"]["phase"] == "running"
}

// `batch.remove` answers {"removed":true,...} on success; anything else leaves the queue unchanged.
fn remove_reply_ok(reply: &Value) -> bool {
    reply["ok"] == true && reply["data"]["removed"] == true
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

#[path = "desktop/i18n.rs"]
mod desktop_i18n;
use desktop_i18n::Locale;

#[path = "desktop/printer_picker.rs"]
mod desktop_printer_picker;
use desktop_printer_picker::PrinterCard;

#[path = "desktop/printer_settings_ui.rs"]
mod desktop_printer_settings_ui;

struct SliceBridgeWindow {
    commands: Registry,
    last_message: String,
    destination: Option<String>,
    destination_source: Option<String>,
    // Set by `batch.remove` so a removed source is not shown again while the Registry still reports it.
    suppressed_source: Option<String>,
    // Per queued path: true only for an exact high-confidence MakerWorld origin. Filled outside render.
    makerworld_origins: BTreeMap<String, bool>,
    makerworld_pending: BTreeMap<String, u64>,
    makerworld_token: u64,
    locale: Locale,
    printers_open: bool,
    printers_first_run: bool,
    printers_cards: Vec<PrinterCard>,
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
    about_open: bool,
    about_developer_mode: bool,
    about_developer_error: bool,
    about_link_error: bool,
    about_document: Option<AboutDocument>,
}

// Matches the installed-version string the stable Go app shows (`appVersion` in native/updates_bridge.go).
const ABOUT_INSTALLED_VERSION: &str = "0.4.0-beta.6.dev.3";

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
// values returned by `about_page_url`.
fn about_open_command(url: &'static str) -> Command {
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

impl SliceBridgeWindow {
    fn new(registry: Registry, window: &mut Window, cx: &mut Context<Self>) -> Self {
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
        let cards = if catalog["ok"] == true {
            desktop_printer_picker::original_catalogue_cards(&catalog["data"])
        } else {
            Vec::new()
        };
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
            commands: registry,
            last_message:
                "No UI action yet. Agent / CLI / MCP changes appear in the project fields above."
                    .into(),
            destination: None,
            destination_source: None,
            suppressed_source: None,
            makerworld_origins: BTreeMap::new(),
            makerworld_pending: BTreeMap::new(),
            makerworld_token: 0,
            locale,
            printers_open: false,
            printers_first_run: prefs["data"]["onboardingPending"] == true,
            printers_cards: cards,
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
            about_open: false,
            about_developer_mode: false,
            about_developer_error: false,
            about_link_error: false,
            about_document: None,
        }
    }

    // Repaint when CLI, MCP or attach changes the same Registry outside GPUI.
    // This is UI-only: no conversion polling command, slicer or printer action.
    fn start_live_refresh(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let mut previous = Value::Null;
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
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
        self.last_message = if result["ok"] == true {
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
        cx.notify();
    }

    fn close_about(&mut self, cx: &mut Context<Self>) {
        self.about_open = false;
        self.about_link_error = false;
        self.about_document = None;
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
        let selected: Vec<String> = paths
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect();
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
                    // Newly selected paths are inspected again; an older file with the same name never lends its badge.
                    for path in &selected {
                        self.makerworld_origins.remove(path);
                        self.makerworld_pending.remove(path);
                    }
                    if snap["source"].is_null() {
                        self.run(
                            "source.select",
                            json!({"path":files[0].to_string_lossy()}),
                            cx,
                        );
                    }
                    self.last_message =
                        format!("{}: {}", self.locale.text("queuedFiles"), files.len());
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
        if !visible.is_empty()
            && visible
                .iter()
                .all(|id| self.selected_batch_targets.contains(id))
        {
            // Go's button toggles between selecting and clearing shown targets.
            self.selected_batch_targets.clear();
        } else {
            self.selected_batch_targets.extend(visible);
        }
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

#[path = "desktop/layout.rs"]
mod desktop_layout;

impl Render for SliceBridgeWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        desktop_layout::render_window(self, window, cx)
    }
}

fn main() {
    let registry = Registry::new();
    let attach_status = protocol::start_attach(registry.clone(), "127.0.0.1:48637");
    if let Err(error) = attach_status {
        eprintln!("local attach not available: {error}");
    }
    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        let commands = registry.clone();
        gpui_kit::open_window(WindowOptions::default(), cx, move |window, cx| {
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
    use super::desktop_layout::{empty_target_notice_visible, receipt_error, result_counts};
    use serde_json::json;
    use serde_json::Value;
    use std::collections::BTreeSet;

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
    use super::{file_button_id, queue_display_paths, remove_reply_ok, snapshot_running};
    use serde_json::json;

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
