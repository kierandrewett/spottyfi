//! A generic pagination wrapper.

use serde::{Deserialize, Serialize};

/// One page of a paginated Spotify collection.
///
/// Mirrors the shape of Spotify's offset-based paging object, decoupled from
/// `rspotify`. `next.is_some()` indicates a further page exists; the `api`
/// crate's stream methods follow that link until it is `None`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Page<T> {
    /// The items on this page.
    pub items: Vec<T>,
    /// The number of items requested per page.
    pub limit: u32,
    /// The index of the first item on this page within the full collection.
    pub offset: u32,
    /// The total number of items in the full collection, if known.
    pub total: u32,
    /// Whether a further page exists after this one.
    pub has_next: bool,
}

impl<T> Page<T> {
    /// Whether this page carries no items.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The number of items on this page.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Map every item, preserving the pagination metadata.
    #[must_use]
    pub fn map<U, F: FnMut(T) -> U>(self, f: F) -> Page<U> {
        Page {
            items: self.items.into_iter().map(f).collect(),
            limit: self.limit,
            offset: self.offset,
            total: self.total,
            has_next: self.has_next,
        }
    }
}

impl<T> Default for Page<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            limit: 0,
            offset: 0,
            total: 0,
            has_next: false,
        }
    }
}
