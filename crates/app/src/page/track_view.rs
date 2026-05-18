//! Shared helpers for the table-backed pages (playlist, album, liked songs).
//!
//! The `ui` crate's [`track_table`](spottyfi_ui::track_table) widget is purely
//! a renderer: it reports header sorts and row interactions but never mutates
//! data. This module supplies the page-side glue — sorting a row list by the
//! chosen column, and translating a [`TrackAction`] into a [`PageAction`].

use std::time::Duration;

use spottyfi_audio::QueueTrack;
use spottyfi_models::{SpotifyId as _, Track};
use spottyfi_ui::track_table::{SortColumn, TrackAction};

use crate::page::PageAction;
use crate::shell::Tab;

/// One row of a track page: a track plus the metadata the table shows.
#[derive(Debug, Clone)]
pub struct Entry {
    /// The track itself.
    pub track: Track,
    /// The RFC 3339 timestamp the track was added, if the page has one.
    pub added_at: Option<String>,
}

/// The playback context a page can play through: a playlist/album's own URI
/// and display name, used to seed [`PageAction::PlayContext`].
#[derive(Debug, Clone)]
pub struct PlayContext {
    /// The context's own Spotify URI (`spotify:playlist:…`, `spotify:album:…`).
    pub uri: String,
    /// The context's human-readable display name.
    pub name: String,
}

/// Project a [`Track`] into the `audio` crate's [`QueueTrack`].
///
/// Returns `None` for a track with no id (a local file) — the queue is
/// keyed by canonical `spotify:track:` URIs and cannot play those.
#[must_use]
pub fn to_queue_track(track: &Track) -> Option<QueueTrack> {
    let uri = track.id.as_ref()?.uri();
    Some(QueueTrack {
        uri,
        title: track.name.clone(),
        artists: track.artists.iter().map(|a| a.name.clone()).collect(),
        album: track.album.name.clone(),
        art_url: track.album.images.first().map(|i| i.url.clone()),
        duration: Duration::from_millis(u64::from(track.duration_ms)),
    })
}

/// Project every playable track in `entries` into [`QueueTrack`]s, keeping the
/// order. Local-file tracks (no id) are dropped.
#[must_use]
pub fn queue_tracks(entries: &[Entry]) -> Vec<QueueTrack> {
    entries
        .iter()
        .filter_map(|e| to_queue_track(&e.track))
        .collect()
}

/// Sort `entries` in place by `column`, ascending or descending.
///
/// The `Index` column restores the original order, so it relies on the caller
/// keeping `entries` in load order; pages re-derive the index from position.
pub fn sort_entries(
    entries: &mut [Entry],
    original: &[Entry],
    column: SortColumn,
    descending: bool,
) {
    match column {
        SortColumn::Index => {
            entries.clone_from_slice(original);
        }
        SortColumn::Title => {
            entries.sort_by(|a, b| {
                a.track
                    .name
                    .to_lowercase()
                    .cmp(&b.track.name.to_lowercase())
            });
        }
        SortColumn::Album => {
            entries.sort_by(|a, b| {
                a.track
                    .album
                    .name
                    .to_lowercase()
                    .cmp(&b.track.album.name.to_lowercase())
            });
        }
        SortColumn::DateAdded => {
            // RFC 3339 timestamps sort correctly as plain strings.
            entries.sort_by(|a, b| a.added_at.cmp(&b.added_at));
        }
        SortColumn::Duration => {
            entries.sort_by_key(|entry| entry.track.duration_ms);
        }
    }
    if descending {
        entries.reverse();
    }
}

/// Translate a track-table [`TrackAction`] into a page-level [`PageAction`].
///
/// `entries` is the *currently displayed* (possibly sorted) row list, so a
/// position-based action resolves to the right track. `context` names the
/// playback context the rows belong to: playing a row plays the whole list
/// (so Next/Prev walk it) starting at that row.
///
/// Returns `None` only for actions the page handles itself (a header `Sort`)
/// or for a row that cannot be played (a local file with no id).
pub fn resolve_action(
    action: TrackAction,
    entries: &[Entry],
    context: &PlayContext,
) -> Option<PageAction> {
    match action {
        TrackAction::Play(position) => {
            let offset = position.checked_sub(1)?;
            let tracks = queue_tracks(entries);
            if tracks.is_empty() {
                return None;
            }
            // `offset` is a position in the displayed list, but `tracks` drops
            // unplayable rows; clamp so the offset stays in range.
            let offset = offset.min(tracks.len() - 1);
            Some(PageAction::PlayContext {
                uri: context.uri.clone(),
                name: context.name.clone(),
                tracks,
                offset,
            })
        }
        TrackAction::CopyUri(position) => {
            let track = track_at(entries, position)?;
            track
                .id
                .as_ref()
                .map(|id| PageAction::CopyToClipboard(id.uri()))
        }
        TrackAction::GoToAlbum(id) => Some(PageAction::Open(Tab::Album(id))),
        TrackAction::GoToArtist(id) => Some(PageAction::Open(Tab::Artist(id))),
        TrackAction::PlayNext(position) => {
            let track = track_at(entries, position)?;
            to_queue_track(track).map(PageAction::PlayNext)
        }
        TrackAction::AddToQueue(position) => {
            let track = track_at(entries, position)?;
            to_queue_track(track).map(PageAction::Enqueue)
        }
        TrackAction::SaveToLiked(position) => {
            let track = track_at(entries, position)?;
            track
                .id
                .as_ref()
                .map(|id| PageAction::SaveTrack(id.id().to_owned()))
        }
        TrackAction::RemoveFromLiked(position) => {
            let track = track_at(entries, position)?;
            track
                .id
                .as_ref()
                .map(|id| PageAction::UnsaveTrack(id.id().to_owned()))
        }
        // The page applies header sorts itself; nothing to dispatch.
        TrackAction::Sort(_) => None,
    }
}

