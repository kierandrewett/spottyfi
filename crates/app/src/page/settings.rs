//! The Settings page — a real, full page tab (not a modal).
//!
//! Replaces the small floating settings window. The page is organised into
//! power-user sections — Playback / Audio, Equalizer, Local Files, Appearance,
//! Hotkeys — each a flat, dense panel matching `docs/ui-reference.md`.
//!
//! Unlike a registry-backed [`Page`](crate::page::Page), the Settings page
//! needs **mutable** access to the persisted shell state (theme, density,
//! layout, the [`AppSettings`] block), so it is rendered as a special-cased
//! dock tab straight from the shell rather than through the page registry. The
//! shell hands it [`SettingsContext`] and collects the [`SettingsAction`]s it
//! raises.

use spottyfi_ui::components::{self, Density};
use spottyfi_ui::theme::{Palette, Theme};

use crate::settings::{
    AppSettings, AudioSettings, EqPreset, StreamTier, EQ_BAND_FREQUENCIES_HZ, EQ_GAIN_LIMIT_DB,
};
use crate::shell::Layout;

/// The keyboard shortcuts surfaced read-only on the Hotkeys section.
///
/// Sourced from `docs/docking.md` and the shell's `apply_shortcuts`. Full
/// rebinding is a later workstream — this is the documented, current set.
const HOTKEYS: &[(&str, &str)] = &[
    ("Close tab", "Cmd/Ctrl + W"),
    ("New Home tab", "Cmd/Ctrl + T"),
    ("Reopen closed tab", "Cmd/Ctrl + Shift + T"),
    ("Open Search", "Cmd/Ctrl + K"),
    ("Open in new tab", "Cmd/Ctrl + click a link"),
    ("Close a tab", "Middle-click the tab"),
];

/// Everything the Settings page borrows from the shell for one frame.
pub struct SettingsContext<'a> {
    /// The active theme palette.
    pub palette: Palette,
    /// The selected colour theme (mutated in place by the Appearance section).
    pub theme: &'a mut Theme,
    /// The selected row density (mutated by the Appearance section).
    pub density: &'a mut Density,
    /// The currently-applied dock layout, shown in the Appearance section.
    pub layout: Layout,
    /// The power-user settings block (audio, equalizer, local files).
    pub settings: &'a mut AppSettings,
    /// A draft folder path the user is typing in the Local Files section.
    pub local_folder_draft: &'a mut String,
}

/// Something the Settings page asked the shell to do this frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsAction {
    /// Apply a predefined dock layout.
    ApplyLayout(Layout),
    /// Reset the dock layout to the default.
    ResetLayout,
    /// The audio engine settings changed; the engine must restart to pick them
    /// up (librespot bakes them in at connect time).
    AudioChanged,
    /// The equaliser settings changed; the new gains are pushed live to the
    /// audio engine (no restart — the DSP picks them up on the next packet).
    EqualizerChanged,
}

/// Render the Settings page body, returning every [`SettingsAction`] raised.
pub fn settings_page(ui: &mut egui::Ui, ctx: &mut SettingsContext<'_>) -> Vec<SettingsAction> {
    let palette = ctx.palette;
    let mut actions = Vec::new();

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_max_width(720.0);

            page_title(ui, &palette);

            audio_section(ui, &palette, ctx, &mut actions);
            ui.add_space(20.0);
            equalizer_section(ui, &palette, ctx, &mut actions);
            ui.add_space(20.0);
            local_files_section(ui, &palette, ctx);
            ui.add_space(20.0);
            appearance_section(ui, &palette, ctx, &mut actions);
            ui.add_space(20.0);
            hotkeys_section(ui, &palette);

            ui.add_space(16.0);
            ui.label(components::muted(
                &palette,
                "All settings persist across restarts.",
                11.0,
            ));
            ui.add_space(8.0);
        });

    actions
}

