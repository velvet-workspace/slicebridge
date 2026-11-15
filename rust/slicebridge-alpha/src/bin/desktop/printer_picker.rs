//! Stable native/frontend/dist/my-printers.js structure in GPUI.
//! Source-locked catalogue membership, never a substitute for OEM readiness.
//! Detailed per-printer nozzle/slicer preferences are a separate pending parity gap.
use super::*;
use gpui_kit::base::Disableable;
use gpui_kit::component::input::Input;
use gpui_kit::component::Sizable;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const MAX_PICKS: usize = 32;

// A saved catalogue item is inventory, not permission to convert. Expose only
// original built-in targets that currently resolve in this Rust engine.
pub(super) fn selected_ready_legacy_rows(
    saved: &[String],
    ready: &BTreeSet<String>,
) -> Vec<(&'static str, &'static str)> {
    [
        ("u1", "snapmaker-u1", "Snapmaker U1"),
        ("qidi", "qidi-x-plus-5", "QIDI Plus5"),
    ]
    .into_iter()
    .filter(|(target, catalogue, _)| {
        ready.contains(*target) && saved.iter().any(|id| id == catalogue)
    })
    .map(|(target, _, name)| (target, name))
    .collect()
}

fn line(rtl: bool) -> Div {
    if rtl {
        div().flex().flex_row_reverse()
    } else {
        div().flex().flex_row()
    }
}
const CURATED: &[&str] = &[
    "anker",
    "anycubic",
    "creality",
    "elegoo",
    "flashforge",
    "flsun",
    "prusa research",
    "qidi",
    "snapmaker",
    "sovol",
    "voron",
];

#[derive(Clone, Debug)]
pub(super) struct PrinterCard {
    pub id: String,
    pub ids: Vec<String>,
    pub manufacturer: String,
    pub model: String,
    pub aliases: Vec<String>,
    pub release_date: Option<String>,
}

fn maker_key(maker: &str) -> String {
    match maker.trim().to_ascii_lowercase().as_str() {
        "prusa" => "prusa research".into(),
        "x 5 series" => "qidi".into(),
        other => other.into(),
    }
}

fn maker_label(maker: &str) -> String {
    match maker_key(maker).as_str() {
        "qidi" => "QIDI".into(),
        "elegoo" => "ELEGOO".into(),
        "flsun" => "FLSun".into(),
        "prusa research" => "Prusa Research".into(),
        _ => maker.trim().to_string(),
    }
}

fn displayed_model(raw: &str, maker: &str) -> String {
    // Same source rule as SliceBridgePicker.withoutMakerPrefix.
    let name = raw.trim();
    let prefix = maker.to_ascii_lowercase() + " ";
    if name.to_ascii_lowercase().starts_with(&prefix) {
        return name[prefix.len()..].trim().to_owned();
    }
    if maker.eq_ignore_ascii_case("Anker") {
        for alias in ["ankermake ", "anker "] {
            if name.to_ascii_lowercase().starts_with(alias) {
                return name[alias.len()..].trim().to_owned();
            }
        }
    }
    name.to_string()
}

fn physical_key(maker: &str, model: &str) -> String {
    let canonical: String = model
        .to_ascii_lowercase()
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace() && *ch != '-' && *ch != '_')
        .collect();
    format!("{}|{canonical}", maker_key(maker))
}

