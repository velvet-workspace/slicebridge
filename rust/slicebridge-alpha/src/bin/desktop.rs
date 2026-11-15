//! Native GPUI pilot: never embeds a browser or calls Go/Wails.
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::*;
use serde_json::{json, Value};
use slicebridge_alpha::{protocol, Registry};
use std::path::{Path, PathBuf};
use std::time::Duration;

const TEXT: &str = "en"; // Localization slot: UI text is English-first.

#[derive(Clone, Copy, PartialEq, Eq)]
enum WorkspacePage {
    Project,
    Batch,
}

struct SliceBridgeWindow {
    commands: Registry,
    last_message: String,
    destination: Option<String>,
    destination_source: Option<String>,
    active_page: WorkspacePage,
}

impl SliceBridgeWindow {
    fn new(registry: Registry) -> Self {
        Self {
            commands: registry,
            last_message:
                "No UI action yet. Agent / CLI / MCP changes appear in the project fields above."
                    .into(),
            destination: None,
            destination_source: None,
            active_page: WorkspacePage::Project,
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
            prompt: Some("Choose up to 32 3MF files for a batch".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picker.await {
                let _ = this.update(cx, |view, cx| {
                    let paths: Vec<String> = paths
                        .iter()
                        .map(|p| p.to_string_lossy().to_string())
                        .collect();
                    view.run("batch.stage", json!({"paths":paths}), cx);
                });
            }
        })
        .detach();
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

    fn start_batch(&mut self, cx: &mut Context<Self>) {
        self.active_page = WorkspacePage::Batch;
        self.run("batch.start", json!({}), cx);
    }

    fn navigate(&mut self, page: WorkspacePage, cx: &mut Context<Self>) {
        self.active_page = page;
        cx.notify();
    }

    fn handle_dropped_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        // Native OS file drops, not an HTML/WebView surrogate.
        // Validation and all resulting state changes belong to the Registry.
        match paths.as_slice() {
            [single] => {
                self.run("source.select", json!({"path":single}), cx);
            }
            [] => {
                self.last_message = "Drop a .3mf file or up to 32 files for batch.".into();
                cx.notify();
            }
            many => {
                self.run("batch.stage", json!({"paths":many}), cx);
            }
        }
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
        gpui_kit::open_window(WindowOptions::default(), cx, move |_, cx| {
            cx.new(|cx| {
                let mut view = SliceBridgeWindow::new(commands);
                view.start_live_refresh(cx);
                view
            })
        })
        .expect("SliceBridge GPUI window failed to open");
    });
}
