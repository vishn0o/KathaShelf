//! Plain data types shared by the database layer and the UI.

use std::fmt;

/// The three reading states stored in the `status` column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReadStatus {
    #[default]
    WantToRead,
    Reading,
    Read,
}

impl ReadStatus {
    /// The exact strings persisted in SQLite (kept in sync with the CHECK constraint).
    pub const fn as_str(self) -> &'static str {
        match self {
            ReadStatus::Read => "Read",
            ReadStatus::Reading => "Reading",
            ReadStatus::WantToRead => "WantToRead",
        }
    }

    pub fn from_db(s: &str) -> Self {
        match s {
            "Read" => ReadStatus::Read,
            "Reading" => ReadStatus::Reading,
            _ => ReadStatus::WantToRead,
        }
    }

    /// Friendly label shown in the UI.
    pub const fn label(self) -> &'static str {
        match self {
            ReadStatus::Read => "Read",
            ReadStatus::Reading => "Reading",
            ReadStatus::WantToRead => "Want to read",
        }
    }

    pub const ALL: [ReadStatus; 3] = [
        ReadStatus::WantToRead,
        ReadStatus::Reading,
        ReadStatus::Read,
    ];
}

impl fmt::Display for ReadStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Editable reader identity stored in the singleton `profile` table.
#[derive(Debug, Clone)]
pub struct Profile {
    pub name: String,
    pub avatar_initial: String,
    pub avatar_color: String,
    pub bio: String,
    pub yearly_goal: i64,
    pub preferred_language: String,
    pub favorite_publisher: String,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            name: "Reader".into(),
            avatar_initial: "R".into(),
            avatar_color: "#b87333".into(),
            bio: String::new(),
            yearly_goal: 0,
            preferred_language: "Both".into(),
            favorite_publisher: String::new(),
        }
    }
}

/// A single row of the `books` table.
#[derive(Debug, Clone)]
pub struct Book {
    pub id: i64,
    pub title: String,
    /// Malayalam-script title (മലയാളം), stored alongside the English title.
    pub malayalam_title: Option<String>,
    pub publisher: Option<String>,
    /// Comma-separated genres / tags used by the reader archetype.
    pub tags: String,
    pub author: String,
    pub isbn: String,
    /// Relative path inside the app-data directory, e.g. `covers/9780140328721.jpg`.
    pub cover_path: Option<String>,
    /// Absolute path to the source EPUB/PDF imported by the folder scanner.
    pub file_path: Option<String>,
    pub status: ReadStatus,
    /// Half-star increments between 0.5 and 5.0. `None` means "not rated".
    pub rating: Option<f64>,
    pub review: String,
    /// Quick note about where you left off ("page 142 — the reveal!").
    pub bookmark_note: String,
    /// ISO date (`YYYY-MM-DD`), or `None` when the book is not finished.
    pub date_finished: Option<String>,
    pub current_page: i64,
    pub total_pages: i64,
    pub is_favorite: bool,
}

impl Book {
    /// Returns an empty book ready to be filled in by the "Add book" form.
    /// `id == 0` signals "not saved yet" to the database layer.
    pub fn new() -> Self {
        Self {
            id: 0,
            title: String::new(),
            malayalam_title: None,
            publisher: None,
            tags: String::new(),
            author: String::new(),
            isbn: String::new(),
            cover_path: None,
            file_path: None,
            status: ReadStatus::WantToRead,
            rating: None,
            review: String::new(),
            bookmark_note: String::new(),
            date_finished: None,
            current_page: 0,
            total_pages: 0,
            is_favorite: false,
        }
    }
}

impl Default for Book {
    fn default() -> Self {
        Self::new()
    }
}

/// Clamp + snap a raw value to the nearest valid half-star rating.
/// Values <= 0.0 mean "no rating" and are returned as `None`.
pub fn normalize_rating(value: f64) -> Option<f64> {
    if !value.is_finite() || value <= 0.0 {
        return None;
    }
    let snapped = (value * 2.0).round() / 2.0;
    Some(snapped.clamp(0.5, 5.0))
}

/// Only allow `YYYY-MM-DD`-shaped strings (content is not strictly validated as a real date).
pub fn normalize_date(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let bytes = s.as_bytes();
    let valid = bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit());
    if valid {
        Some(s.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratings_snap_to_half_stars() {
        assert_eq!(normalize_rating(0.0), None);
        assert_eq!(normalize_rating(-1.0), None);
        assert_eq!(normalize_rating(f64::NAN), None);
        assert_eq!(normalize_rating(0.3), Some(0.5));
        assert_eq!(normalize_rating(4.2), Some(4.0));
        assert_eq!(normalize_rating(4.8), Some(5.0));
        assert_eq!(normalize_rating(9.9), Some(5.0));
    }

    #[test]
    fn dates_must_be_iso_shaped() {
        assert_eq!(normalize_date("2026-11-19"), Some("2026-11-19".into()));
        assert_eq!(normalize_date(" 2026-11-19 "), Some("2026-11-19".into()));
        assert_eq!(normalize_date("19/11/2026"), None);
        assert_eq!(normalize_date(""), None);
    }
}
