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

// Mirrors app.js resultLine: a receipt reads as `CODE: error`, keeping whichever parts exist.
pub(super) fn receipt_error(receipt: &Value) -> String {
    let error = receipt["error"]
        .as_str()
        .or_else(|| receipt["data"]["error"].as_str())
        .unwrap_or("");
    let code = receipt["code"].as_str().unwrap_or("");
    match (code.is_empty(), error.is_empty()) {
        (true, _) => error.to_string(),
        (false, true) => code.to_string(),
        (false, false) => format!("{code}: {error}"),
    }
}

// Mirrors app.js renderResults: successes out of all finished results for one source file.
pub(super) fn result_counts(item: &Value) -> (usize, usize) {
    let results = item["results"].as_array();
    let failures = item["failures"].as_array();
    if results.is_some() || failures.is_some() {
        let succeeded = results.map_or(0, Vec::len);
        let failed = failures.map_or(0, Vec::len);
        return (succeeded, succeeded + failed);
    }
    // Legacy single-target Registry batches do not have nested result arrays.
    // Do not render their successful outputs as "0 / 0".
    match item["phase"].as_str() {
        Some("completed") => (1, 1),
        Some("failed") => (0, 1),
        _ => (0, 0),
    }
}

// Mirrors app.js renderTargets: the notice shows unless a ready printer is selected for conversion.
pub(super) fn empty_target_notice_visible(
    ready_rows: &[(&str, &str)],
    selected: &BTreeSet<String>,
) -> bool {
    !ready_rows.iter().any(|(id, _)| selected.contains(*id))
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
    let display_files = queue_display_paths(&status, view.suppressed_source.as_deref());
    // Both staged files and the lone directly selected 3MF are removable.
    // The Registry handles either without ever deleting a source file.
    let file_count = display_files.len();
    let phase = status["phase"].as_str().unwrap_or("idle");
    let result = status["result"].clone();
    let batch = status["batch"].clone();
    let batch_phase = batch["phase"].as_str().unwrap_or("idle");
    let batch_total = batch["total"].as_u64().unwrap_or(0);
    let batch_completed = batch["completed"].as_u64().unwrap_or(0);
    let batch_failures = batch["failures"].as_u64().unwrap_or(0);
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

    // Locked like the stable app: enabled only while Registry developer mode is an explicit On.
    let integrations = Button::new("integrations")
        .disabled(!view.integrations_developer)
        .label(label(locale, "makerworldSetup"));
    let integrations = if view.integrations_developer {
        integrations
    } else {
        integrations.tooltip(label(locale, "makerworldLocked"))
    };
    let integrations =
        integrations.on_click(cx.listener(|this, _, _, cx| this.open_integrations(cx)));

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
                        .disabled(running)
                        .label(label(locale, "settings"))
                        .on_click(cx.listener(|this, _, _, cx| this.open_settings(cx))),
                )
                .child(integrations)
                .child(
                    Button::new("about")
                        .label(label(locale, "aboutOpen"))
                        .on_click(cx.listener(|this, _, _, cx| this.open_about(cx))),
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
    for path in display_files.iter() {
        let open_path = path.clone();
        let remove_path = path.clone();
        let active = source.as_deref() == Some(path.as_str());
        queue_items = queue_items.child(
            line(rtl)
                .items_center()
                .gap_3()
                .py_2()
                .border_b_1()
                .border_color(rgb(0xEDF1EF))
                .child({
                    // Mirrors app.js renderQueue: only an exact high-confidence MakerWorld origin reads as MW.
                    let verified = view.makerworld_verified(path);
                    div()
                        .w(px(34.))
                        .h(px(34.))
                        .rounded_md()
                        .bg(rgb(if verified { 0xDFF6EE } else { 0xEBF4EE }))
                        .border_1()
                        .border_color(rgb(if verified { 0xA9E5D0 } else { 0xEBF4EE }))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_xs()
                        .text_color(rgb(if verified { INK } else { BRAND }))
                        .child(if verified { "MW" } else { "3MF" })
                })
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
                    let button = Button::new(file_button_id("source-open", path))
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
                })
                // A peer of the open button, not nested in it. Disabled during a conversion, as in app.js.
                .child(
                    Button::new(file_button_id("source-remove", path))
                        .disabled(running)
                        .label("×")
                        .tooltip(if rtl {
                            "הסר מהתור"
                        } else {
                            "Remove from queue"
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if snapshot_running(&this.commands.snapshot()) {
                                return;
                            }
                            this.run("batch.remove", json!({"path":remove_path.clone()}), cx);
                        })),
                ),
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
        let selected = view.selected_batch_targets.contains(id);
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
                            this.toggle_batch_target(id, cx);
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
                            let saved = view.settings_loaded["printers"][id]["nozzle"]
                                .as_str()
                                .unwrap_or("auto");
                            if saved == "auto" {
                                "OEM · auto".to_string()
                            } else {
                                format!("OEM · {saved} mm")
                            }
                        } else {
                            "—".to_string()
                        })),
                ),
        );
    }
    if empty_target_notice_visible(&ready_rows, &view.selected_batch_targets) {
        targets = targets.child(div().p_3().text_sm().text_color(rgb(MUTED)).child(if rtl {
            "לא נבחרו מדפסות יעד זמינות. אפשר להוסיף דרך סמל המדפסת בראש החלון."
        } else {
            "No target printers selected. Use the printer icon at the top to add them."
        }));
    }
    let all_ready_checked = !ready_rows.is_empty()
        && ready_rows
            .iter()
            .all(|(id, _)| view.selected_batch_targets.contains(*id));
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
                        .disabled(running || ready_rows.is_empty())
                        .label(label(
                            locale,
                            if all_ready_checked {
                                "deselectAll"
                            } else {
                                "selectAll"
                            },
                        ))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.select_all_batch_targets(cx);
                        })),
                ),
        )
        .child(targets);

    let status_result = if batch_total > 0 {
        if batch_failures > 0 {
            format!(
                "{batch_completed}/{batch_total} · {batch_phase} · {batch_failures} {}",
                label(locale, "summaryFailure")
            )
        } else {
            format!("{batch_completed}/{batch_total} · {batch_phase}")
        }
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
            let message = receipt_error(&item["receipt"]);
            let message = if message.is_empty() {
                item["error"].as_str().unwrap_or("").to_owned()
            } else {
                message
            };
            let (succeeded, finished) = result_counts(item);
            let mut name_row = line(rtl)
                .items_center()
                .justify_between()
                .gap_2()
                .child(div().text_sm().text_color(rgb(INK)).child(name));
            if finished != 0 {
                name_row = name_row.child(
                    div()
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .child(format!("{succeeded} / {finished}")),
                );
            }
            let mut file_report = div()
                .border_b_1()
                .border_color(rgb(STROKE))
                .pb_3()
                .flex()
                .flex_col()
                .gap_1()
                .child(name_row)
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .child(phase.to_string()),
                );
            if let Some(targets) = item["targets"].as_array() {
                for target in targets {
                    let id = target["target"].as_str().unwrap_or("unknown");
                    let state = target["phase"].as_str().unwrap_or("queued");
                    let error = receipt_error(&target["receipt"]);
                    let detail = if error.is_empty() {
                        format!("{id} · {state}")
                    } else {
                        format!("{id} · {state} · {error}")
                    };
                    file_report = file_report.child(
                        div()
                            .text_xs()
                            .text_color(rgb(if state == "failed" { RED } else { MUTED }))
                            .child(detail),
                    );
                }
            }
            if !message.is_empty() {
                file_report =
                    file_report.child(div().text_xs().text_color(rgb(RED)).child(message));
            }
            results_content = results_content.child(file_report);
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
    } else if batch_total > 0 && batch_phase == "completed" && batch_failures > 0 {
        label(locale, "summaryFailure")
    } else if batch_total > 0 && batch_phase == "completed" {
        label(locale, "finished")
    } else if file_count > 0
        && ready_rows
            .iter()
            .any(|(id, _)| view.selected_batch_targets.contains(*id))
    {
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
                                || !ready_rows
                                    .iter()
                                    .any(|(id, _)| view.selected_batch_targets.contains(*id)),
                        )
                        .primary()
                        .label(label(locale, "convert"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.start_batch(cx);
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
    if view.settings_open {
        root = root.child(view.render_settings(cx));
    }
    if view.about_open {
        root = root.child(render_about(view, cx));
    }
    if view.integrations_open {
        root = root.child(render_integrations(view, cx));
    }
    root
}

// Mirrors index.html #updateNotice and app.js renderUpdates: one status line, at most one link,
// and a dismiss control. Nothing shows until a deliberate check or a link attempt produces state.
fn render_update_notice(
    view: &SliceBridgeWindow,
    cx: &mut Context<SliceBridgeWindow>,
) -> Option<Div> {
    let locale = view.locale;
    let rtl = locale.rtl();
    let update = &view.update;
    let (text, failed, action) = if update.checking {
        (label(locale, "checkingUpdates"), false, None)
    } else if !update.error.is_empty() {
        (
            format!("{}: {}", label(locale, "updateError"), update.error),
            true,
            None,
        )
    } else if let Some(data) = &update.data {
        let notice = update_notice_for(data);
        (
            label(locale, notice.message_key).replace("{v}", &notice.version),
            false,
            notice.action,
        )
    } else {
        return None;
    };

    let mut row = line(rtl).items_center().gap_2().child(
        div()
            .flex_1()
            .text_xs()
            .text_color(if failed { rgb(RED) } else { rgb(INK) })
            .child(text),
    );
    if let Some(action) = action {
        row = row.child(
            Button::new("update-action")
                .label(label(locale, action.label_key()))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.open_update_action(action.clone(), cx);
                })),
        );
    }
    Some(
        row.child(
            Button::new("dismiss-update")
                .label("×")
                .tooltip(label(locale, "dismiss"))
                .on_click(cx.listener(|this, _, _, cx| this.dismiss_update_notice(cx))),
        ),
    )
}

