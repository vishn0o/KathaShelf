//! Online book metadata search: Google Books + Open Library + Goodreads HTML,
//! merged, deduplicated and ranked cover-first.
//!
//! All network work runs on background `std::thread`s (see `LogEdit::start_search`
//! and `BookApp` cover downloads) and is only triggered by explicit user
//! actions. Downloaded covers are cached on disk by `covers.rs`.
//!
//! Endpoints:
//!   https://www.googleapis.com/books/v1/volumes?q=...&maxResults=10
//!   https://openlibrary.org/search.json?q=...&limit=10
//!   https://www.goodreads.com/search?q=...

use reqwest::blocking::Client;
use scraper::{Html, Selector};
use serde::Deserialize;
use std::collections::HashMap;
use std::time::Duration;

pub(crate) const APP_USER_AGENT: &str = "KathaShelf/0.1 (local book tracker)";
const BROWSER_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) \
    AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);

/// Shared blocking HTTP client. Kept here so search and cover downloads use
/// the same User-Agent and timeout policy.
pub(crate) fn http_client() -> Result<Client, String> {
    Client::builder()
        .user_agent(APP_USER_AGENT)
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| format!("Could not create HTTP client: {e}"))
}

fn browser_client() -> Result<Client, String> {
    Client::builder()
        .user_agent(BROWSER_USER_AGENT)
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| format!("Could not create HTTP client: {e}"))
}

// ---------------------------------------------------------------------------
// Public, unified result type
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchSource {
    GoogleBooks,
    OpenLibrary,
    Web,
}

impl SearchSource {
    pub fn label(self) -> &'static str {
        match self {
            SearchSource::GoogleBooks => "Google Books",
            SearchSource::OpenLibrary => "Open Library",
            SearchSource::Web => "Web",
        }
    }
}

/// One normalized hit from any of the three providers.
#[derive(Debug, Clone)]
pub struct SearchHit {
    pub title: String,
    pub authors: Vec<String>,
    pub year: Option<i32>,
    pub isbn: Vec<String>,
    pub cover_id: Option<i64>,
    pub cover_url: Option<String>,
    pub publisher: Option<String>,
    pub average_rating: Option<f64>,
    pub page_count: Option<i64>,
    pub description: Option<String>,
    pub source: SearchSource,
}

impl SearchHit {
    pub fn author(&self) -> &str {
        self.authors
            .first()
            .map(String::as_str)
            .unwrap_or("Unknown")
    }

    pub fn isbn(&self) -> &str {
        self.isbn
            .iter()
            .find(|value| value.len() >= 10)
            .map(String::as_str)
            .unwrap_or("")
    }

    pub fn has_cover(&self) -> bool {
        self.cover_id.is_some() || self.cover_url.is_some()
    }

    pub fn source_label(&self) -> &'static str {
        self.source.label()
    }
}

// ---------------------------------------------------------------------------
// Provider response shapes (private)
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
struct OpenLibraryResponse {
    #[serde(default)]
    docs: Vec<OpenLibraryDoc>,
}

#[derive(Debug, Default, Deserialize)]
struct OpenLibraryDoc {
    #[serde(default)]
    title: String,
    #[serde(default, rename = "author_name")]
    authors: Vec<String>,
    #[serde(default)]
    publisher: Vec<String>,
    #[serde(default, rename = "first_publish_year")]
    year: Option<i32>,
    #[serde(default, rename = "number_of_pages_median")]
    page_count: Option<i64>,
    #[serde(default)]
    isbn: Vec<String>,
    #[serde(default, rename = "cover_i")]
    cover_id: Option<i64>,
    #[serde(default, rename = "ratings_average")]
    rating: Option<f64>,
    #[serde(default, rename = "ratings_count")]
    ratings_count: Option<i64>,
    #[serde(default, rename = "readinglog_count")]
    reading_count: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
struct GoogleBooksResponse {
    #[serde(default)]
    items: Vec<GoogleItem>,
}

#[derive(Debug, Default, Deserialize)]
struct GoogleItem {
    #[serde(default, rename = "volumeInfo")]
    volume_info: VolumeInfo,
}

#[derive(Debug, Default, Deserialize)]
struct VolumeInfo {
    #[serde(default)]
    title: String,
    #[serde(default)]
    authors: Vec<String>,
    #[serde(default, rename = "publishedDate")]
    published_date: Option<String>,
    #[serde(default, rename = "pageCount")]
    page_count: Option<i64>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default, rename = "industryIdentifiers")]
    identifiers: Vec<Identifier>,
    #[serde(default, rename = "imageLinks")]
    image_links: ImageLinks,
    #[serde(default)]
    publisher: Option<String>,
    #[serde(default, rename = "averageRating")]
    rating: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
