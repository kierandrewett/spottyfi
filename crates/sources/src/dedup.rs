//! Cross-source de-duplication.
//!
//! Searching every library at once would otherwise show the same song three
//! times. These functions group entities that are "the same" across sources
//! into one [`Deduped`] entry — a best **primary** plus ranked
//! **alternatives** — so the UI shows one row and the player can switch which
//! source actually plays it.
//!
//! Matching is by a recording id when both sides carry one — MusicBrainz id,
//! then ISRC (Spotify and Apple Music both expose ISRC, so it bridges them) —
//! and otherwise by a normalised title/name + artist key that ignores casing,
//! punctuation and noise like `(feat. …)` or `- 2011 Remaster`.

use std::collections::HashMap;
use std::hash::Hash;

use crate::entity::{Album, Artist, Track};

/// One entity that exists on one or more sources after de-duplication.
#[derive(Debug, Clone)]
pub struct Deduped<T> {
    /// The best source for this entity — what the UI shows and plays first.
    pub primary: T,
    /// The same entity on other sources, ranked best-first.
    pub alternatives: Vec<T>,
}

impl<T> Deduped<T> {
    /// The primary followed by every alternative.
    pub fn all(&self) -> impl Iterator<Item = &T> {
        std::iter::once(&self.primary).chain(self.alternatives.iter())
    }

    /// How many sources carry this entity.
    #[must_use]
    pub fn source_count(&self) -> usize {
        1 + self.alternatives.len()
    }
}

/// A track de-duplicated across sources.
pub type DedupedTrack = Deduped<Track>;

impl DedupedTrack {
    /// The best source that can actually be played, if any — preferring the
    /// primary, then the alternatives in rank order.
    #[must_use]
    pub fn best_playable(&self) -> Option<&Track> {
        self.all().find(|track| track.playable)
    }
}

/// Words that mark a parenthetical or trailing segment as noise rather than a
/// distinct version — stripped before building a fuzzy match key.
const NOISE_WORDS: &[&str] = &[
    "feat",
    "ft",
    "featuring",
    "remaster",
    "remastered",
    "explicit",
    "deluxe",
    "bonus track",
    "album version",
    "single version",
    "radio edit",
    "mono version",
    "stereo version",
];

/// Words that mark a segment as a *distinct recording* — a segment carrying
/// any of these is kept even if it also contains a noise word (e.g. a stray
/// `(feat. … live)`), so a live/acoustic take never merges onto the studio
/// version.
const DISTINCTIVE_WORDS: &[&str] = &[
    "live",
    "acoustic",
    "demo",
    "remix",
    "instrumental",
    "unplugged",
    "session",
    "reprise",
];

/// De-duplicate tracks across sources.
///
/// Group order follows first appearance, so a caller can preserve a relevance
/// ranking from search. Within a group the primary is the most preferred:
/// playable beats unplayable, then [`SourceKind::dedup_priority`] decides.
#[must_use]
pub fn dedup_tracks(tracks: Vec<Track>) -> Vec<DedupedTrack> {
    dedup(tracks, track_key, |track| {
        (track.playable, track.source.kind.dedup_priority())
    })
}

/// De-duplicate albums across sources.
#[must_use]
pub fn dedup_albums(albums: Vec<Album>) -> Vec<Deduped<Album>> {
    dedup(albums, album_key, |album| {
        album.source.kind.dedup_priority()
    })
}

/// De-duplicate artists across sources.
#[must_use]
pub fn dedup_artists(artists: Vec<Artist>) -> Vec<Deduped<Artist>> {
    dedup(artists, artist_key, |artist| {
        artist.source.kind.dedup_priority()
    })
}

/// The generic grouping core: bucket `items` by `key_of`, then within each
/// bucket sort by `rank_of` descending so the best becomes the primary.
fn dedup<T, K, R>(
    items: Vec<T>,
    key_of: impl Fn(&T) -> K,
    rank_of: impl Fn(&T) -> R,
) -> Vec<Deduped<T>>
where
    K: Eq + Hash + Clone,
    R: Ord,
{
    let mut order: Vec<K> = Vec::new();
    let mut groups: HashMap<K, Vec<T>> = HashMap::new();
    for item in items {
        let key = key_of(&item);
        if !groups.contains_key(&key) {
            order.push(key.clone());
        }
        groups.entry(key).or_default().push(item);
    }
    order
        .into_iter()
        .filter_map(|key| {
            let mut group = groups.remove(&key)?;
            // Sort best-first: highest rank becomes the primary.
            group.sort_by_key(|item| std::cmp::Reverse(rank_of(item)));
            if group.is_empty() {
                return None;
            }
            let primary = group.remove(0);
            Some(Deduped {
                primary,
                alternatives: group,
            })
        })
        .collect()
}