// Mirrors index.html #aboutDialog and about.js: same copy, sections and order. Developer mode
// toggles natively, and the source/releases links open the pinned GitHub URLs on click.
fn render_about(view: &SliceBridgeWindow, cx: &mut Context<SliceBridgeWindow>) -> impl IntoElement {
    let locale = view.locale;
    let rtl = locale.rtl();
    let shown = view.about_document;
    let version = format!("v{ABOUT_INSTALLED_VERSION}");
    let developer_state = label(
        locale,
        if view.about_developer_mode {
            "aboutDevOn"
        } else {
            "aboutDevOff"
        },
    );

    let header = line(rtl)
        .items_center()
        .justify_between()
        .px_5()
        .py_4()
        .border_b_1()
        .border_color(rgb(0xEBEFED))
        .child(
            align_text(div().flex().flex_col().gap_1(), rtl)
                .child(
                    div()
                        .text_lg()
                        .text_color(rgb(INK))
                        .child(label(locale, "aboutTitle")),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .child(label(locale, "aboutSubtitle")),
                ),
        )
        .child(
            Button::new("close-about-x")
                .label("×")
                .on_click(cx.listener(|this, _, _, cx| this.close_about(cx))),
        );

    let identity = line(rtl)
        .items_center()
        .gap_3()
        .child(
            img(brand_mark())
                .w(px(50.))
                .h(px(50.))
                .object_fit(ObjectFit::Contain),
        )
        .child(div().text_sm().text_color(rgb(INK)).child("SliceBridge"));

    // Same placement as index.html: the check button sits in the version row, and its notice below.
    let check_button = Button::new("check-updates")
        .disabled(view.update.checking)
        .label(label(locale, "checkUpdates"))
        .on_click(cx.listener(|this, _, _, cx| this.check_for_updates(cx)));
    let version_row = line(rtl)
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            align_text(div().flex().flex_col().gap_1(), rtl)
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(INK))
                        .child(label(locale, "aboutVersion")),
                )
                .child(div().text_sm().text_color(rgb(MUTED)).child(version)),
        )
        .child(check_button);
    let mut version_card = panel().p_4().flex().flex_col().gap_2().child(version_row);
    if let Some(notice) = render_update_notice(view, cx) {
        version_card = version_card.child(notice);
    }
    if view.update.link_error {
        version_card = version_card.child(
            div()
                .text_xs()
                .text_color(rgb(RED))
                .child(label(locale, "aboutLinkError")),
        );
    }

    let mut developer_card = panel().p_4().flex().flex_col().gap_2().child(
        line(rtl)
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                align_text(div().flex().flex_col().gap_1(), rtl)
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(INK))
                            .child(label(locale, "aboutDevTitle")),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child(label(locale, "aboutDevWarning")),
                    ),
            )
            .child(
                Button::new("about-developer-toggle")
                    .label(format!(
                        "{} · {}",
                        label(locale, "aboutDevButton"),
                        developer_state
                    ))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_about_developer_mode(cx);
                    })),
            ),
    );
    if view.about_developer_error {
        developer_card = developer_card.child(
            div()
                .text_xs()
                .text_color(rgb(RED))
                .child(label(locale, "aboutDevError")),
        );
    }

    let mut license_card = panel()
        .p_4()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_sm()
                .text_color(rgb(INK))
                .child(label(locale, "aboutLicense")),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(MUTED))
                .child(label(locale, "aboutLicenseSummary")),
        )
        .child(
            div()
                .text_sm()
                .text_color(rgb(INK))
                .child(label(locale, "aboutCredits")),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(MUTED))
                .child(label(locale, "aboutCreditBl")),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(MUTED))
                .child(label(locale, "aboutCreditWails")),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(MUTED))
                .child(label(locale, "aboutCreditVendor")),
        )
        .child(
            line(rtl)
                .gap_2()
                .child(
                    Button::new("about-license-document")
                        .label(label(
                            locale,
                            if shown == Some(AboutDocument::License) {
                                "aboutHideDocument"
                            } else {
                                "aboutFullLicense"
                            },
                        ))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.toggle_about_document(AboutDocument::License, cx);
                        })),
                )
                .child(
                    Button::new("about-credits-document")
                        .label(label(
                            locale,
                            if shown == Some(AboutDocument::Credits) {
                                "aboutHideDocument"
                            } else {
                                "aboutFullCredits"
                            },
                        ))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.toggle_about_document(AboutDocument::Credits, cx);
                        })),
                ),
        );
    if let Some(document) = shown {
        let mut text = div()
            .id("about-legal-document")
            .max_h(px(260.))
            .overflow_y_scroll()
            .p_3()
            .rounded_md()
            .bg(rgb(PANEL_MUTED))
            .flex()
            .flex_col();
        for text_line in document.text().lines() {
            let text_line = if text_line.is_empty() { " " } else { text_line };
            text = text.child(
                div()
                    .text_xs()
                    .text_color(rgb(INK))
                    .child(text_line.to_string()),
            );
        }
        license_card = license_card.child(text);
    }

    let links = line(rtl)
        .gap_2()
        .child(
            Button::new("about-source")
                .label(label(locale, "aboutSource"))
                .on_click(cx.listener(|this, _, _, cx| this.open_about_link("source", cx))),
        )
        .child(
            Button::new("about-releases")
                .label(label(locale, "aboutReleases"))
                .on_click(cx.listener(|this, _, _, cx| this.open_about_link("releases", cx))),
        );

    let mut body = div()
        .id("about-scroll")
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scroll()
        .p_5()
        .flex()
        .flex_col()
        .gap_3()
        .child(identity)
        .child(version_card)
        .child(developer_card)
        .child(license_card)
        .child(links);
    if view.about_link_error {
        body = body.child(
            div()
                .text_xs()
                .text_color(rgb(RED))
                .child(label(locale, "aboutLinkError")),
        );
    }
    let body = body.child(
        div()
            .text_xs()
            .text_color(rgb(MUTED))
            .child(label(locale, "aboutOffline")),
    );

    let footer = line(rtl)
        .justify_end()
        .px_5()
        .py_3()
        .border_t_1()
        .border_color(rgb(STROKE))
        .child(
            Button::new("close-about")
                .primary()
                .label(label(locale, "aboutClose"))
                .on_click(cx.listener(|this, _, _, cx| this.close_about(cx))),
        );

    div()
        .id("aboutDialog")
        .absolute()
        .inset_0()
        .occlude()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgba(0x193F387F))
        .child(
            div()
                .id("aboutFrame")
                .w(px(760.))
                .max_w(relative(0.94))
                .h(relative(0.86))
                .bg(rgb(WHITE))
                .rounded_lg()
                .border_1()
                .border_color(rgb(STROKE))
                .overflow_hidden()
                .flex()
                .flex_col()
                .child(header)
                .child(body)
                .child(footer),
        )
}