/// The large page title at the top of the Settings body.
fn page_title(ui: &mut egui::Ui, palette: &Palette) {
    ui.label(
        egui::RichText::new("Settings")
            .family(spottyfi_ui::fonts::semibold())
            .size(24.0)
            .color(palette.text),
    );
    ui.add_space(2.0);
    ui.label(components::muted(
        palette,
        "Audio, equalizer, local files and appearance.",
        12.0,
    ));
    ui.add_space(16.0);
}

/// A flat, sharp-cornered section panel with a header and a body closure.
fn section(
    ui: &mut egui::Ui,
    palette: &Palette,
    title: &str,
    subtitle: &str,
    body: impl FnOnce(&mut egui::Ui),
) {
    egui::Frame::new()
        .fill(palette.card)
        .inner_margin(egui::Margin::same(14))
        .corner_radius(0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                egui::RichText::new(title)
                    .family(spottyfi_ui::fonts::semibold())
                    .size(14.0)
                    .color(palette.text),
            );
            if !subtitle.is_empty() {
                ui.label(components::muted(palette, subtitle, 11.0));
            }
            ui.add_space(8.0);
            body(ui);
        });
}

/// One settings row: a left-aligned label and a right-aligned control.
fn setting_row(
    ui: &mut egui::Ui,
    palette: &Palette,
    label: &str,
    control: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal(|ui| {
        ui.add_sized(
            egui::vec2(180.0, 22.0),
            egui::Label::new(
                egui::RichText::new(label)
                    .size(12.5)
                    .color(palette.text)
                    .family(spottyfi_ui::fonts::medium()),
            ),
        );
        control(ui);
    });
    ui.add_space(4.0);
}

/// The Playback / Audio section: stream quality, normalisation, crossfade and
/// the audio backend readout.
fn audio_section(
    ui: &mut egui::Ui,
    palette: &Palette,
    ctx: &mut SettingsContext<'_>,
    actions: &mut Vec<SettingsAction>,
) {
    section(
        ui,
        palette,
        "Playback & Audio",
        "Streaming quality and normalisation apply on the next engine start.",
        |ui| {
            let audio = &mut ctx.settings.audio;

            setting_row(ui, palette, "Streaming quality", |ui| {
                let mut changed = false;
                egui::ComboBox::from_id_salt("settings-quality")
                    .selected_text(audio.quality.label())
                    .show_ui(ui, |ui| {
                        for tier in StreamTier::all() {
                            changed |= ui
                                .selectable_value(&mut audio.quality, tier, tier.label())
                                .changed();
                        }
                    });
                if changed {
                    actions.push(SettingsAction::AudioChanged);
                }
            });

            setting_row(ui, palette, "Volume normalisation", |ui| {
                if ui
                    .checkbox(&mut audio.normalisation, "Even out track loudness")
                    .changed()
                {
                    actions.push(SettingsAction::AudioChanged);
                }
            });

            setting_row(ui, palette, "Crossfade", |ui| {
                ui.add(
                    egui::Slider::new(
                        &mut audio.crossfade_seconds,
                        0.0..=AudioSettings::MAX_CROSSFADE_SECONDS,
                    )
                    .suffix(" s")
                    .fixed_decimals(1),
                );
            });
            ui.label(components::muted(
                palette,
                "Crossfade is saved for a future audio backend; librespot has \
                 no crossfade today, so it does not yet affect playback.",
                10.5,
            ));

            ui.add_space(6.0);
            ui.separator();
            ui.add_space(4.0);
            setting_row(ui, palette, "Audio backend", |ui| {
                ui.label(components::muted(palette, "librespot · rodio (ALSA)", 11.5));
            });
            setting_row(ui, palette, "Codec", |ui| {
                ui.label(components::muted(palette, "Ogg Vorbis", 11.5));
            });
        },
    );
}

