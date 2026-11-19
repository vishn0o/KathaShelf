//! VLC-style local folder scanner for EPUB and PDF books.
//!
//! Scanning only extracts metadata into `ScannedBook` previews (off the UI
//! thread). The user can edit those previews, and `import_scanned_books` then
//! downloads/caches covers and writes everything to SQLite on another thread.

use crate::covers;
use crate::db::Database;
use crate::models::Book;
use lopdf::{Dictionary, Document, Object};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use walkdir::WalkDir;

/// Extracted metadata awaiting the user's approval.
#[derive(Debug, Clone)]
pub struct ScannedBook {
    pub file_path: String,
    pub title: String,
    pub author: String,
    pub publisher: Option<String>,
    pub total_pages: i64,
    /// Embedded cover bytes plus a short tag for the cache filename.
    pub cover: Option<(Vec<u8>, String)>,
}

impl ScannedBook {
    pub fn dedupe_key(&self) -> String {
        normalize_key(&format!("{}|{}", self.title, self.author))
    }
}

/// Snapshot streamed to the UI while scanning or importing.
#[derive(Debug, Clone, Default)]
pub struct ScanProgress {
    pub done: bool,
    pub folders_total: usize,
    pub files_seen: usize,
    pub imported: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
    pub current: String,
    pub message: String,
    pub previews: Vec<ScannedBook>,
    pub importing: bool,
}

/// Extract metadata from the given folders. Called from a background thread.
pub fn scan_folders(folders: Vec<String>, tx: Sender<ScanProgress>) -> ScanProgress {
    let db = match Database::open() {
        Ok(db) => db,
        Err(error) => {
            let progress = ScanProgress {
                done: true,
                folders_total: folders.len(),
                errors: vec![format!("Could not open the library database: {error}")],
                ..ScanProgress::default()
            };
            let _ = tx.send(progress.clone());
            return progress;
        }
    };
    scan_folders_with_db(&db, folders, tx)
}

/// Same scan, but against an already-open database (used by tests).
fn scan_folders_with_db(
    db: &Database,
    folders: Vec<String>,
    tx: Sender<ScanProgress>,
) -> ScanProgress {
    let mut progress = ScanProgress {
        folders_total: folders.len(),
        ..ScanProgress::default()
    };
    let mut known_paths = db.known_file_paths().unwrap_or_else(|_| HashSet::new());
    let mut known_keys: HashSet<String> = db
        .list_books("", None, false, crate::db::SortKey::RecentlyAdded)
        .unwrap_or_default()
        .into_iter()
        .filter(|book| book.file_path.is_some())
        .map(|book| normalize_key(&format!("{}|{}", book.title, book.author)))
        .collect();
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    for folder in &folders {
        progress.current = folder.clone();
        let _ = tx.send(progress.clone());

        for entry in WalkDir::new(folder).into_iter().filter_map(Result::ok) {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if extension != "epub" && extension != "pdf" {
                continue;
            }

            progress.files_seen += 1;
            let file_path = normalized_path(path);
            let duplicate_path = known_paths.contains(&file_path);
            let scanned = if duplicate_path {
                None
            } else {
                match extract_one(path, &extension, &file_path) {
                    Ok(scanned) => Some(scanned),
                    Err(error) => {
                        progress.errors.push(format!("{}: {error}", path.display()));
                        None
                    }
                }
            };

            match scanned {
                Some(scanned) if !known_keys.contains(&scanned.dedupe_key()) => {
                    known_paths.insert(file_path);
                    known_keys.insert(scanned.dedupe_key());
                    progress.previews.push(scanned);
                }
                _ => progress.skipped += 1,
            }
            let _ = tx.send(progress.clone());
        }
        let _ = db.mark_folder_scanned(folder, &timestamp);
    }

    progress.done = true;
    progress.current.clear();
    let _ = tx.send(progress.clone());
    progress
}

