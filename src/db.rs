//! SQLite persistence. The database and cached covers live under the OS app-data dir.

use crate::models::{Book, Profile, ReadStatus};
use directories::ProjectDirs;
use rusqlite::{params, Connection, OptionalExtension};
use std::error::Error;
use std::fs;
use std::path::PathBuf;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS books (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    title          TEXT    NOT NULL,
    author         TEXT    NOT NULL DEFAULT '',
    isbn           TEXT    NOT NULL DEFAULT '',
    cover_path     TEXT,
    status         TEXT    NOT NULL DEFAULT 'WantToRead'
                   CHECK (status IN ('Read', 'Reading', 'WantToRead')),
    rating         REAL    CHECK (rating IS NULL OR
                           (rating >= 0.5 AND rating <= 5.0 AND
                            rating * 2.0 = CAST(rating * 2.0 AS INTEGER))),
    review         TEXT    NOT NULL DEFAULT '',
    bookmark_note  TEXT,
    date_finished  TEXT,
    current_page   INTEGER NOT NULL DEFAULT 0 CHECK (current_page >= 0),
    total_pages    INTEGER NOT NULL DEFAULT 0 CHECK (total_pages >= 0),
    is_favorite    INTEGER NOT NULL DEFAULT 0 CHECK (is_favorite IN (0, 1)),
    malayalam_title TEXT,
    publisher      TEXT,
    tags           TEXT NOT NULL DEFAULT '',
    file_path      TEXT
);

CREATE TABLE IF NOT EXISTS profile (
    id                  INTEGER PRIMARY KEY CHECK (id = 1),
    name                TEXT NOT NULL DEFAULT 'Reader',
    avatar_initial      TEXT NOT NULL DEFAULT 'R',
    avatar_color        TEXT NOT NULL DEFAULT '#b87333',
    bio                 TEXT NOT NULL DEFAULT '',
    yearly_goal         INTEGER NOT NULL DEFAULT 0,
    preferred_language  TEXT NOT NULL DEFAULT 'Both',
    favorite_publisher  TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS watched_folders (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    path         TEXT UNIQUE NOT NULL,
    last_scanned TEXT
);

CREATE INDEX IF NOT EXISTS idx_books_title    ON books (title COLLATE NOCASE);
CREATE INDEX IF NOT EXISTS idx_books_status   ON books (status);
CREATE INDEX IF NOT EXISTS idx_books_favorite ON books (is_favorite);
";

const COLUMNS: &str = "id, title, author, isbn, cover_path, status, rating, \
                       review, date_finished, current_page, total_pages, is_favorite, \
                       bookmark_note, malayalam_title, publisher, tags, file_path";

/// A folder remembered by the VLC-style local library scanner.
#[derive(Debug, Clone)]
pub struct WatchedFolder {
    pub id: i64,
    pub path: String,
    pub last_scanned: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Title,
    Author,
    Rating,
    RecentlyFinished,
    RecentlyAdded,
}

impl SortKey {
    pub const ALL: [SortKey; 5] = [
        SortKey::Title,
        SortKey::Author,
        SortKey::Rating,
        SortKey::RecentlyFinished,
        SortKey::RecentlyAdded,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            SortKey::Title => "Title",
            SortKey::Author => "Author",
            SortKey::Rating => "Highest Rated",
            SortKey::RecentlyFinished => "Date Finished",
            SortKey::RecentlyAdded => "Recently Added",
        }
    }

    const fn sql(self) -> &'static str {
        match self {
            SortKey::Title => "title COLLATE NOCASE ASC, author COLLATE NOCASE ASC",
            SortKey::Author => "author COLLATE NOCASE ASC, title COLLATE NOCASE ASC",
            SortKey::Rating => "rating IS NULL ASC, rating DESC, title COLLATE NOCASE ASC",
            SortKey::RecentlyFinished => "date_finished IS NULL ASC, date_finished DESC, id DESC",
            SortKey::RecentlyAdded => "id DESC",
        }
    }
}

pub struct Database {
    conn: Connection,
    pub data_dir: PathBuf,
    pub covers_dir: PathBuf,
    pub db_path: PathBuf,
}