struct Identifier {
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    identifier: String,
}

#[derive(Debug, Default, Deserialize)]
struct ImageLinks {
    #[serde(default)]
    thumbnail: Option<String>,
    #[serde(default, rename = "smallThumbnail")]
    small_thumbnail: Option<String>,
}

// ---------------------------------------------------------------------------
// Multi-source search
// ---------------------------------------------------------------------------

/// Query all three providers concurrently, then merge/dedupe/rank the results.
/// A failure of any one provider never cancels the other two.
pub fn search_books(query: &str) -> Result<Vec<SearchHit>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let encoded = urlencoding::encode(query).into_owned();

    let (google, open_library, web) = std::thread::scope(|scope| {
        let gb = scope.spawn(|| search_google_books(&encoded));
        let ol = scope.spawn(|| search_open_library(&encoded));
        let goodreads = scope.spawn(|| search_goodreads(&encoded));
        (
            gb.join()
                .unwrap_or_else(|_| Err("Google Books worker failed".into())),
            ol.join()
                .unwrap_or_else(|_| Err("Open Library worker failed".into())),
            goodreads
                .join()
                .unwrap_or_else(|_| Err("Goodreads worker failed".into())),
        )
    });

    let mut merged: Vec<SearchHit> = Vec::new();
    let mut first_error: Option<String> = None;
    for result in [google, open_library, web] {
        match result {
            Ok(mut hits) => merged.append(&mut hits),
            Err(error) => {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }

    if merged.is_empty() {
        if let Some(error) = first_error {
            return Err(error);
        }
    }
    Ok(merge_hits(merged))
}

/// Deduplicate by normalized (title + author), keep the richer/cover-bearing
/// copy, and rank entries with a usable cover first.
fn merge_hits(hits: Vec<SearchHit>) -> Vec<SearchHit> {
    let mut positions: HashMap<String, usize> = HashMap::new();
    let mut unique: Vec<SearchHit> = Vec::new();

    for hit in hits {
        let key = format!(
            "{}|{}",
            normalize_key(&hit.title),
            normalize_key(hit.author())
        );
        match positions.get(&key).copied() {
            Some(index) => {
                if hit.has_cover() && !unique[index].has_cover() {
                    let mut better = hit;
                    fill_missing(&mut better, &unique[index]);
                    unique[index] = better;
                } else {
                    fill_missing(&mut unique[index], &hit);
                }
            }
            None => {
                positions.insert(key, unique.len());
                unique.push(hit);
            }
        }
    }

    unique.sort_by_key(|hit| {
        std::cmp::Reverse((
            hit.has_cover(),
            hit.source == SearchSource::OpenLibrary,
            hit.average_rating.is_some(),
            hit.description.is_some(),
        ))
    });
    unique.truncate(30);
    unique
}

/// When two providers describe the same book, keep whichever metadata the
/// other one supplied but this one lacked.
fn fill_missing(target: &mut SearchHit, other: &SearchHit) {
    if target.year.is_none() {
        target.year = other.year;
    }
    if target.page_count.is_none() {
        target.page_count = other.page_count;
    }
    if target.description.is_none() {
        target.description = other.description.clone();
    }
    if target.publisher.is_none() {
        target.publisher = other.publisher.clone();
    }
    if target.isbn.is_empty() {
        target.isbn = other.isbn.clone();
    }
    if target.authors.is_empty() {
        target.authors = other.authors.clone();
    }
    if target.cover_id.is_none() {
        target.cover_id = other.cover_id;
    }
    if target.cover_url.is_none() {
        target.cover_url = other.cover_url.clone();
    }
    if target.average_rating.is_none() {
        target.average_rating = other.average_rating;
    }
}

fn normalize_key(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

// ---- Source 1: Google Books ------------------------------------------------

fn search_google_books(encoded: &str) -> Result<Vec<SearchHit>, String> {
    let url = format!(
        "https://www.googleapis.com/books/v1/volumes?q={encoded}&maxResults=10&printType=books"
    );
    let body = get_body(&url, "Google Books", http_client()?)?;
    let parsed: GoogleBooksResponse = serde_json::from_str(&body)
        .map_err(|e| format!("Unexpected Google Books response: {e}"))?;

    Ok(parsed
        .items
        .into_iter()
        .map(|item| item.volume_info)
        .filter(|info| !info.title.trim().is_empty())
        .map(|info| {
            let isbn = info
                .identifiers
                .iter()
                .find(|id| id.kind == "ISBN_13" || id.kind == "ISBN_10")
                .map(|id| id.identifier.clone())
                .into_iter()
                .collect();
            let year = info
                .published_date
                .as_deref()
                .and_then(|date| date.get(..4))
                .and_then(|year| year.parse::<i32>().ok());
            let cover_url = info
                .image_links
                .thumbnail
                .or(info.image_links.small_thumbnail)
                .map(|url| url.replace("http://", "https://").replace("&edge=curl", ""));
            SearchHit {
                title: info.title.trim().to_string(),
                authors: info.authors,
                year,
                isbn,
                cover_id: None,
                cover_url,
                publisher: info.publisher,
                average_rating: info.rating,
                page_count: info.page_count,
                description: info.description.and_then(|d| clean_description(&d)),
                source: SearchSource::GoogleBooks,
            }
        })
        .collect())
}

// ---- Source 2: Open Library ------------------------------------------------

fn search_open_library(encoded: &str) -> Result<Vec<SearchHit>, String> {
    let url = format!(
        "https://openlibrary.org/search.json?q={encoded}&limit=10&fields=title,author_name,publisher,first_publish_year,number_of_pages_median,isbn,cover_i,ratings_average"
    );
    let body = get_body(&url, "Open Library", http_client()?)?;
    let parsed: OpenLibraryResponse = serde_json::from_str(&body)
        .map_err(|e| format!("Unexpected Open Library response: {e}"))?;

    Ok(parsed
        .docs
        .into_iter()
        .filter(|doc| !doc.title.trim().is_empty())
        .map(|doc| {
            let cover_url = doc
                .cover_id
                .map(|id| format!("https://covers.openlibrary.org/b/id/{id}-L.jpg?default=false"));
            SearchHit {
                title: doc.title.trim().to_string(),
                authors: doc.authors,
                year: doc.year,
                isbn: doc.isbn,
                cover_id: doc.cover_id,
                cover_url,
                publisher: doc.publisher.into_iter().next(),
                average_rating: doc.rating,
                page_count: doc.page_count,
                description: None,
                source: SearchSource::OpenLibrary,
            }
        })
        .collect())
}

// ---- Source 3: Goodreads HTML ---------------------------------------------

fn search_goodreads(encoded: &str) -> Result<Vec<SearchHit>, String> {
    let url = format!("https://www.goodreads.com/search?q={encoded}");
    let body = get_body(&url, "Goodreads", browser_client()?)?;
    let document = Html::parse_document(&body);

    let primary = Selector::parse(r#"tr[itemtype="http://schema.org"]"#)
        .map_err(|e| format!("Invalid Goodreads row selector: {e}"))?;
    let fallback = Selector::parse(r#"tr[itemtype*="schema.org"]"#)
        .map_err(|e| format!("Invalid Goodreads fallback selector: {e}"))?;
    let rows: Vec<_> = document.select(&primary).collect();
    let rows = if rows.is_empty() {
        document.select(&fallback).collect::<Vec<_>>()
    } else {
        rows
    };

    let title_selector = Selector::parse("a.bookTitle")
        .map_err(|e| format!("Invalid Goodreads title selector: {e}"))?;
    let author_selector = Selector::parse("a.authorName")
        .map_err(|e| format!("Invalid Goodreads author selector: {e}"))?;
    let image_selector =
        Selector::parse("img").map_err(|e| format!("Invalid Goodreads image selector: {e}"))?;

    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let title = node_text(&row, &title_selector);
            if title.is_empty() {
                return None;
            }
            let author = node_text(&row, &author_selector);
            let cover_url = row
                .select(&image_selector)
                .filter_map(|img| {
                    img.value()
                        .attr("src")
                        .or_else(|| img.value().attr("data-src"))
                })
                .find(|src| {
                    src.contains("goodreads") || src.contains("amazon") || src.contains("books")
                })
                .map(full_size_cover);
            Some(SearchHit {
                title,
                authors: if author.is_empty() {
                    Vec::new()
                } else {
                    vec![author]
                },
                year: None,
                isbn: Vec::new(),
                cover_id: None,
                cover_url,
                publisher: None,
                average_rating: None,
                page_count: None,
                description: None,
                source: SearchSource::Web,
            })
        })
        .collect())
}

fn node_text(node: &scraper::ElementRef<'_>, selector: &Selector) -> String {
    node.select(selector)
        .next()
        .map(|element| {
            element
                .text()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// Goodreads serves `._SX50_.jpg` / `._SY75_.jpg` thumbnails. Removing that
/// suffix yields the full-resolution cover hosted by the same CDN.
fn full_size_cover(src: &str) -> String {
    let mut url = src.replace("http://", "https://");
    if let Some(suffix_start) = url.rfind("._") {
        if let Some(ext_offset) = url[suffix_start..].rfind('.') {
            let extension = url[suffix_start + ext_offset..].to_string();
            url.truncate(suffix_start);
            url.push_str(&extension);
        }
    }
    url
}

/// Strip basic HTML from provider descriptions before storing them as a review.
pub fn clean_description(html: &str) -> Option<String> {
    let fragment = Html::parse_fragment(html);
    let raw: String = fragment.root_element().text().collect();
    let collapsed: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        None
    } else {
        Some(collapsed.chars().take(1200).collect())
    }
}

fn get_body(url: &str, provider: &str, client: Client) -> Result<String, String> {
    let response = client
        .get(url)
        .send()
        .map_err(|e| format!("{provider} search failed: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("{provider} search failed (HTTP {status})"));
    }
    response
        .text()
        .map_err(|e| format!("Could not read {provider} results: {e}"))
}

// ---- Cover downloads -------------------------------------------------------

/// Download the large cover for an Open Library cover id.
pub fn fetch_cover(cover_id: i64) -> Result<Vec<u8>, String> {
    let url = format!("https://covers.openlibrary.org/b/id/{cover_id}-L.jpg?default=false");
    fetch_cover_url(&url)
}

/// Download a cover from any direct image URL (Google Books, Goodreads, pasted URL).
pub fn fetch_cover_url(url: &str) -> Result<Vec<u8>, String> {
    let client = http_client()?;
    let response = client
        .get(url)
        .send()
        .map_err(|e| format!("cover download failed: {e}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "cover download failed (HTTP {})",
            response.status()
        ));
    }
    let bytes = response
        .bytes()
        .map_err(|e| format!("could not read cover data: {e}"))?
        .to_vec();

    // Some APIs return a 1x1 "missing" image instead of an error.
    let looks_real = image::load_from_memory(&bytes)
        .map(|image| image.width() > 60 && image.height() > 60)
        .unwrap_or(false);
    if !looks_real {
        return Err("no usable image at that URL".into());
    }
    Ok(bytes)
}

/// Top books for a genre/language, from Open Library's search API.
/// Popularity is ranked by reading-log count, then rating count.
pub fn fetch_chart(genre: &str, language: &str, limit: usize) -> Result<Vec<SearchHit>, String> {
    let limit = limit.clamp(5, 25);
    let encoded_genre = urlencoding::encode(genre.trim());
    let mut url = format!(
        "https://openlibrary.org/search.json?q={encoded_genre}&limit={limit}&fields=title,author_name,publisher,first_publish_year,number_of_pages_median,isbn,cover_i,ratings_average,ratings_count,readinglog_count"
    );
    if language == "Malayalam" {
        url.push_str("&language=mal");
    } else if language == "English" {
        url.push_str("&language=eng");
    }
    let body = get_body(&url, "Open Library chart", http_client()?)?;
    let parsed: OpenLibraryResponse =
        serde_json::from_str(&body).map_err(|e| format!("Unexpected chart response: {e}"))?;
    let mut ranked: Vec<(SearchHit, i64, i64)> = parsed
        .docs
        .into_iter()
        .filter(|doc| !doc.title.trim().is_empty())
        .map(|doc| {
            let cover_url = doc
                .cover_id
                .map(|id| format!("https://covers.openlibrary.org/b/id/{id}-L.jpg?default=false"));
            let popularity = doc.reading_count.unwrap_or(0);
            let ratings = doc.ratings_count.unwrap_or(0);
            let hit = SearchHit {
                title: doc.title.trim().to_string(),
                authors: doc.authors,
                year: doc.year,
                isbn: doc.isbn,
                cover_id: doc.cover_id,
                cover_url,
                publisher: doc.publisher.into_iter().next(),
                average_rating: doc.rating,
                page_count: doc.page_count,
                description: None,
                source: SearchSource::OpenLibrary,
            };
            (hit, popularity, ratings)
        })
        .collect();
    ranked.sort_by_key(|(hit, popularity, ratings)| {
        std::cmp::Reverse((*popularity, *ratings, hit.average_rating.is_some()))
    });
    let hits: Vec<SearchHit> = ranked
        .into_iter()
        .take(limit)
        .map(|(hit, _, _)| hit)
        .collect();
    Ok(hits)
}

/// Wikipedia REST summary text for an award page (used by the monthly cache).
pub fn fetch_wikipedia_summary(page: &str) -> Result<String, String> {
    let url = format!(
        "https://en.wikipedia.org/api/rest_v1/page/summary/{}",
        urlencoding::encode(page)
    );
    let body = get_body(&url, "Wikipedia", http_client()?)?;
    let value: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("Unexpected Wikipedia response: {e}"))?;
    value
        .get("extract")
        .and_then(|v| v.as_str())
        .map(|text| text.chars().take(1800).collect())
        .ok_or_else(|| "Wikipedia page has no summary".to_string())
}

/// Fetch a small JSON cache of award descriptions, one per curated award.
pub fn fetch_wikipedia_awards() -> Result<serde_json::Value, String> {
    let mut map = serde_json::Map::new();
    for award in crate::awards::curated_awards() {
        if let Ok(text) = fetch_wikipedia_summary(award.wikipedia_page) {
            map.insert(award.name.to_string(), serde_json::Value::String(text));
        }
    }
    if map.is_empty() {
        return Err("No Wikipedia award summaries were available".into());
    }
    Ok(serde_json::Value::Object(map))
}

/// A legitimate online source where the reader can read, borrow or buy a book.
#[derive(Debug, Clone)]
pub struct DownloadLink {
    pub label: &'static str,
    pub url: String,
}

#[derive(Debug, Default, Deserialize)]
struct LinkResponse {
    #[serde(default)]
    docs: Vec<LinkDoc>,
}

#[derive(Debug, Default, Deserialize)]
struct LinkDoc {
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    ia: Vec<String>,
    #[serde(default)]
    ebook_access: Option<String>,
}

/// Find legitimate read/borrow/download links for a title.
pub fn fetch_download_links(title: &str, author: &str) -> Result<Vec<DownloadLink>, String> {
    let query = format!("{title} {author}");
    let query = query.trim();
    if query.is_empty() {
        return Err("A title is needed to find a copy.".into());
    }
    let encoded = urlencoding::encode(query);
    let url = format!(
        "https://openlibrary.org/search.json?q={encoded}&limit=3&fields=key,title,author_name,ia,ebook_access"
    );
    let body = get_body(&url, "Open Library links", http_client()?)?;
    let parsed: LinkResponse = serde_json::from_str(&body)
        .map_err(|e| format!("Unexpected Open Library link response: {e}"))?;

    let mut links = Vec::new();
    for doc in parsed.docs {
        if let Some(key) = doc.key {
            let access = doc.ebook_access.as_deref().unwrap_or("");
            let label = if access == "public" || access == "no_restriction" {
                "Open Library — read free"
            } else {
                "Open Library — borrow / read"
            };
            links.push(DownloadLink {
                label,
                url: format!("https://openlibrary.org{key}"),
            });
        }
        if let Some(archive_id) = doc.ia.first() {
            links.push(DownloadLink {
                label: "Internet Archive — borrow / download",
                url: format!("https://archive.org/details/{archive_id}"),
            });
        }
    }
    links.push(DownloadLink {
        label: "Project Gutenberg — free ebooks",
        url: format!(
            "https://www.gutenberg.org/ebooks/search/?query={}",
            urlencoding::encode(title.trim())
        ),
    });
    links.push(DownloadLink {
        label: "DOAB — open access books",
        url: format!(
            "https://www.doabooks.org/en/doab?search={}",
            urlencoding::encode(title.trim())
        ),
    });
    links.push(DownloadLink {
        label: "HathiTrust — catalog / full view",
        url: format!(
            "https://catalog.hathitrust.org/Search/Home?lookfor={}&type=all",
            urlencoding::encode(title.trim())
        ),
    });
    links.push(DownloadLink {
        label: "Google Books — preview / buy",
        url: format!(
            "https://www.google.com/search?tbm=bks&q={}",
            urlencoding::encode(query)
        ),
    });

    let mut seen = std::collections::HashSet::new();
    links.retain(|link| seen.insert(link.url.clone()));
    links.truncate(6);
    Ok(links)
}

/// Short readable summary for a book, from Wikipedia first and the book APIs
/// as a fallback. Used by the "About this book" card in the detail modal.
pub fn fetch_book_summary(title: &str, author: &str) -> Result<String, String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("A title is needed to look up a summary.".into());
    }
    let wiki_url = format!(
        "https://en.wikipedia.org/api/rest_v1/page/summary/{}",
        urlencoding::encode(title)
    );
    if let Ok(client) = http_client() {
        if let Ok(response) = client.get(&wiki_url).send() {
            if response.status().is_success() {
                if let Ok(body) = response.text() {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&body) {
                        if let Some(extract) = value.get("extract").and_then(|v| v.as_str()) {
                            let summary = clean_description(extract).unwrap_or_default();
                            if !summary.is_empty() {
                                return Ok(summary.chars().take(420).collect());
                            }
                        }
                    }
                }
            }
        }
    }
    let query = format!("{title} {author}");
    let summary = search_books(query.trim())?
        .into_iter()
        .find_map(|hit| hit.description)
        .ok_or_else(|| "No online summary found for this book.".to_string())?;
    Ok(summary.chars().take(420).collect())
}

