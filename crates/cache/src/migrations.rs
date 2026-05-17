//! A minimal forward-only SQL migration runner.
//!
//! Migrations are plain `.sql` files under `crates/cache/migrations/`, embedded
//! at compile time with [`include_str!`]. Each migration has a 1-based version;
//! the applied version is tracked in SQLite's `PRAGMA user_version`. On startup
//! the runner applies every migration whose version exceeds the stored one, in
//! order, each inside a transaction.

use rusqlite::Connection;

use crate::error::{CacheError, CacheResult};

/// One embedded migration: its version and SQL body.
struct Migration {
    /// The 1-based migration version.
    version: i64,
    /// The SQL statements to apply.
    sql: &'static str,
}

/// The ordered list of embedded migrations.
///
/// Append new migrations here with the next version number; never edit or
/// reorder an already-shipped entry.
const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: include_str!("../migrations/0001_initial.sql"),
    },
    Migration {
        version: 2,
        sql: include_str!("../migrations/0002_playlist_tracks.sql"),
    },
    Migration {
        version: 3,
        sql: include_str!("../migrations/0003_lyrics.sql"),
    },
];

/// Read the schema version stored in the database (`0` for a fresh DB).
fn current_version(conn: &Connection) -> CacheResult<i64> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    Ok(version)
}

/// Apply every migration newer than the database's current version.
///
/// Idempotent: a DB already at the latest version is left untouched. Each
/// migration runs in its own transaction, so a failure leaves the DB at the
/// last fully-applied version rather than half-migrated.
///
/// # Errors
///
/// Returns [`CacheError::Sqlite`] if a migration's SQL fails, or
/// [`CacheError::Migration`] if the embedded migrations are not a contiguous
/// 1-based sequence (a programming error caught early).
pub fn run(conn: &mut Connection) -> CacheResult<()> {
    // Guard against a mis-numbered migration table at compile-list level.
    for (index, migration) in MIGRATIONS.iter().enumerate() {
        let expected = i64::try_from(index + 1).unwrap_or(i64::MAX);
        if migration.version != expected {
            return Err(CacheError::Migration(format!(
                "migration {} is out of sequence (expected version {expected})",
                migration.version
            )));
        }
    }

    let mut version = current_version(conn)?;
    let latest = MIGRATIONS.last().map_or(0, |m| m.version);
    if version >= latest {
        tracing::debug!(version, "metadata cache schema already current");
        return Ok(());
    }

    let from = version;
    for migration in MIGRATIONS.iter().filter(|m| m.version > from) {
        tracing::info!(version = migration.version, "applying cache migration");
        let tx = conn.transaction()?;
        tx.execute_batch(migration.sql)?;
        // `user_version` cannot be parameterised; the value is a trusted
        // integer constant from the migration list, so the format is safe.
        tx.pragma_update(None, "user_version", migration.version)?;
        tx.commit()?;
        version = migration.version;
    }

    tracing::info!(version, "metadata cache schema migrated");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_a_fresh_database() {
        let mut conn = Connection::open_in_memory().expect("open in-memory db");
        assert_eq!(current_version(&conn).expect("read version"), 0);

        run(&mut conn).expect("run migrations");

        let latest = MIGRATIONS.last().expect("at least one migration").version;
        assert_eq!(current_version(&conn).expect("read version"), latest);

        // The expected tables now exist.
        for table in [
            "tracks",
            "albums",
            "artists",
            "playlists",
            "playlist_tracks",
            "lyrics",
        ] {
            let count: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |row| row.get(0),
                )
                .expect("query table");
            assert_eq!(count, 1, "table {table} should exist");
        }
    }

    #[test]
    fn running_twice_is_idempotent() {
        let mut conn = Connection::open_in_memory().expect("open in-memory db");
        run(&mut conn).expect("first run");
        let after_first = current_version(&conn).expect("read version");
        run(&mut conn).expect("second run");
        let after_second = current_version(&conn).expect("read version");
        assert_eq!(after_first, after_second);
    }

    #[test]
    fn embedded_migrations_are_a_contiguous_sequence() {
        // This is what the runtime guard in `run` checks; assert it directly so
        // a mis-numbered file is caught even if `run` short-circuits.
        for (index, migration) in MIGRATIONS.iter().enumerate() {
            assert_eq!(
                migration.version,
                i64::try_from(index + 1).expect("small index"),
                "migration {index} is mis-numbered",
            );
        }
    }
}