// One setup card, as index.html's .integration-step: number, title, hint. The active card gets the brand border.
fn setup_step(number: &str, title: String, hint: String, active: bool, rtl: bool) -> Div {
    panel()
        .p_4()
        .flex()
        .flex_col()
        .gap_3()
        .border_color(rgb(if active { BRAND } else { STROKE }))
        .child(heading(number, title, rtl))
        .child(align_text(
            div().text_xs().text_color(rgb(MUTED)).child(hint),
            rtl,
        ))
}

fn feedback_line(feedback: &Feedback, rtl: bool) -> Option<Div> {
    (!feedback.text.is_empty()).then(|| {
        align_text(
            div()
                .text_xs()
                .text_color(rgb(if feedback.error { RED } else { INK }))
                .child(feedback.text.clone()),
            rtl,
        )
    })
}

// Mirrors index.html #integrationsDialog and integrations.js: the same three steps, copy and order.
// Every browser action runs from an explicit click, and the topbar entry stays locked until developer
// mode is On. Step 1 reports link status only; no enable control exists until registration is ported.
fn render_integrations(
    view: &SliceBridgeWindow,
    cx: &mut Context<SliceBridgeWindow>,
) -> impl IntoElement {
    let locale = view.locale;
    let rtl = locale.rtl();
    let busy = view.integrations_busy;
    let browser = view.integrations_browser;
    let firefox = browser == BrowserChoice::Firefox;
    let active = active_setup_step(view.integrations_link, view.integrations_install_opened);
    let page = view.integrations_page.unwrap_or("");

    let header = line(rtl)
        .items_center()
        .justify_between()
        .px_5()
        .py_4()
        .border_b_1()
        .border_color(rgb(0xEBEFED))
        .child(
            align_text(div().flex().flex_col().gap_1(), rtl)
                .child(
                    div()
                        .text_lg()
                        .text_color(rgb(INK))
                        .child(label(locale, "integrationTitle")),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .child(label(locale, "integrationHint")),
                ),
        )
        .child(
            Button::new("close-integrations-x")
                .label("×")
                .on_click(cx.listener(|this, _, _, cx| this.close_integrations(cx))),
        );

    let link = view.integrations_link;
    let step_one = setup_step(
        "01",
        label(locale, "stepOneTitle"),
        label(locale, "stepOneHint"),
        active == 1,
        rtl,
    )
    .child(align_text(
        div()
            .text_xs()
            .text_color(rgb(if link == LinkState::Error { RED } else { INK }))
            .child(label(locale, link.message_key())),
        rtl,
    ));

    let mut browser_row = line(rtl).gap_2();
    for choice in BrowserChoice::ALL {
        let button = Button::new(format!("integrations-browser-{}", choice.key()))
            .disabled(busy)
            .label(choice.display_name())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select_integrations_browser(choice, cx);
            }));
        browser_row = browser_row.child(if choice == browser {
            button.primary()
        } else {
            button
        });
    }

    let actions = line(rtl)
        .gap_2()
        .child(
            Button::new("integrations-prepare")
                .disabled(busy)
                .label(label(locale, "prepareExtension"))
                .on_click(cx.listener(|this, _, _, cx| this.prepare_integrations_extension(cx))),
        )
        .child(
            Button::new("integrations-open-manager")
                .disabled(busy)
                .label(label(locale, "openExtensionsPage"))
                .on_click(cx.listener(|this, _, _, cx| this.open_integrations_manager(cx))),
        );

    let (two, three) = if firefox {
        (
            "installInstructionTwoFirefox",
            "installInstructionThreeFirefox",
        )
    } else {
        ("installInstructionTwo", "installInstructionThree")
    };
    let instructions = div()
        .flex()
        .flex_col()
        .gap_1()
        .child(align_text(
            div().text_xs().text_color(rgb(INK)).child(format!(
                "1. {} {page}",
                label(locale, "installInstructionOne")
            )),
            rtl,
        ))
        .child(align_text(
            div()
                .text_xs()
                .text_color(rgb(INK))
                .child(format!("2. {}", label(locale, two))),
            rtl,
        ))
        .child(align_text(
            div()
                .text_xs()
                .text_color(rgb(INK))
                .child(format!("3. {}", label(locale, three))),
            rtl,
        ));

    let mut step_two = setup_step(
        "02",
        label(locale, "stepTwoTitle"),
        label(locale, "stepTwoHint"),
        active == 2,
        rtl,
    )
    .child(align_text(
        div()
            .text_xs()
            .text_color(rgb(MUTED))
            .child(label(locale, "browserLabel")),
        rtl,
    ))
    .child(browser_row)
    .child(actions)
    .child(instructions)
    .child(align_text(
        div()
            .text_xs()
            .text_color(rgb(MUTED))
            .child(label(locale, "storeNotPublished")),
        rtl,
    ))
    .child(align_text(
        div()
            .text_xs()
            .text_color(rgb(MUTED))
            .child(label(locale, "betaInstallInfo")),
        rtl,
    ));
    if firefox {
        step_two = step_two.child(align_text(
            div()
                .text_xs()
                .text_color(rgb(RED))
                .child(label(locale, "firefoxInstallWarning")),
            rtl,
        ));
    }
    if let Some(folder) = &view.integrations_folder {
        step_two = step_two.child(align_text(
            div()
                .text_xs()
                .text_color(rgb(INK))
                .child(format!("{} {folder}", label(locale, "folderPrepared"))),
            rtl,
        ));
    }
    if let Some(feedback) = feedback_line(&view.integrations_install, rtl) {
        step_two = step_two.child(feedback);
    }

    let mut modes = div().flex().flex_col().gap_2();
    for mode in LinkMode::ALL {
        let button = Button::new(format!("integrations-mode-{}", mode.key()))
            .disabled(busy)
            .label(label(locale, mode.title_key()))
            .on_click(cx.listener(move |this, _, _, cx| this.select_integrations_mode(mode, cx)));
        let button = if view.integrations_mode == Some(mode) {
            button.primary()
        } else {
            button
        };
        modes = modes.child(
            line(rtl)
                .items_center()
                .gap_3()
                .child(button)
                .child(align_text(
                    div()
                        .flex_1()
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .child(label(locale, mode.detail_key())),
                    rtl,
                )),
        );
    }

    let confirm = Button::new("integrations-confirm-mode")
        .primary()
        .disabled(busy || view.integrations_mode.is_none())
        .label(label(locale, "confirmInBrowser"))
        .on_click(cx.listener(|this, _, _, cx| this.open_integrations_mode(cx)));
    let mut step_three = setup_step(
        "03",
        label(locale, "stepThreeTitle"),
        label(locale, "stepThreeHint"),
        active == 3,
        rtl,
    )
    .child(modes)
    .child(confirm);
    if let Some(feedback) = feedback_line(&view.integrations_mode_feedback, rtl) {
        step_three = step_three.child(feedback);
    }

    let body = div()
        .id("integrations-scroll")
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scroll()
        .p_5()
        .flex()
        .flex_col()
        .gap_3()
        .child(step_one)
        .child(step_two)
        .child(step_three)
        .child(align_text(
            div()
                .text_xs()
                .text_color(rgb(MUTED))
                .child(label(locale, "modeScopeNote")),
            rtl,
        ));

    let footer = line(rtl)
        .justify_end()
        .px_5()
        .py_3()
        .border_t_1()
        .border_color(rgb(STROKE))
        .child(
            Button::new("close-integrations")
                .primary()
                .label(label(locale, "done"))
                .on_click(cx.listener(|this, _, _, cx| this.close_integrations(cx))),
        );

    div()
        .id("integrationsDialog")
        .absolute()
        .inset_0()
        .occlude()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgba(0x193F387F))
        .child(
            div()
                .id("integrationsFrame")
                .w(px(760.))
                .max_w(relative(0.94))
                .h(relative(0.86))
                .bg(rgb(WHITE))
                .rounded_lg()
                .border_1()
                .border_color(rgb(STROKE))
                .overflow_hidden()
                .flex()
                .flex_col()
                .child(header)
                .child(body)
                .child(footer),
        )
}
