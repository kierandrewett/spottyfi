//! The eframe application.
//!
//! Phase 4 builds the real application shell: when logged out it shows the
//! login screen; when logged in it renders the top bar, sidebar, the
//! `egui_dock` centre and the polished bottom transport. The dock layout and
//! theme persist across restarts.

use std::sync::Arc;

use arc_swap::ArcSwap;
use tokio::runtime::Runtime;

use crate::auth_controller::AuthController;
use crate::avatar::{self, SharedAvatar};
use crate::login::{self, LoginIntent};
use crate::media::single_instance::InstanceGuard;
use crate::media::{self, MediaBridge, MediaCommand};
use crate::playback_controller::PlaybackControllerHandle;
use crate::shell::{self, ShellIntent, ShellState};
use crate::transport::{self, TransportIntent, TransportUiState};

/// Top-level Spottyfi application state held by eframe.
pub struct SpottyfiApp {
    /// The tokio runtime that owns every async flow. Kept alive for the
    /// lifetime of the app; dropped (and shut down) when the window closes.
    _runtime: Runtime,
    /// Drives login / restore / logout and holds the auth state snapshot.
    auth: AuthController,
    /// Drives the audio engine and holds the playback state snapshot.
    playback: PlaybackControllerHandle,
    /// Per-frame UI state for the transport widgets (scrub drag, debug field).
    transport_ui: TransportUiState,
    /// The persisted + per-session shell state (dock layout, theme, sidebar).
    shell: ShellState,
    /// The decoded avatar image, populated by a background task.
    avatar_image: SharedAvatar,
    /// The uploaded avatar texture, created once from `avatar_image`.
    avatar_texture: Option<egui::TextureHandle>,
    /// Whether an avatar fetch has already been kicked off for the session.
    avatar_requested: bool,
    /// Whether the Spotify API has been attached to the shell for the session.
    api_attached: bool,
    /// The single-instance lock — held for the process lifetime so a second
    /// launch is detected. Never read; kept purely for its `Drop`.
    _instance: InstanceGuard,
    /// The desktop-integration hub: the MPRIS / tray / media-key command
    /// channel and the shared playback snapshot they read.
    media: MediaBridge,
    /// Whether the desktop-integration tasks (MPRIS, tray, media keys) have
    /// been started — done once, lazily, on the first frame.
    media_started: bool,
    /// Fires a desktop notification on track change when the user opted in.
    notifier: media::notify::TrackChangeNotifier,
    /// Whether the window is currently shown (toggled by the tray's
    /// Show/Hide). Tracked so `ToggleWindow` knows which way to flip.
    window_visible: bool,
}

