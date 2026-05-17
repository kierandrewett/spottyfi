//! The queue and playback-context state machine.
//!
//! librespot's [`Player`](librespot::playback::player::Player) plays exactly
//! **one track at a time** — it has no notion of a queue or a playlist.
//! Spotify Connect's `spirc` owns that state upstream, and Spottyfi
//! deliberately does not use Connect (local playback only). So Spottyfi must
//! own the queue itself; this module is that owner.
//!
//! A [`Queue`] holds two things:
//!
//! * a **context** — the ordered track list of the playlist/album/artist the
//!   user is playing through, plus a cursor (`context_index`) into it and the
//!   context's own URI and display name;
//! * a **manual queue** — a FIFO of tracks the user explicitly queued. Manual
//!   entries always play *before* the context resumes.
//!
//! [`Queue::advance`] (the engine's auto-advance, and the transport's "next")
//! drains the manual queue first, then walks the context. [`Queue::previous`]
//! steps back through the context only — Spotify's "previous" never revisits
//! the manual queue.

use std::time::Duration;

use rand::seq::SliceRandom;

/// A single playable entry in the queue or a context.
///
/// Carries just enough metadata for the queue panel to render a row without
/// another API round-trip; the canonical source is still the Web API, but the
/// caller (`app`) resolves a context's tracks once and hands them here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueTrack {
    /// The track's canonical Spotify URI (`spotify:track:…`).
    pub uri: String,
    /// Track title.
    pub title: String,
    /// Contributing artist names, in billing order.
    pub artists: Vec<String>,
    /// Album name, if known.
    pub album: String,
    /// URL of the album cover art, if any.
    pub art_url: Option<String>,
    /// Total track duration.
    pub duration: Duration,
}

impl QueueTrack {
    /// The artists joined into a single display string (`"A, B"`).
    #[must_use]
    pub fn artist_line(&self) -> String {
        self.artists.join(", ")
    }
}

/// How playback behaves when it reaches the end of the queue.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RepeatMode {
    /// Stop when the manual queue and context are both exhausted.
    #[default]
    Off,
    /// Loop back to the start of the context at the end.
    Context,
    /// Repeat the current track indefinitely.
    Track,
}

impl RepeatMode {
    /// The next mode in the `Off → Context → Track → Off` cycle.
    #[must_use]
    pub fn cycled(self) -> Self {
        match self {
            RepeatMode::Off => RepeatMode::Context,
            RepeatMode::Context => RepeatMode::Track,
            RepeatMode::Track => RepeatMode::Off,
        }
    }
}

/// The playback context: the ordered track list being played through.
///
/// The track list is stored once, in its original order. Play order is a
/// separate `order` permutation of indices into `tracks`: identity when
/// shuffle is off, a random permutation when it is on. The cursor is a
/// position *within `order`*, so toggling shuffle never disturbs the stored
/// list and the current track can always be kept put.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Context {
    /// The context's own Spotify URI (`spotify:playlist:…`, `spotify:album:…`).
    uri: String,
    /// The context's human-readable display name.
    name: String,
    /// Every track in the context, in the original (unshuffled) order.
    tracks: Vec<QueueTrack>,
    /// The play-order permutation: `order[i]` is an index into `tracks`.
    /// Identity (`0,1,2,…`) when shuffle is off.
    order: Vec<usize>,
    /// The cursor: position within `order` of the *current* context track.
    /// `None` before a context track has been played (i.e. while a
    /// manual-queue track is playing and the context has not started).
    index: Option<usize>,
}

impl Context {
    /// The `tracks` index of the current context track, if any.
    fn current_track_index(&self) -> Option<usize> {
        self.index
            .and_then(|cursor| self.order.get(cursor).copied())
    }
}

