//! Native GPUI pilot: never embeds a browser or calls Go/Wails.
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::*;
use serde_json::{json, Value};
use slicebridge_alpha::{protocol, Registry};
use std::path::Path;
use std::time::Duration;

const TEXT: &str = "en"; // Localization slot: UI text is English-first.

struct SliceBridgeWindow {
    commands: Registry,
    last_message: String,
    destination: Option<String>,
}

impl SliceBridgeWindow {
    fn new(registry: Registry) -> Self {
        Self {
            commands: registry,
            last_message:
                "No UI action yet. Agent / CLI / MCP changes appear in the project fields above."
                    .into(),
            destination: None,
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

impl Render for SliceBridgeWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status = self.commands.snapshot();
        let source = status["source"]
            .as_str()
            .unwrap_or("No 3MF selected")
            .to_string();
        let printer = status["printer"]
            .as_str()
            .unwrap_or("No OEM profile loaded")
            .to_string();
        let bundle = status["bundle_path"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| {
                if status["installed_oem"] == true {
                    "Live installed OEM inheritance".into()
                } else {
                    "None".into()
                }
            });
        let phase = status["phase"].as_str().unwrap_or("idle").to_string();
        let triangles = status["facts"]["triangles"]
            .as_u64()
            .map(|v| v.to_string())
            .unwrap_or_else(|| "-".into());
        let vertices = status["facts"]["vertices"]
            .as_u64()
            .map(|v| v.to_string())
            .unwrap_or_else(|| "-".into());
        let output = self.destination.clone().unwrap_or("Not chosen".into());
        let columns = [
            ("3MF source", source),
            ("OEM printer", printer),
            ("OEM bundle", bundle),
            (
                "Printer target",
                status["target"].as_str().unwrap_or("None").into(),
            ),
            (
                "Nozzle diameter",
                status["nozzle"].as_str().unwrap_or("None").into(),
            ),
            (
                "PLA OEM preset",
                status["materials"]["PLA"].as_str().unwrap_or("None").into(),
            ),
            (
                "PETG OEM preset",
                status["materials"]["PETG"]
                    .as_str()
                    .unwrap_or("None")
                    .into(),
            ),
            ("Triangles", triangles),
            ("Vertices", vertices),
            ("Save target", output),
            ("Job state", phase),
        ];
        let mut table = div().flex().flex_col().gap_0().p_1();
        for (label, value) in columns {
            table = table.child(
                div()
                    .flex()
                    .flex_row()
                    .gap_3()
                    .p_1()
                    .child(div().w_32().child(label))
                    .child(div().flex_1().child(value)),
            );
        }

        div().flex().flex_col().gap_2().p_4()
            .child(div().text_2xl().child("SliceBridge  /  Rust Native Alpha"))
            .child(div().child("GPUI + GPUI Kit  |  Agent-first command registry  |  English"))
            .child(
                div().flex().flex_row().gap_2()
                    .child(Button::new("choose-source").primary().label("Open 3MF")
                        .on_click(cx.listener(|this, _, _, cx| this.choose_file("source.select", cx))))
                    .child(Button::new("discover-oem").label("Scan installed OEM")
                        .on_click(cx.listener(|this, _, _, cx| this.run("profiles.discover", json!({}), cx))))
                    .child(Button::new("choose-target").label("Save as...")
                        .on_click(cx.listener(|this, _, _, cx| this.choose_destination(cx))))
            )
            .child(div().text_lg().child("Snapmaker U1 — installed nozzle presets"))
            .child(
                div().flex().flex_row().gap_2()
                    .child(Button::new("u1-02").label("0.2 mm")
                        .on_click(cx.listener(|this, _, _, cx| this.select_oem("u1", "0.2", cx))))
                    .child(Button::new("u1-04").label("0.4 mm")
                        .on_click(cx.listener(|this, _, _, cx| this.select_oem("u1", "0.4", cx))))
                    .child(Button::new("u1-06").label("0.6 mm")
                        .on_click(cx.listener(|this, _, _, cx| this.select_oem("u1", "0.6", cx))))
                    .child(Button::new("u1-08").label("0.8 mm")
                        .on_click(cx.listener(|this, _, _, cx| this.select_oem("u1", "0.8", cx))))
            )
            .child(div().text_lg().child("QIDI X-Plus 5 — installed nozzle presets"))
            .child(
                div().flex().flex_row().gap_2()
                    .child(Button::new("qidi-02").label("0.2 mm")
                        .on_click(cx.listener(|this, _, _, cx| this.select_oem("qidi", "0.2", cx))))
                    .child(Button::new("qidi-04").label("0.4 mm")
                        .on_click(cx.listener(|this, _, _, cx| this.select_oem("qidi", "0.4", cx))))
                    .child(Button::new("qidi-06").label("0.6 mm")
                        .on_click(cx.listener(|this, _, _, cx| this.select_oem("qidi", "0.6", cx))))
                    .child(Button::new("qidi-08").label("0.8 mm")
                        .on_click(cx.listener(|this, _, _, cx| this.select_oem("qidi", "0.8", cx))))
            )
            .child(div().text_lg().child("Project / Printer / Materials"))
            .child(table)
            .child(
                div().flex().flex_row().gap_2()
                    .child(Button::new("material-pla").label("Next verified PLA preset")
                        .on_click(cx.listener(|this, _, _, cx| this.next_material("PLA", cx))))
                    .child(Button::new("material-petg").label("Next verified PETG preset")
                        .on_click(cx.listener(|this, _, _, cx| this.next_material("PETG", cx))))
            )
            .child(div().child("Material changes rebuild the live installed OEM profile; they affect the Rust conversion."))
            .child(
                div().flex().flex_row().gap_2()
                    .child(Button::new("convert").primary().label("Convert 3MF")
                        .on_click(cx.listener(|this, _, _, cx| this.start_conversion(cx))))
                    .child(Button::new("cancel").label("Cancel")
                        .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))))
                    .child(Button::new("open-selected").label("Open in OEM slicer")
                        .on_click(cx.listener(|this, _, _, cx| this.run("slicer.open_selected", json!({}), cx))))
                    .child(Button::new("open-in-slicer").label("Other slicer...")
                        .on_click(cx.listener(|this, _, _, cx| this.choose_slicer(cx))))
                    .child(Button::new("refresh").label("Refresh status")
                        .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))))
            )
            .child(div().text_lg().child("Last UI action"))
            .child(div().p_2().child(self.last_message.clone()))
            .child(div().child("Local attach: 127.0.0.1:48637  •  MCP: slicebridge-alpha --mcp"))
            .child(div().child(format!("Locale: {TEXT}")))
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
