-- Spottyfi metadata cache — fetched lyrics.
--
-- Caches the lyrics fetched for a track, keyed by Spotify track id. The
-- `payload` is a JSON `CachedLyrics` record (the serialised `Lyrics`, the
-- provider it came from, and a fetched-at timestamp) — or a "no lyrics found"
-- marker, so a miss is not re-fetched on every revisit. Revisiting a track
-- then renders its lyrics straight from cache instead of hitting the network.

CREATE TABLE IF NOT EXISTS lyrics (
    id           TEXT PRIMARY KEY NOT NULL,
    payload      TEXT NOT NULL,
    last_fetched INTEGER NOT NULL
);