/// The queue + context state machine.
///
/// Owned by the audio side; the [`PlaybackController`](crate::PlaybackController)
/// holds one behind a mutex and publishes a [`QueueState`] snapshot for the UI.
#[derive(Debug, Clone, Default)]
pub struct Queue {
    /// The playback context, empty when nothing context-backed is playing.
    context: Context,
    /// The manual queue — user-added tracks that play before the context
    /// resumes. Front is next-up.
    manual: Vec<QueueTrack>,
    /// The track currently loaded in the player, if any.
    current: Option<QueueTrack>,
    /// Whether `current` came from the manual queue (`true`) or the context.
    current_from_manual: bool,
    /// The repeat mode.
    repeat: RepeatMode,
    /// Whether the context plays in a shuffled order.
    shuffle: bool,
}

impl Queue {
    /// Build an empty queue.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The track currently loaded in the player.
    #[must_use]
    pub fn current(&self) -> Option<&QueueTrack> {
        self.current.as_ref()
    }

    /// The repeat mode.
    #[must_use]
    pub fn repeat(&self) -> RepeatMode {
        self.repeat
    }

    /// Set the repeat mode.
    pub fn set_repeat(&mut self, mode: RepeatMode) {
        self.repeat = mode;
    }

    /// Whether the context plays in a shuffled order.
    #[must_use]
    pub fn shuffle(&self) -> bool {
        self.shuffle
    }

    /// Turn shuffle on or off, rebuilding the context's play order.
    ///
    /// The currently-playing context track is **preserved**: when shuffle is
    /// switched on it becomes the first entry of the new shuffled order, so
    /// playback continues uninterrupted and only the *upcoming* tracks are
    /// reordered. Switching shuffle off restores the original track order with
    /// the cursor moved to wherever the current track sits in it.
    pub fn set_shuffle(&mut self, shuffle: bool) {
        if self.shuffle == shuffle {
            return;
        }
        self.shuffle = shuffle;
        self.rebuild_context_order();
    }

    /// Rebuild [`Context::order`] for the current `shuffle` flag, keeping the
    /// current context track at the cursor.
    fn rebuild_context_order(&mut self) {
        let len = self.context.tracks.len();
        if len == 0 {
            self.context.order = Vec::new();
            self.context.index = None;
            return;
        }

        // The `tracks`-index the cursor currently points at, if any.
        let current = self.context.current_track_index();

        if self.shuffle {
            let mut rest: Vec<usize> = (0..len).filter(|i| Some(*i) != current).collect();
            rest.shuffle(&mut rand::rng());
            let mut order = Vec::with_capacity(len);
            // Pin the current track to the front so playback is uninterrupted.
            if let Some(current) = current {
                order.push(current);
            }
            order.extend(rest);
            self.context.order = order;
            self.context.index = current.map(|_| 0);
        } else {
            self.context.order = (0..len).collect();
            // The identity permutation maps each cursor straight to its track.
            self.context.index = current;
        }
    }

    /// Replace the context and start playing at `offset`.
    ///
    /// Returns the track that should now be loaded into the player, or `None`
    /// when `tracks` is empty. `offset` is clamped into range.
    pub fn play_context(
        &mut self,
        uri: String,
        name: String,
        tracks: Vec<QueueTrack>,
        offset: usize,
    ) -> Option<QueueTrack> {
        if tracks.is_empty() {
            self.context = Context::default();
            return None;
        }
        let len = tracks.len();
        let start = offset.min(len - 1);
        // Build the play order. With shuffle on, the chosen track leads and the
        // rest is shuffled behind it; otherwise it is the identity order.
        let (order, index) = if self.shuffle {
            let mut rest: Vec<usize> = (0..len).filter(|i| *i != start).collect();
            rest.shuffle(&mut rand::rng());
            let mut order = Vec::with_capacity(len);
            order.push(start);
            order.extend(rest);
            (order, 0)
        } else {
            ((0..len).collect(), start)
        };
        self.context = Context {
            uri,
            name,
            tracks,
            order,
            index: Some(index),
        };
        self.current = self
            .context
            .current_track_index()
            .map(|i| self.context.tracks[i].clone());
        self.current_from_manual = false;
        self.current.clone()
    }

    /// Set the current track to a single, context-free track (`play_uri`).
    ///
    /// Clears the context so Next/Prev have nothing to walk — a one-off play.
    pub fn play_single(&mut self, track: QueueTrack) {
        self.context = Context::default();
        self.current = Some(track);
        self.current_from_manual = false;
    }

