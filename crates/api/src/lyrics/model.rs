//! The shared lyrics domain model: synced and plain lyrics, plus LRC parsing.
//!
//! A lyrics provider returns [`Lyrics`], which is either:
//!
//! - [`Lyrics::Synced`] — a list of [`SyncedLine`]s, each carrying the
//!   timestamp at which the line begins. This is what drives the panel's
//!   current-line highlight, auto-scroll and click-to-seek.
//! - [`Lyrics::Plain`] — an ordered list of unsynced lines, rendered as a
//!   static scrollable column.
//!
//! The synced shape is parsed from [LRC](https://en.wikipedia.org/wiki/LRC_(file_format))
//! text — the de-facto lyric-timing format both providers speak.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// A single timed lyric line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncedLine {
    /// The position in the track at which this line begins.
    pub at: Duration,
    /// The line's text. May be empty (an instrumental gap).
    pub text: String,
}

/// Lyrics for one track — either time-synced or plain (unsynced).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lyrics {
    /// Time-synced lyrics: lines ordered by their start time.
    ///
    /// Guaranteed sorted ascending by [`SyncedLine::at`] when produced by
    /// [`parse_lrc`]; the panel relies on that for its binary-search line
    /// selection.
    Synced(Vec<SyncedLine>),
    /// Plain, unsynced lyrics: an ordered list of text lines.
    Plain(Vec<String>),
}

impl Lyrics {
    /// Whether these lyrics carry any line at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        match self {
            Lyrics::Synced(lines) => lines.is_empty(),
            Lyrics::Plain(lines) => lines.is_empty(),
        }
    }

    /// The index of the line that is "current" at playback position `pos`.
    ///
    /// For [`Lyrics::Synced`] this is the last line whose timestamp is at or
    /// before `pos`; `None` before the first line begins. For [`Lyrics::Plain`]
    /// there is no timing, so this is always `None`.
    #[must_use]
    pub fn current_line(&self, pos: Duration) -> Option<usize> {
        let Lyrics::Synced(lines) = self else {
            return None;
        };
        current_synced_line(lines, pos)
    }
}

/// The index of the synced line active at position `pos`.
///
/// `lines` must be sorted ascending by timestamp (the [`parse_lrc`]
/// guarantee). Returns the last line whose `at <= pos`, or `None` when `pos`
/// precedes the first line.
#[must_use]
pub fn current_synced_line(lines: &[SyncedLine], pos: Duration) -> Option<usize> {
    if lines.is_empty() || pos < lines[0].at {
        return None;
    }
    // Binary search for the last line at or before `pos`. `partition_point`
    // returns the count of lines with `at <= pos`; the active line is one
    // before that.
    let count = lines.partition_point(|line| line.at <= pos);
    if count == 0 {
        None
    } else {
        Some(count - 1)
    }
}

/// Parse LRC-format lyric text into [`Lyrics`].
///
/// LRC lines look like `[mm:ss.xx] some text`; one text line may carry several
/// timestamp tags. ID tags (`[ar:…]`, `[ti:…]`, `[length:…]`, …) and blank or
/// untimed lines are skipped. The resulting [`SyncedLine`]s are sorted
/// ascending by timestamp.
///
/// If the text carries **no** timestamped line at all it is treated as plain
/// lyrics — every non-empty line, in order, as [`Lyrics::Plain`].
#[must_use]
pub fn parse_lrc(text: &str) -> Lyrics {
    let mut synced: Vec<SyncedLine> = Vec::new();

    for raw in text.lines() {
        let line = raw.trim_end_matches(['\r', '\n']);
        // Pull every leading `[..]` tag off the front of the line.
        let mut rest = line;
        let mut stamps: Vec<Duration> = Vec::new();
        while let Some(stripped) = rest.strip_prefix('[') {
            let Some(close) = stripped.find(']') else {
                break;
            };
            let tag = &stripped[..close];
            rest = &stripped[close + 1..];
            if let Some(stamp) = parse_timestamp(tag) {
                stamps.push(stamp);
            }
            // A non-timestamp tag (`ar:…`, `ti:…`) is simply dropped.
        }
        if stamps.is_empty() {
            continue;
        }
        let content = rest.trim().to_owned();
        for stamp in stamps {
            synced.push(SyncedLine {
                at: stamp,
                text: content.clone(),
            });
        }
    }

    if synced.is_empty() {
        // No timed lines — fall back to plain lyrics.
        let plain: Vec<String> = text
            .lines()
            .map(|l| l.trim().to_owned())
            .filter(|l| !l.is_empty())
            .collect();
        return Lyrics::Plain(plain);
    }

    synced.sort_by_key(|line| line.at);
    Lyrics::Synced(synced)
}

