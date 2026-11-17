//! GPUI port of original native/frontend/dist/index.html #settingsDialog and
//! app.js openSettings/paintSettings/saveSettings. It owns draft state; no
//! WebView, and it never writes installed stable Wails preferences.
use super::*;
use gpui_kit::base::Disableable;
use serde_json::{json, Value};

const TARGETS: &[(&str, &str)] = &[
    ("u1", "Snapmaker U1"),
    ("c5", "Flashforge Creator 5"),
    ("cc2", "ELEGOO CC2"),
    ("qidi", "QIDI Plus5"),
];
const MATERIALS: &[&str] = &["PLA", "PETG", "ABS", "ASA", "TPU"];

// The Wails baseline exposes physical nozzle choices even when no slicer or
// OEM profile is installed. Do not derive this hardware catalog from the
// fallible settings.options discovery response.
fn baseline_nozzles(target: &str) -> &'static [&'static str] {
    match target {
        "u1" | "cc2" | "qidi" => &["0.2", "0.4", "0.6", "0.8"],
        "c5" => &["0.25", "0.4", "0.6", "0.8"],
        _ => &[],
    }
}

fn line(rtl: bool) -> Div {
    if rtl {
        div().flex().flex_row_reverse()
    } else {
        div().flex().flex_row()
    }
}

fn choice_button(
    id: String,
    label: String,
    selected: bool,
    enabled: bool,
    handler: impl Fn(&mut SliceBridgeWindow, &ClickEvent, &mut Window, &mut Context<SliceBridgeWindow>)
        + 'static,
    cx: &mut Context<SliceBridgeWindow>,
) -> Button {
    let action = Button::new(id)
        .label(label)
        .disabled(!enabled)
        .on_click(cx.listener(handler));
    if selected {
        action.primary()
    } else {
        action
    }
}

const OEM_READINESS_TARGETS: &[&str] = &["u1", "c5", "cc2", "qidi"];

// Hardware readiness is checked against the selected PHYSICAL nozzle, not a
// hardcoded 0.4mm variant. Every built-in OEM target participates. Auto with
// no saved material overrides can use the current discovery snapshot; an
// explicit slicer or saved material always gets an exact fresh resolution
// against that selected slicer's installed profile tree.
fn ready_from_preferences_with<F>(
    discovery: &Value,
    settings: &Value,
    source_nozzle: Option<&str>,
    mut resolve: F,
) -> std::collections::BTreeSet<String>
where
    F: FnMut(&str, &str, &str, &std::collections::BTreeMap<String, String>) -> bool,
{
    let mut ready = std::collections::BTreeSet::new();
    for target in OEM_READINESS_TARGETS {
        let selection = if let Some(source) = source_nozzle {
            slicebridge_alpha::printer_settings::profile_choice_for_source(
                settings, target, "auto", source,
            )
        } else {
            slicebridge_alpha::printer_settings::profile_choice(settings, target, "auto")
        };
        let Ok((nozzle, overrides)) = selection else {
            continue;
        };
        let slicer = settings["printers"][*target]["slicer"]
            .as_str()
            .unwrap_or("auto");
        let listed = discovery["profiles"].as_array().is_some_and(|rows| {
            rows.iter().any(|item| {
                item["target"] == *target && item["nozzle"] == nozzle && item["ready"] == true
            })
        });
        let installed = if slicer == "auto" && overrides.is_empty() {
            listed
        } else {
            resolve(target, &nozzle, slicer, &overrides)
        };
        if installed {
            ready.insert((*target).to_owned());
        }
    }
    ready
}

pub(super) fn ready_from_preferences(
    discovery: &Value,
    settings: &Value,
    source_nozzle: Option<&str>,
) -> std::collections::BTreeSet<String> {
    ready_from_preferences_with(
        discovery,
        settings,
        source_nozzle,
        |target, nozzle, slicer, overrides| {
            slicebridge_alpha::oem::resolve_configured(target, nozzle, slicer, overrides).is_ok()
        },
    )
}