impl SpottyfiApp {
    /// Build the app from eframe's creation context.
    ///
    /// Creates the tokio runtime, installs the bundled fonts and image
    /// loaders, restores the persisted shell layout / theme, spawns the
    /// startup session-restore, and prepares the audio engine. `no_audio`
    /// reflects the `--no-audio` CLI flag.
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        no_audio: bool,
        instance: InstanceGuard,
    ) -> anyhow::Result<Self> {
        tracing::debug!("constructing SpottyfiApp");

        // Fonts + image loaders. `egui_extras` provides the stock byte/decode
        // loaders; `spottyfi_ui` adds the network loader and the fonts.
        egui_extras::install_image_loaders(&cc.egui_ctx);
        spottyfi_ui::install_fonts_and_network_loader(&cc.egui_ctx);

        // Restore the persisted shell (dock layout, theme, sidebar state) and
        // apply the theme straight away.
        let mut shell = ShellState::load();
        shell.sync_theme(&cc.egui_ctx);

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("spottyfi-rt")
            .build()?;

        let auth = AuthController::new(runtime.handle().clone(), cc.egui_ctx.clone());
        // Startup: attempt to restore a session from the keyring.
        auth.spawn_restore();

        let playback =
            PlaybackControllerHandle::new(runtime.handle().clone(), cc.egui_ctx.clone(), no_audio);

        Ok(Self {
            _runtime: runtime,
            auth,
            playback,
            transport_ui: TransportUiState::default(),
            shell,
            avatar_image: Arc::new(ArcSwap::from_pointee(None)),
            avatar_texture: None,
            avatar_requested: false,
            api_attached: false,
            _instance: instance,
            media: MediaBridge::new(),
            media_started: false,
            notifier: media::notify::TrackChangeNotifier::new(),
            window_visible: true,
        })
    }

    /// Start the desktop-integration surfaces once, on the first frame.
    ///
    /// MPRIS runs on the tokio runtime; the tray and the media-key fallback
    /// each get their own thread. Every one is best-effort — a surface that
    /// cannot start logs and is skipped.
    fn ensure_media(&mut self) {
        if self.media_started {
            return;
        }
        self.media_started = true;

        let snapshot = self.media.snapshot();
        let sender = self.media.sender();

        // MPRIS2 D-Bus interface — desktop media controls and indicators.
        media::mpris::spawn(self._runtime.handle(), snapshot.clone(), sender.clone());
        // System tray icon + menu.
        media::tray::spawn(snapshot, sender.clone());
        // Global media-key fallback, registered against the persisted hotkeys.
        media::media_keys::spawn(sender, &self.shell.persisted.settings.hotkeys);
    }

    /// Apply one [`MediaCommand`] from MPRIS, the tray or a media key.
    fn apply_media_command(&mut self, ctx: &egui::Context, command: MediaCommand) {
        match command {
            MediaCommand::PlayPause => self.playback.toggle_play_pause(),
            MediaCommand::Play => {
                if !self.playback.state().playing {
                    self.playback.toggle_play_pause();
                }
            }
            MediaCommand::Pause | MediaCommand::Stop => {
                if self.playback.state().playing {
                    self.playback.toggle_play_pause();
                }
            }
            MediaCommand::Next => self.playback.next(),
            MediaCommand::Previous => self.playback.previous(),
            MediaCommand::SeekTo(position) => self.playback.seek(position),
            MediaCommand::SeekBy(offset_us) => {
                // Translate a signed microsecond offset into an absolute seek
                // against the live position, clamped at the track bounds.
                let state = self.playback.state();
                let current = state.position.as_micros() as i64;
                let target = (current + offset_us).max(0) as u64;
                let target = std::time::Duration::from_micros(target);
                let clamped = match &state.track {
                    Some(track) if !track.duration.is_zero() => target.min(track.duration),
                    _ => target,
                };
                self.playback.seek(clamped);
            }
            MediaCommand::SetVolume(volume) => self.playback.set_volume(volume),
            MediaCommand::SetShuffle(shuffle) => self.playback.set_shuffle(shuffle),
            MediaCommand::SetRepeat(mode) => self.playback.set_repeat(mode),
            MediaCommand::RaiseWindow => {
                self.window_visible = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            MediaCommand::ToggleWindow => {
                self.window_visible = !self.window_visible;
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(self.window_visible));
                if self.window_visible {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
            }
            MediaCommand::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
    }

    /// Build the Spotify Web API client from the live session and attach it to
    /// the shell the first time we see a logged-in session. The shell then
    /// builds its page registry and loads the sidebar's real playlists.
    fn ensure_api(&mut self, ctx: &egui::Context) {
        if self.api_attached {
            return;
        }
        let Some(session) = self.auth.session() else {
            return;
        };
        let client = spottyfi_api::SpotifyClient::new(&session);
        let api: std::sync::Arc<dyn spottyfi_api::SpotifyApi> = std::sync::Arc::new(client);
        self.shell
            .attach_api(api, self._runtime.handle().clone(), ctx.clone());
        self.api_attached = true;
    }

    /// Start the audio engine the first time we see a logged-in session.
    ///
    /// The engine is started with the persisted audio settings — librespot
    /// bakes the stream quality and normalisation into its `PlayerConfig` at
    /// connect time.
    fn ensure_audio(&mut self) {
        if let Some(session) = self.auth.session() {
            let settings = &self.shell.persisted.settings;
            let config = settings.audio.engine_config();
            let eq = settings.equalizer;
            // The engine starts with the user's persisted equaliser; both
            // arguments are only consumed on the one start that actually runs.
            self.playback
                .ensure_started(&session, config, (eq.enabled, eq.band_gains_db));
        }
    }

    /// Kick off a background avatar fetch the first time we see a logged-in
    /// session that has an avatar URL. Best-effort and non-blocking.
    fn ensure_avatar(&mut self, ctx: &egui::Context) {
        if self.avatar_requested {
            return;
        }
        let Some(session) = self.auth.session() else {
            return;
        };
        let Some(url) = session.profile().avatar_url.clone() else {
            return;
        };

        self.avatar_requested = true;
        avatar::spawn_fetch(
            self._runtime.handle(),
            ctx.clone(),
            url,
            Arc::clone(&self.avatar_image),
        );
    }

    /// Upload the decoded avatar to a texture once it is available.
    fn ensure_avatar_texture(&mut self, ctx: &egui::Context) {
        if self.avatar_texture.is_some() {
            return;
        }
        if let Some(image) = self.avatar_image.load_full().as_ref() {
            let texture =
                ctx.load_texture("user-avatar", image.clone(), egui::TextureOptions::LINEAR);
            self.avatar_texture = Some(texture);
        }
    }

    /// Apply a transport intent by dispatching it onto the playback engine.
    fn apply_transport_intent(&self, intent: TransportIntent) {
        match intent {
            TransportIntent::TogglePlayPause => self.playback.toggle_play_pause(),
            TransportIntent::Seek(position) => self.playback.seek(position),
            TransportIntent::SetVolume(volume) => self.playback.set_volume(volume),
            TransportIntent::PlayUri(uri) => self.playback.play_uri(uri),
            TransportIntent::Next => self.playback.next(),
            TransportIntent::Previous => self.playback.previous(),
            TransportIntent::PlayContext {
                uri,
                name,
                tracks,
                offset,
            } => self.playback.play_context(uri, name, tracks, offset),
            TransportIntent::PlayNext(track) => self.playback.play_next(track),
            TransportIntent::Enqueue(track) => self.playback.enqueue(track),
            TransportIntent::SkipToManual(index) => self.playback.skip_to_manual(index),
            TransportIntent::SkipToContext(index) => self.playback.skip_to_context(index),
            TransportIntent::ReorderManual { from, to } => {
                self.playback.reorder_manual(from, to);
            }
            TransportIntent::RemoveManual(index) => self.playback.remove_manual(index),
            TransportIntent::SetShuffle(shuffle) => self.playback.set_shuffle(shuffle),
            TransportIntent::SetRepeat(mode) => self.playback.set_repeat(mode),
        }
    }

    /// Tear down the session-scoped state on logout.
    fn handle_logout(&mut self) {
        self.auth.spawn_logout();
        self.playback.shutdown();
        // Drop the page registry and sidebar so a future login starts fresh.
        self.shell.detach_api();
        self.api_attached = false;
        // Drop the avatar so a future login fetches a fresh one.
        self.avatar_texture = None;
        self.avatar_requested = false;
        self.avatar_image.store(Arc::new(None));
        // Reset the notifier so a fresh login's first track is not a "change".
        self.notifier.reset();
        self.media.publish(
            &spottyfi_audio::PlaybackState::default(),
            &spottyfi_audio::QueueState::default(),
        );
    }
}

impl eframe::App for SpottyfiApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // Keep the egui style in sync with the (possibly just-changed) theme.
        self.shell.sync_theme(&ctx);
        let palette = self.shell.theme().palette();

        self.ensure_avatar(&ctx);
        self.ensure_avatar_texture(&ctx);
        self.ensure_audio();
        self.ensure_api(&ctx);
        self.ensure_media();

        // Apply any commands queued by MPRIS, the tray or a media key since
        // the last frame, before drawing this one.
        for command in self.media.drain() {
            self.apply_media_command(&ctx, command);
        }

        let auth_state = self.auth.state();

        match &*auth_state {
            spottyfi_auth::AuthState::LoggedIn(profile) => {
                let playback = self.playback.state();
                let queue = self.playback.queue_state();
                let engine = self.playback.status();

                // Publish the live state for the desktop integrations, and
                // fire a track-change notification if the user opted in.
                self.media.publish(&playback, &queue);
                let snapshot = media::MediaSnapshot::from_engine(&playback, &queue);
                self.notifier.observe(
                    &snapshot,
                    self.shell.persisted.settings.notifications.track_change,
                );

                // The live post-EQ audio envelope for the waveform scrubber —
                // a single lock-free atomic load, empty before the engine has
                // produced any audio (WS7).
                let waveform = self
                    .playback
                    .audio_tap()
                    .map(|tap| tap.snapshot().waveform.clone())
                    .unwrap_or_default();

                // The transport panel is added before the shell's central
                // dock so the dock fills the space above it.
                let transport_intent = transport::transport_bar(
                    ui,
                    &palette,
                    &mut self.transport_ui,
                    &playback,
                    &queue,
                    &waveform,
                );

                let shell_intent = shell::shell(
                    ui,
                    &mut self.shell,
                    profile,
                    self.avatar_texture.as_ref(),
                    &playback,
                    &queue,
                    &mut self.transport_ui,
                    &engine,
                    &self.playback.spectrum(),
                );

                if let Some(intent) = transport_intent {
                    self.apply_transport_intent(intent);
                }
                match shell_intent {
                    Some(ShellIntent::Logout) => self.handle_logout(),
                    Some(ShellIntent::Transport(intent)) => self.apply_transport_intent(intent),
                    Some(ShellIntent::AudioSettingsChanged) => {
                        // librespot bakes bitrate / normalisation into its
                        // `PlayerConfig` at connect, so restart the engine to
                        // apply the new audio settings.
                        let config = self.shell.persisted.settings.audio.engine_config();
                        self.playback.restart_with(config);
                    }
                    Some(ShellIntent::EqualizerChanged) => {
                        // The equaliser applies live — the custom backend's DSP
                        // picks the new gains up on its next decoded packet.
                        let eq = self.shell.persisted.settings.equalizer;
                        self.playback.set_equalizer(eq.enabled, eq.band_gains_db);
                    }
                    None => {}
                }
            }
            other => {
                if let Some(LoginIntent::Login) = login::login_screen(ui, &palette, other) {
                    self.auth.spawn_login();
                }
            }
        }
    }

    fn on_exit(&mut self) {
        // Persist the dock layout + settings so the next launch restores them.
        self.shell.save();
    }
}