pub(super) fn original_catalogue_cards(catalog: &Value) -> Vec<PrinterCard> {
    let mut grouped: BTreeMap<String, PrinterCard> = BTreeMap::new();
    for model in catalog["models"].as_array().into_iter().flatten() {
        let Some(id) = model["id"].as_str() else {
            continue;
        };
        let Some(maker) = model["manufacturer"].as_str() else {
            continue;
        };
        if maker.eq_ignore_ascii_case("Bambu Lab") {
            continue;
        }
        let Some(title) = model["model"].as_str() else {
            continue;
        };
        let manufacturer = maker_label(maker);
        let displayed = displayed_model(title, &manufacturer);
        let key = physical_key(&manufacturer, &displayed);
        let aliases = model["aliases"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let release_date = model["releaseDate"].as_str().map(str::to_owned);
        let entry = grouped.entry(key).or_insert_with(|| PrinterCard {
            id: id.into(),
            ids: Vec::new(),
            manufacturer,
            model: displayed,
            aliases: Vec::new(),
            release_date: release_date.clone(),
        });
        entry.ids.push(id.into());
        for alias in aliases {
            if !entry.aliases.contains(&alias) {
                entry.aliases.push(alias);
            }
        }
        if release_date.as_ref() > entry.release_date.as_ref() {
            entry.release_date = release_date;
        }
    }
    // Exactly the original synthetic generic PrusaSlicer entry. It may be
    // selected as inventory; no conversion is advertised without a probe.
    grouped.insert(
        "generic|prusaslicerproject".into(),
        PrinterCard {
            id: "prusa-project".into(),
            ids: vec!["prusa-project".into()],
            manufacturer: "Generic".into(),
            model: "PrusaSlicer project".into(),
            aliases: vec!["Generic project".into()],
            release_date: None,
        },
    );
    grouped.into_values().collect()
}

fn selected(card: &PrinterCard, ids: &[String]) -> bool {
    card.ids.iter().any(|id| ids.contains(id))
}

pub(super) fn cards_for_view(
    cards: &[PrinterCard],
    draft: &[String],
    query: &str,
    maker: Option<&str>,
    show_all: bool,
) -> Vec<PrinterCard> {
    let query = query.trim().to_lowercase();
    let maker = maker.map(maker_key);
    let mut visible: Vec<PrinterCard> = cards
        .iter()
        .filter(|item| {
            (show_all
                || CURATED.contains(&maker_key(&item.manufacturer).as_str())
                || selected(item, draft))
                && maker
                    .as_ref()
                    .is_none_or(|name| *name == maker_key(&item.manufacturer))
                && (query.is_empty()
                    || format!(
                        "{} {} {}",
                        item.manufacturer,
                        item.model,
                        item.aliases.join(" ")
                    )
                    .to_lowercase()
                    .contains(&query))
        })
        .cloned()
        .collect();
    visible.sort_by(|a, b| {
        let rank = |item: &PrinterCard| {
            draft
                .iter()
                .position(|id| item.ids.contains(id))
                .unwrap_or(usize::MAX)
        };
        rank(a)
            .cmp(&rank(b))
            .then_with(|| {
                a.manufacturer
                    .to_lowercase()
                    .cmp(&b.manufacturer.to_lowercase())
            })
            .then_with(|| b.release_date.cmp(&a.release_date))
            .then_with(|| a.model.to_lowercase().cmp(&b.model.to_lowercase()))
    });
    visible
}

pub(super) fn makers_for_view(
    cards: &[PrinterCard],
    draft: &[String],
    show_all: bool,
) -> Vec<String> {
    let mut makers = BTreeSet::new();
    for item in cards {
        if show_all
            || CURATED.contains(&maker_key(&item.manufacturer).as_str())
            || selected(item, draft)
        {
            makers.insert(item.manufacturer.clone());
        }
    }
    makers.into_iter().collect()
}

fn copy(locale: Locale, key: &str) -> &'static str {
    let he = locale.rtl();
    match (he,key) {
        (true,"title") => "המדפסות שלי", (false,"title") => "My printers",
        (true,"welcome") => "בחירת מדפסות", (false,"welcome") => "Choose printers",
        (true,"body") => "בחר מדפסות לעבודה ולמעקב. הגדרות דיזה, סלייסר ופילמנט קיימות אינן משתנות.",
        (false,"body") => "Add or remove printers without changing your existing nozzle, slicer or filament preferences.",
        (true,"count") => "נבחרו", (false,"count") => "printers selected",
        (true,"none") => "עדיין לא נבחרו מדפסות.", (false,"none") => "No printers selected yet.",
        (true,"manufacturer") => "יצרן", (false,"manufacturer") => "Manufacturer",
        (true,"chooseMaker") => "בחר יצרן…", (false,"chooseMaker") => "Choose manufacturer…",
        (true,"search") => "או חפש דגם", (false,"search") => "Or search a model",
        (true,"searchHint") => "שם יצרן או דגם", (false,"searchHint") => "Manufacturer or model",
        (true,"all") => "הצג יצרנים נוספים", (false,"all") => "Show more manufacturers",
        (true,"curated") => "הצג רק יצרנים נבחרים", (false,"curated") => "Show selected manufacturers only",
        (true,"save") => "שמור מדפסות", (false,"save") => "Save printers",
        (true,"cancel") => "לא עכשיו", (false,"cancel") => "Not now",
        (true,"limit") => "אפשר לבחור עד 32 מדפסות.", (false,"limit") => "You can select up to 32 printers.",
        (true,"noResult") => "לא נמצאו דגמים", (false,"noResult") => "No models matched",
        (true,"close") => "סגירה", (false,"close") => "Close",
        (true,"ready") => "מדפסת שמורה", (false,"ready") => "Saved printer",
        _ => "",
    }
}

