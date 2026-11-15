//! Main GPUI shell follows main/native/frontend/dist/index.html + app.css.
//! Baseline: one conversion workspace, not the experimental Project Studio/Batch Dashboard.
use super::*;
use gpui_kit::base::Disableable;
use std::sync::{Arc, OnceLock};

// Embed the original production artwork in the native binary. No runtime WebView,
// JavaScript, developer workstation path, or external asset dependency.
fn brand_mark() -> Arc<Image> {
    static BRAND: OnceLock<Arc<Image>> = OnceLock::new();
    BRAND
        .get_or_init(|| {
            Arc::new(Image::from_bytes(
                ImageFormat::Png,
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../native/frontend/dist/brand-icon.png"
                ))
                .to_vec(),
            ))
        })
        .clone()
}

fn original_banner() -> Arc<Image> {
    static BANNER: OnceLock<Arc<Image>> = OnceLock::new();
    BANNER
        .get_or_init(|| {
            Arc::new(Image::from_bytes(
                ImageFormat::Png,
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../native/frontend/dist/slicebridge-banner.png"
                ))
                .to_vec(),
            ))
        })
        .clone()
}

const BG: u32 = 0xF5FAF7;
const WHITE: u32 = 0xFFFFFF;
const PANEL_MUTED: u32 = 0xF3FAF6;
const STROKE: u32 = 0xD5E6DD;
const INK: u32 = 0x193F38;
const MUTED: u32 = 0x526B61;
const BRAND: u32 = 0x2F7F73;
const LIGHT_BRAND: u32 = 0xDFF6EE;
const RED: u32 = 0xB34940;

fn line(rtl: bool) -> Div {
    if rtl {
        div().flex().flex_row_reverse()
    } else {
        div().flex().flex_row()
    }
}

fn align_text(element: Div, rtl: bool) -> Div {
    if rtl {
        element.text_right()
    } else {
        element
    }
}

fn panel() -> Div {
    div()
        .rounded_lg()
        .border_1()
        .border_color(rgb(STROKE))
        .bg(rgb(WHITE))
        .overflow_hidden()
}

fn heading(number: &str, title: String, rtl: bool) -> Div {
    line(rtl)
        .items_center()
        .gap_2()
        .child(
            div()
                .rounded_md()
                .bg(rgb(0xEAF6F1))
                .px_2()
                .py_1()
                .text_xs()
                .text_color(rgb(BRAND))
                .child(number.to_string()),
        )
        .child(div().text_sm().text_color(rgb(INK)).child(title))
}

fn label(locale: Locale, key: &str) -> String {
    locale.text(key)
}

