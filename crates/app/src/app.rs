//! The eframe application.
//!
//! Phase 0 is deliberately bare: a single centred placeholder panel. The dock
//! surface, sidebar and transport bar land in Phase 4.

/// Top-level Spottyfi application state held by eframe.
pub struct SpottyfiApp {
    /// Frames rendered since launch — a trivial liveness signal for Phase 0.
    frames: u64,
}

impl SpottyfiApp {
    /// Build the app from eframe's creation context.
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        tracing::debug!("constructing SpottyfiApp");
        Self { frames: 0 }
    }
}

impl eframe::App for SpottyfiApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frames = self.frames.wrapping_add(1);

        egui::Frame::central_panel(ui.style()).show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.4);
                ui.heading("Spottyfi");
                ui.label("Phase 0 — bootstrap");
                ui.add_space(8.0);
                ui.small("An empty window today; a docking Spotify client tomorrow.");
            });
        });
    }
}
