//! The Phase 1 screens: a login screen, an in-progress spinner, and a
//! logged-in view showing the signed-in user.
//!
//! This is deliberately self-contained in `app`; the reusable widget library
//! (`crates/ui`) and the dock shell arrive in Phase 4.

use spottyfi_auth::{AuthState, UserProfile};

/// Spottyfi base background, near-black (`#121212`).
const BG: egui::Color32 = egui::Color32::from_rgb(0x12, 0x12, 0x12);
/// Spotify accent green (`#1ed760`).
const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x1e, 0xd7, 0x60);
/// A darker green for the button's hovered/pressed state.
const ACCENT_DARK: egui::Color32 = egui::Color32::from_rgb(0x1a, 0xbf, 0x54);
/// Muted secondary text.
const MUTED: egui::Color32 = egui::Color32::from_rgb(0xb3, 0xb3, 0xb3);
/// Error red.
const ERROR: egui::Color32 = egui::Color32::from_rgb(0xf1, 0x5e, 0x6c);

/// What the user asked the auth screens to do this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthIntent {
    /// Start the OAuth login flow.
    Login,
    /// Log out and return to the login screen.
    Logout,
}

/// Render the screen appropriate to `state`, returning any user intent.
pub fn auth_screen(
    ui: &mut egui::Ui,
    state: &AuthState,
    avatar: Option<&egui::TextureHandle>,
) -> Option<AuthIntent> {
    egui::Frame::new()
        .fill(BG)
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            match state {
                AuthState::LoggedOut => login_screen(ui, None),
                AuthState::Failed(message) => login_screen(ui, Some(message)),
                AuthState::Restoring => {
                    progress_screen(ui, "Restoring your session…");
                    None
                }
                AuthState::Authorizing => {
                    progress_screen(ui, "Waiting for Spotify… complete sign-in in your browser.");
                    None
                }
                AuthState::LoggedIn(profile) => logged_in_screen(ui, profile, avatar),
            }
        })
        .inner
}

/// Centre `add_contents` vertically and horizontally in the available space.
fn centred(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.32).max(24.0));
        add_contents(ui);
    });
}

/// The login screen. `error` is shown above the button when present.
fn login_screen(ui: &mut egui::Ui, error: Option<&str>) -> Option<AuthIntent> {
    let mut intent = None;
    centred(ui, |ui| {
        ui.heading(
            egui::RichText::new("Spottyfi")
                .size(40.0)
                .color(egui::Color32::WHITE),
        );
        ui.add_space(6.0);
        ui.label(egui::RichText::new("A native Rust Spotify client").color(MUTED));
        ui.add_space(28.0);

        if let Some(error) = error {
            ui.label(egui::RichText::new("Sign-in failed").color(ERROR).strong());
            ui.add_space(4.0);
            ui.label(egui::RichText::new(error).color(MUTED).size(12.0));
            ui.add_space(20.0);
        }

        let label = if error.is_some() {
            "Try again"
        } else {
            "Sign in with Spotify"
        };
        if accent_button(ui, label).clicked() {
            intent = Some(AuthIntent::Login);
        }
    });
    intent
}

/// The in-progress screen: a spinner plus a status line.
fn progress_screen(ui: &mut egui::Ui, status: &str) {
    centred(ui, |ui| {
        ui.heading(
            egui::RichText::new("Spottyfi")
                .size(32.0)
                .color(egui::Color32::WHITE),
        );
        ui.add_space(28.0);
        ui.add(egui::Spinner::new().size(36.0).color(ACCENT));
        ui.add_space(18.0);
        ui.label(egui::RichText::new(status).color(MUTED));
    });
}

/// The logged-in view: avatar (if available), display name, id and log-out.
fn logged_in_screen(
    ui: &mut egui::Ui,
    profile: &UserProfile,
    avatar: Option<&egui::TextureHandle>,
) -> Option<AuthIntent> {
    let mut intent = None;
    centred(ui, |ui| {
        if let Some(texture) = avatar {
            let size = egui::vec2(96.0, 96.0);
            ui.add(egui::Image::new((texture.id(), size)).corner_radius(48.0));
            ui.add_space(16.0);
        }

        let name = profile.display_name.as_deref().unwrap_or("Spotify user");
        ui.heading(
            egui::RichText::new(name)
                .size(28.0)
                .color(egui::Color32::WHITE),
        );
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(format!("id: {}", profile.id))
                .color(MUTED)
                .size(12.0),
        );
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("Signed in — audio and the dock UI land in later phases.")
                .color(MUTED)
                .size(12.0),
        );
        ui.add_space(28.0);

        if accent_button(ui, "Log out").clicked() {
            intent = Some(AuthIntent::Logout);
        }
    });
    intent
}

/// A pill-shaped accent-green button matching the Spotify look.
fn accent_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    // Darken the fill slightly while hovered for affordance.
    let fill = if ui.ui_contains_pointer() {
        ACCENT_DARK
    } else {
        ACCENT
    };

    let button = egui::Button::new(
        egui::RichText::new(label)
            .color(egui::Color32::BLACK)
            .size(15.0)
            .strong(),
    )
    .fill(fill)
    .corner_radius(22.0)
    .min_size(egui::vec2(220.0, 44.0));

    ui.add(button)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}