impl SliceBridgeWindow {
    pub(super) fn open_printers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let response = self.commands.execute("printers.get", &json!({}));
        if response["ok"] != true {
            self.last_message = response["error"]
                .as_str()
                .unwrap_or("My Printers unavailable")
                .into();
            cx.notify();
            return;
        }
        self.printers_saved = response["data"]["selectedIds"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        self.printers_draft = self.printers_saved.clone();
        self.printers_error.clear();
        self.printers_maker = None;
        self.printers_show_all = false;
        self.printers_maker_menu = false;
        self.printers_first_run = response["data"]["onboardingPending"] == true;
        self.printers_search
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.printers_open = true;
        cx.notify();
    }

    pub(super) fn cancel_printers(&mut self, cx: &mut Context<Self>) {
        self.printers_open = false;
        self.printers_draft.clear();
        self.printers_error.clear();
        cx.notify();
    }

    pub(super) fn toggle_printer(
        &mut self,
        ids: &[String],
        preferred: &str,
        cx: &mut Context<Self>,
    ) {
        let was_selected = ids.iter().any(|id| self.printers_draft.contains(id));
        if was_selected {
            self.printers_draft.retain(|entry| !ids.contains(entry));
        } else {
            if self.printers_draft.len() >= MAX_PICKS {
                self.printers_error = copy(self.locale, "limit").into();
                cx.notify();
                return;
            }
            let previous = ids.iter().find(|id| self.printers_saved.contains(*id));
            self.printers_draft
                .push(previous.map(|id| id.as_str()).unwrap_or(preferred).into());
        }
        self.printers_error.clear();
        cx.notify();
    }

