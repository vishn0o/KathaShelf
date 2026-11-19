//! Open-book e-reader interface: two columns, tactile shelves, two themes.

use crate::covers;
use crate::db::{Database, SortKey, WatchedFolder};
use crate::models::{normalize_date, normalize_rating, Book, Profile, ReadStatus};
use crate::openlibrary::{DownloadLink, SearchHit};
use crate::reader;
use crate::scan::{ScanProgress, ScannedBook};
use egui_extras::DatePickerButton;
use std::collections::HashMap;
use std::error::Error;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};

// ---------------------------------------------------------------------------
// Theme
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct Theme {
    is_dark: bool,
    page: egui::Color32,
    sidebar: egui::Color32,
    /// `--bg-elevated`: cards, shelves, modals.
    card: egui::Color32,
    /// `--bg-input`: text fields, dropdowns.
    input: egui::Color32,
    /// `--border-subtle`: 1px hairlines and stitching.
    stitch: egui::Color32,
    ledge: egui::Color32,
    ledge_top: egui::Color32,
    pill_active: egui::Color32,
    accent: egui::Color32,
    text: egui::Color32,
    muted: egui::Color32,
    shadow: egui::Color32,
    danger: egui::Color32,
}

impl Theme {
    fn dark() -> Self {
        Self {
            is_dark: true,
            page: egui::Color32::from_rgb(0x16, 0x15, 0x13),
            sidebar: egui::Color32::from_rgb(0x11, 0x10, 0x0e),
            card: egui::Color32::from_rgb(0x1f, 0x1d, 0x1a),
            input: egui::Color32::from_rgb(0x26, 0x23, 0x20),
            stitch: egui::Color32::from_rgb(0x2e, 0x2a, 0x25),
            ledge: egui::Color32::from_rgb(0x3b, 0x2f, 0x24),
            ledge_top: egui::Color32::from_rgb(0x6b, 0x53, 0x3c),
            pill_active: egui::Color32::from_rgb(0x2b, 0x27, 0x22),
            accent: egui::Color32::from_rgb(0xe0, 0xa9, 0x6d),
            text: egui::Color32::from_rgb(0xf2, 0xed, 0xe4),
            muted: egui::Color32::from_rgb(0x9c, 0x94, 0x88),
            shadow: egui::Color32::from_black_alpha(120),
            danger: egui::Color32::from_rgb(0xe0, 0x8a, 0x7a),
        }
    }

    fn light() -> Self {
        Self {
            is_dark: false,
            page: egui::Color32::from_rgb(0xfa, 0xf6, 0xf0),
            sidebar: egui::Color32::from_rgb(0xf4, 0xef, 0xe6),
            card: egui::Color32::from_rgb(0xff, 0xff, 0xff),
            input: egui::Color32::from_rgb(0xed, 0xe6, 0xda),
            stitch: egui::Color32::from_rgb(0xe2, 0xd9, 0xcb),
            ledge: egui::Color32::from_rgb(0xd9, 0xb8, 0x8f),
            ledge_top: egui::Color32::from_rgb(0xf0, 0xd9, 0xb5),
            pill_active: egui::Color32::from_rgb(0xf0, 0xe9, 0xdf),
            accent: egui::Color32::from_rgb(0xb8, 0x73, 0x33),
            text: egui::Color32::from_rgb(0x26, 0x22, 0x1d),
            muted: egui::Color32::from_rgb(0x78, 0x70, 0x65),
            shadow: egui::Color32::from_black_alpha(40),
            danger: egui::Color32::from_rgb(0xb1, 0x46, 0x2f),
        }
    }

    fn serif(&self, size: f32) -> egui::FontId {
        egui::FontId::new(size, egui::FontFamily::Name("Lora".into()))
    }

    /// Readable text color on top of the warm accent fill.
    fn on_accent(&self) -> egui::Color32 {
        if self.is_dark {
            self.page
        } else {
            egui::Color32::WHITE
        }
    }
}

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MainView {
    Shelves,
    AllBooks,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NavItem {
    Library,
    Diary,
    Favorites,
    Lists,
    Discover,
    Profile,
}

#[derive(Debug, Clone)]
enum UiAction {
    Open(i64),
    OpenFile(String),
    ToggleFavorite(i64),
    SetStatus(i64, ReadStatus),
    SetBookmark(i64, i64),
    Delete(i64),
    Nav(NavItem),
    Shelf(Option<ReadStatus>),
    View(MainView),
    ToggleTheme,
    LogNew,
    ScanFolder,
    GlobalSearch,
    AddGlobalResult(SearchHit),
    LoadChart,
    SaveProfile,
    RemoveFolder(i64),
    RescanAll,
}

/// A cover downloaded off the UI thread after picking a search result.
struct PendingCover {
    job_id: u64,
    temp_id: i64,
    tag: String,
    bytes: Vec<u8>,
}

/// Everything the Log / Edit modal needs. `book.id == 0` means a new book.
struct LogEdit {
    book: Book,
    rating: f64,
    date: jiff::civil::Date,
    date_set: bool,
    cover: Option<String>,
    search_open: bool,
    search_query: String,
    search_results: Vec<SearchHit>,
    selected_search: Option<usize>,
    search_receiver: Option<Receiver<Result<Vec<SearchHit>, String>>>,
    search_error: String,
    /// Identifies the cover download belonging to this modal session.
    cover_job_id: Option<u64>,
    review_preview: bool,
    malayalam_title_text: String,
    publisher_text: String,
    bookmark_note_text: String,
    cover_url_text: String,
    cover_options_open: bool,
    /// Modal-scoped feedback; never shown on the main library page.
    message: String,
}

impl LogEdit {
    fn from_book(book: Book) -> Self {
        let date = book
            .date_finished
            .as_deref()
            .and_then(normalize_date)
            .and_then(|text| text.parse::<jiff::civil::Date>().ok())
            .unwrap_or_else(today);
        let date_set = book.date_finished.is_some();
        let rating = book.rating.unwrap_or(0.0);
        let cover = book.cover_path.clone();
        let malayalam_title_text = book.malayalam_title.clone().unwrap_or_default();
        let publisher_text = book.publisher.clone().unwrap_or_default();
        let bookmark_note_text = book.bookmark_note.clone();
        let is_new_book = book.id == 0;
        Self {
            book,
            rating,
            date,
            date_set,
            cover,
            search_open: is_new_book,
            search_query: String::new(),
            search_results: Vec::new(),
            selected_search: None,
            search_receiver: None,
            search_error: String::new(),
            cover_job_id: None,
            review_preview: false,
            malayalam_title_text,
            publisher_text,
            bookmark_note_text,
            cover_url_text: String::new(),
            cover_options_open: false,
            message: String::new(),
        }
    }

    fn start_search(&mut self, ctx: &egui::Context) {
        let query = self.search_query.clone();
        let repaint_ctx = ctx.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(crate::openlibrary::search_books(&query));
            repaint_ctx.request_repaint();
        });
        self.search_receiver = Some(rx);
        self.search_results.clear();
        self.selected_search = None;
        self.search_error.clear();
    }

    fn poll_search(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.search_receiver else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(results)) => {
                ctx.request_repaint();
                self.search_results = results;
                self.search_receiver = None;
            }
            Ok(Err(error)) => {
                ctx.request_repaint();
                self.search_error = error;
                self.search_receiver = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => self.search_receiver = None,
        }
    }
}

pub struct BookApp {
    db: Database,
    books: Vec<Book>,
    total_count: usize,
    /// Cached on refresh so the UI never queries SQLite per frame.
    reading_count: usize,
    continue_book: Option<Book>,
    book_count_label: String,
    profile_footer_label: String,
    theme_applied: Option<bool>,

    library_search: String,
    status_filter: Option<ReadStatus>,
    favorites_only: bool,
    sort: SortKey,

    dark_mode: bool,
    view: MainView,
    nav: NavItem,
    expanded_shelf: Option<ReadStatus>,

    /// Editable reader identity shown in the Profile view and sidebar footer.
    profile: Profile,
    /// Discover view: charts, random picks and the monthly Wikipedia cache.
    discover_genre: String,
    discover_language: String,
    discover_limit: usize,
    chart_receiver: Option<Receiver<Result<Vec<SearchHit>, String>>>,
    chart_results: Vec<SearchHit>,
    chart_error: String,
    chart_running: bool,
    random_pick: Option<SearchHit>,
    wiki_cache: serde_json::Value,
    wiki_refresh_started: bool,
    wiki_refresh_receiver: Option<Receiver<Result<serde_json::Value, String>>>,

    cover_textures: HashMap<String, egui::TextureHandle>,
    log_edit: Option<LogEdit>,
    confirm_open: bool,
    confirm_id: Option<i64>,
    /// Feedback that belongs only inside the delete confirmation modal.
    confirm_message: String,
    cover_seq: i64,
    /// Frame counters. An egui Modal's backdrop sees the very click that
    /// opened it, so `should_close` must be ignored for that one frame.
    frame_no: u64,
    log_opened_frame: Option<u64>,
    delete_opened_frame: Option<u64>,
    /// Background cover downloads for picked search results.
    cover_receiver: Option<Receiver<Result<PendingCover, String>>>,
    cover_job_seq: u64,
    /// Global internet search driven by the header field.
    global_query: String,
    global_receiver: Option<Receiver<Result<Vec<SearchHit>, String>>>,
    global_results: Vec<SearchHit>,
    global_error: String,
    global_selected: Option<usize>,
    /// Short online summary shown inside the detail modal.
    summary_receiver: Option<Receiver<Result<String, String>>>,
    summary_text: Option<String>,
    summary_error: String,
    summary_key: String,
    /// Legitimate "get a copy" links for the open book.
    download_links_receiver: Option<Receiver<Result<Vec<DownloadLink>, String>>>,
    download_links: Vec<DownloadLink>,
    download_links_error: String,
    download_links_key: String,
    /// VLC-style local library folders and live scan state.
    watched_folders: Vec<WatchedFolder>,
    show_folders: bool,
    show_local_filter: bool,
    scan_receiver: Option<Receiver<ScanProgress>>,
    scan_status: Option<ScanProgress>,
    /// Editable pre-import review of files found by the scanner.
    scan_preview_open: bool,
    scan_preview: Vec<ScannedBook>,
    scan_preview_opened_frame: Option<u64>,
    import_receiver: Option<Receiver<ScanProgress>>,
    import_status: Option<ScanProgress>,
    /// Inline bookmark editor in the Continue Reading card.
    bookmark_edit: bool,
    bookmark_page: i64,
    /// Confirmation for removing a watched folder.
    folder_remove_open: bool,
    folder_remove_id: Option<i64>,
    folder_remove_path: String,
    folder_remove_opened_frame: Option<u64>,
}

impl BookApp {
    pub fn new() -> Result<Self, Box<dyn Error>> {
        let db = Database::open()?;
        // egui has no localStorage; a tiny prefs file next to the database
        // remembers the theme between launches.
        let dark_mode = load_dark_pref(&db.data_dir).unwrap_or(true);
        let profile = db.load_profile().unwrap_or_default();
        let wiki_cache = load_wiki_cache(&db.data_dir).unwrap_or(serde_json::Value::Null);
        let mut app = Self {
            db,
            books: Vec::new(),
            total_count: 0,
            reading_count: 0,
            continue_book: None,
            book_count_label: "0 books".into(),
            profile_footer_label: "0 books  ·  0 reading".into(),
            theme_applied: None,
            library_search: String::new(),
            status_filter: None,
            favorites_only: false,
            sort: SortKey::RecentlyAdded,
            dark_mode,
            view: MainView::Shelves,
            nav: NavItem::Library,
            expanded_shelf: None,
            profile,
            discover_genre: "Fiction".into(),
            discover_language: "Both".into(),
            discover_limit: 10,
            chart_receiver: None,
            chart_results: Vec::new(),
            chart_error: String::new(),
            chart_running: false,
            random_pick: None,
            wiki_cache,
            wiki_refresh_started: false,
            wiki_refresh_receiver: None,
            cover_textures: HashMap::new(),
            log_edit: None,
            confirm_open: false,
            confirm_id: None,
            confirm_message: String::new(),
            cover_seq: 0,
            frame_no: 0,
            log_opened_frame: None,
            delete_opened_frame: None,
            cover_receiver: None,
            cover_job_seq: 0,
            global_query: String::new(),
            global_receiver: None,
            global_results: Vec::new(),
            global_error: String::new(),
            global_selected: None,
            summary_receiver: None,
            summary_text: None,
            summary_error: String::new(),
            summary_key: String::new(),
            download_links_receiver: None,
            download_links: Vec::new(),
            download_links_error: String::new(),
            download_links_key: String::new(),
            watched_folders: Vec::new(),
            show_folders: false,
            show_local_filter: false,
            scan_receiver: None,
            scan_status: None,
            scan_preview_open: false,
            scan_preview: Vec::new(),
            scan_preview_opened_frame: None,
            import_receiver: None,
            import_status: None,
            bookmark_edit: false,
            bookmark_page: 0,
            folder_remove_open: false,
            folder_remove_id: None,
            folder_remove_path: String::new(),
            folder_remove_opened_frame: None,
        };
        app.refresh()?;
        app.reload_watched_folders()?;
        Ok(app)
    }

    fn theme(&self) -> Theme {
        if self.dark_mode {
            Theme::dark()
        } else {
            Theme::light()
        }
    }

    fn refresh(&mut self) -> Result<(), Box<dyn Error>> {
        let mut books = self.db.list_books(
            &self.library_search,
            self.status_filter,
            self.favorites_only,
            self.sort,
        )?;
        let all = self
            .db
            .list_books("", None, false, SortKey::RecentlyAdded)?;
        self.total_count = all.len();
        self.reading_count = all
            .iter()
            .filter(|book| book.status == ReadStatus::Reading)
            .count();
        self.continue_book = all
            .iter()
            .find(|book| book.status == ReadStatus::Reading)
            .cloned();
        self.book_count_label = format!(
            "{} book{}",
            self.total_count,
            if self.total_count == 1 { "" } else { "s" }
        );
        self.profile_footer_label = format!(
            "{} books  ·  {} reading",
            self.total_count, self.reading_count
        );

        match self.nav {
            NavItem::Diary => books.retain(|book| {
                !book.review.trim().is_empty()
                    || !book.bookmark_note.trim().is_empty()
                    || book.date_finished.is_some()
            }),
            NavItem::Favorites => books.retain(|book| book.is_favorite),
            NavItem::Lists => books.retain(|book| book.publisher.is_some()),
            NavItem::Library | NavItem::Discover | NavItem::Profile => {}
        }
        if let Some(status) = self.expanded_shelf {
            books.retain(|book| book.status == status);
        }
        self.books = books;
        Ok(())
    }

    fn open_log_edit(&mut self, id: i64) {
        if let Ok(Some(book)) = self.db.get_book(id) {
            self.log_edit = Some(LogEdit::from_book(book));
            self.log_opened_frame = Some(self.frame_no);
        }
    }

    fn open_new_log(&mut self) {
        let mut book = Book::new();
        book.status = ReadStatus::WantToRead;
        self.log_edit = Some(LogEdit::from_book(book));
        self.log_opened_frame = Some(self.frame_no);
    }

    fn next_temp_id(&mut self) -> i64 {
        self.cover_seq -= 1;
        self.cover_seq
    }

    fn load_cover_texture(&mut self, ctx: &egui::Context, relative: &str) {
        if self.cover_textures.contains_key(relative) {
            return;
        }
        if let Ok(image) = image::open(self.db.absolute_cover_path(relative)) {
            // Thumbnail before upload: shelf/grid cards never need more than
            // ~300x450px, so this keeps VRAM predictable for large libraries.
            let image = if image.width() > 300 || image.height() > 450 {
                image.thumbnail(300, 450)
            } else {
                image
            };
            let rgba = image.to_rgba8();
            let color = egui::ColorImage::from_rgba_unmultiplied(
                [rgba.width() as usize, rgba.height() as usize],
                rgba.as_raw(),
            );
            self.cover_textures.insert(
                relative.to_string(),
                ctx.load_texture(relative, color, egui::TextureOptions::LINEAR),
            );
        }
    }

    fn save_log_edit(&mut self) {
        let Some(edit) = &mut self.log_edit else {
            return;
        };
        if edit.book.title.trim().is_empty() {
            edit.message = "A title is required.".into();
            return;
        }
        edit.book.title = edit.book.title.trim().to_string();
        edit.book.author = edit.book.author.trim().to_string();
        edit.book.isbn = edit.book.isbn.trim().to_string();
        edit.book.malayalam_title = optional_text(&edit.malayalam_title_text);
        edit.book.publisher = optional_text(&edit.publisher_text);
        edit.book.bookmark_note = edit.bookmark_note_text.trim().to_string();
        edit.book.rating = normalize_rating(edit.rating);
        edit.book.date_finished = if edit.date_set {
            Some(edit.date.to_string())
        } else {
            None
        };
        edit.book.cover_path = edit.cover.clone();
        edit.book.current_page = edit.book.current_page.max(0);
        edit.book.total_pages = edit.book.total_pages.max(0);

        let is_new = edit.book.id == 0;
        let result = if is_new {
            self.db.insert_book(&edit.book)
        } else {
            self.db.update_book(&edit.book).map(|()| edit.book.id)
        };

        match result {
            Ok(id) => {
                if let Some(edit) = &mut self.log_edit {
                    edit.book.id = id;
                }
                self.log_edit = None;
                let _ = self.refresh();
            }
            Err(error) => {
                self.modal_message(format!("Could not save: {error}"));
            }
        }
    }