// ---- Tests -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(title: &str, author: &str, source: SearchSource, cover: bool) -> SearchHit {
        SearchHit {
            title: title.into(),
            authors: vec![author.into()],
            year: None,
            isbn: vec![],
            cover_id: if cover { Some(42) } else { None },
            cover_url: None,
            publisher: None,
            average_rating: None,
            page_count: None,
            description: None,
            source,
        }
    }

    #[test]
    fn dedupe_key_ignores_case_and_punctuation() {
        assert_eq!(
            normalize_key("Oru Desathinte Katha!"),
            normalize_key("oru desathinte katha")
        );
    }

    #[test]
    fn google_books_json_shape_parses() {
        let json = r#"{"items":[{"volumeInfo":{"title":"Dune","authors":["Frank Herbert"],"publishedDate":"1965","pageCount":412,"description":"<b>A desert epic.</b>","industryIdentifiers":[{"type":"ISBN_13","identifier":"9780441013593"}],"imageLinks":{"thumbnail":"http://example.com/c.jpg&edge=curl"},"publisher":"Ace Books","averageRating":4.2}}]}"#;
        let parsed: GoogleBooksResponse = serde_json::from_str(json).unwrap();
        let info = &parsed.items[0].volume_info;
        assert_eq!(info.title, "Dune");
        assert_eq!(info.page_count, Some(412));
        assert_eq!(info.description.as_deref(), Some("<b>A desert epic.</b>"));
        assert_eq!(info.identifiers[0].kind, "ISBN_13");
        assert!(info.image_links.thumbnail.is_some());
    }

    #[test]
    fn open_library_json_shape_parses() {
        let json = r#"{"docs":[{"title":"Khasakkinte Ithihasam","author_name":["O. V. Vijayan"],"publisher":["DC Books"],"first_publish_year":1969,"number_of_pages_median":296,"isbn":["9788171304138"],"cover_i":12345}]}"#;
        let parsed: OpenLibraryResponse = serde_json::from_str(json).unwrap();
        let doc = &parsed.docs[0];
        assert_eq!(doc.publisher.first().map(String::as_str), Some("DC Books"));
        assert_eq!(doc.page_count, Some(296));
        assert_eq!(doc.cover_id, Some(12345));
    }

    #[test]
    fn goodreads_html_parses_title_author_and_full_size_cover() {
        let html = r#"<html><body><table><tr itemtype="http://schema.org">
            <td><img src="https://images-na.ssl-images-amazon.com/images/books/42._SX50_.jpg"/></td>
            <td><a class="bookTitle" href="/book/show/1"><span>Oru Desathinte Katha</span></a>
                <a class="authorName" href="/author/show/2"><span>S. K. Pottekkatt</span></a></td>
        </tr></table></body></html>"#;
        let document = Html::parse_document(html);
        let row_selector = Selector::parse(r#"tr[itemtype="http://schema.org"]"#).unwrap();
        let title_selector = Selector::parse("a.bookTitle").unwrap();
        let author_selector = Selector::parse("a.authorName").unwrap();
        let image_selector = Selector::parse("img").unwrap();
        let row = document.select(&row_selector).next().unwrap();
        assert_eq!(node_text(&row, &title_selector), "Oru Desathinte Katha");
        assert_eq!(node_text(&row, &author_selector), "S. K. Pottekkatt");
        let src = row
            .select(&image_selector)
            .next()
            .unwrap()
            .value()
            .attr("src")
            .unwrap();
        assert_eq!(
            full_size_cover(src),
            "https://images-na.ssl-images-amazon.com/images/books/42.jpg"
        );
    }

    #[test]
    fn merge_prefers_covers_and_removes_duplicates() {
        let bare = hit("Dune", "Frank Herbert", SearchSource::OpenLibrary, false);
        let with_cover = hit("Dune!", "frank herbert", SearchSource::GoogleBooks, true);
        let other = hit(
            "Dune Messiah",
            "Frank Herbert",
            SearchSource::OpenLibrary,
            false,
        );

        let merged = merge_hits(vec![bare, other, with_cover]);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].title, "Dune!");
        assert!(merged[0].has_cover());
        assert_eq!(merged[0].source_label(), "Google Books");
        assert_eq!(merged[1].title, "Dune Messiah");
    }

    #[test]
    fn description_html_is_flattened() {
        assert_eq!(
            clean_description("<p>Hello <b>world</b>.<br/> Again.</p>").as_deref(),
            Some("Hello world. Again.")
        );
    }

    #[test]
    #[ignore = "requires internet access"]
    fn live_provider_errors() {
        println!("google: {:?}", search_google_books("Dune").err());
        println!("openlibrary: {:?}", search_open_library("Dune").err());
        println!(
            "goodreads hits: {}",
            search_goodreads("Dune").map(|h| h.len()).unwrap_or(0)
        );
    }

    /// Manual smoke test: `cargo test live_api_sources_search -- --ignored --nocapture`
    #[test]
    #[ignore = "requires internet access"]
    fn live_api_sources_search() {
        let hits = search_books("Dune").expect("live search failed");
        println!("{} results", hits.len());
        for hit in hits.iter().take(5) {
            println!(
                "{} | {} | cover={} | {}",
                hit.title,
                hit.author(),
                hit.has_cover(),
                hit.source_label()
            );
        }
        assert!(!hits.is_empty());
    }

    #[test]
    #[ignore = "requires internet access"]
    fn live_download_links() {
        let links = fetch_download_links("Khasakkinte Ithihasam", "O. V. Vijayan")
            .expect("link fetch failed");
        println!("{} links", links.len());
        for link in &links {
            println!("{} -> {}", link.label, link.url);
        }
        assert!(!links.is_empty());
    }

    #[test]
    #[ignore = "requires internet access"]
    fn live_chart_and_wikipedia() {
        let hits =
            fetch_chart("Malayalam literature", "Malayalam", 10).expect("chart fetch failed");
        println!("chart: {} results", hits.len());
        for hit in hits.iter().take(3) {
            println!("{} | {}", hit.title, hit.author());
        }
        assert!(!hits.is_empty());
        let wiki = fetch_wikipedia_awards().expect("Wikipedia fetch failed");
        println!(
            "award summaries: {}",
            wiki.as_object().map(|m| m.len()).unwrap_or(0)
        );
    }

    /// Manual smoke test: `cargo test live_multi_source_search -- --ignored --nocapture`
    #[test]
    #[ignore = "requires internet access"]
    fn live_multi_source_search() {
        let hits = search_books("Khasakkinte Ithihasam").expect("live search failed");
        println!("{} results", hits.len());
        for hit in hits.iter().take(5) {
            println!(
                "{} | {} | cover={} | {}",
                hit.title,
                hit.author(),
                hit.has_cover(),
                hit.source_label()
            );
        }
        assert!(!hits.is_empty());
    }
}