// Mirrors the source-locked Go guard in saveSettings: Auto nozzle cannot carry
// a saved filament preset. Only used to show the localized reason before the
// request; printer_settings::validate_for_save remains the authoritative check.
fn auto_nozzle_has_filament_override(draft: &Value) -> bool {
    let auto = match draft["nozzle"].as_str() {
        None => true,
        Some(nozzle) => nozzle.is_empty() || nozzle == "auto",
    };
    auto && draft["filaments"].as_object().is_some_and(|map| {
        map.values()
            .any(|v| v.as_str().is_some_and(|s| !s.is_empty()))
    })
}

impl SliceBridgeWindow {
    pub(super) fn open_settings(&mut self, cx: &mut Context<Self>) {
        let reply = self.commands.execute("settings.get", &json!({}));
        if reply["ok"] != true {
            self.last_message = format!(
                "{}: {}",
                self.locale.text("settingsError"),
                reply["error"].as_str().unwrap_or("unknown")
            );
            cx.notify();
            return;
        }
        self.settings_loaded = reply["data"].clone();
        self.settings_target = "u1".into();
        self.settings_material = "PLA".into();
        self.settings_draft = self.settings_loaded["printers"]["u1"].clone();
        self.settings_error.clear();
        self.refresh_settings_options();
        self.settings_open = true;
        self.printers_open = false;
        cx.notify();
    }

    fn refresh_settings_options(&mut self) {
        let reply = self.commands.execute(
            "settings.options",
            &json!({
                "target":self.settings_target,
                "nozzle":self.settings_draft["nozzle"],
                "slicer":self.settings_draft["slicer"],
                "material":self.settings_material,
            }),
        );
        if reply["ok"] == true {
            self.settings_options = reply["data"].clone();
            self.settings_error.clear();
        } else {
            self.settings_options = Value::Null;
            self.settings_error = reply["error"]
                .as_str()
                .unwrap_or("Installed profile choices unavailable")
                .into();
        }
    }

    pub(super) fn choose_settings_target(&mut self, target: &str, cx: &mut Context<Self>) {
        if !TARGETS.iter().any(|(id, _)| *id == target) {
            return;
        }
        self.settings_target = target.to_owned();
        self.settings_material = "PLA".into();
        self.settings_draft = self.settings_loaded["printers"][target].clone();
        self.refresh_settings_options();
        cx.notify();
    }

    pub(super) fn choose_settings_nozzle(&mut self, nozzle: &str, cx: &mut Context<Self>) {
        self.settings_draft["nozzle"] = json!(nozzle);
        self.refresh_settings_options();
        cx.notify();
    }
    pub(super) fn choose_settings_slicer(&mut self, slicer: &str, cx: &mut Context<Self>) {
        self.settings_draft["slicer"] = json!(slicer);
        self.refresh_settings_options();
        cx.notify();
    }
    pub(super) fn choose_settings_material(&mut self, material: &str, cx: &mut Context<Self>) {
        if !MATERIALS.contains(&material) {
            return;
        }
        self.settings_material = material.to_owned();
        self.refresh_settings_options();
        cx.notify();
    }
    pub(super) fn choose_settings_filament(&mut self, preset: &str, cx: &mut Context<Self>) {
        let Some(filaments) = self.settings_draft["filaments"].as_object_mut() else {
            return;
        };
        if preset == "auto" {
            filaments.remove(&self.settings_material);
        } else {
            filaments.insert(self.settings_material.clone(), json!(preset));
        }
        self.settings_error.clear();
        cx.notify();
    }

    pub(super) fn close_settings(&mut self, cx: &mut Context<Self>) {
        self.settings_open = false;
        self.settings_draft = Value::Null;
        self.settings_options = Value::Null;
        self.settings_error.clear();
        cx.notify();
    }
    pub(super) fn save_settings(&mut self, cx: &mut Context<Self>) {
        if auto_nozzle_has_filament_override(&self.settings_draft) {
            self.settings_error = self.locale.text("autoPresetWarning");
            cx.notify();
            return;
        }
        let reply = self.commands.execute(
            "settings.save",
            &json!({
                "target":self.settings_target,
                "pref":self.settings_draft,
            }),
        );
        if reply["ok"] == true {
            // Echoed readback is the ground truth after atomic write.
            self.settings_loaded = reply["data"].clone();
            let fresh = self.commands.execute("profiles.discover", &json!({}));
            self.ready_profiles = fresh["data"].clone();
            self.ready_oem_targets = ready_from_preferences(
                &self.ready_profiles,
                &self.settings_loaded,
                self.source_nozzle.as_deref(),
            );
            self.settings_open = false;
            self.settings_draft = Value::Null;
            self.settings_error.clear();
            self.last_message = self.locale.text("saved");
        } else {
            self.settings_error = reply["error"]
                .as_str()
                .unwrap_or("Settings save failed")
                .into();
        }
        cx.notify();
    }