    fn cancel_log_edit(&mut self) {
        if let Some(edit) = &self.log_edit {
            if edit.book.id == 0 {
                if let Some(relative) = &edit.cover {
                    crate::covers::remove_cover(&self.db, relative);
                    self.cover_textures.remove(relative);
                }
            }
        }
        self.log_edit = None;
        self.cover_receiver = None;
    }

    fn toggle_favorite(&mut self, id: i64) {
        if let Ok(Some(mut book)) = self.db.get_book(id) {
            book.is_favorite = !book.is_favorite;
            if self.db.update_book(&book).is_ok() {
                let _ = self.refresh();
            }
        }
    }

    fn delete_book(&mut self, id: i64) {
        if let Ok(Some(book)) = self.db.get_book(id) {
            if let Some(relative) = &book.cover_path {
                crate::covers::remove_cover(&self.db, relative);
                self.cover_textures.remove(relative);
            }
        }
        match self.db.delete_book(id) {
            Ok(()) => {
                self.log_edit = None;
                self.confirm_open = false;
                self.confirm_id = None;
                self.confirm_message.clear();
                let _ = self.refresh();
            }
            Err(error) => self.confirm_message = format!("Could not delete: {error}"),
        }
    }

    fn apply_theme(&self, ctx: &egui::Context, theme: Theme) {
        let mut visuals = if theme.is_dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        visuals.panel_fill = theme.page;
        visuals.window_fill = theme.card;
        visuals.extreme_bg_color = theme.input;
        visuals.text_edit_bg_color = Some(theme.input);
        visuals.faint_bg_color = theme.card;
        visuals.hyperlink_color = theme.accent;
        visuals.window_stroke = egui::Stroke::new(1.0, theme.stitch);
        visuals.window_corner_radius = egui::CornerRadius::same(16);
        visuals.menu_corner_radius = egui::CornerRadius::same(12);
        visuals.window_shadow = egui::epaint::Shadow {
            offset: [0, 16],
            blur: 40,
            spread: 0,
            color: theme.shadow,
        };
        visuals.popup_shadow = egui::epaint::Shadow {
            offset: [0, 8],
            blur: 24,
            spread: 0,
            color: theme.shadow,
        };
        visuals.selection.bg_fill = theme.pill_active;
        visuals.selection.stroke = egui::Stroke::new(1.0, theme.accent);

        // One set of visuals for every widget state: no native black-on-black
        // dropdowns or unstyled inputs in either theme.
        visuals.widgets.noninteractive.bg_fill = theme.card;
        visuals.widgets.noninteractive.weak_bg_fill = theme.card;
        visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, theme.stitch);
        visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, theme.muted);

        visuals.widgets.inactive.bg_fill = theme.input;
        visuals.widgets.inactive.weak_bg_fill = theme.input;
        visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, theme.stitch);
        visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, theme.text);

        visuals.widgets.hovered.bg_fill = theme.pill_active;
        visuals.widgets.hovered.weak_bg_fill = theme.pill_active;
        visuals.widgets.hovered.bg_stroke =
            egui::Stroke::new(1.0, theme.accent.gamma_multiply(0.55));
        visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, theme.text);

        visuals.widgets.active.bg_fill = theme.pill_active;
        visuals.widgets.active.weak_bg_fill = theme.pill_active;
        visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, theme.accent);
        visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0, theme.text);

        for widgets in [
            &mut visuals.widgets.noninteractive,
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
        ] {
            widgets.corner_radius = egui::CornerRadius::same(10);
        }

        let active_theme = if theme.is_dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        };
        ctx.set_theme(if theme.is_dark {
            egui::ThemePreference::Dark
        } else {
            egui::ThemePreference::Light
        });
        let mut style = (*ctx.style_of(active_theme)).clone();
        style.visuals = visuals;
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(14.0, 6.0);
        // Uniform 32px control height in both themes keeps pills, buttons,
        // ComboBoxes and TextEdits on the same vertical centre line.
        style.spacing.interact_size = egui::vec2(0.0, 32.0);
        style.spacing.text_edit_width = 280.0;
        style.spacing.combo_width = 148.0;
        style.spacing.menu_margin = egui::Margin::same(6);
        style.spacing.scroll.bar_width = 8.0;
        style.spacing.scroll.floating = true;
        style.spacing.scroll.content_margin = egui::Margin::ZERO;

        // Strict typography scale.
        style.text_styles.insert(
            egui::TextStyle::Heading,
            egui::FontId::new(20.0, egui::FontFamily::Name("Lora".into())),
        );
        style.text_styles.insert(
            egui::TextStyle::Body,
            egui::FontId::new(13.0, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Button,
            egui::FontId::new(13.0, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Small,
            egui::FontId::new(12.0, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Monospace,
            egui::FontId::new(12.0, egui::FontFamily::Monospace),
        );
        ctx.set_style_of(active_theme, style);
    }

    /// Feedback that belongs only inside the Log / Edit modal.
    fn modal_message(&mut self, text: impl Into<String>) {
        if let Some(edit) = &mut self.log_edit {
            edit.message = text.into();
        }
    }

    fn ensure_cover_textures(&mut self, ctx: &egui::Context) {
        // Only clone paths that are not already cached — normally zero allocations.
        let paths: Vec<String> = self
            .books
            .iter()
            .filter_map(|book| book.cover_path.as_deref())
            .filter(|relative| !self.cover_textures.contains_key(*relative))
            .take(400)
            .map(str::to_string)
            .collect();
        for relative in paths {
            self.load_cover_texture(ctx, &relative);
        }
        // The continue-reading widget may show a book outside the current filter.
        if let Some(relative) = self
            .continue_book
            .as_ref()
            .and_then(|book| book.cover_path.as_deref())
            .filter(|relative| !self.cover_textures.contains_key(*relative))
            .map(str::to_string)
        {
            self.load_cover_texture(ctx, &relative);
        }
    }

    /// Apply a cover that finished downloading on a background thread.
    fn poll_cover_download(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.cover_receiver else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(job)) => {
                ctx.request_repaint();
                self.cover_receiver = None;
                let belongs_to_modal =
                    self.log_edit.as_ref().and_then(|edit| edit.cover_job_id) == Some(job.job_id);
                if !belongs_to_modal {
                    return;
                }
                match crate::covers::save_cover_bytes(&self.db, job.temp_id, &job.tag, &job.bytes) {
                    Ok(relative) => {
                        if let Some(edit) = &mut self.log_edit {
                            edit.cover = Some(relative.clone());
                            edit.message.clear();
                        }
                        self.load_cover_texture(ctx, &relative);
                    }
                    Err(error) => self.modal_message(format!("Cover could not be cached: {error}")),
                }
            }
            Ok(Err(error)) => {
                ctx.request_repaint();
                self.cover_receiver = None;
                self.modal_message(format!("Cover download failed: {error}"));
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => self.cover_receiver = None,
        }
    }

    fn reload_watched_folders(&mut self) -> Result<(), Box<dyn Error>> {
        self.watched_folders = self.db.list_watched_folders()?;
        Ok(())
    }

    fn add_folder(&mut self, ctx: &egui::Context, path: PathBuf) {
        let folder = path.to_string_lossy().into_owned();
        if self.db.add_watched_folder(&folder).is_ok() {
            let _ = self.reload_watched_folders();
            self.start_scan(ctx, vec![folder]);
        }
    }

    /// Launch the folder scanner off the UI thread.
    fn start_scan(&mut self, ctx: &egui::Context, folders: Vec<String>) {
        if folders.is_empty() {
            return;
        }
        let folders_total = folders.len();
        let repaint_ctx = ctx.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = crate::scan::scan_folders(folders, tx);
            repaint_ctx.request_repaint();
        });
        self.scan_receiver = Some(rx);
        self.scan_status = Some(ScanProgress {
            folders_total,
            ..ScanProgress::default()
        });
    }

    /// Apply scan progress snapshots; refresh the shelf when a scan finishes.
    fn poll_scan(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.scan_receiver else {
            return;
        };
        match rx.try_recv() {
            Ok(progress) => {
                ctx.request_repaint();
                let finished = progress.done;
                let previews = progress.previews.clone();
                self.scan_status = Some(progress);
                if finished {
                    self.scan_receiver = None;
                    let _ = self.reload_watched_folders();
                    let _ = self.refresh();
                    if !previews.is_empty() {
                        self.scan_preview = previews;
                        self.scan_preview_open = true;
                        self.scan_preview_opened_frame = Some(self.frame_no);
                    }
                }
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => self.scan_receiver = None,
        }
    }

    /// Commit reviewed files on a background thread (covers + SQLite).
    fn start_import(&mut self, ctx: &egui::Context, books: Vec<ScannedBook>) {
        if books.is_empty() {
            return;
        }
        let books_total = books.len();
        let repaint_ctx = ctx.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = crate::scan::import_scanned_books(books, tx);
            repaint_ctx.request_repaint();
        });
        self.import_receiver = Some(rx);
        self.import_status = Some(ScanProgress {
            importing: true,
            folders_total: books_total,
            ..ScanProgress::default()
        });
    }

    fn poll_import(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.import_receiver else {
            return;
        };
        match rx.try_recv() {
            Ok(progress) => {
                ctx.request_repaint();
                let finished = progress.done;
                self.import_status = Some(progress);
                if finished {
                    self.import_receiver = None;
                    let _ = self.refresh();
                }
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => self.import_receiver = None,
        }
    }

    fn start_wiki_refresh_if_due(&mut self, ctx: &egui::Context) {
        if self.wiki_refresh_started {
            return;
        }
        self.wiki_refresh_started = true;
        let path = wiki_cache_path(&self.db.data_dir);
        let due = !path.exists()
            || std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .map(|time| {
                    std::time::SystemTime::now()
                        .duration_since(time)
                        .unwrap_or_default()
                        .as_secs()
                        > 30 * 24 * 60 * 60
                })
                .unwrap_or(true);
        if !due {
            return;
        }
        let data_dir = self.db.data_dir.clone();
        let repaint_ctx = ctx.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = crate::openlibrary::fetch_wikipedia_awards();
            if let Ok(value) = &result {
                let _ = save_wiki_cache(&data_dir, value);
            }
            let _ = tx.send(result);
            repaint_ctx.request_repaint();
        });
        self.wiki_refresh_receiver = Some(rx);
    }

    fn poll_wiki_refresh(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.wiki_refresh_receiver else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(value)) => {
                ctx.request_repaint();
                self.wiki_cache = value;
                self.wiki_refresh_receiver = None;
            }
            Ok(Err(_)) => {
                ctx.request_repaint();
                self.wiki_refresh_receiver = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => self.wiki_refresh_receiver = None,
        }
    }

    fn start_global_search(&mut self, ctx: &egui::Context) {
        let query = self.global_query.trim().to_string();
        if query.is_empty() {
            self.global_results.clear();
            self.global_error.clear();
            return;
        }
        let repaint_ctx = ctx.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(crate::openlibrary::search_books(&query));
            repaint_ctx.request_repaint();
        });
        self.global_receiver = Some(rx);
        self.global_results.clear();
        self.global_error.clear();
        self.global_selected = None;
    }

    fn start_chart(&mut self, ctx: &egui::Context) {
        let genre = self.discover_genre.clone();
        let language = self.discover_language.clone();
        let limit = self.discover_limit;
        let repaint_ctx = ctx.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(crate::openlibrary::fetch_chart(&genre, &language, limit));
            repaint_ctx.request_repaint();
        });
        self.chart_receiver = Some(rx);
        self.chart_running = true;
        self.chart_error.clear();
        self.random_pick = None;
    }

    fn poll_chart(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.chart_receiver else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(results)) => {
                ctx.request_repaint();
                self.chart_results = results;
                self.chart_running = false;
                self.chart_receiver = None;
            }
            Ok(Err(error)) => {
                ctx.request_repaint();
                self.chart_error = error;
                self.chart_running = false;
                self.chart_receiver = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.chart_running = false;
                self.chart_receiver = None;
            }
        }
    }

    fn poll_global_search(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.global_receiver else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(results)) => {
                ctx.request_repaint();
                self.global_results = results;
                self.global_receiver = None;
            }
            Ok(Err(error)) => {
                ctx.request_repaint();
                self.global_error = error;
                self.global_receiver = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => self.global_receiver = None,
        }
    }

    fn start_summary(&mut self, ctx: &egui::Context, title: &str, author: &str) {
        let key = format!("{title}|{author}");
        if self.summary_key == key
            && (self.summary_text.is_some() || self.summary_receiver.is_some())
        {
            return;
        }
        let title = title.trim().to_string();
        let author = author.trim().to_string();
        if title.is_empty() {
            self.summary_text = None;
            self.summary_error.clear();
            return;
        }
        let repaint_ctx = ctx.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(crate::openlibrary::fetch_book_summary(&title, &author));
            repaint_ctx.request_repaint();
        });
        self.summary_receiver = Some(rx);
        self.summary_key = key;
        self.summary_text = None;
        self.summary_error.clear();
    }

    fn start_download_links(&mut self, ctx: &egui::Context, title: &str, author: &str) {
        let key = format!("{title}|{author}");
        if self.download_links_key == key
            && (!self.download_links.is_empty() || self.download_links_receiver.is_some())
        {
            return;
        }
        let title = title.trim().to_string();
        let author = author.trim().to_string();
        if title.is_empty() {
            return;
        }
        let repaint_ctx = ctx.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(crate::openlibrary::fetch_download_links(&title, &author));
            repaint_ctx.request_repaint();
        });
        self.download_links_receiver = Some(rx);
        self.download_links_key = key;
        self.download_links.clear();
        self.download_links_error.clear();
    }

    fn poll_download_links(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.download_links_receiver else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(links)) => {
                ctx.request_repaint();
                self.download_links = links;
                self.download_links_error.clear();
                self.download_links_receiver = None;
            }
            Ok(Err(error)) => {
                ctx.request_repaint();
                self.download_links_error = error;
                self.download_links_receiver = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => self.download_links_receiver = None,
        }
    }

    fn poll_summary(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.summary_receiver else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(text)) => {
                ctx.request_repaint();
                self.summary_text = Some(text);
                self.summary_error.clear();
                self.summary_receiver = None;
            }
            Ok(Err(error)) => {
                ctx.request_repaint();
                self.summary_error = error;
                self.summary_receiver = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => self.summary_receiver = None,
        }
    }

    /// Fill the whole form from one search result and download its cover.
    /// Shared by the in-modal search and the global header search.
    fn apply_search_hit(&mut self, ctx: &egui::Context, hit: &SearchHit) {
        let temp_id = self.next_temp_id();
        self.cover_job_seq += 1;
        let job_id = self.cover_job_seq;
        let (english_title, malayalam_title) = split_malayalam_title(&hit.title);

        if self.log_edit.is_none() {
            self.open_new_log();
        }
        if let Some(edit) = &mut self.log_edit {
            edit.book.title = english_title;
            if let Some(malayalam) = malayalam_title {
                edit.malayalam_title_text = malayalam;
            }
            edit.book.author = hit.author().to_string();
            edit.book.isbn = hit.isbn().to_string();
            edit.book.total_pages = hit.page_count.unwrap_or(0).max(0);
            edit.book.current_page = 0;
            edit.book.date_finished = None;
            edit.book.status = ReadStatus::WantToRead;
            edit.book.rating = normalize_rating(hit.average_rating.unwrap_or(0.0));
            edit.rating = hit.average_rating.unwrap_or(0.0);
            edit.publisher_text = hit.publisher.clone().unwrap_or_default();
            if let Some(description) = &hit.description {
                edit.book.review = description.clone();
            }
            edit.cover = None;
            edit.cover_job_id = Some(job_id);
            edit.search_open = false;
            edit.message.clear();
        }

        let cover_url = hit.cover_url.clone();
        let cover_id = hit.cover_id;
        if cover_url.is_some() || cover_id.is_some() {
            let repaint_ctx = ctx.clone();
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let result: Result<(String, Vec<u8>), String> = if let Some(url) = cover_url {
                    crate::openlibrary::fetch_cover_url(&url)
                        .map(|bytes| ("url".to_string(), bytes))
                } else if let Some(id) = cover_id {
                    crate::openlibrary::fetch_cover(id).map(|bytes| (format!("ol{id}"), bytes))
                } else {
                    Err("no cover URL in this result".into())
                };
                let _ = tx.send(result.map(|(tag, bytes)| PendingCover {
                    job_id,
                    temp_id,
                    tag,
                    bytes,
                }));
                repaint_ctx.request_repaint();
            });
            self.cover_receiver = Some(rx);
            self.modal_message("Downloading cover…");
        } else if let Ok(relative) = covers::save_placeholder(&self.db, temp_id) {
            if let Some(edit) = &mut self.log_edit {
                edit.cover = Some(relative.clone());
                edit.cover_job_id = None;
                edit.message = format!("Details filled from {}.", hit.source_label());
            }
            self.cover_receiver = None;
            self.load_cover_texture(ctx, &relative);
        }
        self.start_summary(ctx, &hit.title, hit.author());
        self.start_download_links(ctx, &hit.title, hit.author());
    }

    fn set_status(&mut self, id: i64, status: ReadStatus) {
        if let Ok(Some(mut book)) = self.db.get_book(id) {
            book.status = status;
            if self.db.update_book(&book).is_ok() {
                let _ = self.refresh();
            }
        }
    }

    fn set_bookmark(&mut self, id: i64, page: i64) {
        if let Ok(Some(mut book)) = self.db.get_book(id) {
            book.current_page = page.max(0);
            if self.db.update_book(&book).is_ok() {
                self.bookmark_edit = false;
                let _ = self.refresh();
            }
        }
    }

    fn discover_view(&mut self, ui: &mut egui::Ui, theme: Theme, actions: &mut Vec<UiAction>) {
        const GENRES: [&str; 8] = [
            "Fiction",
            "Mystery",
            "Poetry",
            "Science Fiction",
            "History",
            "Biography",
            "Children",
            "Malayalam literature",
        ];
        const LANGUAGES: [&str; 3] = ["Both", "English", "Malayalam"];

        ui.label(
            egui::RichText::new("Discover")
                .font(theme.serif(26.0))
                .strong()
                .color(theme.text),
        );
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(
                "Book charts, a random pick, and award winners — from the internet.",
            )
            .size(13.0)
            .color(theme.muted),
        );
        ui.add_space(18.0);

        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("chart_genre")
                .width(180.0)
                .selected_text(self.discover_genre.clone())
                .show_ui(ui, |ui| {
                    for genre in GENRES {
                        ui.selectable_value(&mut self.discover_genre, genre.to_string(), genre);
                    }
                });
            egui::ComboBox::from_id_salt("chart_language")
                .width(120.0)
                .selected_text(self.discover_language.clone())
                .show_ui(ui, |ui| {
                    for language in LANGUAGES {
                        ui.selectable_value(
                            &mut self.discover_language,
                            language.to_string(),
                            language,
                        );
                    }
                });
            egui::ComboBox::from_id_salt("chart_limit")
                .width(70.0)
                .selected_text(format!("Top {}", self.discover_limit))
                .show_ui(ui, |ui| {
                    for limit in [10, 20, 25] {
                        ui.selectable_value(
                            &mut self.discover_limit,
                            limit,
                            format!("Top {limit}"),
                        );
                    }
                });
            if ui.button("Load chart").clicked() {
                actions.push(UiAction::LoadChart);
            }
            if ui.button("Surprise me").clicked() && !self.chart_results.is_empty() {
                let nanos = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos() as usize;
                let index = nanos % self.chart_results.len();
                self.random_pick = Some(self.chart_results[index].clone());
            }
        });
        ui.add_space(10.0);

        if self.chart_running {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    egui::RichText::new("Loading chart from Open Library…")
                        .small()
                        .color(theme.muted),
                );
            });
        }
        if !self.chart_error.is_empty() {
            ui.label(
                egui::RichText::new(&self.chart_error)
                    .small()
                    .color(theme.danger),
            );
        }
        if self.chart_results.is_empty() && !self.chart_running {
            ui.label(
                egui::RichText::new("Pick a genre and language, then load a chart.")
                    .small()
                    .color(theme.muted),
            );
        }
        egui::ScrollArea::vertical()
            .id_salt("discover_chart_scroll")
            .max_height(320.0)
            .show(ui, |ui| {
                for (index, hit) in self.chart_results.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(format!("{:>2}", index + 1))
                                .strong()
                                .color(theme.accent),
                        );
                        if ui
                            .add(
                                egui::Button::new(hit.title.clone())
                                    .truncate()
                                    .fill(egui::Color32::TRANSPARENT)
                                    .min_size(egui::vec2(360.0, 28.0)),
                            )
                            .clicked()
                        {
                            self.random_pick = Some(hit.clone());
                        }
                        ui.label(
                            egui::RichText::new(hit.source_label())
                                .small()
                                .color(theme.muted),
                        );
                    });
                    ui.label(egui::RichText::new(hit.author()).small().color(theme.muted));
                }
            });

        if let Some(hit) = &self.random_pick {
            ui.add_space(12.0);
            egui::Frame::new()
                .fill(theme.card)
                .stroke(egui::Stroke::new(1.0, theme.accent.gamma_multiply(0.55)))
                .corner_radius(12.0)
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new("Your random pick")
                            .strong()
                            .color(theme.accent),
                    );
                    ui.label(
                        egui::RichText::new(&hit.title)
                            .font(theme.serif(18.0))
                            .color(theme.text),
                    );
                    ui.label(
                        egui::RichText::new(format!("{} · {}", hit.author(), hit.source_label()))
                            .small()
                            .color(theme.muted),
                    );
                    if ui.button("＋ Add to library").clicked() {
                        actions.push(UiAction::AddGlobalResult(hit.clone()));
                    }
                });
        }

        ui.add_space(24.0);
        ui.label(
            egui::RichText::new("Award-winning books")
                .font(theme.serif(20.0))
                .strong()
                .color(theme.text),
        );
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(
                "Curated winners plus Wikipedia summaries, refreshed automatically once a month.",
            )
            .small()
            .color(theme.muted),
        );
        ui.add_space(10.0);
        egui::ScrollArea::vertical()
            .id_salt("discover_awards_scroll")
            .max_height(340.0)
            .show(ui, |ui| {
                for award in crate::awards::curated_awards() {
                    egui::Frame::new()
                        .fill(theme.card)
                        .stroke(egui::Stroke::new(1.0, theme.stitch))
                        .corner_radius(10.0)
                        .inner_margin(10.0)
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new(award.name).strong().color(theme.text));
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} · {}",
                                    award.language, award.region
                                ))
                                .small()
                                .color(theme.muted),
                            );
                            for (year, title, author) in award.winners {
                                ui.label(
                                    egui::RichText::new(format!("{year} — {title} — {author}"))
                                        .small()
                                        .color(theme.text),
                                );
                            }
                            if let Some(summary) = self
                                .wiki_cache
                                .get(award.name)
                                .and_then(|value| value.as_str())
                            {
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(truncate(summary, 240))
                                            .small()
                                            .color(theme.muted),
                                    )
                                    .wrap(),
                                );
                            }
                        });
                    ui.add_space(8.0);
                }
            });
    }

    fn profile_view(&mut self, ui: &mut egui::Ui, theme: Theme, actions: &mut Vec<UiAction>) {
        let all_books = self
            .db
            .list_books("", None, false, SortKey::RecentlyAdded)
            .unwrap_or_default();
        let archetype = reader::archetype(&all_books, &self.profile);
        let badges = reader::badges(&all_books);

        ui.label(
            egui::RichText::new("Reader profile")
                .font(theme.serif(26.0))
                .strong()
                .color(theme.text),
        );
        ui.add_space(18.0);

        egui::Frame::new()
            .fill(theme.card)
            .stroke(egui::Stroke::new(1.0, theme.stitch))
            .corner_radius(14.0)
            .inner_margin(16.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("{} {}", archetype.emoji, archetype.name))
                            .font(theme.serif(22.0))
                            .strong()
                            .color(theme.accent),
                    );
                });
                ui.label(
                    egui::RichText::new(archetype.description)
                        .size(13.0)
                        .color(theme.text),
                );
                ui.label(
                    egui::RichText::new(archetype.evidence)
                        .small()
                        .color(theme.muted),
                );
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    for badge in badges {
                        ui.add(
                            egui::Button::new(format!("{} {}", badge.emoji, badge.label))
                                .fill(theme.pill_active),
                        );
                    }
                });
            });
        ui.add_space(18.0);

        ui.label(
            egui::RichText::new("Your details")
                .font(theme.serif(20.0))
                .strong()
                .color(theme.text),
        );
        ui.add_space(10.0);
        ui.columns(2, |columns| {
            let left = &mut columns[0];
            left.label(
                egui::RichText::new("Display name")
                    .color(theme.muted)
                    .small(),
            );
            left.add(
                egui::TextEdit::singleline(&mut self.profile.name)
                    .desired_width(f32::INFINITY)
                    .margin(egui::Margin::symmetric(10, 8)),
            );
            left.label(
                egui::RichText::new("Avatar initial")
                    .color(theme.muted)
                    .small(),
            );
            left.add(
                egui::TextEdit::singleline(&mut self.profile.avatar_initial)
                    .desired_width(60.0)
                    .margin(egui::Margin::symmetric(10, 8)),
            );
            left.label(
                egui::RichText::new("Avatar colour (hex)")
                    .color(theme.muted)
                    .small(),
            );
            left.add(
                egui::TextEdit::singleline(&mut self.profile.avatar_color)
                    .desired_width(120.0)
                    .margin(egui::Margin::symmetric(10, 8)),
            );

            let right = &mut columns[1];
            right.label(
                egui::RichText::new("Yearly reading goal")
                    .color(theme.muted)
                    .small(),
            );
            right.add(egui::DragValue::new(&mut self.profile.yearly_goal).range(0..=1000));
            right.label(
                egui::RichText::new("Preferred language")
                    .color(theme.muted)
                    .small(),
            );
            egui::ComboBox::from_id_salt("profile_language")
                .width(160.0)
                .selected_text(self.profile.preferred_language.clone())
                .show_ui(right, |ui| {
                    for language in ["Both", "English", "Malayalam"] {
                        ui.selectable_value(
                            &mut self.profile.preferred_language,
                            language.to_string(),
                            language,
                        );
                    }
                });
            right.label(
                egui::RichText::new("Favourite publisher")
                    .color(theme.muted)
                    .small(),
            );
            right.add(
                egui::TextEdit::singleline(&mut self.profile.favorite_publisher)
                    .desired_width(f32::INFINITY)
                    .margin(egui::Margin::symmetric(10, 8))
                    .hint_text("DC Books, Mathrubhumi Books…"),
            );
        });
        ui.label(egui::RichText::new("About you").color(theme.muted).small());
        ui.add(
            egui::TextEdit::multiline(&mut self.profile.bio)
                .desired_width(f32::INFINITY)
                .desired_rows(3)
                .hint_text("What kind of reader are you?"),
        );
        if ui.button("Save profile").clicked() {
            actions.push(UiAction::SaveProfile);
        }

        ui.add_space(20.0);
        ui.label(
            egui::RichText::new("Recommended for you")
                .font(theme.serif(20.0))
                .strong()
                .color(theme.text),
        );
        ui.add_space(8.0);
        let mut unread: Vec<Book> = all_books
            .into_iter()
            .filter(|book| book.status != ReadStatus::Read)
            .collect();
        unread.sort_by(|a, b| {
            b.rating
                .unwrap_or(0.0)
                .partial_cmp(&a.rating.unwrap_or(0.0))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for book in unread.into_iter().take(5) {
            ui.horizontal(|ui| {
                if ui.button(book.title.clone()).clicked() {
                    actions.push(UiAction::Open(book.id));
                }
                ui.label(
                    egui::RichText::new(format!("— {}", book.author))
                        .small()
                        .color(theme.muted),
                );
            });
        }
        if ui.button("Find more like this").clicked() {
            self.global_query = reader::recommendation_seed(&self.books, &self.profile);
            actions.push(UiAction::GlobalSearch);
        }
    }

    fn remove_folder(&mut self, id: i64, delete_books: bool) {
        if delete_books {
            let prefix = self
                .watched_folders
                .iter()
                .find(|folder| folder.id == id)
                .map(|folder| folder.path.clone())
                .unwrap_or_default();
            if !prefix.is_empty() {
                let targets: Vec<(i64, Option<String>)> = self
                    .db
                    .list_books("", None, false, SortKey::RecentlyAdded)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|book| {
                        book.file_path
                            .as_deref()
                            .map(|path| path.starts_with(&prefix))
                            .unwrap_or(false)
                    })
                    .map(|book| (book.id, book.cover_path))
                    .collect();
                for (book_id, cover) in targets {
                    if let Some(relative) = cover {
                        covers::remove_cover(&self.db, &relative);
                        self.cover_textures.remove(&relative);
                    }
                    let _ = self.db.delete_book(book_id);
                }
            }
        }
        if self.db.remove_watched_folder(id).is_ok() {
            let _ = self.reload_watched_folders();
            let _ = self.refresh();
        }
        self.folder_remove_open = false;
        self.folder_remove_id = None;
    }

    fn continue_reading_book(&self) -> Option<Book> {
        self.continue_book.clone()
    }
}