impl Database {
    /// Open (or create) the app database and its `covers/` folder automatically.
    pub fn open() -> Result<Self, Box<dyn Error>> {
        let new_dir = ProjectDirs::from("com", "KathaShelf", "KathaShelf")
            .map(|project| project.data_dir().to_path_buf());
        let legacy_dir = ProjectDirs::from("com", "KadhaShelf", "KadhaShelf")
            .map(|project| project.data_dir().to_path_buf());
        // Keep libraries created by the old name intact on first run.
        let data_dir = if new_dir
            .as_ref()
            .map(|dir| dir.join("books.sqlite3").is_file())
            .unwrap_or(false)
            || legacy_dir
                .as_ref()
                .map(|dir| dir.join("books.sqlite3").is_file())
                .unwrap_or(false)
        {
            new_dir
                .filter(|dir| dir.join("books.sqlite3").is_file())
                .or(legacy_dir)
        } else {
            new_dir
        }
        .ok_or("could not determine the OS application data directory")?;
        Self::open_at(data_dir)
    }

    /// Open a database rooted at a custom data directory (also used by tests).
    pub fn open_at(data_dir: PathBuf) -> Result<Self, Box<dyn Error>> {
        let covers_dir = data_dir.join("covers");
        let db_path = data_dir.join("books.sqlite3");

        fs::create_dir_all(&data_dir)?;
        fs::create_dir_all(&covers_dir)?;

        let conn = Connection::open(&db_path)?;
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA cache_size = -8000;
             PRAGMA temp_store = MEMORY;",
        )?;
        conn.execute_batch(SCHEMA)?;
        Self::migrate(&conn)?;