/// The track at a 1-based table position within `entries`.
fn track_at(entries: &[Entry], position: usize) -> Option<&Track> {
    position
        .checked_sub(1)
        .and_then(|index| entries.get(index))
        .map(|entry| &entry.track)
}

#[cfg(test)]
mod tests {
    use super::*;
    use spottyfi_models::{Image, SimplifiedAlbum, SimplifiedArtist, TrackId};

    fn track(name: &str, album: &str, duration_ms: u32) -> Track {
        Track {
            id: Some(TrackId::new(format!("id-{name}"))),
            name: name.to_owned(),
            artists: vec![SimplifiedArtist {
                id: None,
                name: "Artist".to_owned(),
            }],
            album: SimplifiedAlbum {
                id: None,
                name: album.to_owned(),
                images: Vec::<Image>::new(),
                artists: Vec::new(),
                release_date: None,
            },
            duration_ms,
            explicit: false,
            popularity: 0,
            track_number: 1,
            is_local: false,
        }
    }

    fn entries() -> Vec<Entry> {
        vec![
            Entry {
                track: track("Bravo", "Zeta", 200_000),
                added_at: Some("2024-02-01T00:00:00Z".to_owned()),
            },
            Entry {
                track: track("Alpha", "Yotta", 100_000),
                added_at: Some("2024-01-01T00:00:00Z".to_owned()),
            },
            Entry {
                track: track("Charlie", "Xenon", 300_000),
                added_at: Some("2024-03-01T00:00:00Z".to_owned()),
            },
        ]
    }

    #[test]
    fn sorts_by_title_ascending_and_descending() {
        let original = entries();
        let mut rows = original.clone();
        sort_entries(&mut rows, &original, SortColumn::Title, false);
        assert_eq!(rows[0].track.name, "Alpha");
        sort_entries(&mut rows, &original, SortColumn::Title, true);
        assert_eq!(rows[0].track.name, "Charlie");
    }

    #[test]
    fn sorts_by_duration_and_date() {
        let original = entries();
        let mut rows = original.clone();
        sort_entries(&mut rows, &original, SortColumn::Duration, false);
        assert_eq!(rows[0].track.duration_ms, 100_000);
        sort_entries(&mut rows, &original, SortColumn::DateAdded, false);
        assert_eq!(rows[0].added_at.as_deref(), Some("2024-01-01T00:00:00Z"));
    }

    #[test]
    fn index_sort_restores_original_order() {
        let original = entries();
        let mut rows = original.clone();
        sort_entries(&mut rows, &original, SortColumn::Title, false);
        sort_entries(&mut rows, &original, SortColumn::Index, false);
        assert_eq!(rows[0].track.name, "Bravo");
    }

    fn ctx() -> PlayContext {
        PlayContext {
            uri: "spotify:playlist:ctx".to_owned(),
            name: "Ctx".to_owned(),
        }
    }

    #[test]
    fn play_resolves_to_a_context_starting_at_the_displayed_track() {
        let rows = entries();
        let action = resolve_action(TrackAction::Play(2), &rows, &ctx());
        match action {
            Some(PageAction::PlayContext {
                uri,
                tracks,
                offset,
                ..
            }) => {
                assert_eq!(uri, "spotify:playlist:ctx");
                assert_eq!(offset, 1);
                assert_eq!(tracks.len(), 3);
                assert_eq!(tracks[1].uri, "spotify:track:id-Alpha");
            }
            other => panic!("expected PlayContext, got {other:?}"),
        }
    }

    #[test]
    fn queue_actions_resolve_to_queue_tracks() {
        let rows = entries();
        match resolve_action(TrackAction::PlayNext(1), &rows, &ctx()) {
            Some(PageAction::PlayNext(track)) => {
                assert_eq!(track.uri, "spotify:track:id-Bravo");
            }
            other => panic!("expected PlayNext, got {other:?}"),
        }
        match resolve_action(TrackAction::AddToQueue(3), &rows, &ctx()) {
            Some(PageAction::Enqueue(track)) => {
                assert_eq!(track.uri, "spotify:track:id-Charlie");
            }
            other => panic!("expected Enqueue, got {other:?}"),
        }
    }

    #[test]
    fn navigation_actions_open_the_right_tab() {
        let rows = entries();
        assert_eq!(
            resolve_action(TrackAction::GoToAlbum("abc".into()), &rows, &ctx()),
            Some(PageAction::Open(Tab::Album("abc".into())))
        );
    }
}