// ---------------------------------------------------------------------------
// eframe App
// ---------------------------------------------------------------------------

impl eframe::App for BookApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frame_no = self.frame_no.wrapping_add(1);
        let ctx = ui.ctx().clone();
        let theme = self.theme();
        if self.theme_applied != Some(self.dark_mode) {
            self.apply_theme(&ctx, theme);
            self.theme_applied = Some(self.dark_mode);
        }
        self.ensure_cover_textures(&ctx);
        self.poll_cover_download(&ctx);
        self.poll_scan(&ctx);
        self.poll_import(&ctx);
        self.poll_global_search(&ctx);
        self.poll_summary(&ctx);
        self.poll_download_links(&ctx);
        self.poll_chart(&ctx);
        self.poll_wiki_refresh(&ctx);
        self.start_wiki_refresh_if_due(&ctx);
        if let Some(edit) = &mut self.log_edit {
            edit.poll_search(&ctx);
        }

        let mut actions: Vec<UiAction> = Vec::new();
        let mut needs_refresh = false;

        egui::Panel::left("sidebar")
            .resizable(false)
            .default_size(240.0)
            .min_size(240.0)
            .max_size(240.0)
            .show(ui, |ui| {
                self.sidebar(ui, theme, &mut actions);
            });

        egui::CentralPanel::default().show(ui, |ui| {
            needs_refresh = self.main_area(ui, theme, &mut actions);
        });

        for action in actions {
            match action {
                UiAction::Open(id) => {
                    self.open_log_edit(id);
                    let target = self
                        .log_edit
                        .as_ref()
                        .map(|edit| (edit.book.title.clone(), edit.book.author.clone()));
                    if let Some((title, author)) = target {
                        self.start_summary(&ctx, &title, &author);
                        self.start_download_links(&ctx, &title, &author);
                    }
                }
                UiAction::OpenFile(path) => {
                    let _ = open::that(&path);
                }
                UiAction::ToggleFavorite(id) => self.toggle_favorite(id),
                UiAction::SetStatus(id, status) => self.set_status(id, status),
                UiAction::SetBookmark(id, page) => self.set_bookmark(id, page),
                UiAction::Delete(id) => {
                    self.confirm_id = Some(id);
                    self.confirm_open = true;
                    self.confirm_message.clear();
                    self.delete_opened_frame = Some(self.frame_no);
                }
                UiAction::Nav(nav) => {
                    self.nav = nav;
                    self.expanded_shelf = None;
                    // Diary / Favorites / Lists are filtered collections, so
                    // they always open as a grid rather than the shelf overview.
                    if matches!(
                        nav,
                        NavItem::Diary
                            | NavItem::Favorites
                            | NavItem::Lists
                            | NavItem::Discover
                            | NavItem::Profile
                    ) {
                        self.view = MainView::AllBooks;
                    }
                    needs_refresh = true;
                }
                UiAction::Shelf(status) => {
                    self.expanded_shelf = status;
                    self.nav = NavItem::Library;
                    needs_refresh = true;
                }
                UiAction::View(view) => {
                    self.view = view;
                    if view == MainView::Shelves {
                        self.nav = NavItem::Library;
                        self.expanded_shelf = None;
                    }
                    needs_refresh = true;
                }
                UiAction::ToggleTheme => {
                    self.dark_mode = !self.dark_mode;
                    save_dark_pref(&self.db.data_dir, self.dark_mode);
                }
                UiAction::LogNew => self.open_new_log(),
                UiAction::ScanFolder => {
                    if let Some(path) = rfd::FileDialog::new().pick_folder() {
                        self.add_folder(&ctx, path);
                    }
                }
                UiAction::GlobalSearch => self.start_global_search(&ctx),
                UiAction::LoadChart => self.start_chart(&ctx),
                UiAction::SaveProfile => {
                    let profile = self.profile.clone();
                    if let Err(error) = self.db.save_profile(&profile) {
                        self.scan_status = Some(ScanProgress {
                            done: true,
                            message: format!("Could not save profile: {error}"),
                            ..ScanProgress::default()
                        });
                    }
                }
                UiAction::AddGlobalResult(hit) => {
                    self.apply_search_hit(&ctx, &hit);
                    self.global_results.clear();
                    self.global_error.clear();
                }
                UiAction::RemoveFolder(id) => {
                    self.folder_remove_open = true;
                    self.folder_remove_id = Some(id);
                    self.folder_remove_path = self
                        .watched_folders
                        .iter()
                        .find(|folder| folder.id == id)
                        .map(|folder| folder.path.clone())
                        .unwrap_or_default();
                    self.folder_remove_opened_frame = Some(self.frame_no);
                }
                UiAction::RescanAll => {
                    let folders: Vec<String> = self
                        .watched_folders
                        .iter()
                        .map(|folder| folder.path.clone())
                        .collect();
                    self.start_scan(&ctx, folders);
                }
            }
        }

        self.log_edit_modal(&ctx, theme);
        self.delete_modal(&ctx, theme);
        self.scan_preview_modal(&ctx, theme);
        self.folder_remove_modal(&ctx, theme);

        if needs_refresh {
            let _ = self.refresh();
        }
    }
}