        Ok(Self {
            conn,
            data_dir,
            covers_dir,
            db_path,
        })
    }

    /// Add columns introduced in later versions to databases created by older builds.
    fn migrate(conn: &Connection) -> Result<(), rusqlite::Error> {
        let mut statement = conn.prepare("PRAGMA table_info(books)")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
        let mut columns = Vec::new();
        for name in rows {
            columns.push(name?);
        }
        if !columns.iter().any(|name| name == "bookmark_note") {
            conn.execute("ALTER TABLE books ADD COLUMN bookmark_note TEXT", [])?;
        }
        if !columns.iter().any(|name| name == "malayalam_title") {
            conn.execute("ALTER TABLE books ADD COLUMN malayalam_title TEXT", [])?;
        }
        if !columns.iter().any(|name| name == "publisher") {
            conn.execute("ALTER TABLE books ADD COLUMN publisher TEXT", [])?;
        }
        if !columns.iter().any(|name| name == "tags") {
            conn.execute(
                "ALTER TABLE books ADD COLUMN tags TEXT NOT NULL DEFAULT ''",
                [],
            )?;
        }
        if !columns.iter().any(|name| name == "file_path") {
            conn.execute("ALTER TABLE books ADD COLUMN file_path TEXT", [])?;
        }
        // Created after the ALTER TABLE so old databases can be upgraded.
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_books_file ON books (file_path)",
            [],
        )?;
        // Older databases may predate the watched_folders table.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS profile (
    id                  INTEGER PRIMARY KEY CHECK (id = 1),
    name                TEXT NOT NULL DEFAULT 'Reader',
    avatar_initial      TEXT NOT NULL DEFAULT 'R',
    avatar_color        TEXT NOT NULL DEFAULT '#b87333',
    bio                 TEXT NOT NULL DEFAULT '',
    yearly_goal         INTEGER NOT NULL DEFAULT 0,
    preferred_language  TEXT NOT NULL DEFAULT 'Both',
    favorite_publisher  TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS watched_folders (
                 id           INTEGER PRIMARY KEY AUTOINCREMENT,
                 path         TEXT UNIQUE NOT NULL,
                 last_scanned TEXT
             );",
        )?;
        Ok(())
    }

    /// Resolve a stored relative cover path (e.g. `covers/123.jpg`) to an absolute path.
    pub fn absolute_cover_path(&self, relative: &str) -> PathBuf {
        let path = PathBuf::from(relative);
        if path.is_absolute() {
            path
        } else {
            self.data_dir.join(path)
        }
    }

    pub fn list_books(
        &self,
        search: &str,
        status: Option<ReadStatus>,
        favorites_only: bool,
        sort: SortKey,
    ) -> Result<Vec<Book>, rusqlite::Error> {
        let mut sql = format!("SELECT {COLUMNS} FROM books WHERE 1 = 1");
        // Owned boxed params let us build the WHERE clause dynamically.
        let mut bound: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        let needle = search.trim().to_lowercase();
        if !needle.is_empty() {
            sql.push_str(
                " AND (LOWER(title) LIKE ? OR LOWER(COALESCE(malayalam_title, '')) LIKE ? \
                 OR LOWER(author) LIKE ? OR LOWER(COALESCE(publisher, '')) LIKE ? \
                 OR LOWER(isbn) LIKE ?)",
            );
            let pattern = format!("%{needle}%");
            for _ in 0..5 {
                bound.push(Box::new(pattern.clone()));
            }
        }
        if let Some(status) = status {
            sql.push_str(" AND status = ?");
            bound.push(Box::new(status.as_str().to_string()));
        }
        if favorites_only {
            sql.push_str(" AND is_favorite = 1");
        }

        sql.push_str(" ORDER BY ");
        sql.push_str(sort.sql());

        let refs: Vec<&dyn rusqlite::ToSql> = bound.iter().map(|b| b.as_ref()).collect();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(refs.as_slice(), row_to_book)?;
        rows.collect()
    }

    pub fn get_book(&self, id: i64) -> Result<Option<Book>, rusqlite::Error> {
        let sql = format!("SELECT {COLUMNS} FROM books WHERE id = ?1");
        self.conn
            .query_row(&sql, params![id], row_to_book)
            .optional()
    }

    pub fn insert_book(&self, book: &Book) -> Result<i64, rusqlite::Error> {
        self.conn.execute(
            "INSERT INTO books (title, author, isbn, cover_path, status, rating, review, \
             date_finished, current_page, total_pages, is_favorite, bookmark_note, \
             malayalam_title, publisher, tags, file_path) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                book.title,
                book.author,
                book.isbn,
                book.cover_path,
                book.status.as_str(),
                book.rating,
                book.review,
                book.date_finished,
                book.current_page,
                book.total_pages,
                book.is_favorite as i64,
                book.bookmark_note.as_str(),
                book.malayalam_title.as_deref(),
                book.publisher.as_deref(),
                book.tags.as_str(),
                book.file_path.as_deref(),
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_book(&self, book: &Book) -> Result<(), rusqlite::Error> {
        self.conn.execute(
            "UPDATE books SET title = ?2, author = ?3, isbn = ?4, cover_path = ?5, status = ?6, \
             rating = ?7, review = ?8, date_finished = ?9, current_page = ?10, \
             total_pages = ?11, is_favorite = ?12, bookmark_note = ?13, \
             malayalam_title = ?14, publisher = ?15, tags = ?16, file_path = ?17 WHERE id = ?1",
            params![
                book.id,
                book.title,
                book.author,
                book.isbn,
                book.cover_path,
                book.status.as_str(),
                book.rating,
                book.review,
                book.date_finished,
                book.current_page,
                book.total_pages,
                book.is_favorite as i64,
                book.bookmark_note.as_str(),
                book.malayalam_title.as_deref(),
                book.publisher.as_deref(),
                book.tags.as_str(),
                book.file_path.as_deref(),
            ],
        )?;
        Ok(())
    }

    // ---- Reader profile ---------------------------------------------------

    pub fn load_profile(&self) -> Result<Profile, rusqlite::Error> {
        let row = self.conn.query_row(
            "SELECT name, avatar_initial, avatar_color, bio, yearly_goal, \
             preferred_language, favorite_publisher FROM profile WHERE id = 1",
            [],
            |row| {
                Ok(Profile {
                    name: row
                        .get::<_, Option<String>>(0)?
                        .unwrap_or_else(|| "Reader".into()),
                    avatar_initial: row
                        .get::<_, Option<String>>(1)?
                        .unwrap_or_else(|| "R".into()),
                    avatar_color: row
                        .get::<_, Option<String>>(2)?
                        .unwrap_or_else(|| "#b87333".into()),
                    bio: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                    yearly_goal: row.get::<_, Option<i64>>(4)?.unwrap_or(0),
                    preferred_language: row
                        .get::<_, Option<String>>(5)?
                        .unwrap_or_else(|| "Both".into()),
                    favorite_publisher: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
                })
            },
        );
        match row {
            Ok(profile) => Ok(profile),
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                let profile = Profile::default();
                self.save_profile(&profile)?;
                Ok(profile)
            }
            Err(error) => Err(error),
        }
    }

    pub fn save_profile(&self, profile: &Profile) -> Result<(), rusqlite::Error> {
        self.conn.execute(
            "INSERT INTO profile (id, name, avatar_initial, avatar_color, bio, yearly_goal, \
             preferred_language, favorite_publisher) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7) \
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, \
             avatar_initial = excluded.avatar_initial, avatar_color = excluded.avatar_color, \
             bio = excluded.bio, yearly_goal = excluded.yearly_goal, \
             preferred_language = excluded.preferred_language, \
             favorite_publisher = excluded.favorite_publisher",
            params![
                profile.name,
                profile.avatar_initial,
                profile.avatar_color,
                profile.bio,
                profile.yearly_goal,
                profile.preferred_language,
                profile.favorite_publisher,
            ],
        )?;
        Ok(())
    }

    // ---- VLC-style watched folders ---------------------------------------

    pub fn list_watched_folders(&self) -> Result<Vec<WatchedFolder>, rusqlite::Error> {
        let mut stmt = self.conn.prepare(
            "SELECT id, path, last_scanned FROM watched_folders ORDER BY path COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(WatchedFolder {
                id: row.get(0)?,
                path: row.get(1)?,
                last_scanned: row.get(2)?,
            })
        })?;
        rows.collect()
    }

    pub fn add_watched_folder(&self, path: &str) -> Result<(), rusqlite::Error> {
        self.conn.execute(
            "INSERT OR IGNORE INTO watched_folders (path) VALUES (?1)",
            params![path],
        )?;
        Ok(())
    }

    pub fn remove_watched_folder(&self, id: i64) -> Result<(), rusqlite::Error> {
        self.conn
            .execute("DELETE FROM watched_folders WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn mark_folder_scanned(&self, path: &str, timestamp: &str) -> Result<(), rusqlite::Error> {
        self.conn.execute(
            "UPDATE watched_folders SET last_scanned = ?2 WHERE path = ?1",
            params![path, timestamp],
        )?;
        Ok(())
    }

    /// Paths already imported by the folder scanner, used to skip duplicates.
    pub fn known_file_paths(&self) -> Result<std::collections::HashSet<String>, rusqlite::Error> {
        let mut stmt = self
            .conn
            .prepare("SELECT file_path FROM books WHERE file_path IS NOT NULL")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect()
    }

    pub fn delete_book(&self, id: i64) -> Result<(), rusqlite::Error> {
        self.conn
            .execute("DELETE FROM books WHERE id = ?1", params![id])?;
        Ok(())
    }
}

