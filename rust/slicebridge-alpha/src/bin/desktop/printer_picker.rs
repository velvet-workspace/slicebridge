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
        ("c5", "flashforge-creator-5", "Flashforge Creator 5"),
        ("cc2", "elegoo-centauri-carbon-2", "ELEGOO CC2"),
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

#[derive(Clone, Copy)]
enum InstalledKind {
    Orca,
    Prusa,
}

impl InstalledKind {
    fn accepts_id(self, id: &str) -> bool {
        match self {
            Self::Orca => is_target_id(id, "orca-"),
            Self::Prusa => is_target_id(id, "prusa-"),
        }
    }

    fn fallback_vendor(self) -> &'static str {
        match self {
            Self::Orca => "Orca",
            Self::Prusa => "Prusa",
        }
    }
}

// Vendor forks bundle other manufacturers' Orca presets, so each fork lists only
// under its owning maker. Same table as SliceBridgePicker.forkMaker; an unbranded
// slicer is not restricted. maker_key maps "X 5 Series" to QIDI.
fn fork_vendor_allowed(slicer: &str, vendor: &str) -> bool {
    let app: String = slicer
        .to_ascii_lowercase()
        .chars()
        .filter(|ch| !(ch.is_whitespace() || *ch == '-' || *ch == '_'))
        .collect();
    let owner = match app.as_str() {
        "snapmakerorca" => "snapmaker",
        "flashstudio" | "flashstudiodesktop" => "flashforge",
        "elegooslicer" => "elegoo",
        "qidistudio" => "qidi",
        _ => return true,
    };
    maker_key(vendor) == owner
}

// Installed printers from printers.installed_orca, then printers.installed_prusa.
// Same rules as SliceBridgePicker.dynamicItem, prusaDynamicItem and
// groupDestinations: only structurally valid, non-Bambu targets are listed, each
// folded into the card of its physical model. Installed rows are never
// conversion-ready here: both backends report `ready: false` and per-printer
// export is unverified, so no ready flag is kept.
pub(super) fn cards_with_installed_printers(
    catalogue: &[PrinterCard],
    orca_reply: &Value,
    prusa_reply: &Value,
) -> Vec<PrinterCard> {
    let mut cards = catalogue.to_vec();
    let mut by_key: BTreeMap<String, usize> = cards
        .iter()
        .enumerate()
        .map(|(index, card)| (physical_key(&card.manufacturer, &card.model), index))
        .collect();
    // Installed-only cards show their lowest-nozzle target, as defaultVariant does.
    let mut installed_nozzle: BTreeMap<usize, f64> = BTreeMap::new();
    let targets = [
        (InstalledKind::Orca, orca_reply),
        (InstalledKind::Prusa, prusa_reply),
    ]
    .into_iter()
    .flat_map(|(kind, reply)| {
        reply["targets"]
            .as_array()
            .into_iter()
            .flatten()
            .map(move |target| (kind, target))
    });
    for (kind, target) in targets {
        let Some(card) = installed_card(kind, target) else {
            continue;
        };
        if cards.iter().any(|known| known.ids.contains(&card.id)) {
            continue;
        }
        let nozzle = target["nozzle"]
            .as_str()
            .and_then(|text| text.trim().parse::<f64>().ok())
            .unwrap_or(f64::INFINITY);
        let key = physical_key(&card.manufacturer, &card.model);
        match by_key.get(&key).copied() {
            Some(index) => {
                cards[index].ids.push(card.id.clone());
                if let Some(best) = installed_nozzle.get_mut(&index) {
                    if nozzle < *best {
                        *best = nozzle;
                        cards[index].id = card.id.clone();
                    }
                }
            }
            None => {
                installed_nozzle.insert(cards.len(), nozzle);
                by_key.insert(key, cards.len());
                cards.push(card);
            }
        }
    }
    cards
}

