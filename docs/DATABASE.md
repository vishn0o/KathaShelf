# Database reference

KathaShelf stores everything in a single SQLite database next to a local cover
cache. The schema is created automatically on first launch and upgraded in
place on later launches.

## Location

Resolved via the [`directories`](https://crates.io/crates/directories) crate:

| OS | Data directory |
|---|---|
| Windows | `%APPDATA%\KathaShelf\KathaShelf\data\` |
| macOS | `~/Library/Application Support/com.KathaShelf.KathaShelf/` |
| Linux | `~/.local/share/KathaShelf/` |

Contents:

```text
books.sqlite3          # main database
books.sqlite3-wal      # WAL sidecars (while running)
books.sqlite3-shm
covers/                # cached cover images (.jpg, max 600x900)
prefs.txt              # persisted theme preference
wiki_awards.json       # monthly Wikipedia award-summary cache
```

Hover the book count in the toolbar to see the exact SQLite path.

## Connection settings

Applied on every connection:

```sql
PRAGMA foreign_keys = ON;
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA cache_size = -8000;   -- 8 MB
PRAGMA temp_store = MEMORY;
```

Indexes exist on `title` (case-insensitive), `status`, `is_favorite`, and
`file_path`.

## Schema

```sql
CREATE TABLE books (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    title           TEXT    NOT NULL,
    malayalam_title TEXT,
    publisher       TEXT,
    author          TEXT    NOT NULL DEFAULT '',
    isbn            TEXT    NOT NULL DEFAULT '',
    cover_path      TEXT,
    tags            TEXT    NOT NULL DEFAULT '',
    file_path       TEXT,
    status          TEXT    NOT NULL DEFAULT 'WantToRead'
                    CHECK (status IN ('Read','Reading','WantToRead')),
    rating          REAL    CHECK (rating IS NULL OR
                           (rating >= 0.5 AND rating <= 5.0 AND
                            rating * 2.0 = CAST(rating * 2.0 AS INTEGER))),
    review          TEXT    NOT NULL DEFAULT '',
    bookmark_note   TEXT,
    date_finished   TEXT,    -- YYYY-MM-DD
    current_page    INTEGER NOT NULL DEFAULT 0 CHECK (current_page >= 0),
    total_pages     INTEGER NOT NULL DEFAULT 0 CHECK (total_pages >= 0),
    is_favorite     INTEGER NOT NULL DEFAULT 0 CHECK (is_favorite IN (0,1))
);

CREATE TABLE profile (
    id                  INTEGER PRIMARY KEY CHECK (id = 1),
    name                TEXT NOT NULL DEFAULT 'Reader',
    avatar_initial      TEXT NOT NULL DEFAULT 'R',
    avatar_color        TEXT NOT NULL DEFAULT '#b87333',
    bio                 TEXT NOT NULL DEFAULT '',
    yearly_goal         INTEGER NOT NULL DEFAULT 0,
    preferred_language  TEXT NOT NULL DEFAULT 'Both',
    favorite_publisher  TEXT NOT NULL DEFAULT ''
);

CREATE TABLE watched_folders (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    path         TEXT UNIQUE NOT NULL,
    last_scanned TEXT
);
```

## Migrations

`Database::migrate()` adds any missing columns with safe `ALTER TABLE ... ADD
COLUMN` statements and creates the `profile`, `watched_folders`, and index
objects if absent. Missing/NULL text and integer values are tolerated when
reading rows, so hand-edited databases do not break the app.

Libraries created under the old **KadhaShelf** application-data name are reused
automatically on first run; no manual copy is needed.
