-- Spottyfi metadata cache — assembled collections.
--
-- Caches list-shaped results that are not keyed by a single Spotify object id
-- — the user's full saved-tracks ("Liked Songs") listing, for one — under a
-- fixed string key. A relaunch then serves the collection straight from cache
-- and stale-while-revalidate refreshes it in the background, instead of
-- re-streaming the whole list every time.

CREATE TABLE IF NOT EXISTS collections (
    id           TEXT PRIMARY KEY NOT NULL,
    payload      TEXT NOT NULL,
    last_fetched INTEGER NOT NULL
);