fn installed_card(kind: InstalledKind, target: &Value) -> Option<PrinterCard> {
    let id = target["id"].as_str()?;
    if !kind.accepts_id(id) {
        return None;
    }
    let raw_vendor = clean_text(target["vendor"].as_str().unwrap_or(""));
    if matches!(kind, InstalledKind::Orca)
        && !fork_vendor_allowed(target["slicer"].as_str().unwrap_or(""), &raw_vendor)
    {
        return None;
    }
    let vendor = if raw_vendor.is_empty() {
        kind.fallback_vendor().to_owned()
    } else {
        raw_vendor
    };
    let title = clean_text(target["title"].as_str().unwrap_or(""));
    let title = if title.is_empty() {
        clean_text(target["model"].as_str().unwrap_or(""))
    } else {
        title
    };
    let name = strip_nozzle(&title);
    if name.is_empty() || is_bambu_name(&vendor) || is_bambu_name(&name) {
        return None;
    }
    let manufacturer = maker_label(&vendor);
    let model = displayed_model(&displayed_model(&name, &vendor), &manufacturer);
    Some(PrinterCard {
        id: id.into(),
        ids: vec![id.into()],
        manufacturer,
        model: if model.is_empty() { name } else { model },
        aliases: Vec::new(),
        release_date: None,
    })
}

// Mirrors ^<prefix>[a-z0-9]+(?:-[a-z0-9]+)*-[0-9a-f]{10}$, the shape both backends issue.
fn is_target_id(id: &str, prefix: &str) -> bool {
    let Some((slug, hash)) = id
        .strip_prefix(prefix)
        .and_then(|rest| rest.rsplit_once('-'))
    else {
        return false;
    };
    let hash_ok = hash.len() == 10
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    let slug_ok = slug.split('-').all(|part| {
        !part.is_empty()
            && part
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    });
    hash_ok && slug_ok
}

fn clean_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

// Same word rule as SliceBridgePicker.isBambuName: "bambu" as a whole word, or "bbl".
fn is_bambu_name(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .any(|word| word == "bambu")
        || lower == "bbl"
}

// Same as SliceBridgePicker.stripNozzle: removes a trailing nozzle size such as
// " 0.4 nozzle", " 0.4+0.6 mm" or " nozzle". A bare trailing number is kept.
fn strip_nozzle(name: &str) -> String {
    for (at, ch) in name.char_indices() {
        if ch.is_whitespace() && nozzle_tail(name[at..].trim_start()) {
            return name[..at].trim().to_owned();
        }
    }
    name.trim().to_owned()
}

fn nozzle_tail(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    let stem = match lower.strip_suffix("nozzle") {
        Some(rest) if rest.ends_with(char::is_whitespace) => &body[..rest.len()],
        _ => body,
    };
    stem.trim().eq_ignore_ascii_case("nozzle") || is_nozzle_diameter(stem)
}

