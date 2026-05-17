//! Candidate match scoring for lyrics lookups.
//!
//! A lyrics search (lrclib's `/api/search`, or any future provider that
//! returns several candidates) can hand back the wrong version of a song —
//! a live take, a remix, an extended edit. Blindly taking the first result is
//! a common cause of "the lyrics don't line up". This module scores each
//! candidate against the track Spottyfi actually wants and picks the best.
//!
//! The dominant signal is **track duration**: a synced LRC is timed against
//! one specific recording, so a candidate whose duration is far from the
//! playing track's is almost certainly the wrong version. Title, artist and
//! album similarity break ties and guard against a same-length but unrelated
//! song.

use std::time::Duration;

/// The fields a candidate exposes for scoring.
///
/// Every field is optional-by-emptiness: a missing duration is
/// [`Duration::ZERO`], a missing string is empty. Absent fields simply do not
/// contribute to (or subtract from) the score.
#[derive(Debug, Clone, Default)]
pub struct Candidate {
    /// The candidate's track title.
    pub title: String,
    /// The candidate's artist name.
    pub artist: String,
    /// The candidate's album name.
    pub album: String,
    /// The candidate's track duration ([`Duration::ZERO`] when unknown).
    pub duration: Duration,
}

/// What we are matching candidates *against* — the track being played.
#[derive(Debug, Clone, Default)]
pub struct Query {
    /// The wanted track title.
    pub title: String,
    /// The wanted artist name.
    pub artist: String,
    /// The wanted album name (empty when unknown).
    pub album: String,
    /// The wanted track duration ([`Duration::ZERO`] when unknown).
    pub duration: Duration,
}

/// The largest duration gap, in seconds, still treated as a plausible match.
///
/// Beyond this the duration component of the score bottoms out at zero — a
/// candidate two seconds off can still win on title/artist, but one a minute
/// off is rejected on duration alone.
const DURATION_TOLERANCE_SECS: f64 = 12.0;

/// Score one `candidate` against the `query`, higher is better.
///
/// The score is in `0.0..=1.0`:
///
/// - **duration** (weight 0.55) — a triangular falloff: an exact match scores
///   1, the score decays linearly to 0 at [`DURATION_TOLERANCE_SECS`] apart.
///   When either side has no duration this component is neutral (0.5).
/// - **title** (weight 0.25) — normalised token-set similarity.
/// - **artist** (weight 0.15) — normalised token-set similarity.
/// - **album** (weight 0.05) — normalised token-set similarity; neutral (0.5)
///   when either side has no album.
#[must_use]
pub fn score(query: &Query, candidate: &Candidate) -> f64 {
    let duration = duration_score(query.duration, candidate.duration);
    let title = similarity(&query.title, &candidate.title);
    let artist = similarity(&query.artist, &candidate.artist);
    let album = if query.album.is_empty() || candidate.album.is_empty() {
        0.5
    } else {
        similarity(&query.album, &candidate.album)
    };
    0.55 * duration + 0.25 * title + 0.15 * artist + 0.05 * album
}

/// Pick the highest-scoring candidate, returning its index and score.
///
/// Returns `None` for an empty slice. Ties keep the earlier candidate (search
/// endpoints already return results best-first).
#[must_use]
pub fn best_match(query: &Query, candidates: &[Candidate]) -> Option<(usize, f64)> {
    candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| (index, score(query, candidate)))
        .reduce(|best, next| if next.1 > best.1 { next } else { best })
}

/// The duration component of the score: a triangular falloff around an exact
/// match. Neutral (0.5) when either duration is unknown.
fn duration_score(want: Duration, have: Duration) -> f64 {
    if want.is_zero() || have.is_zero() {
        return 0.5;
    }
    let gap = want.as_secs_f64() - have.as_secs_f64();
    let gap = gap.abs();
    (1.0 - gap / DURATION_TOLERANCE_SECS).max(0.0)
}

