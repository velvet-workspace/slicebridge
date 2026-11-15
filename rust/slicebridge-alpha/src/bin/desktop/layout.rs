//! Native GPUI workspace composition. This is intentionally not HTML or a WebView.
use super::*;
use gpui_kit::prelude::FluentBuilder;

const NAV: u32 = 0x142238;
const NAV_TINT: u32 = 0x203450;
const CANVAS: u32 = 0xF0F4F9;
const WHITE: u32 = 0xFFFFFF;
const INK: u32 = 0x192B45;
const MUTED: u32 = 0x65758C;
const HAIRLINE: u32 = 0xDBE4EE;
const BLUE: u32 = 0x245CC8;
const BLUE_PALE: u32 = 0xE9F1FF;
const TEAL: u32 = 0x15A895;
const TEAL_PALE: u32 = 0xE4F8F3;
const WARNING: u32 = 0xA66C25;
const WARNING_PALE: u32 = 0xFFF5E4;
const RED: u32 = 0xAC3B44;
const RED_PALE: u32 = 0xFDECEF;

fn panel() -> Div {
    div()
        .bg(rgb(WHITE))
        .rounded_lg()
        .border_1()
        .border_color(rgb(HAIRLINE))
        .p_4()
}

fn overline(label: &str) -> Div {
    div()
        .text_xs()
        .text_color(rgb(MUTED))
        .child(label.to_string())
}

fn heading(title: &str, subtitle: &str) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_lg()
                .text_color(rgb(INK))
                .child(title.to_string()),
        )
        .child(
            div()
                .text_sm()
                .text_color(rgb(MUTED))
                .child(subtitle.to_string()),
        )
}

fn badge(label: String, bg: u32, foreground: u32) -> Div {
    div()
        .px_3()
        .py_1()
        .rounded_full()
        .bg(rgb(bg))
        .text_sm()
        .text_color(rgb(foreground))
        .child(label)
}

fn metric(label: &str, number: &str, note: &str) -> Div {
    div()
        .flex_1()
        .bg(rgb(0xF7FAFE))
        .border_1()
        .border_color(rgb(HAIRLINE))
        .rounded_lg()
        .p_3()
        .flex()
        .flex_col()
        .gap_1()
        .child(overline(label))
        .child(
            div()
                .text_2xl()
                .text_color(rgb(INK))
                .child(number.to_string()),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(MUTED))
                .child(note.to_string()),
        )
}

fn short_path(value: &str, max_chars: usize) -> String {
    let length = value.chars().count();
    if length <= max_chars {
        return value.to_string();
    }
    format!(
        "…{}",
        value
            .chars()
            .skip(length.saturating_sub(max_chars))
            .collect::<String>()
    )
}

fn job_tone(phase: &str) -> (u32, u32) {
    match phase {
        "completed" => (TEAL_PALE, 0x12725E),
        "running" => (BLUE_PALE, BLUE),
        "queued" | "idle" => (0xEFF2F7, MUTED),
        "cancelled" | "skipped" => (WARNING_PALE, WARNING),
        "failed" => (RED_PALE, RED),
        _ => (0xEFF2F7, MUTED),
    }
}

fn model_tile() -> Div {
    div()
        .w(px(76.))
        .h(px(86.))
        .rounded_lg()
        .bg(rgb(BLUE_PALE))
        .flex()
        .flex_col()
        .justify_center()
        .items_center()
        .gap_2()
        .child(
            div()
                .w(px(32.))
                .h(px(32.))
                .rounded_md()
                .bg(rgb(BLUE))
                .border_1()
                .border_color(rgb(0xB4D1FF)),
        )
        .child(div().text_sm().text_color(rgb(BLUE)).child("3MF"))
}