/// The match key for a track: a recording id (MusicBrainz, then ISRC) when
/// present, else a fuzzy title + primary-artist key.
///
/// Recording ids are case-insensitive, so they are lower-cased first. A
/// track with no artist cannot be safely fuzzy-matched (two unrelated songs
/// could share a title), so it is keyed uniquely and never de-duplicated.
fn track_key(track: &Track) -> String {
    if let Some(mbid) = &track.mbid {
        return format!("mbid:{}", mbid.to_lowercase());
    }
    if let Some(isrc) = &track.isrc {
        return format!("isrc:{}", isrc.to_lowercase());
    }
    let artist = normalize_name(&track.artist);
    if artist.is_empty() {
        return unique_key(&track.source.source.0, &track.source.id);
    }
    format!("fuzzy:{}|{artist}", normalize_title(&track.title))
}

/// The match key for an album: MusicBrainz id, else album name + artist.
fn album_key(album: &Album) -> String {
    if let Some(mbid) = &album.mbid {
        return format!("mbid:{}", mbid.to_lowercase());
    }
    let artist = normalize_name(&album.artist);
    if artist.is_empty() {
        return unique_key(&album.source.source.0, &album.source.id);
    }
    format!("fuzzy:{}|{artist}", normalize_title(&album.name))
}

/// The match key for an artist: MusicBrainz id, else the normalised name.
fn artist_key(artist: &Artist) -> String {
    if let Some(mbid) = &artist.mbid {
        return format!("mbid:{}", mbid.to_lowercase());
    }
    let name = normalize_name(&artist.name);
    if name.is_empty() {
        return unique_key(&artist.source.source.0, &artist.source.id);
    }
    format!("fuzzy:{name}")
}

/// A key that never collides — used for entities too sparse to match safely.
///
/// The source component is length-prefixed: `SourceId`s routinely contain a
/// `:` (`subsonic:<uuid>`), so a plain `source:id` join would let
/// `("a", "b:c")` and `("a:b", "c")` collide.
fn unique_key(source: &str, id: &str) -> String {
    format!("unique:{}:{source}:{id}", source.len())
}

/// Normalise a title for fuzzy matching: lower-case, drop noise parentheticals
/// and trailing noise segments, then keep only alphanumerics and single spaces.
fn normalize_title(title: &str) -> String {
    let lowered = title.to_lowercase();
    let mut cleaned = strip_noise_brackets(&lowered);
    // Drop a trailing " - <noise>" segment, e.g. "song - 2011 remaster".
    if let Some((head, tail)) = cleaned.rsplit_once(" - ") {
        if NOISE_WORDS.iter().any(|word| tail.contains(word)) {
            cleaned = head.to_owned();
        }
    }
    alphanumeric_collapse(&cleaned)
}

/// Normalise an artist/name: lower-case, then alphanumerics and single spaces.
fn normalize_name(name: &str) -> String {
    alphanumeric_collapse(&name.to_lowercase())
}

/// Remove `(…)` / `[…]` segments whose contents are pure noise (a feat credit,
/// a remaster note, …); distinctive segments like `(Live)` are kept so a live
/// take never de-duplicates onto the studio version.
fn strip_noise_brackets(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut depth = 0_i32;
    let mut segment = String::new();
    for ch in text.chars() {
        match ch {
            '(' | '[' if depth == 0 => {
                depth = 1;
                segment.clear();
            }
            '(' | '[' => depth += 1,
            ')' | ']' if depth > 0 => {
                depth -= 1;
                let has_noise = NOISE_WORDS.iter().any(|word| segment.contains(word));
                let has_distinctive = DISTINCTIVE_WORDS.iter().any(|word| segment.contains(word));
                if depth == 0 && (!has_noise || has_distinctive) {
                    // Keep a distinctive bracket segment verbatim.
                    out.push('(');
                    out.push_str(&segment);
                    out.push(')');
                }
            }
            _ if depth > 0 => segment.push(ch),
            _ => out.push(ch),
        }
    }
    out
}