    /// Add `track` to the end of the manual queue.
    pub fn enqueue(&mut self, track: QueueTrack) {
        self.manual.push(track);
    }

    /// Add `track` to the front of the manual queue — it plays next.
    pub fn play_next(&mut self, track: QueueTrack) {
        self.manual.insert(0, track);
    }

    /// Advance to the next track and return it, or `None` at the end.
    ///
    /// The manual queue takes priority: if it is non-empty its front entry is
    /// popped and becomes current. Otherwise the context cursor steps forward.
    /// With [`RepeatMode::Track`] the current track is returned unchanged; with
    /// [`RepeatMode::Context`] the cursor wraps to the start at the end.
    pub fn advance(&mut self) -> Option<QueueTrack> {
        if self.repeat == RepeatMode::Track {
            if let Some(current) = &self.current {
                return Some(current.clone());
            }
        }

        if !self.manual.is_empty() {
            let next = self.manual.remove(0);
            self.current = Some(next.clone());
            self.current_from_manual = true;
            return Some(next);
        }

        self.advance_context()
    }

    /// Step the context cursor forward, honouring [`RepeatMode::Context`].
    ///
    /// The cursor walks the `order` permutation, so the steps follow the
    /// shuffled order when shuffle is on.
    fn advance_context(&mut self) -> Option<QueueTrack> {
        let len = self.context.order.len();
        if len == 0 {
            return None;
        }
        // The cursor `index` is the *current* position in `order`. If a manual
        // track was just playing, it still points at the last context track
        // played, so the next is `index + 1`.
        let next = match self.context.index {
            Some(i) if i + 1 < len => i + 1,
            Some(_) => {
                if self.repeat == RepeatMode::Context {
                    0
                } else {
                    return None;
                }
            }
            None => 0,
        };
        self.context.index = Some(next);
        self.current = self
            .context
            .current_track_index()
            .map(|i| self.context.tracks[i].clone());
        self.current_from_manual = false;
        self.current.clone()
    }

    /// Step back to the previous context track and return it, or `None`.
    ///
    /// "Previous" walks the context only — it never revisits manual entries,
    /// matching Spotify's behaviour. At the start of the context it stays put
    /// (the caller typically restarts the current track instead).
    pub fn previous(&mut self) -> Option<QueueTrack> {
        let prev = match self.context.index {
            Some(i) if i > 0 => i - 1,
            _ => return None,
        };
        self.context.index = Some(prev);
        self.current = self
            .context
            .current_track_index()
            .map(|i| self.context.tracks[i].clone());
        self.current_from_manual = false;
        self.current.clone()
    }

    /// Jump straight to manual-queue entry `index`, dropping the entries before
    /// it. Returns the track now playing, or `None` if `index` is out of range.
    pub fn skip_to_manual(&mut self, index: usize) -> Option<QueueTrack> {
        if index >= self.manual.len() {
            return None;
        }
        // Drop everything up to and including `index`; the entry at `index`
        // becomes the current track.
        let track = self.manual.drain(..=index).next_back()?;
        self.current = Some(track.clone());
        self.current_from_manual = true;
        Some(track)
    }

    /// Jump straight to context entry `index`. Returns the track now playing,
    /// or `None` if `index` is out of range.
    ///
    /// `index` is a position in the (possibly shuffled) play order — the same
    /// space as [`QueueState::context_index`] — so a click on an upcoming
    /// entry maps directly here.
    pub fn skip_to_context(&mut self, index: usize) -> Option<QueueTrack> {
        let track_index = self.context.order.get(index).copied()?;
        let track = self.context.tracks.get(track_index).cloned()?;
        self.context.index = Some(index);
        self.current = Some(track.clone());
        self.current_from_manual = false;
        Some(track)
    }

    /// Move manual-queue entry `from` to `to`, shifting the rest — the
    /// drag-to-reorder primitive for the queue panel.
    pub fn reorder_manual(&mut self, from: usize, to: usize) {
        if from >= self.manual.len() || to >= self.manual.len() || from == to {
            return;
        }
        let track = self.manual.remove(from);
        self.manual.insert(to, track);
    }

