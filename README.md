# KathaShelf

A local-first desktop book-tracking app written in pure Rust with
**[eframe/egui](https://github.com/emilk/egui)** (GUI) and
**[rusqlite](https://github.com/rusqlite/rusqlite)** (SQLite). No Node.js, no web
server, no cloud account — the database and all cover images live on your machine.

## Features

- Track English title plus an optional **Malayalam title** (`malayalam_title`,
  മലയാളം), author, optional **publisher** (DC Books, Mathrubhumi Books, Olive,
  Green Books…), ISBN, status, rating, review, bookmark note, finish date, page
  progress and a favourite flag. The **Manjari** font is bundled so Malayalam
  renders correctly (native egui apps load fonts via `FontDefinitions`, not CSS).
- Ratings snap to **half-star increments from 0.5 to 5.0** (enforced in both the
  UI and a SQLite `CHECK` constraint).
- **Three-source book search** across **Google Books**, **Open Library** and a
  **Goodreads HTML scraper**, queried concurrently on background threads with
  `std::thread::scope` (the egui UI thread never blocks). Results carry a source
  badge and full metadata — title, author(s), publisher, year, page count,
  description, ISBN and cover URL. Duplicates are merged by normalized
  title+author, missing fields are filled from other providers, and entries with
  a usable cover are ranked first. The Goodreads fallback is especially useful
  for Malayalam/Indian-publisher titles that the APIs miss; its `._SX50_` /
  `._SY75_` thumbnails are upgraded to full-size covers. A failing or
  rate-limited provider never hides the other results.
  **Find book online** shows results with a source badge; select one to preview
  it, then click **Use this result** to apply it. Malayalam text found in a result
  is automatically moved into the Malayalam title field. The cover is downloaded
  on a **background thread**, cached in `covers/`, decoded into an egui texture,
  and the metadata is saved to SQLite. If no cover exists, a generated placeholder
  is used instead.
- **VLC-style local folder scanner**: pick a folder with **📁 Scan Folder**, and
  the app remembers it in `watched_folders` just like a media library. A
  background `std::thread` recursively finds `.epub` and `.pdf` files, skipping
  anything already in SQLite by exact `file_path` **or** normalized title+author.
  EPUB title/author/publisher and embedded covers, plus the PDF Info dictionary
  and page count (with a cleaned filename fallback), are extracted into an
  **editable Review scanned books** dialog — you can fix metadata and drop rows
  before importing. Confirming downloads any missing covers online on another
  background thread and writes everything to SQLite. **Rescan All** and per-folder
  **Remove** live in the collapsible **Library folders** toolbar; Remove asks
  whether imported books should also be deleted. Live progress, a scan report of
  skipped/failed files, and automatic shelf reload are included. Double-clicking
  an imported cover opens the source file; the detail modal also has **📖 Open
  Book File**. Hover actions on covers offer ▶ Reading / ✓ Finished / ♥ / ✕.
- **Discover view**: Spotify-style top **10 / 20 / 25** book charts by genre
  (Fiction, Mystery, Poetry, Science Fiction, History, Biography, Children,
  Malayalam literature) and language (English / Malayalam / Both), fetched from
  Open Library. A **🎲 Surprise me** button picks a random chart entry and can
  add it straight to the library. An **Award-winning books** section shows
  curated winners for the Booker, International Booker, Pulitzer, Jnanpith,
  Kerala Sahitya Akademi, Sahitya Akademi, National Book Award, Women's Prize
  and JCB Prize, plus Wikipedia summaries cached locally and refreshed
  automatically once a month in the background.
- **Reader profile**: click the sidebar footer to open a Profile view with
  editable name, avatar, bio, yearly goal, preferred language and favourite
  publisher. A playful **reader archetype** (e.g. *Malayalam Monsoon Reader*,
  *Cover Collector*, *Half-Star Mystic*) and badges are derived from your
  library, together with local recommendations and a "find more like this"
  action.
- **Global header search**: the header field searches Google Books, Open Library
  and Goodreads worldwide; selecting a result fills **all** form fields
  (title, Malayalam title, author, publisher, ISBN, pages, rating, description)
  and downloads the cover. A compact "Filter this shelf…" field in the sort row
  keeps local filtering.
- **Unified Log / Edit Book modal** — opens when you click any book cover or hit
  **＋ Log Book**. It contains:
  - **Letterboxd Log**: calendar date-finished picker, interactive half-star
    rating (0.5–5.0), ♥ favorite toggle, status selector, and a Markdown-friendly
    review/notes box with a rendered preview (`# heading`, `- bullet`,
    `**bold**`, `*italic*`).
  - **Personal Bookmark Tracker**: `current_page` / `total_pages` inputs, a live
    progress bar, and a **Bookmark Note** field for the exact page and thought
    where you left off.
  - Inline three-source search, cover-by-ISBN download, and local cover picker.
  - Validation feedback (including ISBN errors) is **modal-scoped only** — it
    never leaks onto the library page. Clicking the backdrop, pressing
    **Escape**, or using **Cancel / ✕** closes the dialog and resets its form.
  - **Cover dropzone**: drop an image file, paste a copied cover with
    **Ctrl+V / ⌘V**, or paste a **direct image URL** (from DC Books,
    Mathrubhumi, Goodreads, etc.). Everything is cached locally into `covers/`.
- **Manual add** with a locally chosen cover image (file picker), no network needed.
- **Open-book e-reader UI**: fixed **240px** sidebar + scrollable main content
  pane with a 28px/36px inner gutter, **Cozy Night (default, persisted)** and
  **Warm Paper** themes toggled from the header pill. A single token set drives
  every surface, border, control, input, modal and shelf in both themes.
  - Sidebar: **Thalir / തളിർ** serif brand, left-aligned pill navigation (My
    Library, Reading Diary, Favorites, Publishers, Log Book), a pinned
    **Continue Reading** bookmark card with mini shelf and page progress, and a
    compact profile footer. A dashed "stitching" border separates it from the
    page.
  - Main area: segmented **Shelves / All Books** toggle, rounded full-width
    search, sort/favorites row, and theme toggle.
  - **Shelves view**: tactile 3D shelves for Currently reading, Next up and
    Finished (plus a Favorites shelf), with "Full shelf →" links. Covers are
    2:3 with spine highlights and sit on rounded ledges with soft shadows;
    ratings, hearts and page progress appear subtly on hover.
  - **All Books view**: responsive poster grid with hover quick-actions.
- Search across **title, Malayalam title, author and publisher**, with a
  favorites toggle and sort by Date Finished / Highest Rated / Title.
- Bundled **Lora** (serif headings), **Inter** (UI) and **Manjari** (Malayalam)
  fonts — no web fonts or runtime downloads.
- **Offline-first covers**: every cover (downloaded, imported, or placeholder)
  is cached locally, so after adding a book the app never needs the network.
- SQLite database and cover folder are created automatically on first launch.

## Requirements

- Rust toolchain (stable; install from <https://rustup.rs>).
- A C compiler, because `rusqlite` builds a bundled SQLite from source
  (`bundled` feature). On Windows, the MSVC build tools or a MinGW-w64 `gcc`
  both work; on macOS/Linux, Xcode CLT / GCC or Clang.

## Build & run

This is a pure eframe/egui app (no Tauri / web stack), so there is no
`src-tauri/` directory and `cargo tauri build` does not apply. Build the
standalone executable with:

```bash
cargo build --release
```

The resulting `.exe` is `target\release\kathashelf.exe` on Windows. The release
build sets `windows_subsystem = "windows"`, so it is a GUI-subsystem binary and
opens with **no cmd/terminal window**. Debug builds (`cargo run`) keep the
console so compiler/runtime logs remain visible.

## Where data is stored

Everything goes under the OS application-data directory (resolved with the
`directories` crate), e.g.:

| OS      | Location |
|---------|----------|
| Windows | `%APPDATA%\KathaShelf\KathaShelf\data\` |
| macOS   | `~/Library/Application Support/com.KathaShelf.KathaShelf/` (or the `directories` equivalent) |
| Linux   | `~/.local/share/KathaShelf/` (or `$XDG_DATA_HOME/KathaShelf/`) |

Inside that folder:

```
books.sqlite3          # the database
books.sqlite3-wal      # SQLite WAL sidecars
books.sqlite3-shm
covers/                # cached cover images (.jpg)
```

Hover the book count in the filter bar to see the exact SQLite path.

## Database schema

```sql
CREATE TABLE books (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    title          TEXT    NOT NULL,
    malayalam_title TEXT,                        -- Malayalam-script title (nullable)
    publisher      TEXT,                        -- e.g. DC Books, Olive (nullable)
    author         TEXT    NOT NULL DEFAULT '',
    isbn           TEXT    NOT NULL DEFAULT '',
    cover_path     TEXT,                         -- relative path into the data dir
    tags           TEXT NOT NULL DEFAULT '',     -- genres / tags
    file_path      TEXT,                         -- absolute path of an imported EPUB/PDF
    status         TEXT    NOT NULL DEFAULT 'WantToRead'
                   CHECK (status IN ('Read','Reading','WantToRead')),
    rating         REAL    CHECK (rating IS NULL OR
                           (rating >= 0.5 AND rating <= 5.0 AND
                            rating * 2.0 = CAST(rating * 2.0 AS INTEGER))),
    review         TEXT    NOT NULL DEFAULT '',
    bookmark_note  TEXT,                         -- page + thought where you left off (nullable)
    date_finished  TEXT,                         -- YYYY-MM-DD
    current_page   INTEGER NOT NULL DEFAULT 0 CHECK (current_page >= 0),
    total_pages    INTEGER NOT NULL DEFAULT 0 CHECK (total_pages >= 0),
    is_favorite    INTEGER NOT NULL DEFAULT 0 CHECK (is_favorite IN (0,1))
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

Existing databases are upgraded automatically with `ALTER TABLE ... ADD COLUMN`
for any missing `bookmark_note`, `malayalam_title`, `publisher` or `file_path`
column, and the `watched_folders` table/indexes are created if absent.

## Project layout

```
src/
  main.rs     # entry point / eframe setup
  app.rs      # egui UI and app state
  db.rs       # schema, connection setup, CRUD, filters/sorting
  covers.rs   # cover download, placeholder generation, local import, cleanup
  openlibrary.rs # Google Books + Open Library + Goodreads search, charts, Wikipedia cache
  awards.rs     # curated literary-award winners
  reader.rs     # reader archetype, badges and profile stats
  scan.rs       # background EPUB/PDF folder scanner and metadata extraction
  models.rs   # Book / ReadStatus + validation helpers
assets/
  Lora.ttf             # serif brand + shelf headings
  Inter.ttf            # UI body text
  Manjari-Regular.ttf  # Malayalam glyphs
```