/// Keep only alphanumerics (Unicode-aware, so accents and non-Latin scripts
/// survive), folding everything else to single spaces.
fn alphanumeric_collapse(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_was_space = true;
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            out.push(ch);
            last_was_space = false;
        } else if !last_was_space {
            out.push(' ');
            last_was_space = true;
        }
    }
    out.trim_end().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{SourceId, SourceKind, SourceRef};
    use std::time::Duration;

    fn track(kind: SourceKind, title: &str, artist: &str, playable: bool) -> Track {
        Track {
            source: SourceRef::new(SourceId(kind.label().to_owned()), kind, "x"),
            title: title.to_owned(),
            artist: artist.to_owned(),
            artists: vec![artist.to_owned()],
            album: "Album".to_owned(),
            album_ref: None,
            artist_ref: None,
            duration: Duration::from_secs(200),
            track_number: None,
            art_url: None,
            mbid: None,
            isrc: None,
            playable,
        }
    }

    #[test]
    fn normalize_ignores_case_punctuation_and_feat() {
        assert_eq!(
            normalize_title("Creep (feat. Someone)"),
            normalize_title("creep"),
        );
        assert_eq!(
            normalize_title("Karma Police - 2011 Remaster"),
            normalize_title("Karma Police"),
        );
    }

    #[test]
    fn normalize_keeps_distinctive_versions_apart() {
        assert_ne!(normalize_title("Creep (Live)"), normalize_title("Creep"),);
    }

    #[test]
    fn same_song_across_sources_collapses_to_one() {
        let tracks = vec![
            track(SourceKind::Spotify, "Creep", "Radiohead", true),
            track(SourceKind::Subsonic, "creep", "radiohead", true),
            track(
                SourceKind::AppleMusic,
                "Creep (feat. nobody)",
                "Radiohead",
                false,
            ),
        ];
        let deduped = dedup_tracks(tracks);
        assert_eq!(deduped.len(), 1, "all three are the same track");
        // Subsonic outranks Spotify outranks Apple Music.
        assert_eq!(deduped[0].primary.source.kind, SourceKind::Subsonic);
        assert_eq!(deduped[0].source_count(), 3);
    }

    #[test]
    fn unplayable_source_never_becomes_primary() {
        let tracks = vec![
            track(SourceKind::AppleMusic, "Song", "Artist", false),
            track(SourceKind::Spotify, "Song", "Artist", true),
        ];
        let deduped = dedup_tracks(tracks);
        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].primary.source.kind, SourceKind::Spotify);
        assert_eq!(
            deduped[0].best_playable().map(|t| t.source.kind),
            Some(SourceKind::Spotify),
        );
    }

    #[test]
    fn distinct_songs_stay_separate() {
        let tracks = vec![
            track(SourceKind::Spotify, "Creep", "Radiohead", true),
            track(SourceKind::Spotify, "Karma Police", "Radiohead", true),
        ];
        assert_eq!(dedup_tracks(tracks).len(), 2);
    }

    #[test]
    fn mbid_match_beats_a_title_difference() {
        let mut a = track(SourceKind::Spotify, "Song", "Artist", true);
        let mut b = track(SourceKind::Subsonic, "Song (Remastered)", "Artist", true);
        a.mbid = Some("mb-123".to_owned());
        b.mbid = Some("mb-123".to_owned());
        assert_eq!(dedup_tracks(vec![a, b]).len(), 1);
    }

    #[test]
    fn mbid_match_is_case_insensitive() {
        let mut a = track(SourceKind::Spotify, "A", "X", true);
        let mut b = track(SourceKind::Subsonic, "B", "Y", true);
        a.mbid = Some("ABC-DEF".to_owned());
        b.mbid = Some("abc-def".to_owned());
        assert_eq!(dedup_tracks(vec![a, b]).len(), 1, "MBIDs are UUIDs");
    }

    #[test]
    fn tracks_with_no_artist_never_merge() {
        // Two same-titled tracks with no artist must stay separate — they
        // could be entirely different songs.
        let a = track(SourceKind::Subsonic, "Untitled", "", true);
        let mut b = track(SourceKind::Subsonic, "Untitled", "", true);
        b.source.id = "different".to_owned();
        assert_eq!(dedup_tracks(vec![a, b]).len(), 2);
    }

    #[test]
    fn isrc_match_collapses_across_services() {
        // Spotify and Apple Music both expose ISRC — a shared code dedups
        // them even when the titles differ cosmetically.
        let mut a = track(SourceKind::Spotify, "Song", "Artist", true);
        let mut b = track(
            SourceKind::AppleMusic,
            "Song (Single Version)",
            "Artist",
            false,
        );
        a.isrc = Some("USABC1234567".to_owned());
        b.isrc = Some("usabc1234567".to_owned());
        let deduped = dedup_tracks(vec![a, b]);
        assert_eq!(deduped.len(), 1, "ISRC collapses the two");
        assert!(deduped[0].primary.playable, "the playable source wins");
    }

    #[test]
    fn normalize_preserves_non_ascii_letters() {
        // Accented / non-Latin letters must survive so they still match.
        assert_eq!(normalize_name("Beyoncé"), normalize_name("beyoncé"));
        assert!(!normalize_name("宇多田ヒカル").is_empty());
    }
}