impl BookApp {
    // ---- Sidebar ----------------------------------------------------------
    fn sidebar(&mut self, ui: &mut egui::Ui, theme: Theme, actions: &mut Vec<UiAction>) {
        ui.painter()
            .rect_filled(ui.max_rect(), 0.0f32, theme.sidebar);
        // Dashed stitching separator on the right edge of the sidebar.
        // Painted first so content sits cleanly on top.
        let panel_rect = ui.max_rect();
        let x = panel_rect.right() - 0.5;
        let mut y = panel_rect.top() + 8.0;
        while y + 5.0 < panel_rect.bottom() {
            ui.painter().line_segment(
                [egui::pos2(x, y), egui::pos2(x, y + 5.0)],
                egui::Stroke::new(1.0, theme.stitch),
            );
            y += 10.0;
        }

        ui.add_space(24.0);
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                // A small ornament above the wordmark.
                ui.label(
                    egui::RichText::new("◆")
                        .size(9.0)
                        .color(theme.accent.gamma_multiply(0.85)),
                );
                // Wordmark: larger Lora with generous presence.
                ui.label(
                    egui::RichText::new("Thalir")
                        .font(theme.serif(29.0))
                        .strong()
                        .color(theme.text),
                );
                // Malayalam name in the accented voice.
                ui.label(egui::RichText::new("തളിർ").size(15.0).color(theme.accent));
                ui.add_space(3.0);
                // Italic serif tagline.
                ui.add(
                    egui::Label::new(
                        egui::RichText::new("a quiet place for your books")
                            .font(theme.serif(12.5))
                            .italics()
                            .color(theme.muted),
                    )
                    .wrap(),
                );
                // Short accent rule — a tiny bookmark ribbon.
                let rule_start = ui.next_widget_position();
                ui.painter().line_segment(
                    [
                        egui::pos2(rule_start.x + 1.0, rule_start.y + 6.0),
                        egui::pos2(rule_start.x + 29.0, rule_start.y + 6.0),
                    ],
                    egui::Stroke::new(2.0, theme.accent.gamma_multiply(0.75)),
                );
                ui.add_space(10.0);
            });
        });
        ui.add_space(24.0);

        let nav_items: [(&str, NavItem, NavIcon); 4] = [
            ("My Library", NavItem::Library, NavIcon::Library),
            ("Reading Diary", NavItem::Diary, NavIcon::Diary),
            ("Favorites", NavItem::Favorites, NavIcon::Heart),
            ("Publishers", NavItem::Lists, NavIcon::List),
        ];
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            for (label, item, icon) in nav_items {
                let active = self.nav == item && self.expanded_shelf.is_none();
                if nav_pill(ui, label, icon, active, theme) {
                    actions.push(UiAction::Nav(item));
                }
            }
            ui.add_space(6.0);
            if nav_pill(ui, "Log Book", NavIcon::Plus, false, theme) {
                actions.push(UiAction::LogNew);
            }
        });

        // Pin the continue-reading card and profile bar to the bottom.
        let used = ui.min_rect().bottom() - ui.max_rect().top();
        let bottom_widgets = 168.0 + 12.0 + 48.0;
        let spacer = (ui.max_rect().height() - used - bottom_widgets).max(12.0);
        ui.add_space(spacer);

        // Continue reading card: 1px subtle border, 12px radius, elevated fill.
        let continue_book = self.continue_reading_book();
        let continue_cover = continue_book
            .as_ref()
            .and_then(|book| book.cover_path.as_deref())
            .and_then(|path| self.cover_textures.get(path))
            .cloned();
        let widget_height = 168.0;
        let (widget_rect, card_response) = ui.allocate_exact_size(
            egui::vec2((ui.available_width() - 32.0).max(120.0), widget_height),
            egui::Sense::click(),
        );
        ui.painter().rect_filled(widget_rect, 12.0f32, theme.card);
        ui.painter().rect_stroke(
            widget_rect,
            12.0f32,
            egui::Stroke::new(1.0, theme.stitch),
            egui::StrokeKind::Inside,
        );

        if let Some(book) = &continue_book {
            let pad = 14.0;
            let cover_w = 58.0;
            let cover_h = 84.0;
            let cover_rect = egui::Rect::from_min_size(
                widget_rect.min + egui::vec2(pad, pad),
                egui::vec2(cover_w, cover_h),
            );
            draw_book_cover(ui, continue_cover.as_ref(), cover_rect, theme);
            draw_shelf(
                ui,
                egui::Rect::from_min_size(
                    egui::pos2(cover_rect.min.x - 2.0, cover_rect.max.y),
                    egui::vec2(cover_w + 4.0, 8.0),
                ),
                theme,
            );

            let text_x = cover_rect.max.x + 12.0;
            let text_width = (widget_rect.max.x - pad - text_x).max(60.0);
            ui.painter().text(
                egui::pos2(text_x, widget_rect.min.y + pad),
                egui::Align2::LEFT_TOP,
                "CONTINUE READING",
                egui::FontId::proportional(9.0),
                theme.muted,
            );
            let title_rect = egui::Rect::from_min_size(
                egui::pos2(text_x, widget_rect.min.y + pad + 16.0),
                egui::vec2(text_width, 36.0),
            );
            draw_clamped_title(ui, title_rect, &book.title, theme, 13.0);

            let editor_y = title_rect.max.y + 2.0;
            if self.bookmark_edit {
                // Inline page stepper — no need to open the whole editor.
                let minus =
                    egui::Rect::from_min_size(egui::pos2(text_x, editor_y), egui::vec2(22.0, 20.0));
                let number = egui::Rect::from_min_size(
                    egui::pos2(text_x + 26.0, editor_y),
                    egui::vec2(50.0, 20.0),
                );
                let plus = egui::Rect::from_min_size(
                    egui::pos2(text_x + 80.0, editor_y),
                    egui::vec2(22.0, 20.0),
                );
                for (rect, glyph) in [(minus, "−"), (plus, "+")] {
                    let hit = ui.allocate_rect(rect, egui::Sense::click());
                    ui.painter().rect_filled(rect, 5.0f32, theme.pill_active);
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        glyph,
                        egui::FontId::proportional(13.0),
                        theme.text,
                    );
                    if hit.clicked() {
                        if glyph == "−" {
                            self.bookmark_page = (self.bookmark_page - 1).max(0);
                        } else {
                            self.bookmark_page += 1;
                        }
                    }
                }
                ui.painter().text(
                    number.center(),
                    egui::Align2::CENTER_CENTER,
                    self.bookmark_page.to_string(),
                    egui::FontId::proportional(11.0),
                    theme.text,
                );
                let suffix = if book.total_pages > 0 {
                    format!("of {}", book.total_pages)
                } else {
                    "of ?".to_string()
                };
                ui.painter().text(
                    egui::pos2(text_x + 108.0, editor_y),
                    egui::Align2::LEFT_TOP,
                    suffix,
                    egui::FontId::proportional(10.0),
                    theme.muted,
                );

                let button_y = widget_rect.max.y - 34.0;
                let save_rect = egui::Rect::from_min_size(
                    egui::pos2(widget_rect.min.x + pad, button_y),
                    egui::vec2((widget_rect.width() - pad * 2.0) * 0.6, 24.0),
                );
                let cancel_rect = egui::Rect::from_min_size(
                    egui::pos2(save_rect.max.x + 4.0, button_y),
                    egui::vec2(save_rect.width() * 0.66, 24.0),
                );
                if ui.allocate_rect(save_rect, egui::Sense::click()).clicked() {
                    actions.push(UiAction::SetBookmark(book.id, self.bookmark_page));
                }
                if ui
                    .allocate_rect(cancel_rect, egui::Sense::click())
                    .clicked()
                {
                    self.bookmark_edit = false;
                }
                ui.painter()
                    .rect_filled(save_rect, 12.0f32, theme.pill_active);
                ui.painter().rect_filled(cancel_rect, 12.0f32, theme.card);
                ui.painter().text(
                    save_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Save page",
                    egui::FontId::proportional(11.0),
                    theme.accent,
                );
                ui.painter().text(
                    cancel_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Cancel",
                    egui::FontId::proportional(11.0),
                    theme.muted,
                );
            } else {
                let progress = if book.total_pages > 0 {
                    format!("Page {} of {}", book.current_page, book.total_pages)
                } else {
                    format!("Page {}", book.current_page)
                };
                ui.painter().text(
                    egui::pos2(text_x, editor_y),
                    egui::Align2::LEFT_TOP,
                    progress,
                    egui::FontId::proportional(10.0),
                    theme.muted,
                );
                let button = egui::Rect::from_min_size(
                    egui::pos2(widget_rect.min.x + pad, widget_rect.max.y - 34.0),
                    egui::vec2(widget_rect.width() - pad * 2.0, 24.0),
                );
                let hit = ui.allocate_rect(button, egui::Sense::click());
                ui.painter().rect_filled(button, 12.0f32, theme.pill_active);
                ui.painter().text(
                    button.center(),
                    egui::Align2::CENTER_CENTER,
                    "Update bookmark  →",
                    egui::FontId::proportional(11.0),
                    theme.accent,
                );
                if hit.clicked() {
                    self.bookmark_page = book.current_page;
                    self.bookmark_edit = true;
                }
            }
        } else {
            ui.painter().text(
                widget_rect.center(),
                egui::Align2::CENTER_CENTER,
                "Nothing in progress\nClick to log a book",
                egui::FontId::proportional(12.0),
                theme.muted,
            );
            if card_response.clicked() {
                actions.push(UiAction::LogNew);
            }
        }

        ui.add_space(12.0);
        // Compact profile footer.
        let (profile_rect, profile_response) = ui.allocate_exact_size(
            egui::vec2((ui.available_width() - 32.0).max(120.0), 48.0),
            egui::Sense::click(),
        );
        let profile_name = self.profile.name.clone();
        let profile_initial = self.profile.avatar_initial.clone();
        let profile_color = self.profile.avatar_color.clone();
        ui.painter().rect_filled(profile_rect, 12.0f32, theme.card);
        let avatar = egui::Rect::from_min_size(
            profile_rect.min + egui::vec2(8.0, 9.0),
            egui::vec2(30.0, 30.0),
        );
        let tint = egui::Color32::from_hex(&profile_color).unwrap_or(theme.accent);
        ui.painter()
            .circle_filled(avatar.center(), 15.0, tint.gamma_multiply(0.30));
        ui.painter().text(
            avatar.center(),
            egui::Align2::CENTER_CENTER,
            profile_initial,
            egui::FontId::proportional(13.0),
            tint,
        );
        ui.painter().text(
            profile_rect.min + egui::vec2(48.0, 9.0),
            egui::Align2::LEFT_TOP,
            profile_name,
            egui::FontId::proportional(13.0),
            theme.text,
        );
        if profile_response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if profile_response.clicked() {
            actions.push(UiAction::Nav(NavItem::Profile));
        }
        ui.painter().text(
            profile_rect.min + egui::vec2(48.0, 27.0),
            egui::Align2::LEFT_TOP,
            self.profile_footer_label.clone(),
            egui::FontId::proportional(11.0),
            theme.muted,
        );
    }

    // ---- Main area --------------------------------------------------------
    fn main_area(&mut self, ui: &mut egui::Ui, theme: Theme, actions: &mut Vec<UiAction>) -> bool {
        let mut needs_refresh = false;

        // Only this pane scrolls. A real 28px/36px inner margin guarantees the
        // right edge (book count, shelf links, covers) never clips the window.
        egui::ScrollArea::vertical()
            .id_salt("main_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Frame::new()
                    .inner_margin(egui::Margin {
                        left: 36,
                        right: 36,
                        top: 28,
                        bottom: 32,
                    })
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        // Hide any stray seam at the very top of the content pane.
                        let top_strip = egui::Rect::from_min_size(
                            ui.max_rect().min,
                            egui::vec2(ui.available_width(), 10.0),
                        );
                        ui.painter().rect_filled(top_strip, 0.0f32, theme.page);

                        // ---- Header --------------------------------------------------
                        ui.horizontal(|ui| {
                            egui::Frame::new()
                                .fill(theme.card)
                                .stroke(egui::Stroke::new(1.0, theme.stitch))
                                .corner_radius(12.0)
                                .inner_margin(2.0)
                                .show(ui, |ui| {
                                    ui.spacing_mut().item_spacing.x = 4.0;
                                    for (label, view) in [
                                        ("Shelves", MainView::Shelves),
                                        ("All Books", MainView::AllBooks),
                                    ] {
                                        let active =
                                            self.view == view && self.expanded_shelf.is_none();
                                        let button = egui::Button::new(
                                            egui::RichText::new(label)
                                                .size(13.0)
                                                .color(if active {
                                                    theme.text
                                                } else {
                                                    theme.muted
                                                }),
                                        )
                                        .fill(if active {
                                            theme.pill_active
                                        } else {
                                            egui::Color32::TRANSPARENT
                                        })
                                        .corner_radius(9.0)
                                        .stroke(egui::Stroke::new(
                                            1.0,
                                            egui::Color32::TRANSPARENT,
                                        ))
                                        .min_size(egui::vec2(90.0, 32.0));
                                        if ui.add(button).clicked() {
                                            actions.push(UiAction::View(view));
                                        }
                                    }
                                });

                            ui.add_space(12.0);
                            if self.expanded_shelf.is_some() && ui.button("←  All shelves").clicked()
                            {
                                actions.push(UiAction::Shelf(None));
                            }

                            let gap = 12.0;
                            let theme_width = 164.0;
                            let search_width = (ui.available_width() - theme_width - gap * 2.0)
                                .clamp(180.0, 480.0);
                            // The compact Go button lives inside the pill, so
                            // reserve its width before sizing the text input.
                            let text_width = (search_width - 74.0).max(120.0);

                            // Rounded search field.
                            egui::Frame::new()
                                .fill(theme.card)
                                .stroke(egui::Stroke::new(1.0, theme.stitch))
                                .corner_radius(12.0)
                                .inner_margin(egui::Margin::symmetric(12, 2))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        let response = ui.add_sized(
                                            [text_width, 32.0],
                                            egui::TextEdit::singleline(&mut self.global_query)
                                                .margin(egui::Margin::symmetric(0, 5))
                                                .hint_text("Search books, authors, ISBN…")
                                                .frame(egui::Frame::NONE),
                                        );
                                        if response.lost_focus()
                                            && ui.input(|state| state.key_pressed(egui::Key::Enter))
                                        {
                                            actions.push(UiAction::GlobalSearch);
                                        }
                                        response.on_hover_text(
                                            "Global search: Google Books + Open Library + Goodreads",
                                        );
                                        if ui
                                            .add(
                                                egui::Button::new("Go")
                                                    .min_size(egui::vec2(36.0, 32.0)),
                                            )
                                            .clicked()
                                        {
                                            actions.push(UiAction::GlobalSearch);
                                        }
                                    });
                                });

                            ui.add_space(gap);
                            // Balanced Dark / Light pill toggle.
                            egui::Frame::new()
                                .fill(theme.card)
                                .stroke(egui::Stroke::new(1.0, theme.stitch))
                                .corner_radius(12.0)
                                .inner_margin(2.0)
                                .show(ui, |ui| {
                                    ui.spacing_mut().item_spacing.x = 4.0;
                                    for (label, is_dark, icon) in
                                        [("Dark", true, "🌙"), ("Light", false, "☀️")]
                                    {
                                        let active = self.dark_mode == is_dark;
                                        let button = egui::Button::new(
                                            egui::RichText::new(format!("{icon} {label}"))
                                                .size(13.0)
                                                .color(if active { theme.text } else { theme.muted }),
                                        )
                                        .fill(if active {
                                            theme.pill_active
                                        } else {
                                            egui::Color32::TRANSPARENT
                                        })
                                        .corner_radius(9.0)
                                        .stroke(egui::Stroke::new(
                                            1.0,
                                            egui::Color32::TRANSPARENT,
                                        ))
                                        .min_size(egui::vec2(78.0, 32.0));
                                        if ui.add(button).clicked() && !active {
                                            actions.push(UiAction::ToggleTheme);
                                        }
                                    }
                                });
                        });

                        // ---- Global search results -----------------------------------
                        if self.global_receiver.is_some()
                            || !self.global_results.is_empty()
                            || !self.global_error.is_empty()
                        {
                            ui.add_space(8.0);
                            egui::Frame::new()
                                .fill(theme.card)
                                .stroke(egui::Stroke::new(1.0, theme.stitch))
                                .corner_radius(12.0)
                                .inner_margin(10.0)
                                .show(ui, |ui| {
                                    if self.global_receiver.is_some() {
                                        ui.horizontal(|ui| {
                                            ui.spinner();
                                            ui.label(
                                                egui::RichText::new(
                                                    "Searching Google Books + Open Library + Goodreads…",
                                                )
                                                .small()
                                                .color(theme.muted),
                                            );
                                        });
                                    }
                                    if !self.global_error.is_empty() {
                                        ui.label(
                                            egui::RichText::new(&self.global_error)
                                                .small()
                                                .color(theme.danger),
                                        );
                                    }
                                    if self.global_receiver.is_none()
                                        && self.global_results.is_empty()
                                        && self.global_error.is_empty()
                                    {
                                        ui.label(
                                            egui::RichText::new("No books found. Try another title or author.")
                                                .small()
                                                .color(theme.muted),
                                        );
                                    }
                                    egui::ScrollArea::vertical()
                                        .max_height(220.0)
                                        .show(ui, |ui| {
                                            for index in 0..self.global_results.len() {
                                                let hit = self.global_results[index].clone();
                                                let selected = self.global_selected == Some(index);
                                                ui.horizontal(|ui| {
                                                    let label = format!(
                                                        "{}{}",
                                                        hit.title,
                                                        hit.year
                                                            .map(|year| format!(" ({year})"))
                                                            .unwrap_or_default()
                                                    );
                                                    if ui
                                                        .add(
                                                            egui::Button::new(label)
                                                                .truncate()
                                                                .fill(if selected {
                                                                    theme.pill_active
                                                                } else {
                                                                    egui::Color32::TRANSPARENT
                                                                })
                                                                .stroke(egui::Stroke::new(
                                                                    1.0,
                                                                    if selected {
                                                                        theme.accent
                                                                    } else {
                                                                        egui::Color32::TRANSPARENT
                                                                    },
                                                                ))
                                                                .min_size(egui::vec2(420.0, 28.0)),
                                                        )
                                                        .clicked()
                                                    {
                                                        self.global_selected = Some(index);
                                                    }
                                                    ui.label(
                                                        egui::RichText::new(hit.source_label())
                                                            .small()
                                                            .strong()
                                                            .color(theme.accent),
                                                    );
                                                });
                                                ui.add(
                                                    egui::Label::new(
                                                        egui::RichText::new(format!(
                                                            "{} · {}",
                                                            hit.author(),
                                                            hit.publisher.as_deref().unwrap_or("-")
                                                        ))
                                                        .small()
                                                        .color(theme.muted),
                                                    )
                                                    .truncate(),
                                                );
                                            }
                                        });
                                    if let Some(index) = self.global_selected {
                                        if let Some(hit) = self.global_results.get(index) {
                                            ui.add_space(6.0);
                                            if ui
                                                .add(
                                                    egui::Button::new("＋ Add selected book to library")
                                                        .fill(theme.accent),
                                                )
                                                .clicked()
                                            {
                                                actions.push(UiAction::AddGlobalResult(hit.clone()));
                                            }
                                        }
                                    }
                                });
                            ui.add_space(8.0);
                        }

                        ui.add_space(12.0);

                        // ---- Single compact toolbar --------------------------------
                        ui.horizontal(|ui| {
                            let mut discover = false;
                            let mut scan = false;
                            let mut rescan = false;
                            let mut folders = false;
                            ui.menu_button("Library ▾", |ui| {
                                if ui.button("Discover").clicked() {
                                    discover = true;
                                }
                                if ui.button("Scan Folder").clicked() {
                                    scan = true;
                                }
                                if ui.button("Rescan All").clicked() {
                                    rescan = true;
                                }
                                if ui
                                    .button(format!(
                                        "Library folders ({})",
                                        self.watched_folders.len()
                                    ))
                                    .clicked()
                                {
                                    folders = true;
                                }
                            });
                            if discover {
                                actions.push(UiAction::Nav(NavItem::Discover));
                            }
                            if scan {
                                actions.push(UiAction::ScanFolder);
                            }
                            if rescan {
                                actions.push(UiAction::RescanAll);
                            }
                            if folders {
                                self.show_folders = !self.show_folders;
                            }

                            if ui.button("Filter").clicked() {
                                self.show_local_filter = !self.show_local_filter;
                            }
                            if self.show_local_filter {
                                needs_refresh |= ui
                                    .add(
                                        egui::TextEdit::singleline(&mut self.library_search)
                                            .desired_width(140.0)
                                            .hint_text("Filter this shelf…"),
                                    )
                                    .changed();
                            }
                            ui.add_space(8.0);
                            ui.label(egui::RichText::new("Sort").size(13.0).color(theme.muted));
                            let mut new_sort = self.sort;
                            egui::ComboBox::from_id_salt("sort_combo")
                                .width(148.0)
                                .selected_text(self.sort.label())
                                .show_ui(ui, |ui| {
                                    for key in SortKey::ALL {
                                        ui.selectable_value(&mut new_sort, key, key.label());
                                    }
                                });
                            if new_sort != self.sort {
                                self.sort = new_sort;
                                needs_refresh = true;
                            }
                            ui.add_space(8.0);
                            needs_refresh |= ui
                                .checkbox(&mut self.favorites_only, "♥ Favorites")
                                .changed();
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        egui::RichText::new(self.book_count_label.clone())
                                            .size(13.0)
                                            .color(theme.muted),
                                    )
                                    .on_hover_text(self.db.db_path.to_string_lossy());
                                },
                            );
                        });
                        if let Some(status) = &self.scan_status {
                            ui.horizontal(|ui| {
                                if status.done {
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "Scan complete · {} found · {} skipped · {} errors",
                                            status.previews.len(),
                                            status.skipped,
                                            status.errors.len()
                                        ))
                                        .small()
                                        .color(theme.accent),
                                    );
                                } else {
                                    ui.spinner();
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "Scanning {} folder(s) · {} files · {} found",
                                            status.folders_total,
                                            status.files_seen,
                                            status.previews.len()
                                        ))
                                        .small()
                                        .color(theme.muted),
                                    );
                                }
                            });
                        }
                        if let Some(status) = &self.import_status {
                            ui.horizontal(|ui| {
                                if status.done {
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "Imported {} book(s) · {} errors",
                                            status.imported,
                                            status.errors.len()
                                        ))
                                        .small()
                                        .color(theme.accent),
                                    );
                                } else if status.importing {
                                    ui.spinner();
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "Importing… {} of {}",
                                            status.imported, status.folders_total
                                        ))
                                        .small()
                                        .color(theme.muted),
                                    );
                                }
                            });
                        }
                        if let Some(status) = &self.scan_status {
                            if !status.errors.is_empty() {
                                egui::Frame::new()
                                    .fill(theme.input)
                                    .stroke(egui::Stroke::new(1.0, theme.stitch))
                                    .corner_radius(10.0)
                                    .inner_margin(8.0)
                                    .show(ui, |ui| {
                                        ui.label(
                                            egui::RichText::new("Scan report").small().strong().color(theme.danger),
                                        );
                                        if !status.message.is_empty() {
                                            ui.add(
                                                egui::Label::new(
                                                    egui::RichText::new(&status.message)
                                                        .small()
                                                        .color(theme.muted),
                                                )
                                                .wrap(),
                                            );
                                        }
                                        egui::ScrollArea::vertical().max_height(80.0).show(ui, |ui| {
                                            for error in &status.errors {
                                                ui.label(
                                                    egui::RichText::new(error)
                                                        .small()
                                                        .color(theme.danger),
                                                );
                                            }
                                        });
                                    });
                            }
                        }
                        if self.show_folders {
                            egui::Frame::new()
                                .fill(theme.card)
                                .stroke(egui::Stroke::new(1.0, theme.stitch))
                                .corner_radius(12.0)
                                .inner_margin(10.0)
                                .show(ui, |ui| {
                                    if self.watched_folders.is_empty() {
                                        ui.label(
                                            egui::RichText::new(
                                                "No folders yet — use Scan Folder to add one.",
                                            )
                                            .small()
                                            .color(theme.muted),
                                        );
                                    }
                                    for folder in &self.watched_folders {
                                        ui.horizontal(|ui| {
                                            let name = std::path::Path::new(&folder.path)
                                                .file_name()
                                                .and_then(|value| value.to_str())
                                                .unwrap_or(&folder.path);
                                            ui.add(egui::Label::new(name.to_string()).truncate())
                                                .on_hover_text(&folder.path);
                                            if let Some(when) = &folder.last_scanned {
                                                ui.label(
                                                    egui::RichText::new(when)
                                                        .small()
                                                        .color(theme.muted),
                                                );
                                            }
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    if ui.button("Remove").clicked() {
                                                        actions.push(UiAction::RemoveFolder(
                                                            folder.id,
                                                        ));
                                                    }
                                                },
                                            );
                                        });
                                    }
                                });
                            ui.add_space(8.0);
                        }

                        // ---- Content -------------------------------------------------
                        if self.nav == NavItem::Profile {
                            self.profile_view(ui, theme, actions);
                        } else if self.nav == NavItem::Discover {
                            self.discover_view(ui, theme, actions);
                        } else if let Some(status) = self.expanded_shelf {
                            self.book_grid(ui, theme, actions, Some(status));
                        } else if self.view == MainView::Shelves
                            && !matches!(
                                self.nav,
                                NavItem::Lists | NavItem::Discover | NavItem::Profile
                            )
                        {
                            self.shelves_view(ui, theme, actions);
                        } else {
                            self.book_grid(ui, theme, actions, None);
                        }
                        ui.add_space(32.0);
                    });
            });

        needs_refresh
    }

    fn shelves_view(&mut self, ui: &mut egui::Ui, theme: Theme, actions: &mut Vec<UiAction>) {
        let shelves: [(&str, ReadStatus); 3] = [
            ("Currently reading", ReadStatus::Reading),
            ("Next up", ReadStatus::WantToRead),
            ("Finished", ReadStatus::Read),
        ];

        for (heading, status) in shelves {
            let books: Vec<Book> = self
                .books
                .iter()
                .filter(|book| book.status == status)
                .cloned()
                .collect();
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(heading)
                        .font(theme.serif(20.0))
                        .strong()
                        .color(theme.text),
                );
                ui.add_space(2.0);
                ui.label(
                    egui::RichText::new(books.len().to_string())
                        .size(12.0)
                        .color(theme.muted),
                );
                if !books.is_empty() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("Full shelf  →")
                                        .size(12.0)
                                        .color(theme.accent),
                                )
                                .fill(egui::Color32::TRANSPARENT)
                                .stroke(egui::Stroke::NONE),
                            )
                            .clicked()
                        {
                            actions.push(UiAction::Shelf(Some(status)));
                        }
                    });
                }
            });
            ui.add_space(if books.is_empty() { 10.0 } else { 18.0 });

            if books.is_empty() {
                ui.label(
                    egui::RichText::new("This shelf is waiting for its first book.")
                        .size(13.0)
                        .color(theme.muted),
                );
                ui.horizontal(|ui| {
                    if ui.button("＋ Log a book").clicked() {
                        actions.push(UiAction::LogNew);
                    }
                    if ui.button("📁 Scan folder").clicked() {
                        actions.push(UiAction::ScanFolder);
                    }
                });
                self.shelf_ledge(ui, theme);
            } else {
                self.shelf_row(ui, theme, &books, actions);
            }
            ui.add_space(if books.is_empty() { 14.0 } else { 28.0 });
        }

        // A small "custom shelf" for favorites.
        let favorites: Vec<Book> = self
            .books
            .iter()
            .filter(|book| book.is_favorite)
            .cloned()
            .collect();
        if !favorites.is_empty() {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Favorites")
                        .font(theme.serif(20.0))
                        .strong()
                        .color(theme.text),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("Full shelf  →")
                                    .size(12.0)
                                    .color(theme.accent),
                            )
                            .fill(egui::Color32::TRANSPARENT)
                            .stroke(egui::Stroke::NONE),
                        )
                        .clicked()
                    {
                        actions.push(UiAction::Nav(NavItem::Favorites));
                    }
                });
            });
            ui.add_space(18.0);
            self.shelf_row(ui, theme, &favorites, actions);
        }
    }

    fn shelf_ledge(&self, ui: &mut egui::Ui, theme: Theme) {
        let start = ui.next_widget_position();
        let ledge =
            egui::Rect::from_min_size(start, egui::vec2(ui.available_width().max(40.0), 14.0));
        draw_shelf(ui, ledge, theme);
        ui.add_space(14.0);
    }

    fn shelf_row(
        &mut self,
        ui: &mut egui::Ui,
        theme: Theme,
        books: &[Book],
        actions: &mut Vec<UiAction>,
    ) {
        // 135 × 202 covers resting directly on a 14px wooden ledge.
        let card_w = 135.0;
        let card_h = 202.0;
        let gap = 22.0;
        let start = ui.next_widget_position();
        let available = ui.available_width().max(40.0);
        let fits = (((available + gap) / (card_w + gap)).floor() as usize).max(1);

        let mut clicked: Option<i64> = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for book in books.iter().take(fits) {
                let texture = book
                    .cover_path
                    .as_deref()
                    .and_then(|path| self.cover_textures.get(path));
                if let Some(id) = shelf_book(ui, book, texture, card_w, card_h, theme) {
                    clicked = Some(id);
                }
            }
        });

        // Ledge is painted last so covers sit flush on top of it.
        let ledge = egui::Rect::from_min_size(
            egui::pos2(start.x, start.y + card_h),
            egui::vec2(available, 14.0),
        );
        draw_shelf(ui, ledge, theme);
        ui.add_space(14.0);

        if let Some(id) = clicked {
            actions.push(UiAction::Open(id));
        }
    }

    fn book_grid(
        &mut self,
        ui: &mut egui::Ui,
        theme: Theme,
        actions: &mut Vec<UiAction>,
        status: Option<ReadStatus>,
    ) {
        let label = match status {
            Some(value) => value.label(),
            None => match self.nav {
                NavItem::Diary => "Reading diary",
                NavItem::Favorites => "Favorites",
                NavItem::Lists => "Publishers",
                NavItem::Discover => "Discover",
                NavItem::Profile => "Reader profile",
                NavItem::Library => "All books",
            },
        };
        ui.label(
            egui::RichText::new(label)
                .font(theme.serif(20.0))
                .strong()
                .color(theme.text),
        );
        ui.add_space(18.0);

        if self.books.is_empty() {
            ui.label(
                egui::RichText::new("No books here yet — try another tab or log a new one.")
                    .size(13.0)
                    .color(theme.muted),
            );
            ui.horizontal(|ui| {
                if ui.button("＋ Log a book").clicked() {
                    actions.push(UiAction::LogNew);
                }
                if ui.button("📁 Scan folder").clicked() {
                    actions.push(UiAction::ScanFolder);
                }
            });
            return;
        }

        // CSS: repeat(auto-fill, minmax(145px, 165px)); gap: 28px 22px.
        let gap_x = 22.0;
        let gap_y = 28.0;
        let min_card = 145.0;
        let max_card = 165.0;
        let available = ui.available_width().max(120.0);
        let columns = (((available + gap_x) / (min_card + gap_x)).floor() as usize).max(1);
        let card_width = ((available - gap_x * (columns.saturating_sub(1)) as f32)
            / columns as f32)
            .min(max_card);

        let mut clicked: Option<UiAction> = None;
        for row in self.books.chunks(columns) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap_x;
                for book in row {
                    let texture = book
                        .cover_path
                        .as_deref()
                        .and_then(|path| self.cover_textures.get(path));
                    if let Some(action) = poster_card(ui, book, texture, card_width, theme) {
                        clicked = Some(action);
                    }
                }
            });
            ui.add_space(gap_y);
        }
        if let Some(action) = clicked {
            actions.push(action);
        }
    }

    // ---- Modals -----------------------------------------------------------
    fn log_edit_modal(&mut self, ctx: &egui::Context, theme: Theme) {
        let Some(mut edit) = self.log_edit.take() else {
            return;
        };
        let is_new = edit.book.id == 0;

        let mut picked: Option<SearchHit> = None;
        let mut local_cover: Option<PathBuf> = None;
        let mut cover_removed: Option<String> = None;
        let mut want_search = false;
        let mut want_download = false;
        let mut want_paste_clipboard = false;
        let mut want_paste_shortcut = false;
        let mut want_url_cover = false;
        let mut want_save = false;
        let mut want_cancel = false;
        let mut want_delete = false;
        let mut want_open_file: Option<String> = None;
        let mut want_summary_refresh = false;
        let mut want_open_url: Option<String> = None;
        let mut want_status: Option<ReadStatus> = None;

        let title = if is_new {
            "Log a Book"
        } else {
            "Edit Book & Bookmark"
        };
        let modal = egui::Modal::new(egui::Id::unique("log_book_modal"))
            .backdrop_color(egui::Color32::from_black_alpha(if theme.is_dark {
                150
            } else {
                80
            }))
            .frame(
                egui::Frame::new()
                    .fill(theme.card)
                    .stroke(egui::Stroke::new(1.0, theme.stitch))
                    .corner_radius(16.0)
                    .inner_margin(egui::Margin::same(22))
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 18],
                        blur: 44,
                        spread: 0,
                        color: theme.shadow,
                    }),
            )
            .show(ctx, |ui| {
                ui.set_width(700.0);
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(title)
                                .font(theme.serif(22.0))
                                .strong()
                                .color(theme.text),
                        )
                        .truncate(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("✕").size(14.0).color(theme.muted),
                                )
                                .fill(egui::Color32::TRANSPARENT)
                                .stroke(egui::Stroke::NONE)
                                .min_size(egui::vec2(32.0, 32.0)),
                            )
                            .clicked()
                        {
                            want_cancel = true;
                        }
                    });
                });
                ui.add_space(2.0);
                egui::ScrollArea::vertical()
                    .id_salt("log_modal_scroll")
                    .min_scrolled_height(540.0)
                    .max_height(600.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if let Some(relative) = &edit.cover {
                        if let Some(texture) = self.cover_textures.get(relative) {
                            ui.add(egui::Image::from_texture(texture).max_height(86.0));
                        } else {
                            ui.label(egui::RichText::new("📕").size(48.0));
                        }
                    } else {
                        ui.label(egui::RichText::new("📕").size(48.0));
                    }
                    ui.vertical(|ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(if edit.book.title.trim().is_empty() {
                                    "Untitled book"
                                } else {
                                    &edit.book.title
                                })
                                .font(theme.serif(20.0))
                                .color(theme.text),
                            )
                            .truncate(),
                        );
                        if !edit.malayalam_title_text.trim().is_empty() {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(edit.malayalam_title_text.trim())
                                        .size(15.0)
                                        .color(theme.accent),
                                )
                                .wrap(),
                            );
                        }
                        ui.add(
                            egui::Label::new(egui::RichText::new(&edit.book.author).color(theme.muted))
                                .truncate(),
                        );
                        if !edit.publisher_text.trim().is_empty() {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(edit.publisher_text.trim())
                                        .color(theme.muted)
                                        .small(),
                                )
                                .truncate(),
                            );
                        }
                    });
                });

                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("🔍 Find book online").clicked() {
                        edit.search_open = !edit.search_open;
                    }
                    let cover_label = if edit.cover_options_open {
                        "Hide cover options"
                    } else {
                        "Cover options"
                    };
                    if ui.button(cover_label).clicked() {
                        edit.cover_options_open = !edit.cover_options_open;
                    }
                    if let Some(path) = &edit.book.file_path {
                        if ui.button("📖 Open Book File").clicked() {
                            want_open_file = Some(path.clone());
                        }
                    }
                });

                // One-click shelf movement for saved books.
                if edit.book.id > 0 {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("Move to")
                                .small()
                                .color(theme.muted),
                        );
                        for (status, label) in [
                            (ReadStatus::WantToRead, "○ Want to read"),
                            (ReadStatus::Reading, "▶ Currently reading"),
                            (ReadStatus::Read, "✓ Finished"),
                        ] {
                            let active = edit.book.status == status;
                            if ui
                                .add(
                                    egui::Button::new(label)
                                        .fill(if active { theme.accent } else { theme.card })
                                        .stroke(egui::Stroke::new(1.0, theme.stitch))
                                        .min_size(egui::vec2(130.0, 30.0)),
                                )
                                .clicked()
                                && !active
                            {
                                want_status = Some(status);
                            }
                        }
                    });
                }

                // Short online summary for this book.
                if self.summary_receiver.is_some()
                    || self.summary_text.is_some()
                    || !self.summary_error.is_empty()
                {
                    ui.add_space(6.0);
                    egui::Frame::new()
                        .fill(theme.card)
                        .stroke(egui::Stroke::new(1.0, theme.stitch))
                        .corner_radius(10.0)
                        .inner_margin(10.0)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new("About this book")
                                        .small()
                                        .strong()
                                        .color(theme.accent),
                                );
                                if ui.button("↻").clicked() {
                                    want_summary_refresh = true;
                                }
                            });
                            if self.summary_receiver.is_some() {
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label(
                                        egui::RichText::new("Fetching a short summary from the internet…")
                                            .small()
                                            .color(theme.muted),
                                    );
                                });
                            }
                            if let Some(summary) = &self.summary_text {
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(summary).small().color(theme.text),
                                    )
                                    .wrap(),
                                );
                                ui.label(
                                    egui::RichText::new("Wikipedia / book APIs")
                                        .small()
                                        .color(theme.muted),
                                );
                            }
                            if !self.summary_error.is_empty() {
                                ui.label(
                                    egui::RichText::new(&self.summary_error)
                                        .small()
                                        .color(theme.muted),
                                );
                            }
                        });
                }

                // Legitimate places to read, borrow or buy this book.
                if self.download_links_receiver.is_some()
                    || !self.download_links.is_empty()
                    || !self.download_links_error.is_empty()
                {
                    ui.add_space(6.0);
                    egui::Frame::new()
                        .fill(theme.card)
                        .stroke(egui::Stroke::new(1.0, theme.stitch))
                        .corner_radius(10.0)
                        .inner_margin(10.0)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new("Get a copy")
                                        .small()
                                        .strong()
                                        .color(theme.accent),
                                );
                                ui.label(
                                    egui::RichText::new("(read, borrow or buy — legal sources)")
                                        .small()
                                        .color(theme.muted),
                                );
                            });
                            if self.download_links_receiver.is_some() {
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label(
                                        egui::RichText::new("Finding online copies…")
                                            .small()
                                            .color(theme.muted),
                                    );
                                });
                            }
                            for link in &self.download_links {
                                if ui.button(link.label).clicked() {
                                    want_open_url = Some(link.url.clone());
                                }
                            }
                            if !self.download_links_error.is_empty() {
                                ui.label(
                                    egui::RichText::new(&self.download_links_error)
                                        .small()
                                        .color(theme.muted),
                                );
                            }
                        });
                }

                // Modal-only feedback (ISBN errors, clipboard/URL issues…).
                if !edit.message.is_empty() {
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(&edit.message)
                            .size(13.0)
                            .color(theme.danger),
                    );
                }

                // Cover options are secondary — the online book search comes first.
                if ui.input(|state| {
                    state.key_pressed(egui::Key::V)
                        && (state.modifiers.ctrl || state.modifiers.command)
                }) {
                    want_paste_shortcut = true;
                }
                let dropped: Vec<PathBuf> = ui.input(|state| {
                    state
                        .raw
                        .dropped_files
                        .iter()
                        .map(|file| file.path().to_path_buf())
                        .collect()
                });
                if let Some(path) = dropped.first() {
                    local_cover = Some(path.clone());
                }
                if edit.cover_options_open {
                    egui::Frame::new()
                        .fill(theme.input)
                        .corner_radius(8.0)
                        .inner_margin(10.0)
                        .stroke(egui::Stroke::new(1.0, theme.stitch))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                if ui.button("📁 Choose cover image").clicked() {
                                    local_cover = rfd::FileDialog::new()
                                        .add_filter(
                                            "Images",
                                            &["png", "jpg", "jpeg", "webp", "gif", "bmp"],
                                        )
                                        .pick_file();
                                }
                                if ui.button("⬇ Cover by ISBN").clicked() {
                                    want_download = true;
                                }
                                if edit.cover.is_some() && ui.button("Remove cover").clicked() {
                                    if let Some(relative) = edit.cover.take() {
                                        cover_removed = Some(relative);
                                    }
                                }
                            });
                            ui.label(
                                egui::RichText::new(
                                    "Drop an image, paste with Ctrl+V / ⌘V, or paste a direct image URL.",
                                )
                                .size(12.0)
                                .color(theme.muted),
                            );
                            ui.horizontal(|ui| {
                                if ui.button("📋 Paste image from clipboard").clicked() {
                                    want_paste_clipboard = true;
                                }
                                ui.add(
                                    egui::TextEdit::singleline(&mut edit.cover_url_text)
                                        .desired_width(320.0)
                                        .margin(egui::Margin::symmetric(10, 8))
                                        .hint_text("https://example.com/cover.jpg"),
                                );
                                if ui.button("Load URL").clicked() {
                                    want_url_cover = true;
                                }
                            });
                        });
                }

                if edit.search_open {
                    ui.add_space(8.0);
                    ui.separator();
                    ui.horizontal(|ui| {
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut edit.search_query)
                                .desired_width(420.0)
                                .margin(egui::Margin::symmetric(10, 8))
                                .hint_text("Title, author, or ISBN…"),
                        );
                        if response.lost_focus()
                            && ui.input(|state| state.key_pressed(egui::Key::Enter))
                        {
                            want_search = true;
                        }
                        if ui
                            .add(egui::Button::new("Search").fill(theme.accent))
                            .clicked()
                        {
                            want_search = true;
                        }
                    });
                    if edit.search_receiver.is_some() {
                        ui.label(
                            egui::RichText::new(
                                "Searching Google Books + Open Library + Goodreads…",
                            )
                            .color(theme.muted),
                        );
                    }
                    if !edit.search_error.is_empty() {
                        ui.colored_label(theme.danger, &edit.search_error);
                    }
                    egui::ScrollArea::vertical()
                        .max_height(160.0)
                        .show(ui, |ui| {
                            for index in 0..edit.search_results.len() {
                                let hit = edit.search_results[index].clone();
                                let selected = edit.selected_search == Some(index);
                                let label = format!(
                                    "{}{}{}",
                                    hit.title,
                                    hit.year.map(|year| format!(" ({year})")).unwrap_or_default(),
                                    hit.average_rating
                                        .map(|rating| format!("  ★ {rating:.1}"))
                                        .unwrap_or_default()
                                );
                                ui.horizontal(|ui| {
                                    if ui
                                        .add(
                                            egui::Button::new(label)
                                                .truncate()
                                                .fill(if selected {
                                                    theme.pill_active
                                                } else {
                                                    theme.card
                                                })
                                                .stroke(egui::Stroke::new(
                                                    1.0,
                                                    if selected {
                                                        theme.accent
                                                    } else {
                                                        theme.stitch
                                                    },
                                                ))
                                                .min_size(egui::vec2(500.0, 28.0)),
                                        )
                                        .clicked()
                                    {
                                        edit.selected_search = Some(index);
                                    }
                                    // Source badge: where this metadata came from.
                                    ui.label(
                                        egui::RichText::new(hit.source_label())
                                            .small()
                                            .strong()
                                            .color(theme.accent),
                                    );
                                    if hit.has_cover() {
                                        ui.label(
                                            egui::RichText::new("▣")
                                                .small()
                                                .color(theme.accent),
                                        );
                                    }
                                });
                                let detail = match &hit.publisher {
                                    Some(publisher) => {
                                        format!("{} · {publisher}", hit.author())
                                    }
                                    None => hit.author().to_string(),
                                };
                                let mut detail = detail;
                                if let Some(pages) = hit.page_count {
                                    detail.push_str(&format!(" · {pages} pages"));
                                }
                                if let Some(description) = &hit.description {
                                    detail.push_str(&format!(" · {}", truncate(description, 70)));
                                }
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(detail).color(theme.muted).small(),
                                    )
                                    .truncate(),
                                );
                            }
                        });
                        if let Some(index) = edit.selected_search {
                            if let Some(hit) = edit.search_results.get(index) {
                                egui::Frame::new()
                                    .fill(theme.card)
                                    .stroke(egui::Stroke::new(1.0, theme.accent.gamma_multiply(0.55)))
                                    .corner_radius(10.0)
                                    .inner_margin(10.0)
                                    .show(ui, |ui| {
                                        ui.label(
                                            egui::RichText::new("Selected result")
                                                .small()
                                                .strong()
                                                .color(theme.accent),
                                        );
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(&hit.title).color(theme.text),
                                            )
                                            .wrap(),
                                        );
                                        let mut detail = format!(
                                            "{} · {}",
                                            hit.author(),
                                            hit.source_label()
                                        );
                                        if let Some(publisher) = &hit.publisher {
                                            detail.push_str(&format!(" · {publisher}"));
                                        }
                                        if let Some(pages) = hit.page_count {
                                            detail.push_str(&format!(" · {pages} pages"));
                                        }
                                        ui.label(
                                            egui::RichText::new(detail).small().color(theme.muted),
                                        );
                                        if let Some(description) = &hit.description {
                                            ui.add(
                                                egui::Label::new(
                                                    egui::RichText::new(truncate(description, 180))
                                                        .small()
                                                        .color(theme.muted),
                                                )
                                                .wrap(),
                                            );
                                        }
                                        if ui
                                            .add(
                                                egui::Button::new("Use this result")
                                                    .fill(theme.accent)
                                                    .min_size(egui::vec2(150.0, 32.0)),
                                            )
                                            .clicked()
                                        {
                                            picked = Some(hit.clone());
                                        }
                                    });
                            }
                        }
                    ui.separator();
                }

                ui.add_space(8.0);
                ui.columns(2, |columns| {
                    {
                        let left = &mut columns[0];
                        left.heading("Log");
                        left.label(egui::RichText::new("Title").color(theme.muted).small());
                        left.add(
                            egui::TextEdit::singleline(&mut edit.book.title)
                                .margin(egui::Margin::symmetric(10, 8))
                                .desired_width(f32::INFINITY),
                        );
                        left.label(egui::RichText::new("Author").color(theme.muted).small());
                        left.add(
                            egui::TextEdit::singleline(&mut edit.book.author)
                                .margin(egui::Margin::symmetric(10, 8))
                                .desired_width(f32::INFINITY),
                        );
                        left.label(
                            egui::RichText::new("Malayalam title (മലയാളം)")
                                .color(theme.accent)
                                .small(),
                        );
                        left.add(
                            egui::TextEdit::singleline(&mut edit.malayalam_title_text)
                                .margin(egui::Margin::symmetric(10, 8))
                                .desired_width(f32::INFINITY)
                                .hint_text("ഒരു ദേശത്തിന്റെ കഥ"),
                        );
                        left.label(egui::RichText::new("Publisher").color(theme.muted).small());
                        left.add(
                            egui::TextEdit::singleline(&mut edit.publisher_text)
                                .margin(egui::Margin::symmetric(10, 8))
                                .desired_width(f32::INFINITY)
                                .hint_text("DC Books, Mathrubhumi Books, Olive…"),
                        );

                        left.add_space(8.0);
                        left.horizontal(|ui| {
                            ui.checkbox(&mut edit.date_set, "Date finished");
                            if edit.date_set {
                                ui.add(DatePickerButton::new(&mut edit.date));
                            }
                        });
                        left.add_space(8.0);
                        left.label(
                            egui::RichText::new("Your rating").color(theme.muted).small(),
                        );
                        star_rating(left, &mut edit.rating, theme);

                        left.add_space(6.0);
                        left.horizontal(|ui| {
                            let heart = if edit.book.is_favorite {
                                "♥ Favorite"
                            } else {
                                "♡ Favorite"
                            };
                            let button = egui::Button::new(
                                egui::RichText::new(heart).color(if edit.book.is_favorite {
                                    theme.accent
                                } else {
                                    theme.muted
                                }),
                            )
                            .fill(if edit.book.is_favorite {
                                theme.pill_active
                            } else {
                                egui::Color32::TRANSPARENT
                            })
                            .corner_radius(12.0);
                            if ui.add(button).clicked() {
                                edit.book.is_favorite = !edit.book.is_favorite;
                            }
                            egui::ComboBox::from_id_salt("log_status")
                                .selected_text(edit.book.status.label())
                                .show_ui(ui, |ui| {
                                    for status in ReadStatus::ALL {
                                        ui.selectable_value(
                                            &mut edit.book.status,
                                            status,
                                            status.label(),
                                        );
                                    }
                                });
                        });

                        left.add_space(8.0);
                        left.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("Review / Notes")
                                    .color(theme.muted)
                                    .small(),
                            );
                            ui.checkbox(&mut edit.review_preview, "Preview");
                        });
                        if edit.review_preview {
                            egui::Frame::new()
                                .fill(theme.input)
                                .corner_radius(8.0)
                                .inner_margin(8.0)
                                .stroke(egui::Stroke::new(1.0, theme.stitch))
                                .show(left, |ui| {
                                    markdown_preview(ui, &edit.book.review, theme);
                                });
                        } else {
                            left.add(
                                egui::TextEdit::multiline(&mut edit.book.review)
                                    .margin(egui::Margin::symmetric(10, 8))
                                    .desired_rows(7)
                                    .desired_width(f32::INFINITY)
                                    .hint_text("Markdown: # heading, - bullet, **bold**, *italic*…"),
                            );
                        }
                    }

                    {
                        let right = &mut columns[1];
                        right.heading("Bookmark Tracker");
                        right.label(
                            egui::RichText::new("Current page").color(theme.muted).small(),
                        );
                        right.add(
                            egui::DragValue::new(&mut edit.book.current_page).range(0..=1_000_000),
                        );
                        right.label(egui::RichText::new("Total pages").color(theme.muted).small());
                        right.add(
                            egui::DragValue::new(&mut edit.book.total_pages).range(0..=1_000_000),
                        );
                        right.add_space(8.0);
                        progress_bar(right, edit.book.current_page, edit.book.total_pages, theme);
                        right.add_space(12.0);
                        right.label(
                            egui::RichText::new("Bookmark Note").color(theme.muted).small(),
                        );
                        right.add(
                            egui::TextEdit::multiline(&mut edit.bookmark_note_text)
                                .margin(egui::Margin::symmetric(10, 8))
                                .desired_rows(5)
                                .desired_width(f32::INFINITY)
                                .hint_text("Where you left off and what you were thinking…"),
                        );
                    }
                });

                ui.add_space(10.0);
                ui.separator();
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("Save Book").color(theme.on_accent()),
                            )
                            .fill(theme.accent)
                            .min_size(egui::vec2(120.0, 32.0)),
                        )
                        .clicked()
                    {
                        want_save = true;
                    }
                    if ui
                        .add(egui::Button::new("Cancel").min_size(egui::vec2(88.0, 32.0)))
                        .clicked()
                    {
                        want_cancel = true;
                    }
                    if !is_new
                        && ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("Delete book").color(theme.danger),
                                )
                                .fill(theme.input),
                            )
                            .clicked()
                    {
                        want_delete = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut edit.book.isbn)
                                .margin(egui::Margin::symmetric(10, 8))
                                .desired_width(180.0),
                        );
                        ui.label(egui::RichText::new("ISBN").size(13.0).color(theme.muted));
                    });
                });
                    });
            });

        self.log_edit = Some(edit);

        if let Some(path) = want_open_file {
            if let Err(error) = open::that(&path) {
                self.modal_message(format!("Could not open book file: {error}"));
            }
        }
        if let Some(url) = want_open_url {
            if let Err(error) = open::that(&url) {
                self.modal_message(format!("Could not open the link: {error}"));
            }
        }
        if let Some(status) = want_status {
            let id = self.log_edit.as_ref().map(|edit| edit.book.id);
            if let Some(id) = id {
                if id > 0 {
                    self.set_status(id, status);
                    if let Some(edit) = &mut self.log_edit {
                        edit.book.status = status;
                        edit.message = format!("Moved to {}.", status.label());
                    }
                }
            }
        }
        if want_summary_refresh {
            let target = self
                .log_edit
                .as_ref()
                .map(|edit| (edit.book.title.clone(), edit.book.author.clone()));
            if let Some((title, author)) = target {
                self.start_summary(ctx, &title, &author);
            }
        }
        if let Some(relative) = cover_removed {
            crate::covers::remove_cover(&self.db, &relative);
        }
        if want_search {
            if let Some(edit) = &mut self.log_edit {
                edit.start_search(ctx);
            }
        }
        if let Some(hit) = picked {
            self.apply_search_hit(ctx, &hit);
        }
        if let Some(path) = local_cover {
            let temp_id = self.next_temp_id();
            match crate::covers::import_local_cover(&self.db, temp_id, &path) {
                Ok(relative) => {
                    if let Some(edit) = &mut self.log_edit {
                        edit.cover = Some(relative.clone());
                    }
                    self.load_cover_texture(ctx, &relative);
                }
                Err(error) => self.modal_message(error.to_string()),
            }
        }
        if want_download {
            let temp_id = self.next_temp_id();
            let isbn = self
                .log_edit
                .as_ref()
                .map(|edit| edit.book.isbn.clone())
                .unwrap_or_default();
            match crate::covers::download_cover(&self.db, temp_id, &isbn) {
                Ok(relative) => {
                    if let Some(edit) = &mut self.log_edit {
                        edit.cover = Some(relative.clone());
                    }
                    self.load_cover_texture(ctx, &relative);
                }
                Err(error) => self.modal_message(error.to_string()),
            }
        }
        if want_paste_clipboard || want_paste_shortcut {
            match arboard::Clipboard::new() {
                Ok(mut clipboard) => match clipboard.get_image() {
                    Ok(image) => {
                        let temp_id = self.next_temp_id();
                        match crate::covers::save_rgba_bytes(
                            &self.db,
                            temp_id,
                            "paste",
                            image.width as u32,
                            image.height as u32,
                            image.bytes.as_ref(),
                        ) {
                            Ok(relative) => {
                                if let Some(edit) = &mut self.log_edit {
                                    edit.cover = Some(relative.clone());
                                }
                                self.load_cover_texture(ctx, &relative);
                                self.modal_message("Cover pasted from clipboard.");
                            }
                            Err(error) => self.modal_message(error.to_string()),
                        }
                    }
                    Err(error) => {
                        if want_paste_clipboard {
                            self.modal_message(format!("No image on clipboard: {error}"));
                        }
                    }
                },
                Err(error) => {
                    if want_paste_clipboard {
                        self.modal_message(format!("Clipboard unavailable: {error}"));
                    }
                }
            }
        }
        if want_url_cover {
            let url = self
                .log_edit
                .as_ref()
                .map(|edit| edit.cover_url_text.trim().to_string())
                .unwrap_or_default();
            if url.is_empty() {
                self.modal_message("Paste a direct image URL first.");
            } else {
                let temp_id = self.next_temp_id();
                match crate::openlibrary::fetch_cover_url(&url) {
                    Ok(bytes) => {
                        match crate::covers::save_cover_bytes(&self.db, temp_id, "url", &bytes) {
                            Ok(relative) => {
                                if let Some(edit) = &mut self.log_edit {
                                    edit.cover = Some(relative.clone());
                                }
                                self.load_cover_texture(ctx, &relative);
                                self.modal_message("Cover downloaded from URL.");
                            }
                            Err(error) => self.modal_message(error.to_string()),
                        }
                    }
                    Err(error) => self.modal_message(error.to_string()),
                }
            }
        }

        if want_save {
            self.save_log_edit();
        } else if want_cancel {
            self.cancel_log_edit();
        } else if want_delete {
            if let Some(edit) = &self.log_edit {
                self.confirm_id = Some(edit.book.id);
                self.confirm_open = true;
                self.delete_opened_frame = Some(self.frame_no);
            }
            self.log_edit = None;
        } else if self.log_opened_frame != Some(self.frame_no) && modal.should_close() {
            self.cancel_log_edit();
        }
    }

    fn delete_modal(&mut self, ctx: &egui::Context, theme: Theme) {
        if !self.confirm_open {
            return;
        }
        let mut cancel = false;
        let mut remove = false;
        let modal = egui::Modal::new(egui::Id::unique("delete_book_modal"))
            .backdrop_color(egui::Color32::from_black_alpha(if theme.is_dark {
                150
            } else {
                80
            }))
            .frame(
                egui::Frame::new()
                    .fill(theme.card)
                    .stroke(egui::Stroke::new(1.0, theme.stitch))
                    .corner_radius(16.0)
                    .inner_margin(egui::Margin::same(22))
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 18],
                        blur: 44,
                        spread: 0,
                        color: theme.shadow,
                    }),
            )
            .show(ctx, |ui| {
                ui.set_width(360.0);
                ui.label(
                    egui::RichText::new("Remove book?")
                        .font(theme.serif(20.0))
                        .strong()
                        .color(theme.text),
                );
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new("This also removes the cached cover image.")
                        .size(13.0)
                        .color(theme.muted),
                );
                if !self.confirm_message.is_empty() {
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(&self.confirm_message)
                            .size(13.0)
                            .color(theme.danger),
                    );
                }
                ui.add_space(18.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("Remove").color(theme.on_accent()),
                            )
                            .fill(theme.accent)
                            .min_size(egui::vec2(96.0, 32.0)),
                        )
                        .clicked()
                    {
                        remove = true;
                    }
                    if ui
                        .add(egui::Button::new("Cancel").min_size(egui::vec2(88.0, 32.0)))
                        .clicked()
                    {
                        cancel = true;
                    }
                });
            });

        if remove {
            if let Some(id) = self.confirm_id.take() {
                self.delete_book(id);
            }
        } else if cancel
            || (self.delete_opened_frame != Some(self.frame_no) && modal.should_close())
        {
            self.confirm_open = false;
            self.confirm_id = None;
            self.confirm_message.clear();
        }
    }

    /// Editable list of books found by the folder scanner before committing.
    fn scan_preview_modal(&mut self, ctx: &egui::Context, theme: Theme) {
        if !self.scan_preview_open {
            return;
        }
        let mut previews = std::mem::take(&mut self.scan_preview);
        if previews.is_empty() {
            self.scan_preview_open = false;
            return;
        };
        let mut want_import = false;
        let mut want_cancel = false;
        let mut removed: Vec<usize> = Vec::new();

        let modal = egui::Modal::new(egui::Id::unique("scan_preview_modal"))
            .backdrop_color(egui::Color32::from_black_alpha(if theme.is_dark { 150 } else { 80 }))
            .frame(
                egui::Frame::new()
                    .fill(theme.card)
                    .stroke(egui::Stroke::new(1.0, theme.stitch))
                    .corner_radius(16.0)
                    .inner_margin(egui::Margin::same(22))
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 18],
                        blur: 44,
                        spread: 0,
                        color: theme.shadow,
                    }),
            )
            .show(ctx, |ui| {
                ui.set_width(760.0);
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Review scanned books")
                            .font(theme.serif(20.0))
                            .strong()
                            .color(theme.text),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("✕").clicked() {
                            want_cancel = true;
                        }
                    });
                });
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new("Edit the extracted metadata before importing. Covers are downloaded when you confirm.")
                        .small()
                        .color(theme.muted),
                );
                ui.add_space(10.0);
                egui::ScrollArea::vertical()
                    .max_height(420.0)
                    .show(ui, |ui| {
                        for (index, book) in previews.iter_mut().enumerate() {
                            ui.horizontal(|ui| {
                                ui.label(format!("{:>2}", index + 1)).on_hover_text(&book.file_path);
                                ui.add(
                                    egui::TextEdit::singleline(&mut book.title)
                                        .desired_width(260.0)
                                        .hint_text("Title"),
                                );
                                ui.add(
                                    egui::TextEdit::singleline(&mut book.author)
                                        .desired_width(170.0)
                                        .hint_text("Author"),
                                );
                                ui.add(
                                    egui::DragValue::new(&mut book.total_pages)
                                        .range(0..=1_000_000)
                                        .prefix("p "),
                                );
                                if ui.button("✕").clicked() {
                                    removed.push(index);
                                }
                            });
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(&book.file_path)
                                        .small()
                                        .color(theme.muted),
                                )
                                .truncate(),
                            );
                            ui.add_space(6.0);
                        }
                    });
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(format!("Import {} books", previews.len()))
                                .fill(theme.accent),
                        )
                        .clicked()
                    {
                        want_import = true;
                    }
                    if ui.button("Cancel").clicked() {
                        want_cancel = true;
                    }
                });
            });

        for index in removed.into_iter().rev() {
            if index < previews.len() {
                previews.remove(index);
            }
        }
        if want_import {
            self.scan_preview = previews;
            self.scan_preview_open = false;
            let approved = self.scan_preview.clone();
            self.start_import(ctx, approved);
            return;
        }
        if want_cancel
            || (self.scan_preview_opened_frame != Some(self.frame_no) && modal.should_close())
        {
            self.scan_preview_open = false;
        }
        self.scan_preview = previews;
    }

    fn folder_remove_modal(&mut self, ctx: &egui::Context, theme: Theme) {
        if !self.folder_remove_open {
            return;
        }
        let Some(id) = self.folder_remove_id else {
            self.folder_remove_open = false;
            return;
        };
        let path = self.folder_remove_path.clone();
        let mut folder_only = false;
        let mut with_books = false;
        let mut cancel = false;

        let modal = egui::Modal::new(egui::Id::unique("folder_remove_modal"))
            .backdrop_color(egui::Color32::from_black_alpha(if theme.is_dark { 150 } else { 80 }))
            .frame(
                egui::Frame::new()
                    .fill(theme.card)
                    .stroke(egui::Stroke::new(1.0, theme.stitch))
                    .corner_radius(16.0)
                    .inner_margin(egui::Margin::same(22))
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 18],
                        blur: 44,
                        spread: 0,
                        color: theme.shadow,
                    }),
            )
            .show(ctx, |ui| {
                ui.set_width(420.0);
                ui.label(
                    egui::RichText::new("Remove library folder?")
                        .font(theme.serif(20.0))
                        .strong()
                        .color(theme.text),
                );
                ui.add_space(8.0);
                ui.add(
                    egui::Label::new(egui::RichText::new(&path).color(theme.text)).wrap(),
                );
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new("Removing the folder stops future scans. Imported books stay in your library unless you also delete them.")
                        .small()
                        .color(theme.muted),
                );
                ui.add_space(18.0);
                ui.horizontal(|ui| {
                    if ui.button("Remove folder").clicked() {
                        folder_only = true;
                    }
                    if ui
                        .add(
                            egui::Button::new("Remove + books")
                                .fill(theme.danger)
                                .min_size(egui::vec2(120.0, 32.0)),
                        )
                        .clicked()
                    {
                        with_books = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });

        if folder_only || with_books {
            self.remove_folder(id, with_books);
        } else if cancel
            || (self.folder_remove_opened_frame != Some(self.frame_no) && modal.should_close())
        {
            self.folder_remove_open = false;
            self.folder_remove_id = None;
        }
    }
}