fn filename(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

fn short_path(path: &str, max_chars: usize) -> String {
    let len = path.chars().count();
    if len <= max_chars {
        return path.into();
    }
    format!(
        "…{}",
        path.chars().skip(len - max_chars).collect::<String>()
    )
}

fn unavailable(view: &mut SliceBridgeWindow, area: &str, cx: &mut Context<SliceBridgeWindow>) {
    view.last_message = format!("{area}: original application behavior not yet migrated to Rust");
    cx.notify();
}

pub(super) fn render_window(
    view: &mut SliceBridgeWindow,
    _window: &mut Window,
    cx: &mut Context<SliceBridgeWindow>,
) -> impl IntoElement {
    let locale = view.locale;
    let rtl = locale.rtl();
    let status = view.commands.snapshot();
    let source = status["source"].as_str().map(str::to_owned);
    let queued = status["batch_selection"]["paths"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let display_files: Vec<String> = if queued.is_empty() {
        source.clone().into_iter().collect()
    } else {
        queued
    };
    let file_count = display_files.len();
    let target = status["target"].as_str().unwrap_or("");
    let nozzle = status["nozzle"].as_str().unwrap_or("");
    let phase = status["phase"].as_str().unwrap_or("idle");
    let result = status["result"].clone();
    let batch = status["batch"].clone();
    let batch_phase = batch["phase"].as_str().unwrap_or("idle");
    let batch_total = batch["total"].as_u64().unwrap_or(0);
    let batch_completed = batch["completed"].as_u64().unwrap_or(0);
    let batch_percent = batch["progress_percent"].as_u64().unwrap_or(0).min(100);
    let running = phase == "running" || batch_phase == "running";

    let brand = line(rtl)
        .items_center()
        .gap_3()
        .child(
            img(brand_mark())
                .w(px(35.))
                .h(px(35.))
                .object_fit(ObjectFit::Contain),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_sm().text_color(rgb(INK)).child("SliceBridge"))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .child(label(locale, "brandTagline")),
                ),
        );

    let topbar = line(rtl)
        .h(px(56.))
        .px_5()
        .items_center()
        .justify_between()
        .bg(rgb(WHITE))
        .border_b_1()
        .border_color(rgb(STROKE))
        .child(brand)
        .child(
            line(rtl)
                .items_center()
                .gap_2()
                .child(
                    Button::new("my-printers")
                        .disabled(running || view.printers_cards.is_empty())
                        .label(if rtl {
                            "המדפסות שלי"
                        } else {
                            "My printers"
                        })
                        .on_click(
                            cx.listener(|this, _, window, cx| this.open_printers(window, cx)),
                        ),
                )
                .child(
                    Button::new("settings")
                        .disabled(true) // Do not expose a nonfunctional settings dialog.
                        .label(label(locale, "settings"))
                        .on_click(
                            cx.listener(|this, _, _, cx| unavailable(this, "Printer settings", cx)),
                        ),
                )
                .child(
                    Button::new("integrations")
                        .disabled(true) // Same default state as the stable developer gate.
                        .label(label(locale, "makerworldSetup"))
                        .on_click(
                            cx.listener(|this, _, _, cx| unavailable(this, "MakerWorld", cx)),
                        ),
                )
                .child(
                    Button::new("about")
                        .disabled(true) // About dialog is still awaiting 1:1 port.
                        .label(if rtl { "אודות" } else { "About" })
                        .on_click(cx.listener(|this, _, _, cx| unavailable(this, "About", cx))),
                )
                .child(
                    line(false)
                        .gap_1()
                        .bg(rgb(0xEDF1F0))
                        .rounded_md()
                        .p_1()
                        .child({
                            let button =
                                Button::new("locale-he")
                                    .label("עברית")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.set_locale(Locale::He, window, cx)
                                    }));
                            if locale == Locale::He {
                                button.primary()
                            } else {
                                button
                            }
                        })
                        .child({
                            let button =
                                Button::new("locale-en").label("EN").on_click(cx.listener(
                                    |this, _, window, cx| this.set_locale(Locale::En, window, cx),
                                ));
                            if locale == Locale::En {
                                button.primary()
                            } else {
                                button
                            }
                        }),
                ),
        );

    let sectionbar = line(rtl)
        .h(px(81.))
        .px_5()
        .items_center()
        .justify_between()
        .child(
            align_text(div().flex().flex_col().gap_1(), rtl)
                .child(
                    div()
                        .text_xl()
                        .text_color(rgb(INK))
                        .child(label(locale, "workspaceTitle")),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(MUTED))
                        .child(label(locale, "workspaceSubtitle")),
                ),
        )
        .child(
            line(rtl)
                .gap_2()
                .items_center()
                .child(div().w(px(7.)).h(px(7.)).rounded_full().bg(rgb(0x2DA078)))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .child(label(locale, "localOnly")),
                ),
        );

    let mut queue_items = div()
        .id("sb-source-queue-list")
        .flex_1()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .px_3();
    if display_files.is_empty() {
        queue_items = queue_items.child(
            div()
                .p_4()
                .text_sm()
                .text_color(rgb(MUTED))
                .child(label(locale, "queueEmpty")),
        );
    }
    for (index, path) in display_files.iter().enumerate() {
        let open_path = path.clone();
        let active = source.as_deref() == Some(path.as_str());
        queue_items = queue_items.child(
            line(rtl)
                .items_center()
                .gap_3()
                .py_2()
                .border_b_1()
                .border_color(rgb(0xEDF1EF))
                .child(
                    div()
                        .w(px(34.))
                        .h(px(34.))
                        .rounded_md()
                        .bg(rgb(0xEBF4EE))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_xs()
                        .text_color(rgb(BRAND))
                        .child("3MF"),
                )
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(INK))
                                .child(short_path(&filename(path), 48)),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(MUTED))
                                .child(short_path(path, 80)),
                        ),
                )
                .child({
                    let button = Button::new(format!("source-open-{index}"))
                        .label(if active {
                            if rtl {
                                "פעיל"
                            } else {
                                "Active"
                            }
                        } else if rtl {
                            "בחר"
                        } else {
                            "Select"
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.run("source.select", json!({"path":open_path.clone()}), cx);
                        }));
                    if active {
                        button.primary()
                    } else {
                        button
                    }
                }),
        );
    }

    let queue_panel = panel()
        .flex_1()
        .flex()
        .flex_col()
        .child(
            line(rtl)
                .items_center()
                .justify_between()
                .h(px(51.))
                .px_4()
                .border_b_1()
                .border_color(rgb(0xEBEFED))
                .child(
                    line(rtl)
                        .items_center()
                        .gap_2()
                        .child(heading("01", label(locale, "queueTitle"), rtl))
                        .child(
                            div()
                                .rounded_full()
                                .bg(rgb(0xEEF1F0))
                                .px_2()
                                .text_xs()
                                .text_color(rgb(MUTED))
                                .child(file_count.to_string()),
                        ),
                )
                .child(
                    Button::new("clear-source-queue")
                        .disabled(running || file_count == 0)
                        .label(label(locale, "clear"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.run("batch.clear", json!({}), cx);
                        })),
                ),
        )
        .child(
            line(rtl)
                .p_3()
                .gap_2()
                .child(
                    Button::new("add-source-files")
                        .disabled(running)
                        .primary()
                        .label(format!("＋ {}", label(locale, "addFiles")))
                        .on_click(cx.listener(|this, _, _, cx| this.choose_batch_sources(cx))),
                )
                .child(
                    Button::new("add-source-folder")
                        .disabled(running)
                        .label(format!("▤ {}", label(locale, "addFolder")))
                        .on_click(cx.listener(|this, _, _, cx| this.choose_source_folder(cx))),
                ),
        )
        .child(
            div()
                .mx_3()
                .mb_3()
                .rounded_md()
                .border_1()
                .border_color(rgb(0xA7C7BC))
                .bg(rgb(0xF6FAF8))
                .p_3()
                .text_sm()
                .text_color(rgb(MUTED))
                .child(label(locale, "dropHint")),
        )
        .child(queue_items)
        .child(
            div()
                .px_4()
                .py_2()
                .bg(rgb(0xFCFDFC))
                .border_t_1()
                .border_color(rgb(0xF0F2F0))
                .text_xs()
                .text_color(rgb(MUTED))
                .child(label(locale, "queueFootnote")),
        );

    let mut targets = div().flex().flex_col().gap_2().px_3().py_2();
    // The stable app uses the "My printers" library and installed OEM resolver.
    // Until that is ported, these only select targets supported by this Rust Registry.
    let ready_rows = desktop_printer_picker::selected_ready_legacy_rows(
        &view.printers_saved,
        &view.ready_oem_targets,
    );
    for &(id, title) in &ready_rows {
        let selected = target == id;
        targets = targets.child(
            line(rtl)
                .items_center()
                .gap_3()
                .p_2()
                .rounded_md()
                .bg(rgb(if selected { LIGHT_BRAND } else { WHITE }))
                .child(
                    Button::new(format!("target-{id}"))
                        .disabled(running)
                        .label(if selected { "☑" } else { "☐" })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.select_oem(id, "0.4", cx);
                        })),
                )
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(div().text_sm().text_color(rgb(INK)).child(title))
                        .child(div().text_xs().text_color(rgb(MUTED)).child(if selected {
                            format!("OEM · {nozzle} mm")
                        } else {
                            "—".to_string()
                        })),
                ),
        );
    }
    if ready_rows.is_empty() {
        targets = targets.child(div().p_3().text_sm().text_color(rgb(MUTED)).child(if rtl {
            "לא נבחרו מדפסות יעד זמינות. אפשר להוסיף דרך סמל המדפסת בראש החלון."
        } else {
            "No target printers selected. Use the printer icon at the top to add them."
        }));
    }
    let targets_panel = panel()
        .h(px(230.))
        .flex()
        .flex_col()
        .child(
            line(rtl)
                .items_center()
                .justify_between()
                .h(px(50.))
                .px_4()
                .border_b_1()
                .border_color(rgb(0xEBEFED))
                .child(heading("02", label(locale, "targetsTitle"), rtl))
                .child(
                    Button::new("select-all-printers")
                        .disabled(true) // Registry only has one selected OEM target.
                        .label(label(locale, "selectAll"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            unavailable(this, "Multi-target selection", cx)
                        })),
                ),
        )
        .child(targets);

    let status_result = if batch_total > 0 {
        format!("{batch_completed}/{batch_total} · {batch_phase}")
    } else if phase == "completed" {
        label(locale, "summarySuccess")
    } else if phase == "failed" {
        label(locale, "summaryFailure")
    } else {
        label(locale, "emptyTitle")
    };
    let mut results_content = div()
        .id("sb-results-scroll")
        .flex_1()
        .overflow_y_scroll()
        .p_4()
        .flex()
        .flex_col()
        .gap_3();

    if let Some(items) = batch["items"].as_array() {
        for (idx, item) in items.iter().enumerate() {
            let name = item["source"]
                .as_str()
                .map(filename)
                .unwrap_or_else(|| format!("#{idx}"));
            let phase = item["phase"].as_str().unwrap_or("queued");
            let message = item["receipt"]["error"]
                .as_str()
                .or_else(|| item["receipt"]["data"]["error"].as_str())
                .unwrap_or("");
            results_content = results_content.child(
                div()
                    .border_b_1()
                    .border_color(rgb(STROKE))
                    .pb_3()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_sm().text_color(rgb(INK)).child(name))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child(phase.to_string()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(RED))
                            .child(message.to_string()),
                    ),
            );
        }
    } else if phase == "idle" || (result.is_null() && batch_total == 0) {
        results_content = results_content.child(
            div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .child(div().text_xl().text_color(rgb(BRAND)).child("↗"))
                .child(
                    div()
                        .text_lg()
                        .text_color(rgb(INK))
                        .child(label(locale, "emptyTitle")),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(MUTED))
                        .child(label(locale, "emptyDescription")),
                ),
        );
    } else {
        results_content = results_content.child(
            div()
                .rounded_md()
                .bg(rgb(PANEL_MUTED))
                .p_3()
                .text_sm()
                .text_color(rgb(if phase == "failed" { RED } else { INK }))
                .child(result.to_string()),
        );
    }

    let result_panel = panel()
        .flex_1()
        .h_full()
        .flex()
        .flex_col()
        .child(
            line(rtl)
                .items_center()
                .justify_between()
                .h(px(51.))
                .px_4()
                .border_b_1()
                .border_color(rgb(0xEBEFED))
                .child(heading("03", label(locale, "resultsTitle"), rtl))
                .child(
                    Button::new("clear-results")
                        .disabled(running || (result.is_null() && batch_total == 0))
                        .label(label(locale, "clearResults"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.run("results.clear", json!({}), cx);
                        })),
                ),
        )
        .child(results_content);

    let left_column = div()
        .w(px(370.))
        .h_full()
        .flex()
        .flex_col()
        .gap_3()
        .child(queue_panel)
        .child(targets_panel);

    let workspace = line(rtl)
        .id("sb-main-workspace")
        .flex_1()
        .min_h(px(0.))
        .px_5()
        .pb_4()
        .gap_3()
        .child(left_column)
        .child(result_panel);

    let activity_text = if running {
        label(locale, "converting")
    } else if batch_total > 0 && batch_phase == "completed" {
        label(locale, "finished")
    } else if file_count > 0 && !target.is_empty() {
        label(locale, "ready")
    } else {
        label(locale, "idle")
    };
    let activity_detail = if view.last_message.starts_with("No UI action yet") {
        label(locale, "idleHint")
    } else {
        short_path(&view.last_message, 155)
    };

    let actionbar = line(rtl)
        .h(px(76.))
        .px_5()
        .justify_between()
        .items_center()
        .bg(rgb(WHITE))
        .border_t_1()
        .border_color(rgb(STROKE))
        .child(
            line(rtl)
                .items_center()
                .gap_3()
                .child(div().w(px(10.)).h(px(10.)).rounded_full().bg(rgb(BRAND)))
                .child(
                    align_text(div().flex().flex_col().gap_1(), rtl)
                        .child(div().text_sm().text_color(rgb(INK)).child(activity_text))
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(MUTED))
                                .child(activity_detail),
                        ),
                ),
        )
        .child(
            line(rtl)
                .gap_3()
                .items_center()
                .child(
                    Button::new("output-folder")
                        .disabled(output_folder(&status).is_none())
                        .label(label(locale, "openFolder"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.open_output_folder(cx);
                        })),
                )
                .child(
                    Button::new("convert-main")
                        .disabled(
                            running
                                || file_count == 0
                                || target.is_empty()
                                || !ready_rows.iter().any(|(id, _)| *id == target),
                        )
                        .primary()
                        .label(label(locale, "convert"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if file_count > 1 {
                                this.start_batch(cx);
                            } else if this.commands.snapshot()["source"].is_null() {
                                this.last_message = this.locale.text("noSource");
                                cx.notify();
                            } else if this.destination.is_none() {
                                this.choose_destination(cx);
                            } else {
                                this.start_conversion(cx);
                            }
                        })),
                ),
        );

    // Exact stable element order: actionbar, original 91px brand banner,
    // then a separate 49px conversion-progress row (not a status strip).
    let brand_banner = div()
        .id("sb-brand-banner")
        .relative()
        .w_full()
        .h(px(91.))
        .flex_none()
        .overflow_hidden()
        .bg(rgb(0x0E3D36))
        .border_t_1()
        .border_color(rgb(0x256C59))
        .child(
            div()
                .absolute()
                .right_0()
                .top_0()
                .h_full()
                .w(relative(0.78))
                .child(
                    img(original_banner())
                        .size_full()
                        .object_fit(ObjectFit::Cover),
                ),
        )
        .child(
            div()
                .relative()
                .flex()
                .flex_col()
                .justify_center()
                .h_full()
                .px_9()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(0xF2FFF9))
                        .child("DIFFERENT SLICERS."),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(0xF2FFF9))
                        .child("ONE SIMPLE WORKFLOW."),
                )
                .child(div().w(px(26.)).h(px(2.)).bg(rgb(0xA7F3D0)))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(0xDEEEE8))
                        .child("SliceBridge makes your 3D printing workflow more open, flexible, and connected."),
                ),
        );

    let progress = div()
        .id("sb-conversion-progress")
        .h(px(49.))
        .flex_none()
        .px_5()
        .py_2()
        .flex()
        .flex_col()
        .justify_center()
        .gap_2()
        .bg(rgb(0xF9FFFB))
        .border_t_1()
        .border_color(rgb(0xB9D9CA))
        .child(
            line(false)
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(0x426959))
                        .child(status_result),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(INK))
                        .child(format!("{batch_percent}%")),
                ),
        )
        .child(
            div()
                .w_full()
                .h(px(7.))
                .rounded_full()
                .bg(rgb(0xD9EEE5))
                .child(
                    div()
                        .w(relative(batch_percent as f32 / 100.0))
                        .h_full()
                        .rounded_full()
                        .bg(rgb(BRAND)),
                ),
        );

    let mut root = div()
        .id("slicebridge-workspace")
        .relative()
        .size_full()
        .flex()
        .flex_col()
        .bg(rgb(BG))
        .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
            this.handle_dropped_paths(paths.paths().to_vec(), cx);
        }))
        .child(topbar)
        .child(sectionbar)
        .child(workspace)
        .child(actionbar)
        .child(brand_banner)
        .child(progress);
    if view.printers_open {
        root = root.child(view.render_printers(cx));
    }
    root
}
