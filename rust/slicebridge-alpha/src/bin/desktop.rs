//! Native GPUI pilot: never embeds a browser or calls Go/Wails.
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use serde_json::{json, Value};
use slicebridge_alpha::{protocol, Registry};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

fn output_folder(status: &Value) -> Option<PathBuf> {
    let batch_output = status["batch"]["items"]
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .rev()
                .find(|entry| entry["phase"] == "completed")
        })
        .and_then(|entry| entry["output"].as_str());
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

struct SliceBridgeWindow {
    commands: Registry,
    last_message: String,
    destination: Option<String>,
    destination_source: Option<String>,
    locale: Locale,
    printers_open: bool,
    printers_first_run: bool,
    printers_cards: Vec<PrinterCard>,
    ready_oem_targets: BTreeSet<String>,
    printers_saved: Vec<String>,
    printers_draft: Vec<String>,
    printers_maker: Option<String>,
    printers_maker_menu: bool,
    printers_show_all: bool,
    printers_error: String,
    printers_search: Entity<InputState>,
    _printer_subscription: Subscription,
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
        let ready_oem_targets: BTreeSet<String> = discovered["data"]["profiles"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|item| item["ready"] == true && item["nozzle"] == "0.4")
            .filter_map(|item| item["target"].as_str().map(str::to_owned))
            .collect();
        let prefs = registry.execute("printers.get", &json!({}));
        let saved = prefs["data"]["selectedIds"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        Self {
            commands: registry,
            last_message:
                "No UI action yet. Agent / CLI / MCP changes appear in the project fields above."
                    .into(),
            destination: None,
            destination_source: None,
            locale,
            printers_open: false,
            printers_first_run: prefs["data"]["onboardingPending"] == true,
            printers_cards: cards,
            ready_oem_targets,
            printers_saved: saved,
            printers_draft: Vec::new(),
            printers_maker: None,
            printers_maker_menu: false,
            printers_show_all: false,
            printers_error: String::new(),
            printers_search: search,
            _printer_subscription: subscription,
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
        let result = self.commands.execute(name, &args);
        if name == "source.select" && result["ok"] == true {
            self.destination = None;
            self.destination_source = None;
        }
        self.last_message = if result["ok"] == true {
            format!("{name}: {}", result["data"])
        } else {
            format!("Error: {}", result["error"].as_str().unwrap_or("unknown"))
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
        let mut existing: Vec<PathBuf> = snap["batch_selection"]["paths"]
            .as_array()
            .into_iter()
            .flat_map(|items| items.iter())
            .filter_map(|value| value.as_str().map(PathBuf::from))
            .collect();
        if existing.is_empty() {
            if let Some(source) = snap["source"].as_str() {
                existing.push(PathBuf::from(source));
            }
        }
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

    fn start_batch(&mut self, cx: &mut Context<Self>) {
        self.run("batch.start", json!({}), cx);
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
    }
}