// ---------------------------------------------------------------------------
// Drawing helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NavIcon {
    Library,
    Diary,
    Heart,
    List,
    Plus,
}

/// Full-width 38px nav pill: left-aligned icon + label, never centered.
fn nav_pill(ui: &mut egui::Ui, label: &str, icon: NavIcon, active: bool, theme: Theme) -> bool {
    let (outer, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 38.0), egui::Sense::click());
    let rect = egui::Rect::from_min_max(
        egui::pos2(outer.left() + 16.0, outer.top()),
        egui::pos2(outer.right() - 16.0, outer.bottom()),
    );
    let hovered = response.hovered();

    if active || hovered {
        ui.painter().rect_filled(
            rect,
            10.0f32,
            if active {
                theme.pill_active
            } else {
                theme.card
            },
        );
        if active {
            ui.painter().rect_stroke(
                rect,
                10.0f32,
                egui::Stroke::new(1.0, theme.stitch),
                egui::StrokeKind::Inside,
            );
        }
    }

    let color = if active { theme.accent } else { theme.text };
    let icon_center = egui::pos2(rect.left() + 14.0 + 7.0, rect.center().y);
    draw_nav_icon(ui.painter(), icon_center, icon, color);
    ui.painter().text(
        egui::pos2(rect.left() + 38.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(13.0),
        color,
    );

    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.clicked()
}