/// The Equalizer section: an on/off toggle, presets and ten band sliders.
///
/// Every control that mutates the equaliser raises [`SettingsAction::
/// EqualizerChanged`]; `app` pushes the new gains straight to the running
/// audio engine — the DSP picks them up on its next decoded packet, no
/// restart.
fn equalizer_section(
    ui: &mut egui::Ui,
    palette: &Palette,
    ctx: &mut SettingsContext<'_>,
    actions: &mut Vec<SettingsAction>,
) {
    section(
        ui,
        palette,
        "Equalizer",
        "A 10-band graphic equalizer applied live to the audio stream.",
        |ui| {
            let eq = &mut ctx.settings.equalizer;
            let mut changed = false;

            ui.horizontal(|ui| {
                changed |= ui.checkbox(&mut eq.enabled, "Enable equalizer").changed();
                ui.add_space(16.0);
                ui.label(components::muted(palette, "Preset", 11.5));
                for preset in EqPreset::all() {
                    if components::filter_chip(ui, palette, preset.label(), false).clicked() {
                        eq.apply_preset(preset);
                        changed = true;
                    }
                }
            });
            ui.add_space(10.0);

            // The band sliders — vertical faders, low frequency on the left.
            let enabled = eq.enabled;
            ui.add_enabled_ui(enabled, |ui| {
                ui.horizontal(|ui| {
                    for (index, freq) in EQ_BAND_FREQUENCIES_HZ.iter().enumerate() {
                        ui.vertical(|ui| {
                            ui.set_width(54.0);
                            changed |= ui
                                .add(
                                    egui::Slider::new(
                                        &mut eq.band_gains_db[index],
                                        -EQ_GAIN_LIMIT_DB..=EQ_GAIN_LIMIT_DB,
                                    )
                                    .vertical()
                                    .show_value(false),
                                )
                                .changed();
                            ui.vertical_centered(|ui| {
                                ui.label(
                                    egui::RichText::new(format!("{:+.0}", eq.band_gains_db[index]))
                                        .size(10.0)
                                        .color(palette.text_muted),
                                );
                                ui.label(
                                    egui::RichText::new(format_hz(*freq))
                                        .size(9.5)
                                        .color(palette.text_muted),
                                );
                            });
                        });
                    }
                });
            });

            ui.add_space(8.0);
            if ui.button("Reset bands to flat").clicked() {
                eq.apply_preset(EqPreset::Flat);
                changed = true;
            }
            ui.add_space(4.0);
            ui.label(components::muted(
                palette,
                "Band gains apply live; the equalizer is a bank of biquad \
                 peaking filters in the custom audio backend.",
                10.5,
            ));

            if changed {
                actions.push(SettingsAction::EqualizerChanged);
            }
        },
    );
}

/// Format a band centre frequency for its slider caption (`500`, `1k`, `16k`).
fn format_hz(hz: u32) -> String {
    if hz >= 1_000 {
        format!("{}k", hz / 1_000)
    } else {
        hz.to_string()
    }
}

/// The Local Files section: a list of local-music folders with add / remove.
///
/// Library scanning and local playback are out of scope here — this is the
/// persisted folder list only. The folder is added by typing/pasting its path
/// (no native picker, which would need a windowing dialog).
fn local_files_section(ui: &mut egui::Ui, palette: &Palette, ctx: &mut SettingsContext<'_>) {
    section(
        ui,
        palette,
        "Local Files",
        "Folders Spottyfi will look in for local music. Scanning lands later.",
        |ui| {
            let local = &mut ctx.settings.local_files;

            if local.folders.is_empty() {
                ui.label(components::muted(palette, "No folders added yet.", 11.5));
            } else {
                let mut remove: Option<usize> = None;
                for (index, folder) in local.folders.iter().enumerate() {
                    ui.horizontal(|ui| {
                        spottyfi_ui::icons::icon(
                            ui,
                            spottyfi_ui::Icon::Library,
                            13.0,
                            palette.text_muted,
                        );
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(folder.display().to_string())
                                    .size(11.5)
                                    .color(palette.text),
                            )
                            .truncate(),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Remove").clicked() {
                                remove = Some(index);
                            }
                        });
                    });
                    ui.add_space(2.0);
                }
                if let Some(index) = remove {
                    local.remove_folder(index);
                }
            }

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(components::muted(palette, "Add folder", 11.5));
                let response = ui.add(
                    egui::TextEdit::singleline(ctx.local_folder_draft)
                        .hint_text("/path/to/music")
                        .desired_width(360.0),
                );
                let submit = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (ui.button("Add").clicked() || submit)
                    && !ctx.local_folder_draft.trim().is_empty()
                {
                    let path = std::path::PathBuf::from(ctx.local_folder_draft.trim());
                    ctx.settings.local_files.add_folder(path);
                    ctx.local_folder_draft.clear();
                }
            });
        },
    );
}