    pub(super) fn render_settings(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let locale = self.locale;
        let rtl = locale.rtl();
        let saved_nozzle = self.settings_draft["nozzle"].as_str().unwrap_or("auto");
        let saved_slicer = self.settings_draft["slicer"].as_str().unwrap_or("auto");
        let material = self.settings_material.as_str();
        let selected_preset = self.settings_draft["filaments"][material]
            .as_str()
            .unwrap_or("auto");

        let mut tabs = line(rtl)
            .gap_2()
            .p_3()
            .flex_wrap()
            .border_b_1()
            .border_color(rgb(0xD5E6DD));
        for (target, title) in TARGETS {
            let target_id = *target;
            let is_selected = self.settings_target == *target;
            tabs = tabs.child(choice_button(
                format!("settings-target-{target}"),
                (*title).into(),
                is_selected,
                true,
                move |this, _, _, cx| this.choose_settings_target(target_id, cx),
                cx,
            ));
        }

        let mut nozzles = line(rtl).gap_2().flex_wrap();
        // Wails native/frontend/dist/app.js defines these as a static hardware
        // catalog; missing installed Orca/QIDI/Prusa profiles must not hide
        // valid physical nozzle choices in either English or Hebrew.
        for nozzle in
            std::iter::once("auto").chain(baseline_nozzles(&self.settings_target).iter().copied())
        {
            let active = saved_nozzle == nozzle;
            let label = if nozzle == "auto" {
                locale.text("autoNozzle")
            } else {
                format!("{nozzle} mm")
            };
            let value = nozzle.to_owned();
            nozzles = nozzles.child(choice_button(
                format!("settings-nozzle-{nozzle}"),
                label,
                active,
                true,
                move |this, _, _, cx| this.choose_settings_nozzle(&value, cx),
                cx,
            ));
        }

        let mut slicers = line(rtl).flex_wrap().gap_2();
        slicers = slicers.child(choice_button(
            "settings-slicer-auto".into(),
            locale.text("autoSlicer"),
            saved_slicer == "auto",
            true,
            |this, _, _, cx| this.choose_settings_slicer("auto", cx),
            cx,
        ));
        let slicer_records = self.settings_options["slicers"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut slicer_is_unavailable = saved_slicer != "auto";
        for entry in slicer_records {
            let Some(id) = entry["id"].as_str() else {
                continue;
            };
            let id = id.to_owned();
            let name = entry["name"].as_str().unwrap_or(&id).to_owned();
            let installed = entry["installed"] == true;
            if installed && id == saved_slicer {
                slicer_is_unavailable = false;
            }
            let active = saved_slicer == id;
            let title = if installed {
                name
            } else {
                format!("{} — {}", name, locale.text("unsupportedPrinter"))
            };
            let value = id.clone();
            slicers = slicers.child(choice_button(
                format!("settings-slicer-{id}"),
                title,
                active,
                installed,
                move |this, _, _, cx| this.choose_settings_slicer(&value, cx),
                cx,
            ));
        }

        let mut materials = line(rtl).flex_wrap().gap_2();
        for kind in MATERIALS {
            let value = *kind;
            materials = materials.child(choice_button(
                format!("settings-material-{kind}"),
                (*kind).into(),
                material == *kind,
                true,
                move |this, _, _, cx| this.choose_settings_material(value, cx),
                cx,
            ));
        }

        let allow_profile = saved_nozzle != "auto" && !slicer_is_unavailable;
        let existing = self.settings_options["filaments"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut filament_picker = div().id("settingsFilament").flex().flex_col().gap_2();
        filament_picker = filament_picker.child(choice_button(
            "settings-filament-auto".into(),
            locale.text("autoFilament"),
            selected_preset == "auto" || selected_preset.is_empty(),
            true,
            |this, _, _, cx| this.choose_settings_filament("auto", cx),
            cx,
        ));
        let mut saved_profile_available = selected_preset == "auto" || selected_preset.is_empty();
        for (idx, entry) in existing.iter().enumerate() {
            let Some(preset) = entry.as_str() else {
                continue;
            };
            if preset == selected_preset {
                saved_profile_available = true;
            }
            let value = preset.to_owned();
            filament_picker = filament_picker.child(choice_button(
                format!("settings-filament-{idx}"),
                value.clone(),
                selected_preset == preset,
                allow_profile,
                move |this, _, _, cx| this.choose_settings_filament(&value, cx),
                cx,
            ));
        }
        if !saved_profile_available {
            filament_picker =
                filament_picker.child(div().text_xs().text_color(rgb(0xB34940)).child(format!(
                    "{} — {}",
                    selected_preset,
                    locale.text("unsupportedPrinter")
                )));
        }

        let has_saved_profiles = self.settings_draft["filaments"]
            .as_object()
            .is_some_and(|map| {
                map.values()
                    .any(|v| v.as_str().is_some_and(|s| !s.is_empty()))
            });
        let hint = if saved_nozzle == "auto" && has_saved_profiles {
            locale.text("autoPresetWarning")
        } else if saved_nozzle == "auto" {
            locale.text("autoPresetHint")
        } else if existing.is_empty() {
            locale.text("noNativeProfiles")
        } else {
            String::new()
        };

        let text_field = |key: &str, subkey: &str| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(0x193F38))
                        .child(locale.text(key)),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(0x526B61))
                        .child(locale.text(subkey)),
                )
        };

        let content = div()
            .id("settings-scroll")
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_5()
            .px_5()
            .py_4()
            .child(
                div()
                    .id("settingsNozzle")
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(text_field("physicalNozzle", "nozzleHint"))
                    .child(nozzles),
            )
            .child(
                div()
                    .id("settingsSlicer")
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(text_field("preferredSlicer", "slicerHint"))
                    .child(slicers),
            )
            .child(
                div()
                    .id("settingsMaterial")
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(text_field("filamentPreferences", "filamentHint"))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x526B61))
                            .child(locale.text("material")),
                    )
                    .child(materials)
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x526B61))
                            .child(locale.text("preferredFilament")),
                    )
                    .child(
                        div()
                            .id("settings-filament-scroll")
                            .max_h(px(180.))
                            .overflow_y_scroll()
                            .child(filament_picker),
                    ),
            )
            .child(div().text_xs().text_color(rgb(0x526B61)).child(hint))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0xB34940))
                    .child(self.settings_error.clone()),
            );

        let header = line(rtl)
            .justify_between()
            .items_center()
            .px_5()
            .py_4()
            .border_b_1()
            .border_color(rgb(0xD5E6DD))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_lg()
                            .text_color(rgb(0x193F38))
                            .child(locale.text("settingsTitle")),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x526B61))
                            .child(locale.text("settingsHint")),
                    ),
            )
            .child(
                Button::new("closeSettingsBtn")
                    .label("×")
                    .on_click(cx.listener(|this, _, _, cx| this.close_settings(cx))),
            );
        let footer = line(rtl)
            .gap_2()
            .justify_end()
            .items_center()
            .p_4()
            .border_t_1()
            .border_color(rgb(0xD5E6DD))
            .child(
                Button::new("cancelSettingsBtn")
                    .label(locale.text("cancel"))
                    .on_click(cx.listener(|this, _, _, cx| this.close_settings(cx))),
            )
            .child(
                Button::new("saveSettingsBtn")
                    .primary()
                    .label(locale.text("saveSettings"))
                    .on_click(cx.listener(|this, _, _, cx| this.save_settings(cx))),
            );

        div()
            .id("settingsDialog")
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(0x193F387F))
            .child(
                div()
                    .id("settingsFrame")
                    .w(px(820.))
                    .max_w(relative(0.94))
                    .h(relative(0.80))
                    .bg(rgb(0xFFFFFF))
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(0xD5E6DD))
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .child(header)
                    .child(tabs)
                    .child(content)
                    .child(footer),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{baseline_nozzles, MATERIALS, TARGETS};
    use serde_json::json;

    #[test]
    fn physical_nozzles_match_wails_hardware_catalog_without_installed_profiles() {
        // Source authority: native/frontend/dist/app.js nozzleChoices.
        for target in ["u1", "cc2", "qidi"] {
            assert_eq!(baseline_nozzles(target), &["0.2", "0.4", "0.6", "0.8"]);
        }
        assert_eq!(baseline_nozzles("c5"), &["0.25", "0.4", "0.6", "0.8"]);
        assert!(baseline_nozzles("unknown").is_empty());
        assert_eq!(TARGETS.len(), 4);
    }

    #[test]
    fn target_readiness_follows_saved_physical_nozzle_not_hardcoded_default() {
        let mut saved = slicebridge_alpha::printer_settings::defaults();
        saved["printers"]["u1"]["nozzle"] = json!("0.6");
        let discovered = json!({"profiles":[
            {"target":"u1","nozzle":"0.4","ready":false},
            {"target":"u1","nozzle":"0.6","ready":true},
            {"target":"qidi","nozzle":"0.4","ready":false}
        ]});
        assert!(super::ready_from_preferences(&discovered, &saved, None).contains("u1"));
        saved["printers"]["u1"]["nozzle"] = json!("auto");
        assert!(!super::ready_from_preferences(&discovered, &saved, None).contains("u1"));
        assert!(super::ready_from_preferences(&discovered, &saved, Some("0.6")).contains("u1"));
    }

    #[test]
    fn readiness_includes_creator5_and_cc2_and_honors_the_saved_slicer() {
        let mut saved = slicebridge_alpha::printer_settings::defaults();
        saved["printers"]["c5"]["nozzle"] = json!("0.4");
        saved["printers"]["c5"]["slicer"] = json!("flash-studio");
        saved["printers"]["cc2"]["nozzle"] = json!("0.4");
        saved["printers"]["cc2"]["slicer"] = json!("elegoo-slicer");
        let discovered = json!({"profiles":[
            {"target":"u1","nozzle":"0.4","ready":true},
            {"target":"c5","nozzle":"0.4","ready":false},
            {"target":"cc2","nozzle":"0.4","ready":false},
            {"target":"qidi","nozzle":"0.4","ready":false}
        ]});
        let mut resolved = Vec::new();
        let ready = super::ready_from_preferences_with(
            &discovered,
            &saved,
            None,
            |target, nozzle, slicer, overrides| {
                resolved.push((
                    target.to_owned(),
                    nozzle.to_owned(),
                    slicer.to_owned(),
                    overrides.len(),
                ));
                (target == "c5" && slicer == "flash-studio")
                    || (target == "cc2" && slicer == "elegoo-slicer")
            },
        );
        assert!(ready.contains("u1")); // exact auto discovery
        assert!(ready.contains("c5")); // explicit installed slicer overrides auto discovery
        assert!(ready.contains("cc2"));
        assert!(!ready.contains("qidi")); // neither discovery nor selected resolver is ready
        assert_eq!(
            resolved,
            vec![
                ("c5".into(), "0.4".into(), "flash-studio".into(), 0),
                ("cc2".into(), "0.4".into(), "elegoo-slicer".into(), 0),
            ]
        );
    }

    #[test]
    fn settings_ui_has_exact_legacy_target_and_material_ids() {
        assert_eq!(
            TARGETS.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec!["u1", "c5", "cc2", "qidi"]
        );
        assert_eq!(MATERIALS, &["PLA", "PETG", "ABS", "ASA", "TPU"]);
    }

    #[test]
    fn auto_nozzle_blocks_only_non_empty_filament_overrides() {
        let auto_with_preset = json!({"nozzle":"auto","filaments":{"PLA":"Custom PLA"}});
        assert!(super::auto_nozzle_has_filament_override(&auto_with_preset));
        let auto_empty_preset = json!({"nozzle":"auto","filaments":{"PLA":""}});
        assert!(!super::auto_nozzle_has_filament_override(
            &auto_empty_preset
        ));
        let fixed_with_preset = json!({"nozzle":"0.4","filaments":{"PLA":"Custom PLA"}});
        assert!(!super::auto_nozzle_has_filament_override(
            &fixed_with_preset
        ));
        let missing_nozzle = json!({"filaments":{"PETG":"Custom PETG"}});
        assert!(super::auto_nozzle_has_filament_override(&missing_nozzle));
    }
}