/// A normalised token-set similarity of two strings, in `0.0..=1.0`.
///
/// Both sides are lower-cased and split on non-alphanumeric boundaries; the
/// score is the size of the token intersection over the size of the union
/// (the Jaccard index). This is order-insensitive and tolerant of punctuation
/// and "feat." noise, which is what catalogue titles need.
fn similarity(a: &str, b: &str) -> f64 {
    let tokens_a = tokens(a);
    let tokens_b = tokens(b);
    if tokens_a.is_empty() && tokens_b.is_empty() {
        return 1.0;
    }
    if tokens_a.is_empty() || tokens_b.is_empty() {
        return 0.0;
    }
    let intersection = tokens_a.iter().filter(|t| tokens_b.contains(*t)).count();
    let union = tokens_a.len() + tokens_b.len() - intersection;
    if union == 0 {
        1.0
    } else {
        intersection as f64 / union as f64
    }
}

/// Lower-case alphanumeric tokens of `text`, de-duplicated.
fn tokens(text: &str) -> Vec<String> {
    let mut tokens: Vec<String> = text
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    tokens.sort();
    tokens.dedup();
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    #[test]
    fn an_exact_match_scores_near_one() {
        let query = Query {
            title: "Karma Police".into(),
            artist: "Radiohead".into(),
            album: "OK Computer".into(),
            duration: secs(264),
        };
        let candidate = Candidate {
            title: "Karma Police".into(),
            artist: "Radiohead".into(),
            album: "OK Computer".into(),
            duration: secs(264),
        };
        assert!(score(&query, &candidate) > 0.99);
    }

    #[test]
    fn duration_breaks_a_tie_between_two_versions() {
        // Same song, two versions: a studio cut and a longer live take.
        let query = Query {
            title: "Song".into(),
            artist: "Artist".into(),
            album: String::new(),
            duration: secs(200),
        };
        let studio = Candidate {
            title: "Song".into(),
            artist: "Artist".into(),
            album: String::new(),
            duration: secs(201),
        };
        let live = Candidate {
            title: "Song".into(),
            artist: "Artist".into(),
            album: String::new(),
            duration: secs(320),
        };
        let (index, _) = best_match(&query, &[live, studio]).expect("a match");
        // The studio cut (index 1) is far closer in duration.
        assert_eq!(index, 1);
    }

    #[test]
    fn a_wildly_different_duration_zeroes_the_duration_term() {
        let near = duration_score(secs(200), secs(200));
        let far = duration_score(secs(200), secs(400));
        assert!((near - 1.0).abs() < f64::EPSILON);
        assert!(far.abs() < f64::EPSILON);
    }

    #[test]
    fn an_unknown_duration_is_neutral() {
        // No duration on either side — the term contributes a flat 0.5.
        assert!((duration_score(Duration::ZERO, secs(200)) - 0.5).abs() < f64::EPSILON);
        assert!((duration_score(secs(200), Duration::ZERO) - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn similarity_is_token_order_insensitive() {
        assert!((similarity("Hello World", "world hello") - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn similarity_tolerates_punctuation_and_case() {
        // Trailing punctuation and case differences must not lower the score.
        assert!((similarity("Don't Stop!", "don't stop") - 1.0).abs() < f64::EPSILON);
        assert!((similarity("All Right.", "all right") - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn similarity_of_unrelated_strings_is_low() {
        assert!(similarity("alpha beta", "gamma delta") < 0.1);
    }

    #[test]
    fn best_match_prefers_the_right_title_over_a_closer_duration() {
        // A same-length but unrelated song must not beat the real one.
        let query = Query {
            title: "Paranoid Android".into(),
            artist: "Radiohead".into(),
            album: String::new(),
            duration: secs(383),
        };
        let wrong = Candidate {
            title: "Some Other Track".into(),
            artist: "Another Band".into(),
            album: String::new(),
            duration: secs(383),
        };
        let right = Candidate {
            title: "Paranoid Android".into(),
            artist: "Radiohead".into(),
            album: String::new(),
            duration: secs(380),
        };
        let (index, _) = best_match(&query, &[wrong, right]).expect("a match");
        assert_eq!(index, 1);
    }

    #[test]
    fn best_match_of_empty_is_none() {
        assert!(best_match(&Query::default(), &[]).is_none());
    }
}