/// The Appearance section: theme, density and the dock layout preset — the
/// controls migrated here from the old settings window.
fn appearance_section(
    ui: &mut egui::Ui,
    palette: &Palette,
    ctx: &mut SettingsContext<'_>,
    actions: &mut Vec<SettingsAction>,
) {
    section(
        ui,
        palette,
        "Appearance",
        "Theme, density and the dock layout preset.",
        |ui| {
            setting_row(ui, palette, "Theme", |ui| {
                egui::ComboBox::from_id_salt("settings-theme")
                    .selected_text(ctx.theme.label())
                    .show_ui(ui, |ui| {
                        for theme in Theme::all() {
                            ui.selectable_value(ctx.theme, theme, theme.label());
                        }
                    });
            });
            setting_row(ui, palette, "Density", |ui| {
                egui::ComboBox::from_id_salt("settings-density")
                    .selected_text(ctx.density.label())
                    .show_ui(ui, |ui| {
                        for density in [Density::Comfortable, Density::Compact] {
                            ui.selectable_value(ctx.density, density, density.label());
                        }
                    });
            });
            setting_row(ui, palette, "Layout preset", |ui| {
                egui::ComboBox::from_id_salt("settings-layout")
                    .selected_text(ctx.layout.label())
                    .show_ui(ui, |ui| {
                        for layout in Layout::all() {
                            if ui
                                .selectable_label(ctx.layout == layout, layout.label())
                                .clicked()
                            {
                                actions.push(SettingsAction::ApplyLayout(layout));
                            }
                        }
                    });
            });
            ui.add_space(4.0);
            if ui.button("Reset layout to default").clicked() {
                actions.push(SettingsAction::ResetLayout);
            }
        },
    );
}

/// The Hotkeys section: a read-only list of the current keyboard shortcuts.
///
/// Full rebinding is a later workstream; this documents the live bindings.
fn hotkeys_section(ui: &mut egui::Ui, palette: &Palette) {
    section(
        ui,
        palette,
        "Hotkeys",
        "Current keyboard shortcuts. Rebinding arrives in a later update.",
        |ui| {
            for (action, keys) in HOTKEYS {
                ui.horizontal(|ui| {
                    ui.add_sized(
                        egui::vec2(220.0, 20.0),
                        egui::Label::new(
                            egui::RichText::new(*action).size(12.0).color(palette.text),
                        ),
                    );
                    ui.label(
                        egui::RichText::new(*keys)
                            .size(11.5)
                            .family(spottyfi_ui::fonts::medium())
                            .color(palette.text_muted),
                    );
                });
                ui.add_space(2.0);
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_hz_compacts_kilohertz() {
        assert_eq!(format_hz(31), "31");
        assert_eq!(format_hz(500), "500");
        assert_eq!(format_hz(1_000), "1k");
        assert_eq!(format_hz(16_000), "16k");
    }

    #[test]
    fn hotkey_list_is_non_empty() {
        assert!(!HOTKEYS.is_empty());
        for (action, keys) in HOTKEYS {
            assert!(!action.is_empty());
            assert!(!keys.is_empty());
        }
    }
}