fn draw_nav_icon(painter: &egui::Painter, center: egui::Pos2, icon: NavIcon, color: egui::Color32) {
    let stroke = egui::Stroke::new(1.4, color);
    let rect = egui::Rect::from_center_size(center, egui::vec2(14.0, 14.0));
    match icon {
        NavIcon::Library => {
            // Three book spines of slightly different heights.
            for (index, height) in [11.0f32, 14.0, 9.0].into_iter().enumerate() {
                let x = rect.left() + index as f32 * 5.0;
                painter.rect_filled(
                    egui::Rect::from_min_size(
                        egui::pos2(x, center.y + 7.0 - height),
                        egui::vec2(3.0, height),
                    ),
                    1.0f32,
                    color,
                );
            }
        }
        NavIcon::Diary => {
            painter.rect_stroke(
                egui::Rect::from_center_size(center, egui::vec2(12.0, 14.0)),
                2.0f32,
                stroke,
                egui::StrokeKind::Inside,
            );
            for index in 0..3 {
                let y = rect.top() + 4.0 + index as f32 * 3.0;
                painter.line_segment(
                    [
                        egui::pos2(rect.left() + 3.0, y),
                        egui::pos2(rect.right() - 3.0, y),
                    ],
                    egui::Stroke::new(1.0, color),
                );
            }
        }
        NavIcon::Heart => {
            painter.text(
                center,
                egui::Align2::CENTER_CENTER,
                "♥",
                egui::FontId::proportional(14.0),
                color,
            );
        }
        NavIcon::List => {
            for index in 0..3 {
                let y = rect.top() + 3.0 + index as f32 * 4.0;
                painter.circle_filled(egui::pos2(rect.left() + 1.5, y), 1.2, color);
                painter.line_segment(
                    [
                        egui::pos2(rect.left() + 5.0, y),
                        egui::pos2(rect.right(), y),
                    ],
                    egui::Stroke::new(1.2, color),
                );
            }
        }
        NavIcon::Plus => {
            painter.line_segment(
                [
                    egui::pos2(center.x - 6.0, center.y),
                    egui::pos2(center.x + 6.0, center.y),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x, center.y - 6.0),
                    egui::pos2(center.x, center.y + 6.0),
                ],
                stroke,
            );
        }
    }
}