/// Tolerate NULL/empty values in databases edited outside the app.
fn text_or_empty(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<String> {
    Ok(row.get::<_, Option<String>>(index)?.unwrap_or_default())
}

fn int_or_zero(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<i64> {
    Ok(row.get::<_, Option<i64>>(index)?.unwrap_or(0))
}

fn row_to_book(row: &rusqlite::Row<'_>) -> rusqlite::Result<Book> {
    let status = text_or_empty(row, 5)?;
    let favorite = int_or_zero(row, 11)?;
    Ok(Book {
        id: int_or_zero(row, 0)?,
        title: text_or_empty(row, 1)?,
        author: text_or_empty(row, 2)?,
        isbn: text_or_empty(row, 3)?,
        cover_path: row.get(4)?,
        status: ReadStatus::from_db(&status),
        rating: row.get(6)?,
        review: text_or_empty(row, 7)?,
        date_finished: row.get(8)?,
        current_page: int_or_zero(row, 9)?,
        total_pages: int_or_zero(row, 10)?,
        is_favorite: favorite != 0,
        bookmark_note: text_or_empty(row, 12)?,
        malayalam_title: row.get(13)?,
        publisher: row.get(14)?,
        tags: text_or_empty(row, 15)?,
        file_path: row.get(16)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_db() -> Database {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        Database {
            conn,
            data_dir: PathBuf::new(),
            covers_dir: PathBuf::from("covers"),
            db_path: PathBuf::from("books.sqlite3"),
        }
    }

    #[test]
    fn fresh_data_dir_initializes_schema_and_profile() {
        let root = std::env::temp_dir().join(format!("thalir_fresh_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let db = Database::open_at(root.clone()).unwrap();
        assert!(db.covers_dir.is_dir());
        assert_eq!(db.load_profile().unwrap().name, "Reader");
        assert!(db
            .list_books("", None, false, SortKey::RecentlyAdded)
            .is_ok());
        drop(db);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn migration_adds_all_new_columns_to_old_databases() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE books (id INTEGER PRIMARY KEY, title TEXT NOT NULL);")
            .unwrap();
        Database::migrate(&conn).unwrap();
        let names: Vec<String> = conn
            .prepare("PRAGMA table_info(books)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .map(|name| name.unwrap())
            .collect();
        for column in [
            "bookmark_note",
            "malayalam_title",
            "publisher",
            "tags",
            "file_path",
        ] {
            assert!(names.iter().any(|name| name == column), "missing {column}");
        }
    }

    #[test]
    fn search_covers_malayalam_title_and_publisher() {
        let db = memory_db();
        let mut book = Book::new();
        book.title = "Oru Desathinte Katha".into();
        book.malayalam_title = Some("ഒരു ദേശത്തിന്റെ കഥ".into());
        book.publisher = Some("DC Books".into());
        book.author = "S. K. Pottekkatt".into();
        db.insert_book(&book).unwrap();

        assert_eq!(
            db.list_books("DC Books", None, false, SortKey::Title)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            db.list_books("Pottekkatt", None, false, SortKey::Title)
                .unwrap()
                .len(),
            1
        );
        // Malayalam search is case-insensitive only for ASCII; exact substring works.
        assert_eq!(
            db.list_books("ദേശത്തിന്റെ", None, false, SortKey::Title)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn crud_roundtrip() {
        let db = memory_db();
        let mut book = Book::new();
        book.title = "The Left Hand of Darkness".into();
        book.author = "Ursula K. Le Guin".into();
        book.isbn = "9780441478125".into();
        book.status = ReadStatus::Reading;
        book.rating = Some(4.5);
        book.current_page = 120;
        book.total_pages = 304;
        book.is_favorite = true;
        book.bookmark_note = "page 120 — the reveal".into();
        book.malayalam_title = Some("ഇടതു കൈയിലെ ഇരുള്‍".into());
        book.publisher = Some("DC Books".into());
        book.file_path = Some("C:/Books/left-hand.epub".into());

        let id = db.insert_book(&book).unwrap();
        assert!(id > 0);
        book.id = id;

        let found = db.get_book(id).unwrap().unwrap();
        assert_eq!(found.title, book.title);
        assert_eq!(found.status, ReadStatus::Reading);
        assert_eq!(found.rating, Some(4.5));
        assert!(found.is_favorite);
        assert_eq!(found.bookmark_note, "page 120 — the reveal");
        assert_eq!(found.malayalam_title.as_deref(), Some("ഇടതു കൈയിലെ ഇരുള്‍"));
        assert_eq!(found.publisher.as_deref(), Some("DC Books"));
        assert_eq!(found.file_path.as_deref(), Some("C:/Books/left-hand.epub"));
        assert!(db
            .known_file_paths()
            .unwrap()
            .contains("C:/Books/left-hand.epub"));

        book.rating = None;
        db.update_book(&book).unwrap();
        assert_eq!(db.get_book(id).unwrap().unwrap().rating, None);

        assert_eq!(
            db.list_books("Le Guin", None, false, SortKey::Title)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            db.list_books("", Some(ReadStatus::Read), false, SortKey::Title)
                .unwrap()
                .len(),
            0
        );
        assert_eq!(
            db.list_books("", None, true, SortKey::Title).unwrap().len(),
            1
        );

        db.delete_book(id).unwrap();
        assert!(db.get_book(id).unwrap().is_none());
    }

    /// Manual check against the real app-data database.
    #[test]
    #[ignore = "touches the real app-data database"]
    fn real_db_has_scanner_schema() {
        let db = Database::open().unwrap();
        let columns: Vec<String> = db
            .conn
            .prepare("PRAGMA table_info(books)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert!(columns.iter().any(|name| name == "file_path"));
        let tables: Vec<String> = db
            .conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert!(tables.iter().any(|name| name == "watched_folders"));
    }

    #[test]
    fn watched_folders_roundtrip() {
        let db = memory_db();
        db.add_watched_folder("C:/Books").unwrap();
        db.add_watched_folder("C:/Books").unwrap(); // UNIQUE, must not duplicate
        db.mark_folder_scanned("C:/Books", "2026-11-19 16:00:00")
            .unwrap();
        let folders = db.list_watched_folders().unwrap();
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].path, "C:/Books");
        assert_eq!(
            folders[0].last_scanned.as_deref(),
            Some("2026-11-19 16:00:00")
        );
        db.remove_watched_folder(folders[0].id).unwrap();
        assert!(db.list_watched_folders().unwrap().is_empty());
    }

    #[test]
    fn schema_rejects_invalid_values() {
        let db = memory_db();
        let mut book = Book::new();
        book.title = "Test".into();

        book.rating = Some(4.3); // not a half-star step
        assert!(db.insert_book(&book).is_err());
        book.rating = Some(5.5); // out of range
        assert!(db.insert_book(&book).is_err());
        book.rating = Some(0.5); // valid lower bound
        assert!(db.insert_book(&book).is_ok());

        book.status = ReadStatus::Read;
        book.rating = None;
        assert!(db.insert_book(&book).is_ok());
    }
}