/// Commit approved previews: covers (embedded or online) + SQLite rows.
pub fn import_scanned_books(books: Vec<ScannedBook>, tx: Sender<ScanProgress>) -> ScanProgress {
    let db = match Database::open() {
        Ok(db) => db,
        Err(error) => {
            let progress = ScanProgress {
                done: true,
                importing: true,
                errors: vec![format!("Could not open the library database: {error}")],
                ..ScanProgress::default()
            };
            let _ = tx.send(progress.clone());
            return progress;
        }
    };
    import_scanned_books_with_db(&db, books, tx)
}

fn import_scanned_books_with_db(
    db: &Database,
    books: Vec<ScannedBook>,
    tx: Sender<ScanProgress>,
) -> ScanProgress {
    let mut progress = ScanProgress {
        folders_total: books.len(),
        importing: true,
        ..ScanProgress::default()
    };
    for scanned in books {
        progress.current = scanned.title.clone();
        let title = progress.current.clone();
        let _ = tx.send(progress.clone());
        match commit_one(db, &scanned) {
            Ok(()) => progress.imported += 1,
            Err(error) => progress.errors.push(format!("{title}: {error}")),
        }
        let _ = tx.send(progress.clone());
    }
    progress.done = true;
    progress.current.clear();
    let _ = tx.send(progress.clone());
    progress
}

fn commit_one(db: &Database, scanned: &ScannedBook) -> Result<(), String> {
    let mut book = Book::new();
    book.title = scanned.title.trim().to_string();
    if book.title.is_empty() {
        book.title = "Untitled book".to_string();
    }
    book.author = scanned.author.trim().to_string();
    book.publisher = scanned.publisher.clone();
    book.total_pages = scanned.total_pages.max(0);
    book.file_path = Some(scanned.file_path.clone());

    let id = db.insert_book(&book).map_err(|e| e.to_string())?;
    book.id = id;

    let cover = scanned.cover.clone().or_else(|| fetch_online_cover(&book));
    let relative = match cover {
        Some((bytes, tag)) => covers::save_cover_bytes(db, id, &tag, &bytes)
            .or_else(|_| covers::save_placeholder(db, id)),
        None => covers::save_placeholder(db, id),
    }
    .map_err(|e| e.to_string())?;

    book.cover_path = Some(relative);
    db.update_book(&book).map_err(|e| e.to_string())
}

// ---- Extraction -----------------------------------------------------------

fn extract_one(path: &Path, extension: &str, file_path: &str) -> Result<ScannedBook, String> {
    if extension == "epub" {
        read_epub(path, file_path)
    } else {
        read_pdf(path, file_path)
    }
}

fn read_epub(path: &Path, file_path: &str) -> Result<ScannedBook, String> {
    let mut doc = epub::doc::EpubDoc::new(path).map_err(|e| format!("could not read EPUB: {e}"))?;
    let title = doc
        .get_title()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| filename_title(path));
    let author = doc
        .mdata("creator")
        .map(|item| item.value.trim().to_string())
        .unwrap_or_default();
    let publisher = doc
        .mdata("publisher")
        .map(|item| item.value.trim().to_string())
        .filter(|value| !value.is_empty());
    let total_pages = doc.get_num_chapters() as i64;
    let cover = doc
        .get_cover()
        .map(|(bytes, _mime)| (bytes, "epub".to_string()));

    Ok(ScannedBook {
        file_path: file_path.to_string(),
        title,
        author,
        publisher,
        total_pages,
        cover,
    })
}

fn read_pdf(path: &Path, file_path: &str) -> Result<ScannedBook, String> {
    let document = Document::load(path).map_err(|e| format!("could not read PDF: {e}"))?;
    let total_pages = document.get_pages().len() as i64;
    let info = document
        .trailer
        .get(b"Info")
        .and_then(Object::as_reference)
        .ok()
        .and_then(|id| document.get_dictionary(id).ok());

    let title = info
        .and_then(|dictionary| pdf_text(dictionary, b"Title"))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| filename_title(path));
    let author = info
        .and_then(|dictionary| pdf_text(dictionary, b"Author"))
        .unwrap_or_default();

    Ok(ScannedBook {
        file_path: file_path.to_string(),
        title,
        author,
        publisher: None,
        total_pages,
        // PDFs rarely expose an embedded cover through lopdf.
        cover: None,
    })
}