fn draw_shelf(ui: &mut egui::Ui, rect: egui::Rect, theme: Theme) {
    // Soft shadow cast by the front lip.
    for index in 1..=5 {
        let shadow = rect
            .translate(egui::vec2(0.0, index as f32 * 3.0))
            .expand(index as f32 * 0.6);
        ui.painter().rect_filled(
            shadow,
            3.0f32,
            theme.shadow.gamma_multiply(0.50 - index as f32 * 0.07),
        );
    }

    // Main wooden ledge: bright top surface, darker front edge.
    ui.painter().rect_filled(rect, 3.0f32, theme.ledge);
    ui.painter().rect_filled(
        egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), 2.0)),
        0.0f32,
        theme.ledge_top,
    );
    let lip = egui::Rect::from_min_size(
        egui::pos2(rect.min.x, rect.max.y - 3.0),
        egui::vec2(rect.width(), 3.0),
    );
    ui.painter()
        .rect_filled(lip, 0.0f32, theme.ledge.gamma_multiply(0.68));
    ui.painter().rect_stroke(
        rect,
        3.0f32,
        egui::Stroke::new(1.0, theme.stitch),
        egui::StrokeKind::Inside,
    );

    // Small end brackets make the ledge read as furniture, not a divider.
    for x in [rect.min.x + 6.0, rect.max.x - 12.0] {
        let bracket =
            egui::Rect::from_min_size(egui::pos2(x, rect.max.y - 2.0), egui::vec2(6.0, 7.0));
        ui.painter()
            .rect_filled(bracket, 2.0f32, theme.ledge.gamma_multiply(0.58));
    }
}