fn is_nozzle_diameter(text: &str) -> bool {
    let lower = text.trim_end().to_ascii_lowercase();
    let core = lower
        .strip_suffix("mm")
        .unwrap_or(lower.as_str())
        .trim_end();
    !core.is_empty()
        && core.split('+').all(|part| {
            matches!(part.trim().split_once('.'), Some((whole, frac))
                if !whole.is_empty()
                    && !frac.is_empty()
                    && whole.bytes().all(|b| b.is_ascii_digit())
                    && frac.bytes().all(|b| b.is_ascii_digit()))
        })
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
        // Installed Orca and PrusaSlicer printers are re-read on every open, like printers.get.
        let installed_orca = self.commands.execute("printers.installed_orca", &json!({}));
        let installed_prusa = self
            .commands
            .execute("printers.installed_prusa", &json!({}));
        self.printers_cards = cards_with_installed_printers(
            &self.printers_catalogue_cards,
            &installed_orca["data"],
            &installed_prusa["data"],
        );
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
            // The checkbox glyph alone is not a usable accessibility name. Include the
            // exact catalogue model and its current selection state in both locales.
            let accessible_model = format!(
                "{} {} — {}",
                entry.manufacturer,
                entry.model,
                if is_selected {
                    if rtl { "נבחר" } else { "Selected" }
                } else if rtl {
                    "לא נבחר"
                } else {
                    "Not selected"
                }
            );
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
                        .accessibility_label(accessible_model)
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
                    .accessibility_label(copy(locale, "close"))
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
    use serde_json::{json, Value};
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
        // Newly ported built-ins are selectable ONLY when the exact OEM
        // machine/process/material chain is independently resolved.
        let all_ready = std::collections::BTreeSet::from([
            "u1".to_owned(),
            "c5".to_owned(),
            "cc2".to_owned(),
            "qidi".to_owned(),
        ]);
        assert_eq!(
            super::selected_ready_legacy_rows(
                &[
                    "snapmaker-u1".to_owned(),
                    "flashforge-creator-5".to_owned(),
                    "elegoo-centauri-carbon-2".to_owned(),
                    "qidi-x-plus-5".to_owned()
                ],
                &all_ready
            ),
            vec![
                ("u1", "Snapmaker U1"),
                ("c5", "Flashforge Creator 5"),
                ("cc2", "ELEGOO CC2"),
                ("qidi", "QIDI Plus5")
            ]
        );
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

    fn synthetic_catalogue() -> Vec<super::PrinterCard> {
        let card = |id: &str, maker: &str, model: &str| super::PrinterCard {
            id: id.into(),
            ids: vec![id.into()],
            manufacturer: maker.into(),
            model: model.into(),
            aliases: Vec::new(),
            release_date: None,
        };
        vec![
            card("prusa-mk4-catalogue", "Prusa Research", "MK4"),
            card("snapmaker-u1", "Snapmaker", "U1"),
        ]
    }

    // Shaped like the printers.installed_prusa reply: the backend always sends ready:false.
    fn installed_target(id: &str, title: &str, vendor: &str, nozzle: &str) -> Value {
        json!({
            "id": id, "title": title, "slicer": "PrusaSlicer", "vendor": vendor,
            "model": title, "nozzle": nozzle, "ready": false,
            "reason": "per-printer export not verified",
        })
    }

    #[test]
    fn missing_installed_prusa_leaves_catalogue_cards_unchanged() {
        let catalogue = synthetic_catalogue();
        let expected: Vec<Vec<String>> = catalogue.iter().map(|c| c.ids.clone()).collect();
        for reply in [json!({"targets": []}), Value::Null, json!({"rejected": []})] {
            let merged = super::cards_with_installed_printers(&catalogue, &Value::Null, &reply);
            let ids: Vec<Vec<String>> = merged.iter().map(|c| c.ids.clone()).collect();
            assert_eq!(ids, expected);
        }
    }

    #[test]
    fn installed_target_folds_into_its_catalogue_card_without_a_duplicate() {
        let catalogue = synthetic_catalogue();
        let reply = json!({"targets": [installed_target(
            "prusa-mk4-0123456789", "MK4 0.4", "Prusa Research", "0.4")]});
        let merged = super::cards_with_installed_printers(&catalogue, &Value::Null, &reply);
        assert_eq!(merged.len(), catalogue.len());
        let mk4 = merged.iter().find(|c| c.model == "MK4").unwrap();
        assert_eq!(mk4.id, "prusa-mk4-catalogue");
        assert_eq!(
            mk4.ids,
            vec![
                "prusa-mk4-catalogue".to_owned(),
                "prusa-mk4-0123456789".to_owned()
            ]
        );
        // Refreshing again with the same reply must not add the installed ID twice.
        let again = super::cards_with_installed_printers(&merged, &Value::Null, &reply);
        assert_eq!(again.len(), catalogue.len());
        assert_eq!(
            again.iter().find(|c| c.model == "MK4").unwrap().ids.len(),
            2
        );
    }

    #[test]
    fn installed_only_model_uses_lowest_nozzle_target_and_one_card() {
        let reply = json!({"targets": [
            installed_target("prusa-mk3s-abcdef0123", "MK3S 0.6 nozzle", "Prusa Research", "0.6"),
            installed_target("prusa-mk3s-0123456789", "MK3S 0.4 nozzle", "Prusa Research", "0.4"),
        ]});
        let merged = super::cards_with_installed_printers(&[], &Value::Null, &reply);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].model, "MK3S");
        assert_eq!(merged[0].manufacturer, "Prusa Research");
        assert_eq!(merged[0].id, "prusa-mk3s-0123456789");
        assert_eq!(merged[0].ids.len(), 2);
    }

    #[test]
    fn bambu_and_structurally_invalid_targets_are_never_listed() {
        let catalogue = synthetic_catalogue();
        let reply = json!({"targets": [
            installed_target("prusa-p1s-0123456789", "P1S 0.4", "Bambu Lab", "0.4"),
            installed_target("prusa-x1c-0123456789", "Bambu X1C", "Prusa Research", "0.4"),
            installed_target("prusa-project", "Project", "Prusa Research", "0.4"),
            installed_target("prusa-mk4-XYZ", "MK4", "Prusa Research", "0.4"),
            installed_target("orca-mk4-0123456789", "MK4", "Prusa Research", "0.4"),
            installed_target("prusa-mk4-0123456789", "  ", "Prusa Research", "0.4"),
        ]});
        let merged = super::cards_with_installed_printers(&catalogue, &Value::Null, &reply);
        assert_eq!(merged.len(), catalogue.len());
        assert!(merged
            .iter()
            .all(|card| !card.ids.iter().any(|id| id.starts_with("prusa-p1s")
                || id.starts_with("prusa-x1c")
                || id == "prusa-project")));
    }

    #[test]
    fn selected_installed_target_ranks_first_and_never_grants_readiness() {
        let catalogue = synthetic_catalogue();
        let installed = "prusa-mk3s-0123456789";
        let reply = json!({"targets": [installed_target(installed, "MK3S 0.4 nozzle", "Prusa Research", "0.4")]});
        let merged = super::cards_with_installed_printers(&catalogue, &Value::Null, &reply);
        let saved = vec![installed.to_owned()];
        let rows = cards_for_view(&merged, &saved, "", None, false);
        assert!(rows[0].ids.contains(&installed.to_owned()));
        // A saved installed Prusa ID is inventory only: no built-in conversion row appears.
        let all_ready = std::collections::BTreeSet::from([
            "u1".to_owned(),
            "c5".to_owned(),
            "cc2".to_owned(),
            "qidi".to_owned(),
        ]);
        assert!(super::selected_ready_legacy_rows(&saved, &all_ready).is_empty());
    }

    // Shaped like a printers.installed_orca target: the backend always sends ready:false.
    fn installed_orca_target(
        id: &str,
        title: &str,
        vendor: &str,
        slicer: &str,
        nozzle: &str,
    ) -> Value {
        json!({
            "id": id, "title": title, "slicer": slicer, "vendor": vendor,
            "model": title, "nozzle": nozzle, "ready": false,
            "reason": "dynamic Orca conversion is not verified in Rust",
        })
    }

    #[test]
    fn orca_ids_need_the_exact_shape_and_bambu_is_never_listed() {
        let reply = json!({"targets": [
            installed_orca_target("orca-mk4-0123456789", "MK4 0.4 nozzle", "Prusa Research", "OrcaSlicer", "0.4"),
            installed_orca_target("orca-slicer", "OrcaSlicer", "Prusa Research", "OrcaSlicer", "0.4"),
            installed_orca_target("orca-mk4-XYZ", "MK4", "Prusa Research", "OrcaSlicer", "0.4"),
            installed_orca_target("orca--mk4-0123456789", "MK4", "Prusa Research", "OrcaSlicer", "0.4"),
            installed_orca_target("orca-MK4-0123456789", "MK4", "Prusa Research", "OrcaSlicer", "0.4"),
            installed_orca_target("orca-mk4-012345678A", "MK4", "Prusa Research", "OrcaSlicer", "0.4"),
            installed_orca_target("prusa-mk4-0123456789", "MK4", "Prusa Research", "OrcaSlicer", "0.4"),
            installed_orca_target("orca-p1s-0123456789", "P1S 0.4", "Bambu Lab", "BambuStudio", "0.4"),
            installed_orca_target("orca-x1c-0123456789", "Bambu X1C", "Prusa Research", "OrcaSlicer", "0.4"),
            installed_orca_target("orca-a1-0123456789", "A1 0.4", "BBL", "OrcaSlicer", "0.4"),
        ]});
        let merged = super::cards_with_installed_printers(&[], &reply, &Value::Null);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].ids, vec!["orca-mk4-0123456789".to_owned()]);
        assert_eq!(merged[0].manufacturer, "Prusa Research");
        assert_eq!(merged[0].model, "MK4");
    }

    #[test]
    fn forked_orca_slicers_list_only_under_their_owning_maker() {
        let reply = json!({"targets": [
            installed_orca_target("orca-u1-0123456789", "Snapmaker U1 0.4 nozzle", "Creality", "SnapmakerOrca", "0.4"),
            installed_orca_target("orca-u1-1123456789", "Snapmaker U1 0.4 nozzle", "Snapmaker", "SnapmakerOrca", "0.4"),
            installed_orca_target("orca-c5-0123456789", "Creator 5 0.4 nozzle", "Elegoo", "Flash Studio Desktop", "0.4"),
            installed_orca_target("orca-c5-1123456789", "Creator 5 0.4 nozzle", "Flashforge", "Flash Studio", "0.4"),
            installed_orca_target("orca-cc2-0123456789", "Centauri Carbon 2 0.4 nozzle", "Snapmaker", "ElegooSlicer", "0.4"),
            installed_orca_target("orca-cc2-1123456789", "Centauri Carbon 2 0.4 nozzle", "Elegoo", "ElegooSlicer", "0.4"),
            installed_orca_target("orca-plus5-0123456789", "Plus5 0.4 nozzle", "Elegoo", "QIDIStudio", "0.4"),
            installed_orca_target("orca-plus5-1123456789", "QIDI Plus5 0.4 nozzle", "X 5 Series", "QIDIStudio", "0.4"),
            installed_orca_target("orca-acme-0123456789", "Acme Mini 0.4 nozzle", "Acme", "OrcaSlicer", "0.4"),
        ]});
        let merged = super::cards_with_installed_printers(&[], &reply, &Value::Null);
        let ids: Vec<String> = merged.iter().flat_map(|c| c.ids.clone()).collect();
        assert_eq!(
            ids,
            vec![
                "orca-u1-1123456789",
                "orca-c5-1123456789",
                "orca-cc2-1123456789",
                "orca-plus5-1123456789",
                "orca-acme-0123456789",
            ]
        );
        assert_eq!(merged[0].manufacturer, "Snapmaker");
        assert_eq!(merged[1].manufacturer, "Flashforge");
        assert_eq!(merged[2].manufacturer, "ELEGOO");
        assert_eq!(merged[3].manufacturer, "QIDI");
        assert_eq!(merged[3].model, "Plus5");
    }

    #[test]
    fn orca_and_prusa_fold_into_one_physical_card_without_duplicate_ids() {
        let catalogue = synthetic_catalogue();
        let orca = json!({"targets": [
            installed_orca_target("orca-mk4-0123456789", "MK4 0.6 nozzle", "Prusa Research", "OrcaSlicer", "0.6"),
            installed_orca_target("orca-mk4-abcdef0123", "MK4 0.4 nozzle", "Prusa Research", "OrcaSlicer", "0.4"),
            installed_orca_target("orca-mk4-0123456789", "MK4 0.6 nozzle", "Prusa Research", "OrcaSlicer", "0.6"),
            installed_orca_target("orca-u1-0123456789", "U1 0.4 nozzle", "Snapmaker", "SnapmakerOrca", "0.4"),
        ]});
        let prusa = json!({"targets": [
            installed_target("prusa-mk4-fedcba9876", "MK4 0.4 nozzle", "Prusa Research", "0.4"),
        ]});
        let merged = super::cards_with_installed_printers(&catalogue, &orca, &prusa);
        assert_eq!(merged.len(), catalogue.len());
        let mk4 = merged.iter().find(|c| c.model == "MK4").unwrap();
        assert_eq!(mk4.id, "prusa-mk4-catalogue");
        assert_eq!(
            mk4.ids,
            vec![
                "prusa-mk4-catalogue".to_owned(),
                "orca-mk4-0123456789".to_owned(),
                "orca-mk4-abcdef0123".to_owned(),
                "prusa-mk4-fedcba9876".to_owned(),
            ]
        );
        let u1 = merged.iter().find(|c| c.model == "U1").unwrap();
        assert_eq!(
            u1.ids,
            vec!["snapmaker-u1".to_owned(), "orca-u1-0123456789".to_owned()]
        );
    }

    #[test]
    fn orca_multiple_nozzles_form_one_card_led_by_lowest_nozzle() {
        let reply = json!({"targets": [
            installed_orca_target("orca-k1-0123456789", "K1 0.6 nozzle", "Acme", "OrcaSlicer", "0.6"),
            installed_orca_target("orca-k1-abcdef0123", "K1 0.4 nozzle", "Acme", "OrcaSlicer", "0.4"),
            installed_orca_target("orca-k1-fedcba9876", "K1 0.8 nozzle", "Acme", "OrcaSlicer", "0.8"),
        ]});
        let merged = super::cards_with_installed_printers(&[], &reply, &Value::Null);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].id, "orca-k1-abcdef0123");
        assert_eq!(merged[0].ids.len(), 3);
        // Refreshing the same reply keeps one row per installed ID.
        let again = super::cards_with_installed_printers(&merged, &reply, &Value::Null);
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].ids.len(), 3);
    }

    #[test]
    fn selected_orca_target_ranks_by_selection_and_grants_no_conversion() {
        let catalogue = synthetic_catalogue();
        let orca = json!({"targets": [installed_orca_target(
            "orca-k1-0123456789", "K1 0.4 nozzle", "Acme", "OrcaSlicer", "0.4")]});
        let merged = super::cards_with_installed_printers(&catalogue, &orca, &Value::Null);
        let saved = vec!["orca-k1-0123456789".to_owned(), "snapmaker-u1".to_owned()];
        let rows = cards_for_view(&merged, &saved, "", None, false);
        assert_eq!(rows[0].ids, vec!["orca-k1-0123456789".to_owned()]);
        assert_eq!(rows[1].id, "snapmaker-u1");
        // Saved installed Orca IDs are inventory only: only the built-in U1 converts.
        let all_ready = std::collections::BTreeSet::from(["u1".to_owned()]);
        assert_eq!(
            super::selected_ready_legacy_rows(&saved, &all_ready),
            vec![("u1", "Snapmaker U1")]
        );
    }

    #[test]
    fn installed_orca_card_never_exposes_executable_or_profile_paths() {
        let reply = json!({"targets": [{
            "id": "orca-k1-0123456789", "title": "K1 0.4 nozzle", "slicer": "OrcaSlicer",
            "vendor": "Acme", "machine": "/Applications/OrcaSlicer.app/Contents/x.json",
            "model": "K1", "nozzle": "0.4", "ready": false,
            "root": "/Users/example/profiles", "executable": "/Applications/OrcaSlicer.app",
        }]});
        let merged = super::cards_with_installed_printers(&[], &reply, &Value::Null);
        assert_eq!(merged.len(), 1);
        assert!(!format!("{merged:?}").contains("/Applications"));
        assert!(!format!("{merged:?}").contains("/Users"));
    }
}