pub(super) fn render_window(
    view: &mut SliceBridgeWindow,
    _window: &mut Window,
    cx: &mut Context<SliceBridgeWindow>,
) -> impl IntoElement {
    let status = view.commands.snapshot();
    let source = status["source"].as_str();
    let source_title = source
        .and_then(|path| Path::new(path).file_name())
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| "No project selected".into());
    let source_path = source
        .map(|path| short_path(path, 110))
        .unwrap_or_else(|| "Open an authorized 3MF project to inspect its contents.".into());
    let printer = status["printer"]
        .as_str()
        .unwrap_or("No installed OEM profile selected");
    let target = status["target"].as_str().unwrap_or("");
    let nozzle = status["nozzle"].as_str().unwrap_or("");
    let phase = status["phase"].as_str().unwrap_or("idle");
    let triangle_count = status["facts"]["triangles"]
        .as_u64()
        .map(|value| value.to_string())
        .unwrap_or_else(|| "—".into());
    let vertex_count = status["facts"]["vertices"]
        .as_u64()
        .map(|value| value.to_string())
        .unwrap_or_else(|| "—".into());
    let output = if view.destination_source.as_deref() == source {
        view.destination
            .as_deref()
            .unwrap_or("Select an output destination")
    } else {
        "Select an output destination for this project"
    };
    let material_pla = status["materials"]["PLA"]
        .as_str()
        .unwrap_or("Not assigned");
    let material_petg = status["materials"]["PETG"]
        .as_str()
        .unwrap_or("Not assigned");

    let batch = &status["batch"];
    let batch_phase = batch["phase"].as_str().unwrap_or("idle");
    let batch_total = batch["total"].as_u64().unwrap_or(0);
    let batch_completed = batch["completed"].as_u64().unwrap_or(0);
    let batch_percent = batch["progress_percent"].as_u64().unwrap_or(0).min(100);
    let (batch_badge_bg, batch_badge_text) = job_tone(batch_phase);
    let staged = &status["batch_selection"];
    let staged_files = staged["paths"].as_array().map(|v| v.len()).unwrap_or(0);
    let batch_dir = staged["output_dir"]
        .as_str()
        .map(|path| short_path(path, 90))
        .unwrap_or_else(|| "Choose a destination folder".to_string());

    let mut recent = div().flex().flex_col().gap_2();
    let recent_paths = status["projects"].as_array();
    if let Some(paths) = recent_paths {
        for (index, item) in paths.iter().take(8).enumerate() {
            let Some(full_path) = item.as_str() else {
                continue;
            };
            let open_path = full_path.to_string();
            let remove_path = full_path.to_string();
            let name = Path::new(full_path)
                .file_name()
                .map(|s| short_path(&s.to_string_lossy(), 23))
                .unwrap_or_else(|| short_path(full_path, 23));
            recent = recent.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_2()
                    .rounded_md()
                    .bg(rgb(NAV_TINT))
                    .child(div().text_sm().text_color(rgb(0xEDF3FC)).child(name))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_2()
                            .child(
                                Button::new(format!("recent-open-{index}"))
                                    .label("Open")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.run(
                                            "source.select",
                                            json!({"path":open_path.clone()}),
                                            cx,
                                        );
                                    })),
                            )
                            .child(
                                Button::new(format!("recent-forget-{index}"))
                                    .label("Forget")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.run(
                                            "projects.forget",
                                            json!({"path":remove_path.clone()}),
                                            cx,
                                        );
                                    })),
                            ),
                    ),
            );
        }
    }
    if recent_paths.is_none_or(|paths| paths.is_empty()) {
        recent = recent.child(
            div()
                .text_sm()
                .text_color(rgb(0xACC0D8))
                .child("Opened projects appear here for this session."),
        );
    }

    let sidebar = div()
        .id("sb-sidebar")
        .w(px(262.))
        .h_full()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap_4()
        .bg(rgb(NAV))
        .p_4()
        .text_color(rgb(0xF2F6FC))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .w(px(36.))
                        .h(px(36.))
                        .rounded_lg()
                        .bg(rgb(TEAL))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_lg()
                        .child("S"),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(div().text_lg().child("SliceBridge"))
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(0xA7B9D0))
                                .child("RUST / GPUI ALPHA"),
                        ),
                ),
        )
        .child(div().h(px(1.)).bg(rgb(0x34435B)))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(0x9FB8D3))
                        .child("WORK AREAS"),
                )
                .child({
                    let button =
                        Button::new("nav-project")
                            .label("Project studio")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.navigate(WorkspacePage::Project, cx);
                            }));
                    if view.active_page == WorkspacePage::Project {
                        button.primary()
                    } else {
                        button
                    }
                })
                .child({
                    let button =
                        Button::new("nav-batch")
                            .label("Batch queue")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.navigate(WorkspacePage::Batch, cx);
                            }));
                    if view.active_page == WorkspacePage::Batch {
                        button.primary()
                    } else {
                        button
                    }
                }),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .justify_between()
                .items_center()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(0xA7B9D0))
                        .child("RECENT · SESSION"),
                )
                .child(
                    Button::new("recent-clear")
                        .label("Clear")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.run("projects.clear", json!({}), cx);
                        })),
                ),
        )
        .child(recent)
        .child(div().h(px(1.)).bg(rgb(0x34435B)))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(0xA7B9D0))
                        .child("LOCAL AGENT"),
                )
                .child(badge(
                    "●  Shared registry active".into(),
                    NAV_TINT,
                    0x80E2C4,
                ))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(0xA7B9D0))
                        .child("GUI · CLI · MCP · attach"),
                ),
        );

    let mut profile_options = div().flex().flex_col().gap_3();
    for (machine, title) in [("u1", "Snapmaker U1"), ("qidi", "QIDI X-Plus 5")] {
        let mut choices = div().flex().flex_row().gap_2();
        for (suffix, size) in [("02", "0.2"), ("04", "0.4"), ("06", "0.6"), ("08", "0.8")] {
            let id = format!("{machine}-{suffix}");
            let selected = target == machine && nozzle == size;
            let button = Button::new(id)
                .label(format!("{size} mm"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.select_oem(machine, size, cx);
                }));
            choices = choices.child(if selected { button.primary() } else { button });
        }
        profile_options = profile_options.child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_between()
                        .items_center()
                        .child(div().text_sm().text_color(rgb(INK)).child(title))
                        .child(if target == machine {
                            badge("Selected".into(), TEAL_PALE, 0x12725E)
                        } else {
                            badge("Check locally".into(), 0xF2F5F9, MUTED)
                        }),
                )
                .child(choices),
        );
    }

    let project = panel()
        .flex()
        .flex_col()
        .gap_4()
        .child(
            div()
                .flex()
                .flex_row()
                .justify_between()
                .items_center()
                .child(heading("Current project", "Source 3MF · read-only inspection"))
                .child(
                    if source.is_some() {
                        badge("●  Loaded".into(), TEAL_PALE, 0x12725E)
                    } else {
                        badge("Awaiting source".into(), WARNING_PALE, WARNING)
                    },
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_4()
                .child(model_tile())
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(div().text_xl().text_color(rgb(INK)).child(source_title))
                        .child(div().text_sm().text_color(rgb(MUTED)).child(source_path))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .child(
                                    Button::new("choose-source")
                                        .primary()
                                        .label("Open 3MF")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.choose_file("source.select", cx);
                                        })),
                                ),
                        ),
                ),
        )
        .child(
            div()
                .id("3mf-native-file-drop")
                .rounded_lg()
                .border_1()
                .border_color(rgb(0xB9D2F6))
                .bg(rgb(BLUE_PALE))
                .p_3()
                .text_sm()
                .text_color(rgb(BLUE))
                .child("Drop 3MF into this workspace  ·  single file opens; multiple files stage a batch"),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .gap_3()
                .child(metric("VERTICES", &vertex_count, "Original mesh"))
                .child(metric("TRIANGLES", &triangle_count, "Original mesh")),
        );

    let profiles = panel()
        .flex()
        .flex_col()
        .gap_4()
        .child(
            div()
                .flex()
                .flex_row()
                .justify_between()
                .items_center()
                .child(heading(
                    "Printer profiles",
                    "Native installed OEM presets only",
                ))
                .child(
                    Button::new("discover-oem")
                        .label("Scan OEM")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.run("profiles.discover", json!({}), cx);
                        })),
                ),
        )
        .child(profile_options)
        .child(
            div()
                .rounded_md()
                .bg(rgb(0xF5F8FC))
                .p_3()
                .flex()
                .flex_col()
                .gap_1()
                .child(overline("ACTIVE PRINTER"))
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(INK))
                        .child(printer.to_string()),
                ),
        );

    let materials = panel()
        .flex()
        .flex_col()
        .gap_3()
        .child(heading(
            "Material assignment",
            "Resolve existing vendor PLA / PETG profiles",
        ))
        .child(
            div()
                .rounded_md()
                .bg(rgb(0xF5F8FC))
                .p_3()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(div().w(px(10.)).h(px(10.)).rounded_full().bg(rgb(BLUE)))
                        .child(overline("PLA  ·  INSTALLED PRESET")),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(INK))
                        .child(short_path(material_pla, 76)),
                )
                .child(
                    Button::new("material-pla")
                        .label("Next verified PLA")
                        .on_click(cx.listener(|this, _, _, cx| this.next_material("PLA", cx))),
                ),
        )
        .child(
            div()
                .rounded_md()
                .bg(rgb(0xF5F8FC))
                .p_3()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(div().w(px(10.)).h(px(10.)).rounded_full().bg(rgb(TEAL)))
                        .child(overline("PETG  ·  INSTALLED PRESET")),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(INK))
                        .child(short_path(material_petg, 76)),
                )
                .child(
                    Button::new("material-petg")
                        .label("Next verified PETG")
                        .on_click(cx.listener(|this, _, _, cx| this.next_material("PETG", cx))),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(MUTED))
                .child("Changing material re-resolves the OEM conversion bundle."),
        );

    let conversion = panel()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .flex()
                .flex_row()
                .justify_between()
                .items_center()
                .child(heading(
                    "Convert project",
                    "Save a new 3MF · original is preserved",
                ))
                .child({
                    let (bg, color) = job_tone(phase);
                    badge(format!("●  {phase}"), bg, color)
                }),
        )
        .child(
            div()
                .rounded_md()
                .bg(rgb(0xF5F8FC))
                .p_3()
                .flex()
                .flex_col()
                .gap_1()
                .child(overline("OUTPUT"))
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(INK))
                        .child(short_path(output, 115)),
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .gap_2()
                .child(
                    Button::new("choose-target")
                        .label("Save as…")
                        .on_click(cx.listener(|this, _, _, cx| this.choose_destination(cx))),
                )
                .child(
                    Button::new("convert")
                        .primary()
                        .label("Convert 3MF")
                        .on_click(cx.listener(|this, _, _, cx| this.start_conversion(cx))),
                )
                .child(
                    Button::new("cancel")
                        .label("Cancel")
                        .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                )
                .child(
                    Button::new("open-selected")
                        .label("Open in OEM slicer")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.run("slicer.open_selected", json!({}), cx);
                        })),
                )
                .child(
                    Button::new("open-in-slicer")
                        .label("Other slicer…")
                        .on_click(cx.listener(|this, _, _, cx| this.choose_slicer(cx))),
                ),
        );

    let mut batch_rows = div().flex().flex_col().gap_1();
    if let Some(items) = batch["items"].as_array() {
        for (index, item) in items.iter().enumerate() {
            let name = item["source"]
                .as_str()
                .and_then(|path| Path::new(path).file_name())
                .map(|value| short_path(&value.to_string_lossy(), 45))
                .unwrap_or_else(|| "Unknown 3MF".into());
            let item_phase = item["phase"].as_str().unwrap_or("queued");
            let details = item["receipt"]["error"]
                .as_str()
                .map(|value| short_path(value, 110))
                .or_else(|| {
                    item["receipt"]["bytes"]
                        .as_u64()
                        .map(|bytes| format!("Converted · {bytes} bytes"))
                })
                .unwrap_or_else(|| {
                    if item_phase == "running" {
                        "Converting with verified OEM profile".into()
                    } else {
                        "—".into()
                    }
                });
            let (bg, foreground) = job_tone(item_phase);
            batch_rows = batch_rows.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .p_2()
                    .rounded_md()
                    .bg(rgb(if index % 2 == 0 { 0xF7F9FC } else { WHITE }))
                    .child(
                        div()
                            .w(px(26.))
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child(format!("{:02}", index + 1)),
                    )
                    .child(div().w(px(265.)).text_sm().text_color(rgb(INK)).child(name))
                    .child(
                        div()
                            .w(px(98.))
                            .child(badge(item_phase.into(), bg, foreground)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .text_color(rgb(MUTED))
                            .child(details),
                    ),
            );
        }
    } else {
        batch_rows = batch_rows.child(
            div()
                .rounded_lg()
                .bg(rgb(0xF7FAFE))
                .p_4()
                .text_sm()
                .text_color(rgb(MUTED))
                .child("No batch started. Stage up to 32 authorized 3MF projects, then run the conversion."),
        );
    }

    let batch_panel = panel()
        .flex()
        .flex_col()
        .gap_4()
        .child(
            div()
                .flex()
                .flex_row()
                .justify_between()
                .items_center()
                .child(heading(
                    "Batch workspace",
                    "Sequential conversion · independent file receipts",
                ))
                .child(badge(
                    batch_phase.to_string(),
                    batch_badge_bg,
                    batch_badge_text,
                )),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .gap_2()
                .child(
                    Button::new("batch-sources")
                        .label("Choose 3MF files")
                        .on_click(cx.listener(|this, _, _, cx| this.choose_batch_sources(cx))),
                )
                .child(
                    Button::new("batch-folder")
                        .label("Output folder…")
                        .on_click(cx.listener(|this, _, _, cx| this.choose_batch_directory(cx))),
                )
                .child(
                    Button::new("batch-start")
                        .primary()
                        .label("Start batch")
                        .on_click(cx.listener(|this, _, _, cx| this.start_batch(cx))),
                )
                .child(
                    Button::new("batch-cancel")
                        .label("Cancel batch")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.run("batch.cancel", json!({}), cx);
                        })),
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .justify_between()
                .items_center()
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(MUTED))
                        .child(format!("{staged_files} staged  ·  {batch_dir}")),
                )
                .child(div().text_sm().text_color(rgb(INK)).child(format!(
                    "{batch_completed}/{batch_total} complete  ·  {batch_percent}%"
                ))),
        )
        .child(
            div()
                .w_full()
                .h(px(9.))
                .rounded_full()
                .bg(rgb(0xE6EDF6))
                .child(
                    div()
                        .w(relative(batch_percent as f32 / 100.0))
                        .h_full()
                        .rounded_full()
                        .bg(rgb(if batch_phase == "failed" { RED } else { TEAL })),
                ),
        )
        .when(batch["error"].as_str().is_some(), |panel| {
            panel.child(
                div()
                    .rounded_md()
                    .bg(rgb(RED_PALE))
                    .p_3()
                    .text_sm()
                    .text_color(rgb(RED))
                    .child(batch["error"].as_str().unwrap_or("").to_string()),
            )
        })
        .child(
            div()
                .rounded_lg()
                .border_1()
                .border_color(rgb(HAIRLINE))
                .p_2()
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_3()
                        .p_2()
                        .bg(rgb(BLUE_PALE))
                        .child(div().w(px(26.)).child(overline("#")))
                        .child(div().w(px(265.)).child(overline("SOURCE")))
                        .child(div().w(px(98.)).child(overline("STATUS")))
                        .child(div().flex_1().child(overline("DETAILS / ERRORS"))),
                )
                .child(
                    div()
                        .id("batch-results-scroll")
                        .max_h(px(330.))
                        .overflow_y_scroll()
                        .child(batch_rows),
                ),
        );

    let activity_message = if view.last_message.starts_with("No UI action yet") {
        if batch_total > 0 {
            format!(
                "Live registry · batch {batch_phase} · {batch_completed}/{batch_total} files completed"
            )
        } else if source.is_some() {
            "Live registry · project source selected and ready to configure".to_string()
        } else {
            "Ready for a local file or an agent command".to_string()
        }
    } else {
        view.last_message.clone()
    };
    let activity = panel()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(heading(
                    "Activity",
                    "Live registry state · latest GUI action",
                ))
                .child(
                    Button::new("refresh")
                        .label("Refresh status")
                        .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                ),
        )
        .child(
            div()
                .rounded_md()
                .bg(rgb(0xF7F9FC))
                .p_3()
                .text_sm()
                .text_color(rgb(MUTED))
                .child(short_path(&activity_message, 200)),
        );

    let is_batch_page = view.active_page == WorkspacePage::Batch;
    let (workspace_title, workspace_subtitle) = if is_batch_page {
        (
            "Batch queue",
            "Monitor all 32 file receipts, failures, cancellation, and progress.",
        )
    } else {
        (
            "3MF project studio",
            "Choose a project, verify OEM profiles, then convert confidently.",
        )
    };
    let main_content = if is_batch_page {
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(batch_panel)
            .child(activity)
    } else {
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(project)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_4()
                    .child(profiles.flex_1())
                    .child(materials.flex_1()),
            )
            .child(conversion)
            .child(activity)
    };

    let main_area = div()
        .id("sb-main")
        .flex_1()
        .h_full()
        .overflow_y_scroll()
        .p_6()
        .flex()
        .flex_col()
        .gap_4()
        .bg(rgb(CANVAS))
        .child(
            div()
                .flex()
                .flex_row()
                .justify_between()
                .items_center()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(overline("DESKTOP / PROJECT WORKSPACE"))
                        .child(div().text_2xl().text_color(rgb(INK)).child(workspace_title))
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(MUTED))
                                .child(workspace_subtitle),
                        ),
                )
                .child(badge("Experimental · local only".into(), BLUE_PALE, BLUE)),
        )
        .child(main_content)
        .child(
            div()
                .flex()
                .flex_row()
                .justify_between()
                .text_xs()
                .text_color(rgb(MUTED))
                .child("Native Rust / GPUI Kit · no WebView")
                .child(format!(
                    "Local attach · 127.0.0.1:48637   |   Locale: {TEXT}"
                )),
        );

    div()
        .id("slicebridge-workspace")
        .size_full()
        .flex()
        .flex_row()
        .bg(rgb(CANVAS))
        .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
            this.handle_dropped_paths(paths.paths().to_vec(), cx);
        }))
        .child(sidebar)
        .child(main_area)
}