    pub(super) fn save_printers(&mut self, cx: &mut Context<Self>) {
        let response = self
            .commands
            .execute("printers.save", &json!({"ids":self.printers_draft}));
        if response["ok"] != true {
            self.printers_error = response["error"].as_str().unwrap_or("Save failed").into();
        } else {
            let previously_visible: BTreeSet<String> =
                selected_ready_legacy_rows(&self.printers_saved, &self.ready_oem_targets)
                    .into_iter()
                    .map(|(id, _)| id.to_owned())
                    .collect();
            self.printers_saved = response["data"]["selectedIds"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect();
            let now_visible =
                selected_ready_legacy_rows(&self.printers_saved, &self.ready_oem_targets);
            // Go checks newly displayed targets, but preserves the state of
            // checkboxes which stayed visible.
            self.selected_batch_targets = now_visible
                .into_iter()
                .map(|(id, _)| id.to_owned())
                .filter(|id| {
                    !previously_visible.contains(id) || self.selected_batch_targets.contains(id)
                })
                .collect();
            self.printers_open = false;
            self.printers_error.clear();
        }
        cx.notify();
    }

    pub(super) fn render_printers(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let locale = self.locale;
        let rtl = locale.rtl();
        let saved = self.printers_draft.clone();
        let maker = self.printers_maker.clone();
        let search = self.printers_search.read(cx).value().to_string();
        let entries = cards_for_view(
            &self.printers_cards,
            &saved,
            &search,
            maker.as_deref(),
            self.printers_show_all,
        );
        let manufacturers = makers_for_view(&self.printers_cards, &saved, self.printers_show_all);
        let showing = entries.len();
        let first_run = self.printers_first_run;
        let mut chips = line(rtl).flex().flex_wrap().gap_2();
        let mut groups_seen = BTreeSet::new();
        for item in &self.printers_cards {
            if selected(item, &saved) && groups_seen.insert(item.id.clone()) {
                chips = chips.child(
                    div()
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .bg(rgb(0xDFF6EE))
                        .text_xs()
                        .text_color(rgb(0x193F38))
                        .child(format!("{} {}", item.manufacturer, item.model)),
                );
            }
        }
        if saved.is_empty() {
            chips = chips.child(
                div()
                    .text_xs()
                    .text_color(rgb(0x526B61))
                    .child(copy(locale, "none")),
            );
        }

        let mut dropdown = div().flex().flex_col().gap_1();
        if self.printers_maker_menu {
            dropdown = dropdown.child(
                Button::new("maker-filter-any")
                    .label(copy(locale, "chooseMaker"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.printers_maker = None;
                        this.printers_maker_menu = false;
                        cx.notify();
                    })),
            );
            for name in manufacturers.iter() {
                let selection = name.clone();
                dropdown = dropdown.child(
                    Button::new(format!("maker-filter-{}", name))
                        .label(name.clone())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.printers_maker = Some(selection.clone());
                            this.printers_maker_menu = false;
                            cx.notify();
                        })),
                );
            }
        }

        let maker_label = maker.unwrap_or_else(|| copy(locale, "chooseMaker").into());
        let maker_control = div()
            .flex()
            .flex_col()
            .gap_2()
            .flex_1()
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(0x526B61))
                    .child(copy(locale, "manufacturer")),
            )
            .child(
                Button::new("manufacturer-filter")
                    .label(maker_label)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.printers_maker_menu = !this.printers_maker_menu;
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("printers-manufacturer-dropdown")
                    .max_h(px(170.))
                    .overflow_y_scroll()
                    .child(dropdown),
            );

        let search_control = div()
            .flex()
            .flex_col()
            .gap_2()
            .flex_1()
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(0x526B61))
                    .child(copy(locale, "search")),
            )
            .child(
                Input::new(&self.printers_search)
                    .id("printerCatalogSearch")
                    .aria_label(copy(locale, "searchHint"))
                    .small()
                    .w_full(),
            );

        let mut catalog_list = div()
            .id("catalogModelList")
            .flex_1()
            .min_h(px(100.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_2()
            .px_4()
            .py_2();
        if entries.is_empty() {
            catalog_list = catalog_list.child(
                div()
                    .p_5()
                    .text_sm()
                    .text_color(rgb(0x526B61))
                    .child(copy(locale, "noResult")),
            );
        }
        for (idx, entry) in entries.iter().enumerate() {
            let is_selected = selected(entry, &saved);
            let ids = entry.ids.clone();
            let preferred = entry.id.clone();
            let card = line(rtl)
                .gap_3()
                .items_center()
                .p_2()
                .rounded_md()
                .border_1()
                .border_color(rgb(if is_selected { 0x9BCBB5 } else { 0xD9E8E0 }))
                .bg(rgb(if is_selected { 0xF2FAF5 } else { 0xFFFFFF }))
                .child(
                    Button::new(format!("my-printer-pick-{idx}"))
                        .disabled(!is_selected && saved.len() >= MAX_PICKS)
                        .label(if is_selected { "☑" } else { "☐" })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_printer(&ids, &preferred, cx)
                        })),
                )
                .child(
                    div()
                        .size(px(38.))
                        .rounded_md()
                        .bg(rgb(0xEDF7F1))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_sm()
                        .text_color(rgb(0x2F7F73))
                        .child(
                            entry
                                .manufacturer
                                .chars()
                                .take(2)
                                .collect::<String>()
                                .to_uppercase(),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .flex_1()
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(0x193F38))
                                .child(entry.model.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(0x526B61))
                                .child(entry.manufacturer.clone()),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(0x526B61))
                        .child(if is_selected { "✓" } else { "" }),
                );
            catalog_list = catalog_list.child(card);
        }

        let header = line(rtl)
            .items_center()
            .justify_between()
            .px_5()
            .py_3()
            .bg(rgb(0xFFFFFF))
            .border_b_1()
            .border_color(rgb(0xD9E8E0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_lg()
                            .text_color(rgb(0x193F38))
                            .child(copy(locale, if first_run { "welcome" } else { "title" })),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x526B61))
                            .child(copy(locale, "body")),
                    ),
            )
            .child(
                Button::new("close-myprinters-dialog")
                    .label("×")
                    .on_click(cx.listener(|this, _, _, cx| this.cancel_printers(cx))),
            );

        let content = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .child(
                div()
                    .px_5()
                    .py_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().text_sm().text_color(rgb(0x193F38)).child(if rtl {
                        format!("{} {} מדפסות", copy(locale, "count"), groups_seen.len())
                    } else {
                        format!("{} {}", groups_seen.len(), copy(locale, "count"))
                    }))
                    .child(chips)
                    .child(
                        line(rtl)
                            .gap_4()
                            .items_start()
                            .child(maker_control)
                            .child(search_control),
                    )
                    .child(
                        Button::new("show-other-manufacturers")
                            .label(copy(
                                locale,
                                if self.printers_show_all {
                                    "curated"
                                } else {
                                    "all"
                                },
                            ))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.printers_show_all = !this.printers_show_all;
                                this.printers_maker = None;
                                this.printers_maker_menu = false;
                                cx.notify();
                            })),
                    )
                    .child(div().text_xs().text_color(rgb(0x526B61)).child(if rtl {
                        format!("מציג {showing} דגמים")
                    } else {
                        format!("Showing {showing} models")
                    })),
            )
            .child(catalog_list)
            .child(
                div()
                    .px_5()
                    .text_xs()
                    .text_color(rgb(0xB34940))
                    .child(self.printers_error.clone()),
            );

        let footer = line(rtl)
            .items_center()
            .justify_end()
            .gap_2()
            .p_4()
            .border_t_1()
            .border_color(rgb(0xD9E8E0))
            .child(
                Button::new("cancel-myprinters")
                    .label(copy(locale, "cancel"))
                    .on_click(cx.listener(|this, _, _, cx| this.cancel_printers(cx))),
            )
            .child(
                Button::new("save-myprinters")
                    .primary()
                    .label(copy(locale, "save"))
                    .on_click(cx.listener(|this, _, _, cx| this.save_printers(cx))),
            );

        div()
            .id("myPrintersDialog")
            .absolute()
            .inset_0()
            .bg(rgba(0x193F387F))
            .flex()
            .items_center()
            .justify_center()
            .occlude()
            .child(
                div()
                    .id("myPrintersFrame")
                    .w(px(980.))
                    .max_w(relative(0.95))
                    .h(relative(0.88))
                    .bg(rgb(0xFFFFFF))
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(0xD9E8E0))
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .child(header)
                    .child(content)
                    .child(footer),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{cards_for_view, maker_key, original_catalogue_cards};
    use serde_json::Value;
    fn catalogue() -> Value {
        slicebridge_alpha::my_printers::catalog_snapshot().unwrap()
    }
    #[test]
    fn only_saved_and_installed_printers_appear_as_conversion_destinations() {
        let ready = std::collections::BTreeSet::from(["u1".to_owned(), "qidi".to_owned()]);
        assert!(super::selected_ready_legacy_rows(&[], &ready).is_empty());
        assert_eq!(
            super::selected_ready_legacy_rows(&["snapmaker-u1".to_owned()], &ready),
            vec![("u1", "Snapmaker U1")]
        );
        assert!(super::selected_ready_legacy_rows(
            &[
                "flashforge-creator-5".to_owned(),
                "elegoo-centauri-carbon-2".to_owned()
            ],
            &ready
        )
        .is_empty());
        assert!(super::selected_ready_legacy_rows(
            &["snapmaker-u1".to_owned()],
            &std::collections::BTreeSet::new()
        )
        .is_empty());
    }

    #[test]
    fn stable_hardware_catalogue_has_no_bambu_and_keeps_original_ids() {
        let entries = original_catalogue_cards(&catalogue());
        assert!(entries.len() >= 540);
        assert!(entries
            .iter()
            .any(|item| item.ids.contains(&"snapmaker-u1".to_owned())));
        assert!(entries
            .iter()
            .any(|item| item.ids.contains(&"flashforge-creator-5".to_owned())));
        assert!(!entries
            .iter()
            .any(|item| item.manufacturer.eq_ignore_ascii_case("Bambu Lab")));
        assert!(entries.iter().any(|item| item.id == "prusa-project"));
    }
    #[test]
    fn saved_choice_remains_visible_outside_curated_manufacturers() {
        let entries = original_catalogue_cards(&catalogue());
        let outsider = entries
            .iter()
            .find(|item| maker_key(&item.manufacturer) == "afinia")
            .unwrap();
        let saved = vec![outsider.id.clone()];
        assert!(cards_for_view(&entries, &saved, "", None, false)
            .iter()
            .any(|entry| entry.ids.contains(&outsider.id)));
        assert!(!cards_for_view(&entries, &[], "", None, false)
            .iter()
            .any(|entry| entry.ids.contains(&outsider.id)));
    }
    #[test]
    fn filtered_models_preserve_selected_before_alphabetical() {
        let entries = original_catalogue_cards(&catalogue());
        let saved = vec!["snapmaker-u1".to_owned()];
        let rows = cards_for_view(&entries, &saved, "", None, true);
        assert_eq!(rows[0].id, "snapmaker-u1");
        let snap = cards_for_view(&entries, &saved, "snapmaker u1", None, true);
        assert!(!snap.is_empty());
        assert!(snap.iter().any(|item| item.id == "snapmaker-u1"));
        assert!(cards_for_view(&entries, &saved, "xxyyzz-not-existing", None, true).is_empty());
    }
}