    /// Remove manual-queue entry `index`.
    pub fn remove_manual(&mut self, index: usize) {
        if index < self.manual.len() {
            self.manual.remove(index);
        }
    }

    /// Build the immutable [`QueueState`] snapshot the UI reads each frame.
    #[must_use]
    pub fn snapshot(&self) -> QueueState {
        QueueState {
            current: self.current.clone(),
            context_uri: if self.context.tracks.is_empty() {
                None
            } else {
                Some(self.context.uri.clone())
            },
            context_name: if self.context.tracks.is_empty() {
                None
            } else {
                Some(self.context.name.clone())
            },
            up_next_context: self.upcoming_context(),
            context_index: self.context.index,
            manual: self.manual.clone(),
            repeat: self.repeat,
            shuffle: self.shuffle,
        }
    }

    /// The context tracks that come *after* the current cursor position, in
    /// play order (shuffled when shuffle is on).
    fn upcoming_context(&self) -> Vec<QueueTrack> {
        let start = match self.context.index {
            Some(i) => i + 1,
            // No context track played yet — the whole context is upcoming.
            None => 0,
        };
        self.context
            .order
            .get(start..)
            .unwrap_or(&[])
            .iter()
            .filter_map(|&track_index| self.context.tracks.get(track_index).cloned())
            .collect()
    }
}

/// An immutable snapshot of the queue, published for the UI each frame.
///
/// Mirrors the [`PlaybackState`](crate::PlaybackState) pattern: the controller
/// swaps a fresh `QueueState` into an `ArcSwap` whenever the queue changes, and
/// the queue panel reads it without ever touching the live [`Queue`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QueueState {
    /// The track currently loaded in the player.
    pub current: Option<QueueTrack>,
    /// The current context's URI, if a context is playing.
    pub context_uri: Option<String>,
    /// The current context's display name, if a context is playing.
    pub context_name: Option<String>,
    /// Upcoming context tracks (the "Next from <context>" section).
    pub up_next_context: Vec<QueueTrack>,
    /// The context cursor — the index of the current track within the
    /// context's track list, or `None` before a context track has played.
    /// The queue panel adds `1 + offset` to this to map a click on an
    /// upcoming-context entry to an absolute context index.
    pub context_index: Option<usize>,
    /// The manual queue, front-first (the "Queue" section).
    pub manual: Vec<QueueTrack>,
    /// The repeat mode.
    pub repeat: RepeatMode,
    /// Whether the context plays in a shuffled order.
    pub shuffle: bool,
}

