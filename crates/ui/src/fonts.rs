//! Bundled fonts: Inter for UI text, JetBrains Mono for monospace.
//!
//! Both fonts are OFL-licensed and committed under `assets/fonts/`. They are
//! embedded into the binary with [`include_bytes!`] and registered into egui's
//! [`egui::FontDefinitions`] by [`install`].

/// Inter Regular — the base UI weight.
const INTER_REGULAR: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
/// Inter Medium — used for emphasised UI text.
const INTER_MEDIUM: &[u8] = include_bytes!("../assets/fonts/Inter-Medium.ttf");
/// Inter SemiBold — used for headings and strong text.
const INTER_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/Inter-SemiBold.ttf");
/// JetBrains Mono Regular — the monospace family (the debug panel).
const JETBRAINS_MONO: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf");

/// The font family name used for emphasised UI text (Inter Medium).
pub const FAMILY_MEDIUM: &str = "Inter-Medium";
/// The font family name used for headings / strong text (Inter SemiBold).
pub const FAMILY_SEMIBOLD: &str = "Inter-SemiBold";

/// Install the bundled fonts into `ctx`, making Inter the default UI font and
/// JetBrains Mono the monospace family.
///
/// Inter Medium and SemiBold are also registered as standalone families
/// ([`FAMILY_MEDIUM`], [`FAMILY_SEMIBOLD`]) so widgets can opt into a heavier
/// weight without faux-bolding the Regular face.
pub fn install(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    insert(&mut fonts, "Inter-Regular", INTER_REGULAR);
    insert(&mut fonts, FAMILY_MEDIUM, INTER_MEDIUM);
    insert(&mut fonts, FAMILY_SEMIBOLD, INTER_SEMIBOLD);
    insert(&mut fonts, "JetBrainsMono", JETBRAINS_MONO);

    // Inter Regular becomes the proportional default; the stock fallback fonts
    // stay behind it so missing glyphs still resolve.
    if let Some(list) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
        list.insert(0, "Inter-Regular".to_owned());
    }
    if let Some(list) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
        list.insert(0, "JetBrainsMono".to_owned());
    }

    // Standalone families for the heavier weights.
    fonts.families.insert(
        egui::FontFamily::Name(FAMILY_MEDIUM.into()),
        vec![FAMILY_MEDIUM.to_owned(), "Inter-Regular".to_owned()],
    );
    fonts.families.insert(
        egui::FontFamily::Name(FAMILY_SEMIBOLD.into()),
        vec![FAMILY_SEMIBOLD.to_owned(), "Inter-Regular".to_owned()],
    );

    ctx.set_fonts(fonts);
}

/// Register one font face under `name` from embedded `bytes`.
fn insert(fonts: &mut egui::FontDefinitions, name: &str, bytes: &'static [u8]) {
    fonts.font_data.insert(
        name.to_owned(),
        std::sync::Arc::new(egui::FontData::from_static(bytes)),
    );
}

/// The [`egui::FontFamily`] for Inter Medium.
#[must_use]
pub fn medium() -> egui::FontFamily {
    egui::FontFamily::Name(FAMILY_MEDIUM.into())
}

/// The [`egui::FontFamily`] for Inter SemiBold.
#[must_use]
pub fn semibold() -> egui::FontFamily {
    egui::FontFamily::Name(FAMILY_SEMIBOLD.into())
}
