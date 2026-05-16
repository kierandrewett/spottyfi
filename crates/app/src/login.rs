//! The logged-out screens: a login screen and an in-progress spinner.
//!
//! Once logged in, the real application shell ([`crate::shell`]) takes over —
//! these screens only ever render while the user is logged out, restoring or
//! authorizing.

use spottyfi_auth::AuthState;
use spottyfi_ui::theme::Palette;

/// What the login screen asked the app to do this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginIntent {
    /// Start the OAuth login flow.
    Login,
}

/// Render the appropriate logged-out screen for `state`.
///
/// Returns `Some(LoginIntent::Login)` if the user clicked the sign-in button.
/// Renders nothing meaningful (and returns `None`) when `state` is logged in —
/// the caller switches to the shell in that case.
pub fn login_screen(
    ui: &mut egui::Ui,
    palette: &Palette,
    state: &AuthState,
) -> Option<LoginIntent> {
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(palette.base))
        .show_inside(ui, |ui| {
            ui.set_min_size(ui.available_size());
            match state {
                AuthState::LoggedOut => sign_in(ui, palette, None),
                AuthState::Failed(message) => sign_in(ui, palette, Some(message)),
                AuthState::Restoring => {
                    progress(ui, palette, "Restoring your session…");
                    None
                }
                AuthState::Authorizing => {
                    progress(
                        ui,
                        palette,
                        "Waiting for Spotify… complete sign-in in your browser.",
                    );
                    None
                }
                // The shell renders the logged-in surface; nothing to do here.
                AuthState::LoggedIn(_) => None,
            }
        })
        .inner
}

/// Centre `add_contents` near the top third of the available space.
fn centred(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.30).max(24.0));
        add_contents(ui);
    });
}

/// The sign-in screen. `error` is shown above the button when present.
fn sign_in(ui: &mut egui::Ui, palette: &Palette, error: Option<&str>) -> Option<LoginIntent> {
    let mut intent = None;
    centred(ui, |ui| {
        ui.label(
            egui::RichText::new("Spottyfi")
                .family(spottyfi_ui::fonts::semibold())
                .size(40.0)
                .color(palette.text),
        );
        ui.add_space(6.0);
        ui.label(spottyfi_ui::components::muted(
            palette,
            "A native Rust Spotify client",
            13.0,
        ));
        ui.add_space(28.0);

        if let Some(error) = error {
            ui.label(
                egui::RichText::new("Sign-in failed")
                    .color(palette.error)
                    .strong(),
            );
            ui.add_space(4.0);
            ui.label(spottyfi_ui::components::muted(palette, error, 12.0));
            ui.add_space(20.0);
        }

        let label = if error.is_some() {
            "Try again"
        } else {
            "Sign in with Spotify"
        };
        if spottyfi_ui::components::primary_button(ui, palette, label, egui::vec2(240.0, 44.0))
            .clicked()
        {
            intent = Some(LoginIntent::Login);
        }
    });
    intent
}

/// The in-progress screen: a spinner plus a status line.
fn progress(ui: &mut egui::Ui, palette: &Palette, status: &str) {
    centred(ui, |ui| {
        ui.label(
            egui::RichText::new("Spottyfi")
                .family(spottyfi_ui::fonts::semibold())
                .size(32.0)
                .color(palette.text),
        );
        ui.add_space(28.0);
        ui.add(egui::Spinner::new().size(36.0).color(palette.accent));
        ui.add_space(18.0);
        ui.label(spottyfi_ui::components::muted(palette, status, 13.0));
    });
}