impl QueueState {
    /// Whether there is anything beyond the current track to play.
    #[must_use]
    pub fn has_upcoming(&self) -> bool {
        !self.manual.is_empty() || !self.up_next_context.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(uri: &str) -> QueueTrack {
        QueueTrack {
            uri: uri.to_owned(),
            title: uri.to_owned(),
            artists: vec!["Artist".to_owned()],
            album: "Album".to_owned(),
            art_url: None,
            duration: Duration::from_secs(180),
        }
    }

    fn context(n: usize) -> Vec<QueueTrack> {
        (0..n).map(|i| track(&format!("t{i}"))).collect()
    }

    #[test]
    fn play_context_starts_at_offset() {
        let mut q = Queue::new();
        let started = q.play_context("ctx".into(), "Ctx".into(), context(5), 2);
        assert_eq!(started.unwrap().uri, "t2");
        assert_eq!(q.current().unwrap().uri, "t2");
    }

    #[test]
    fn play_context_clamps_offset() {
        let mut q = Queue::new();
        let started = q.play_context("ctx".into(), "Ctx".into(), context(3), 99);
        assert_eq!(started.unwrap().uri, "t2");
    }

    #[test]
    fn play_context_empty_is_none() {
        let mut q = Queue::new();
        assert!(q
            .play_context("ctx".into(), "Ctx".into(), vec![], 0)
            .is_none());
    }

    #[test]
    fn advance_walks_the_context() {
        let mut q = Queue::new();
        q.play_context("ctx".into(), "Ctx".into(), context(3), 0);
        assert_eq!(q.advance().unwrap().uri, "t1");
        assert_eq!(q.advance().unwrap().uri, "t2");
        // End of context, repeat off — stop.
        assert!(q.advance().is_none());
    }

    #[test]
    fn advance_drains_manual_queue_before_context() {
        let mut q = Queue::new();
        q.play_context("ctx".into(), "Ctx".into(), context(3), 0);
        q.enqueue(track("m0"));
        q.enqueue(track("m1"));
        // Manual queue is exhausted first.
        assert_eq!(q.advance().unwrap().uri, "m0");
        assert_eq!(q.advance().unwrap().uri, "m1");
        // Then the context resumes one past where it was (t0 -> t1).
        assert_eq!(q.advance().unwrap().uri, "t1");
    }

    #[test]
    fn play_next_jumps_the_queue() {
        let mut q = Queue::new();
        q.play_context("ctx".into(), "Ctx".into(), context(2), 0);
        q.enqueue(track("m-tail"));
        q.play_next(track("m-head"));
        assert_eq!(q.advance().unwrap().uri, "m-head");
        assert_eq!(q.advance().unwrap().uri, "m-tail");
    }

    #[test]
    fn previous_walks_back_through_context() {
        let mut q = Queue::new();
        q.play_context("ctx".into(), "Ctx".into(), context(4), 2);
        assert_eq!(q.previous().unwrap().uri, "t1");
        assert_eq!(q.previous().unwrap().uri, "t0");
        // At the start — nothing earlier.
        assert!(q.previous().is_none());
    }

    #[test]
    fn previous_ignores_the_manual_queue() {
        let mut q = Queue::new();
        q.play_context("ctx".into(), "Ctx".into(), context(3), 1);
        q.enqueue(track("m0"));
        q.advance(); // plays m0
        assert_eq!(q.current().unwrap().uri, "m0");
        // Previous steps the context back from index 1 to index 0.
        assert_eq!(q.previous().unwrap().uri, "t0");
    }

    #[test]
    fn repeat_track_replays_current() {
        let mut q = Queue::new();
        q.play_context("ctx".into(), "Ctx".into(), context(3), 0);
        q.set_repeat(RepeatMode::Track);
        assert_eq!(q.advance().unwrap().uri, "t0");
        assert_eq!(q.advance().unwrap().uri, "t0");
    }

    #[test]
    fn repeat_context_wraps_at_the_end() {
        let mut q = Queue::new();
        q.play_context("ctx".into(), "Ctx".into(), context(2), 0);
        q.set_repeat(RepeatMode::Context);
        assert_eq!(q.advance().unwrap().uri, "t1");
        assert_eq!(q.advance().unwrap().uri, "t0");
    }

    #[test]
    fn skip_to_manual_drops_skipped_entries() {
        let mut q = Queue::new();
        q.enqueue(track("m0"));
        q.enqueue(track("m1"));
        q.enqueue(track("m2"));
        assert_eq!(q.skip_to_manual(1).unwrap().uri, "m1");
        // m0 was dropped along with m1; m2 remains.
        let snap = q.snapshot();
        assert_eq!(snap.manual.len(), 1);
        assert_eq!(snap.manual[0].uri, "m2");
    }

    #[test]
    fn skip_to_context_moves_the_cursor() {
        let mut q = Queue::new();
        q.play_context("ctx".into(), "Ctx".into(), context(5), 0);
        assert_eq!(q.skip_to_context(3).unwrap().uri, "t3");
        assert_eq!(q.advance().unwrap().uri, "t4");
    }

    #[test]
    fn reorder_manual_moves_an_entry() {
        let mut q = Queue::new();
        q.enqueue(track("a"));
        q.enqueue(track("b"));
        q.enqueue(track("c"));
        q.reorder_manual(2, 0);
        let uris: Vec<_> = q.snapshot().manual.iter().map(|t| t.uri.clone()).collect();
        assert_eq!(uris, ["c", "a", "b"]);
    }

    #[test]
    fn snapshot_reports_upcoming_context() {
        let mut q = Queue::new();
        q.play_context("ctx".into(), "Cool Playlist".into(), context(4), 1);
        q.enqueue(track("m0"));
        let snap = q.snapshot();
        assert_eq!(snap.context_name.as_deref(), Some("Cool Playlist"));
        assert_eq!(snap.current.as_ref().unwrap().uri, "t1");
        assert_eq!(snap.manual.len(), 1);
        // Upcoming context is everything past index 1: t2, t3.
        let upcoming: Vec<_> = snap.up_next_context.iter().map(|t| t.uri.clone()).collect();
        assert_eq!(upcoming, ["t2", "t3"]);
        assert!(snap.has_upcoming());
    }

    #[test]
    fn snapshot_exposes_the_context_cursor() {
        let mut q = Queue::new();
        assert_eq!(q.snapshot().context_index, None);
        q.play_context("ctx".into(), "Ctx".into(), context(4), 2);
        assert_eq!(q.snapshot().context_index, Some(2));
        // The first upcoming track sits at cursor + 1.
        let snap = q.snapshot();
        assert_eq!(snap.context_index.map(|i| i + 1), Some(3));
        assert_eq!(snap.up_next_context[0].uri, "t3");
    }

    #[test]
    fn shuffle_preserves_the_current_track() {
        let mut q = Queue::new();
        q.play_context("ctx".into(), "Ctx".into(), context(20), 7);
        assert_eq!(q.current().unwrap().uri, "t7");
        q.set_shuffle(true);
        // Toggling shuffle never disturbs what is playing now.
        assert_eq!(q.current().unwrap().uri, "t7");
        assert!(q.shuffle());
        // Switching back off keeps the current track too.
        q.set_shuffle(false);
        assert_eq!(q.current().unwrap().uri, "t7");
        assert!(!q.shuffle());
    }

    #[test]
    fn shuffle_off_restores_original_order() {
        let mut q = Queue::new();
        q.play_context("ctx".into(), "Ctx".into(), context(6), 2);
        q.set_shuffle(true);
        q.set_shuffle(false);
        // From t2, the unshuffled order resumes: t3, t4, t5.
        assert_eq!(q.advance().unwrap().uri, "t3");
        assert_eq!(q.advance().unwrap().uri, "t4");
        assert_eq!(q.advance().unwrap().uri, "t5");
    }

    #[test]
    fn shuffle_covers_every_track_once() {
        let mut q = Queue::new();
        q.play_context("ctx".into(), "Ctx".into(), context(30), 0);
        q.set_shuffle(true);
        let mut seen = vec![q.current().unwrap().uri.clone()];
        while let Some(track) = q.advance() {
            seen.push(track.uri);
        }
        seen.sort();
        seen.dedup();
        // Every one of the 30 tracks appears exactly once.
        assert_eq!(seen.len(), 30);
    }

    #[test]
    fn shuffle_at_play_context_starts_with_offset_track() {
        let mut q = Queue::new();
        q.set_shuffle(true);
        let started = q.play_context("ctx".into(), "Ctx".into(), context(10), 4);
        // The chosen offset track still leads the shuffled order.
        assert_eq!(started.unwrap().uri, "t4");
        assert!(q.shuffle());
    }

    #[test]
    fn shuffle_upcoming_matches_play_order() {
        let mut q = Queue::new();
        q.play_context("ctx".into(), "Ctx".into(), context(8), 0);
        q.set_shuffle(true);
        let snap = q.snapshot();
        assert!(snap.shuffle);
        // The first upcoming entry is the next track `advance` will return.
        let first_upcoming = snap.up_next_context[0].uri.clone();
        assert_eq!(q.advance().unwrap().uri, first_upcoming);
    }

    #[test]
    fn play_single_clears_the_context() {
        let mut q = Queue::new();
        q.play_context("ctx".into(), "Ctx".into(), context(3), 0);
        q.play_single(track("one-off"));
        assert_eq!(q.current().unwrap().uri, "one-off");
        // No context to walk.
        assert!(q.advance().is_none());
        assert!(q.snapshot().context_uri.is_none());
    }
}