fn pdf_text(dictionary: &Dictionary, key: &[u8]) -> Option<String> {
    let bytes = dictionary.get(key).ok()?.as_str().ok()?;
    let decoded = if bytes.starts_with(&[0xFE, 0xFF]) {
        let units: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_be_bytes(*pair))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    };
    let cleaned: String = decoded
        .chars()
        .filter(|character| !character.is_control())
        .collect::<String>()
        .trim()
        .to_string();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

fn fetch_online_cover(book: &Book) -> Option<(Vec<u8>, String)> {
    let query = format!("{} {}", book.title, book.author);
    let query = query.trim();
    if query.is_empty() {
        return None;
    }
    let hit = crate::openlibrary::search_books(query)
        .ok()?
        .into_iter()
        .find(|hit| hit.has_cover())?;
    let bytes = if let Some(url) = &hit.cover_url {
        crate::openlibrary::fetch_cover_url(url).ok()?
    } else {
        crate::openlibrary::fetch_cover(hit.cover_id?).ok()?
    };
    Some((
        bytes,
        format!("online_{}", hit.source.label()).replace(' ', "_"),
    ))
}

fn normalize_key(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn normalized_path(path: &Path) -> String {
    let absolute: PathBuf = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    absolute.to_string_lossy().replace('\\', "/")
}

fn filename_title(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Untitled book");
    let spaced = stem.replace(['_', '-'], " ");
    let collapsed: String = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        "Untitled book".to_string()
    } else {
        collapsed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filename_becomes_a_readable_title() {
        let path = Path::new("/books/O_V_Vijayan_-_Khasakkinte-Ithihasam.epub");
        assert_eq!(filename_title(path), "O V Vijayan Khasakkinte Ithihasam");
    }

    #[test]
    fn normalized_paths_use_forward_slashes() {
        let path = Path::new("C:/Books/Dune.pdf");
        assert_eq!(normalized_path(path), "C:/Books/Dune.pdf");
    }

    /// Manual check with a real PDF:
    /// `THALIR_TEST_PDF=... cargo test real_pdf_parses -- --ignored --nocapture`
    #[test]
    #[ignore = "requires a local PDF path"]
    fn real_pdf_parses() {
        let Ok(path) = std::env::var("THALIR_TEST_PDF") else {
            return;
        };
        let scanned = read_pdf(Path::new(&path), &path).expect("PDF parse failed");
        println!(
            "{} | {} | {} pages | cover={}",
            scanned.title,
            scanned.author,
            scanned.total_pages,
            scanned.cover.is_some()
        );
        assert!(!scanned.title.is_empty());
        assert!(scanned.total_pages > 0);
    }

    /// Manual end-to-end check with a real PDF and a throwaway database:
    /// `THALIR_TEST_PDF=... cargo test scans_a_real_pdf -- --ignored --nocapture`
    #[test]
    #[ignore = "requires a local PDF path"]
    fn scans_a_real_pdf() {
        use std::sync::mpsc;

        let Ok(source) = std::env::var("THALIR_TEST_PDF") else {
            return;
        };
        let root = std::env::temp_dir().join(format!("thalir_scan_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let books_dir = root.join("books");
        std::fs::create_dir_all(&books_dir).unwrap();
        std::fs::copy(&source, books_dir.join("Sample Book.pdf")).unwrap();

        let db = Database::open_at(root.clone()).unwrap();
        let (tx, _rx) = mpsc::channel();
        let progress =
            scan_folders_with_db(&db, vec![books_dir.to_string_lossy().into_owned()], tx);
        println!("{progress:?}");
        assert!(progress.done);
        assert_eq!(progress.previews.len(), 1);

        let (import_tx, _import_rx) = mpsc::channel();
        let imported = import_scanned_books_with_db(&db, progress.previews, import_tx);
        assert_eq!(imported.imported, 1);
        let books = db
            .list_books("", None, false, crate::db::SortKey::RecentlyAdded)
            .unwrap();
        assert_eq!(books.len(), 1);
        assert!(books[0].file_path.is_some());
        assert!(books[0].cover_path.is_some());
        std::fs::remove_dir_all(&root).ok();
    }
}