fn draw_book_cover(
    ui: &mut egui::Ui,
    texture: Option<&egui::TextureHandle>,
    rect: egui::Rect,
    theme: Theme,
) {
    draw_cover(ui, texture, rect, theme);
}

fn draw_cover(
    ui: &mut egui::Ui,
    texture: Option<&egui::TextureHandle>,
    rect: egui::Rect,
    theme: Theme,
) {
    // Deep spine/drop shadow behind the book.
    ui.painter()
        .rect_filled(rect.translate(egui::vec2(-3.0, 4.0)), 6.0f32, theme.shadow);
    // A corrupt/empty texture must degrade to the placeholder, never divide by zero.
    let usable = texture.filter(|handle| {
        let [width, height] = handle.size();
        width > 0 && height > 0 && rect.width() > 0.0 && rect.height() > 0.0
    });
    if let Some(handle) = usable {
        let [width, height] = handle.size();
        let scale = (rect.width() / width as f32).min(rect.height() / height as f32);
        let drawn = egui::Rect::from_center_size(
            rect.center(),
            egui::vec2(width as f32 * scale, height as f32 * scale),
        );
        ui.painter().image(
            handle.id(),
            drawn,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    } else {
        ui.painter().rect_filled(rect, 6.0f32, theme.ledge);
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "NO COVER",
            egui::FontId::proportional(9.0),
            theme.muted,
        );
    }
    // Inset spine highlight and edge.
    ui.painter().rect_filled(
        egui::Rect::from_min_size(rect.min, egui::vec2(2.0, rect.height())),
        1.0f32,
        egui::Color32::from_white_alpha(34),
    );
    ui.painter().rect_stroke(
        rect,
        6.0f32,
        egui::Stroke::new(1.0, egui::Color32::from_black_alpha(70)),
        egui::StrokeKind::Inside,
    );
}

fn shelf_book(
    ui: &mut egui::Ui,
    book: &Book,
    texture: Option<&egui::TextureHandle>,
    width: f32,
    height: f32,
    theme: Theme,
) -> Option<i64> {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
    draw_cover(ui, texture, rect, theme);

    if response.hovered() {
        ui.painter().rect_stroke(
            rect,
            3.0f32,
            egui::Stroke::new(2.0, theme.accent),
            egui::StrokeKind::Inside,
        );
        let glyphs = star_glyphs(book.rating);
        ui.painter().text(
            rect.min + egui::vec2(4.0, 4.0),
            egui::Align2::LEFT_TOP,
            glyphs,
            egui::FontId::proportional(9.0),
            theme.accent,
        );
        if book.is_favorite {
            ui.painter().text(
                rect.max - egui::vec2(6.0, 4.0),
                egui::Align2::RIGHT_TOP,
                "♥",
                egui::FontId::proportional(11.0),
                theme.accent,
            );
        }
        if book.total_pages > 0 {
            let fraction = (book.current_page as f32 / book.total_pages as f32).clamp(0.0, 1.0);
            let bar = egui::Rect::from_min_size(
                rect.min + egui::vec2(4.0, rect.height() - 6.0),
                egui::vec2(rect.width() - 8.0, 3.0),
            );
            ui.painter().rect_filled(bar, 1.5f32, theme.stitch);
            ui.painter().rect_filled(
                egui::Rect::from_min_size(bar.min, egui::vec2(bar.width() * fraction, 3.0)),
                1.5f32,
                theme.accent,
            );
        }
    } else if !book.review.trim().is_empty() || !book.bookmark_note.trim().is_empty() {
        let mark =
            egui::Rect::from_min_size(rect.max - egui::vec2(10.0, 12.0), egui::vec2(6.0, 9.0));
        ui.painter().rect_filled(mark, 1.0f32, theme.accent);
    }

    if response.clicked() {
        Some(book.id)
    } else {
        None
    }
}

fn poster_card(
    ui: &mut egui::Ui,
    book: &Book,
    texture: Option<&egui::TextureHandle>,
    width: f32,
    theme: Theme,
) -> Option<UiAction> {
    let cover_h = width * 1.5;
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, cover_h + 64.0), egui::Sense::click());
    let hovered = response.hovered();

    draw_cover(
        ui,
        texture,
        egui::Rect::from_min_size(rect.min, egui::vec2(width, cover_h)),
        theme,
    );
    if hovered {
        ui.painter().rect_stroke(
            egui::Rect::from_min_size(rect.min, egui::vec2(width, cover_h)),
            6.0f32,
            egui::Stroke::new(2.0, theme.accent),
            egui::StrokeKind::Inside,
        );
    }

    // 13px title, hard-clamped to two lines so cards never misalign.
    // Leave room for the favourite heart so a long title never runs under it.
    let title_width = if book.is_favorite {
        width - 18.0
    } else {
        width
    };
    let title_rect = egui::Rect::from_min_size(
        rect.min + egui::vec2(0.0, cover_h + 10.0),
        egui::vec2(title_width.max(20.0), 34.0),
    );
    draw_clamped_title(ui, title_rect, &book.title, theme, 13.0);

    ui.painter().text(
        rect.min + egui::vec2(0.0, cover_h + 46.0),
        egui::Align2::LEFT_TOP,
        star_glyphs(book.rating),
        egui::FontId::proportional(12.0),
        theme.accent,
    );
    if book.is_favorite {
        ui.painter().text(
            rect.min + egui::vec2(width - 2.0, cover_h + 10.0),
            egui::Align2::RIGHT_TOP,
            "♥",
            egui::FontId::proportional(12.0),
            theme.accent,
        );
    }

    if hovered {
        let mut icons: Vec<(&str, UiAction)> = vec![
            ("★", UiAction::Open(book.id)),
            ("▶", UiAction::SetStatus(book.id, ReadStatus::Reading)),
            ("✓", UiAction::SetStatus(book.id, ReadStatus::Read)),
            ("♥", UiAction::ToggleFavorite(book.id)),
            ("✕", UiAction::Delete(book.id)),
        ];
        if let Some(path) = &book.file_path {
            icons.insert(1, ("📖", UiAction::OpenFile(path.clone())));
        }
        for (index, (glyph, action)) in icons.iter().enumerate() {
            let button = egui::Rect::from_min_size(
                rect.min + egui::vec2(6.0 + index as f32 * 24.0, cover_h - 28.0),
                egui::vec2(22.0, 22.0),
            );
            // `interact` does not consume layout space, so neighbouring cards
            // never shift when the hover actions appear.
            let hit = ui.interact(
                button,
                ui.scope_id().with(("poster_action", book.id, index)),
                egui::Sense::click(),
            );
            ui.painter()
                .rect_filled(button, 6.0f32, egui::Color32::from_black_alpha(180));
            ui.painter().text(
                button.center(),
                egui::Align2::CENTER_CENTER,
                *glyph,
                egui::FontId::proportional(12.0),
                if hit.hovered() {
                    theme.accent
                } else {
                    theme.text
                },
            );
            if hit.clicked() {
                return Some(match action {
                    UiAction::Open(id) => UiAction::Open(*id),
                    UiAction::OpenFile(path) => UiAction::OpenFile(path.clone()),
                    UiAction::ToggleFavorite(id) => UiAction::ToggleFavorite(*id),
                    UiAction::SetStatus(id, status) => UiAction::SetStatus(*id, *status),
                    UiAction::Delete(id) => UiAction::Delete(*id),
                    other => other.clone(),
                });
            }
        }
    }

    if response.double_clicked() {
        if let Some(path) = &book.file_path {
            return Some(UiAction::OpenFile(path.clone()));
        }
    }
    if response.clicked() {
        Some(UiAction::Open(book.id))
    } else {
        None
    }
}

// ---- Small helpers --------------------------------------------------------

/// Persisted theme preference (egui has no localStorage).
fn load_dark_pref(data_dir: &std::path::Path) -> Option<bool> {
    let text = std::fs::read_to_string(data_dir.join("prefs.txt")).ok()?;
    match text.trim() {
        "dark" => Some(true),
        "light" => Some(false),
        _ => None,
    }
}

fn save_dark_pref(data_dir: &std::path::Path, dark: bool) {
    let _ = std::fs::write(
        data_dir.join("prefs.txt"),
        if dark { "dark\n" } else { "light\n" },
    );
}

fn star_glyphs(rating: Option<f64>) -> String {
    let Some(rating) = rating else {
        return "☆☆☆☆☆".to_string();
    };
    let full = rating.floor() as usize;
    let half = rating - full as f64 >= 0.25 && full < 5;
    let mut text = "★".repeat(full);
    if half {
        text.push('½');
    }
    while text.chars().count() < 5 {
        text.push('☆');
    }
    text
}

/// Lays out a title wrapped to at most two lines, ellipsizing the overflow
/// (the native equivalent of `-webkit-line-clamp: 2`).
fn draw_clamped_title(ui: &egui::Ui, rect: egui::Rect, text: &str, theme: Theme, size: f32) {
    let font = egui::FontId::proportional(size);
    let color = theme.text;
    let fits = |candidate: &str| {
        ui.painter()
            .layout(
                candidate.to_owned(),
                font.clone(),
                color,
                rect.width().max(20.0),
            )
            .rows
            .len()
            <= 2
    };

    let shown = if fits(text) {
        text.to_owned()
    } else {
        let chars: Vec<char> = text.chars().collect();
        let mut low = 0usize;
        let mut high = chars.len();
        while low < high {
            let mid = (low + high + 1).div_ceil(2);
            let prefix: String = chars[..mid].iter().collect();
            let candidate = format!("{}…", prefix.trim_end());
            if fits(&candidate) {
                low = mid;
            } else {
                high = mid - 1;
            }
        }
        let prefix: String = chars[..low].iter().collect();
        format!("{}…", prefix.trim_end())
    };

    let galley = ui
        .painter()
        .layout(shown, font, color, rect.width().max(20.0));
    ui.painter().galley(rect.min, galley, color);
}

fn wiki_cache_path(data_dir: &std::path::Path) -> PathBuf {
    data_dir.join("wiki_awards.json")
}

fn load_wiki_cache(data_dir: &std::path::Path) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(wiki_cache_path(data_dir)).ok()?;
    serde_json::from_str(&text).ok()
}

fn save_wiki_cache(data_dir: &std::path::Path, value: &serde_json::Value) -> std::io::Result<()> {
    std::fs::write(
        wiki_cache_path(data_dir),
        serde_json::to_string_pretty(value).unwrap_or_default(),
    )
}

fn has_malayalam(text: &str) -> bool {
    text.chars()
        .any(|character| ('\u{0D00}'..='\u{0D7F}').contains(&character))
}

/// Goodreads often concatenates "Malayalam | English | author" into one title.
fn split_malayalam_title(raw: &str) -> (String, Option<String>) {
    let parts: Vec<&str> = raw
        .split('|')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if parts.len() < 2 {
        return (raw.trim().to_string(), None);
    }
    let malayalam = parts.iter().find(|part| has_malayalam(part)).copied();
    let english = parts
        .iter()
        .find(|part| !has_malayalam(part))
        .copied()
        .unwrap_or(raw.trim());
    (english.to_string(), malayalam.map(str::to_string))
}

fn truncate(text: &str, max_chars: usize) -> String {
    let mut shortened: String = text.chars().take(max_chars).collect();
    if text.chars().count() > max_chars {
        shortened.push('…');
    }
    shortened
}

fn optional_text(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn today() -> jiff::civil::Date {
    let formatted = chrono::Local::now().format("%Y-%m-%d").to_string();
    formatted
        .parse()
        .unwrap_or_else(|_| jiff::civil::Date::constant(1970, 1, 1))
}

fn progress_bar(ui: &mut egui::Ui, current: i64, total: i64, theme: Theme) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 18.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 9.0f32, theme.pill_active);
    let fraction = if total > 0 {
        (current as f32 / total as f32).clamp(0.0, 1.0)
    } else {
        0.0
    };
    if fraction > 0.0 {
        ui.painter().rect_filled(
            egui::Rect::from_min_size(rect.min, egui::vec2(rect.width() * fraction, rect.height())),
            9.0f32,
            theme.accent,
        );
    }
    let label = if total > 0 {
        format!("{} / {}  ·  {:.0}%", current, total, fraction * 100.0)
    } else {
        format!("page {} of ?", current)
    };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(10.0),
        theme.text,
    );
}

fn star_rating(ui: &mut egui::Ui, value: &mut f64, theme: Theme) {
    let pointer = ui.input(|state| state.pointer.hover_pos());
    ui.horizontal(|ui| {
        for star in 1..=5 {
            let full = *value >= star as f64;
            let half = !full && *value >= star as f64 - 0.5;
            let glyph = if full || half { "★" } else { "☆" };
            let color = if full {
                theme.accent
            } else if half {
                theme.accent.gamma_multiply(0.55)
            } else {
                theme.muted
            };
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(23.0, 23.0), egui::Sense::click());
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                glyph,
                egui::FontId::proportional(19.0),
                color,
            );
            if response.hovered() {
                if let Some(position) = pointer {
                    let preview = if position.x < rect.center().x {
                        star as f64 - 0.5
                    } else {
                        star as f64
                    };
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "★",
                        egui::FontId::proportional(19.0),
                        theme.accent.gamma_multiply(0.45),
                    );
                    if response.clicked() {
                        *value = preview;
                    }
                }
            }
        }
        if ui.button("Clear").clicked() {
            *value = 0.0;
        }
        let shown = normalize_rating(*value).unwrap_or(0.0);
        ui.label(
            egui::RichText::new(format!("{shown:.1}"))
                .color(theme.muted)
                .small(),
        );
    });
}

fn markdown_preview(ui: &mut egui::Ui, text: &str, theme: Theme) {
    if text.trim().is_empty() {
        ui.label(
            egui::RichText::new("Nothing written yet.")
                .italics()
                .color(theme.muted),
        );
        return;
    }
    for line in text.lines() {
        if let Some(heading) = line.strip_prefix("### ") {
            ui.label(egui::RichText::new(heading).strong().color(theme.text));
        } else if let Some(heading) = line.strip_prefix("## ") {
            ui.label(
                egui::RichText::new(heading)
                    .strong()
                    .size(16.0)
                    .color(theme.text),
            );
        } else if let Some(heading) = line.strip_prefix("# ") {
            ui.label(
                egui::RichText::new(heading)
                    .strong()
                    .size(19.0)
                    .color(theme.accent),
            );
        } else if let Some(item) = line.strip_prefix("- ") {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("•").color(theme.accent));
                markdown_inline(ui, item, theme);
            });
        } else if let Some(quote) = line.strip_prefix("> ") {
            ui.label(
                egui::RichText::new(format!("“{quote}”"))
                    .italics()
                    .color(theme.muted),
            );
        } else {
            markdown_inline(ui, line, theme);
        }
    }
}

fn markdown_inline(ui: &mut egui::Ui, text: &str, theme: Theme) {
    ui.horizontal_wrapped(|ui| {
        let mut rest = text;
        while !rest.is_empty() {
            if let Some(stripped) = rest.strip_prefix("**") {
                if let Some(end) = stripped.find("**") {
                    ui.label(
                        egui::RichText::new(&stripped[..end])
                            .strong()
                            .color(theme.text),
                    );
                    rest = &stripped[end + 2..];
                    continue;
                }
            }
            if let Some(stripped) = rest.strip_prefix('*') {
                if let Some(end) = stripped.find('*') {
                    ui.label(
                        egui::RichText::new(&stripped[..end])
                            .italics()
                            .color(theme.text),
                    );
                    rest = &stripped[end + 1..];
                    continue;
                }
            }
            let split = rest.find("**").into_iter().chain(rest.find('*')).min();
            match split {
                Some(0) => {
                    ui.label(egui::RichText::new("*").color(theme.text));
                    rest = &rest[1..];
                }
                Some(index) => {
                    ui.label(egui::RichText::new(&rest[..index]).color(theme.text));
                    rest = &rest[index..];
                }
                None => {
                    ui.label(egui::RichText::new(rest).color(theme.text));
                    break;
                }
            }
        }
    });
}