/// Parse a single LRC timestamp tag (`mm:ss`, `mm:ss.xx`, `mm:ss.xxx`).
///
/// Returns `None` for an ID tag or any value that is not a timestamp.
fn parse_timestamp(tag: &str) -> Option<Duration> {
    let (minutes, rest) = tag.split_once(':')?;
    let minutes: u64 = minutes.trim().parse().ok()?;

    // The seconds part may carry a fractional `.xx` / `.xxx` component.
    let (seconds, fraction) = match rest.split_once('.') {
        Some((s, f)) => (s, Some(f)),
        None => (rest, None),
    };
    let seconds: u64 = seconds.trim().parse().ok()?;
    if seconds >= 60 {
        return None;
    }

    let millis = match fraction {
        Some(frac) => {
            let digits: String = frac.chars().take(3).collect();
            if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            // Right-pad to three digits so `.5` reads as 500ms, `.05` as 50ms.
            let padded = format!("{digits:0<3}");
            padded.parse::<u64>().ok()?
        }
        None => 0,
    };

    Some(Duration::from_millis(
        (minutes * 60 + seconds) * 1000 + millis,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(m: u64) -> Duration {
        Duration::from_millis(m)
    }

    #[test]
    fn parses_basic_lrc() {
        let text = "[00:01.00]first line\n[00:03.50]second line\n[00:10.00]third";
        let Lyrics::Synced(lines) = parse_lrc(text) else {
            panic!("expected synced lyrics");
        };
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].at, ms(1000));
        assert_eq!(lines[0].text, "first line");
        assert_eq!(lines[1].at, ms(3500));
        assert_eq!(lines[2].at, ms(10_000));
    }

    #[test]
    fn skips_id_tags_and_blank_lines() {
        let text = "[ar:An Artist]\n[ti:A Title]\n\n[00:02.00]only real line\n";
        let Lyrics::Synced(lines) = parse_lrc(text) else {
            panic!("expected synced lyrics");
        };
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "only real line");
    }

    #[test]
    fn handles_multiple_timestamps_on_one_line() {
        // A repeated chorus line carries several stamps.
        let text = "[00:05.00][00:30.00]chorus";
        let Lyrics::Synced(lines) = parse_lrc(text) else {
            panic!("expected synced lyrics");
        };
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].at, ms(5000));
        assert_eq!(lines[1].at, ms(30_000));
        assert_eq!(lines[0].text, lines[1].text);
    }

    #[test]
    fn sorts_unordered_input() {
        let text = "[00:10.00]late\n[00:01.00]early";
        let Lyrics::Synced(lines) = parse_lrc(text) else {
            panic!("expected synced lyrics");
        };
        assert_eq!(lines[0].text, "early");
        assert_eq!(lines[1].text, "late");
    }

    #[test]
    fn fractional_milliseconds_pad_correctly() {
        // `.5` is 500ms, `.05` is 50ms, `.005` is 5ms.
        assert_eq!(parse_timestamp("00:00.5"), Some(ms(500)));
        assert_eq!(parse_timestamp("00:00.05"), Some(ms(50)));
        assert_eq!(parse_timestamp("00:00.005"), Some(ms(5)));
        assert_eq!(parse_timestamp("01:30.00"), Some(ms(90_000)));
    }

    #[test]
    fn rejects_non_timestamp_tags() {
        assert_eq!(parse_timestamp("ar:Some Artist"), None);
        assert_eq!(parse_timestamp("offset:+100"), None);
        assert_eq!(parse_timestamp("not a stamp"), None);
    }

    #[test]
    fn untimed_text_becomes_plain_lyrics() {
        let text = "just a line\nand another\n\nthird";
        let Lyrics::Plain(lines) = parse_lrc(text) else {
            panic!("expected plain lyrics");
        };
        assert_eq!(lines, vec!["just a line", "and another", "third"]);
    }

    #[test]
    fn current_line_before_first_is_none() {
        let lines = vec![
            SyncedLine {
                at: ms(1000),
                text: "a".into(),
            },
            SyncedLine {
                at: ms(2000),
                text: "b".into(),
            },
        ];
        assert_eq!(current_synced_line(&lines, ms(0)), None);
        assert_eq!(current_synced_line(&lines, ms(999)), None);
    }

    #[test]
    fn current_line_selects_last_at_or_before_position() {
        let lines = vec![
            SyncedLine {
                at: ms(1000),
                text: "a".into(),
            },
            SyncedLine {
                at: ms(3000),
                text: "b".into(),
            },
            SyncedLine {
                at: ms(5000),
                text: "c".into(),
            },
        ];
        // Exactly on a line's timestamp selects that line.
        assert_eq!(current_synced_line(&lines, ms(1000)), Some(0));
        // Between two lines selects the earlier one.
        assert_eq!(current_synced_line(&lines, ms(2999)), Some(0));
        assert_eq!(current_synced_line(&lines, ms(3000)), Some(1));
        assert_eq!(current_synced_line(&lines, ms(4000)), Some(1));
        // Past the last line stays on the last line.
        assert_eq!(current_synced_line(&lines, ms(99_999)), Some(2));
    }

    #[test]
    fn current_line_on_empty_is_none() {
        assert_eq!(current_synced_line(&[], ms(1000)), None);
    }

    #[test]
    fn lyrics_current_line_is_none_for_plain() {
        let plain = Lyrics::Plain(vec!["a".into(), "b".into()]);
        assert_eq!(plain.current_line(ms(5000)), None);
    }

    #[test]
    fn lyrics_current_line_delegates_for_synced() {
        let synced = Lyrics::Synced(vec![
            SyncedLine {
                at: ms(0),
                text: "a".into(),
            },
            SyncedLine {
                at: ms(2000),
                text: "b".into(),
            },
        ]);
        assert_eq!(synced.current_line(ms(2500)), Some(1));
    }
}
