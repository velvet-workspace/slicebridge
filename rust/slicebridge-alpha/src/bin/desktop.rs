//! Native GPUI pilot: never embeds a browser or calls Go/Wails.
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::*;
use serde_json::{json, Value};
use slicebridge_alpha::{protocol, Registry};
use std::path::Path;

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
            last_message: "Ready — choose a source and a verified OEM bundle.".into(),
            destination: None,
        }
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

    fn set_material(&mut self, name: &'static str, cx: &mut Context<Self>) {
        self.run("material.select", json!({"slot":1,"name":name}), cx);
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
        let bundle = status["bundle_path"].as_str().unwrap_or("None").to_string();
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
            ("Triangles", triangles),
            ("Vertices", vertices),
            ("Save target", output),
            ("Job state", phase),
        ];
        let mut table = div().flex().flex_col().gap_1().p_3();
        for (label, value) in columns {
            table = table.child(
                div()
                    .flex()
                    .flex_row()
                    .gap_4()
                    .p_2()
                    .child(div().w_32().child(label))
                    .child(div().flex_1().child(value)),
            );
        }

        div().flex().flex_col().gap_4().p_6()
            .child(div().text_2xl().child("SliceBridge  /  Rust Native Alpha"))
            .child(div().child("GPUI + GPUI Kit  |  Agent-first command registry  |  English"))
            .child(
                div().flex().flex_row().gap_2()
                    .child(Button::new("choose-source").primary().label("Open 3MF")
                        .on_click(cx.listener(|this, _, _, cx| this.choose_file("source.select", cx))))
                    .child(Button::new("choose-oem").label("Load verified OEM")
                        .on_click(cx.listener(|this, _, _, cx| this.choose_file("bundle.load", cx))))
                    .child(Button::new("choose-target").label("Save as...")
                        .on_click(cx.listener(|this, _, _, cx| this.choose_destination(cx))))
            )
            .child(div().text_lg().child("Project / Printer / Materials"))
            .child(table)
            .child(
                div().flex().flex_row().gap_2()
                    .child(Button::new("material-pla").label("PLA (preview)")
                        .on_click(cx.listener(|this, _, _, cx| this.set_material("PLA", cx))))
                    .child(Button::new("material-petg").label("PETG (preview)")
                        .on_click(cx.listener(|this, _, _, cx| this.set_material("PETG", cx))))
            )
            .child(div().child("Material buttons preview selection; conversion is bound to validated OEM profiles."))
            .child(
                div().flex().flex_row().gap_2()
                    .child(Button::new("convert").primary().label("Convert 3MF")
                        .on_click(cx.listener(|this, _, _, cx| this.start_conversion(cx))))
                    .child(Button::new("cancel").label("Cancel")
                        .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))))
                    .child(Button::new("open-in-slicer").label("Open in slicer")
                        .on_click(cx.listener(|this, _, _, cx| this.choose_slicer(cx))))
                    .child(Button::new("refresh").label("Refresh status")
                        .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))))
            )
            .child(div().text_lg().child("Status / Agent log"))
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
            cx.new(|_| SliceBridgeWindow::new(commands))
        })
        .expect("SliceBridge GPUI window failed to open");
    });
}
