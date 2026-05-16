-- Spottyfi metadata cache — initial schema.
--
-- Each table caches one kind of Spotify object. The object itself is stored as
-- a JSON blob in `payload` (the `spottyfi_models` types are all serde types);
-- `last_fetched` is a Unix timestamp in seconds, used by the
-- stale-while-revalidate freshness check in the `api` crate.

CREATE TABLE IF NOT EXISTS tracks (
    id           TEXT PRIMARY KEY NOT NULL,
    payload      TEXT NOT NULL,
    last_fetched INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS albums (
    id           TEXT PRIMARY KEY NOT NULL,
    payload      TEXT NOT NULL,
    last_fetched INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS artists (
    id           TEXT PRIMARY KEY NOT NULL,
    payload      TEXT NOT NULL,
    last_fetched INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS playlists (
    id           TEXT PRIMARY KEY NOT NULL,
    payload      TEXT NOT NULL,
    last_fetched INTEGER NOT NULL
);
