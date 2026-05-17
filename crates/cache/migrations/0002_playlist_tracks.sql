-- Spottyfi metadata cache — playlist track listings.
--
-- Caches the fully-resolved track list of a playlist (a JSON array of
-- `PlaylistTrack`), keyed by playlist id. This lets a revisited playlist page
-- render its tracks instantly from cache, then stale-while-revalidate refreshes
-- the listing in the background — the same pattern the object tables use.

CREATE TABLE IF NOT EXISTS playlist_tracks (
    id           TEXT PRIMARY KEY NOT NULL,
    payload      TEXT NOT NULL,
    last_fetched INTEGER NOT NULL
);
