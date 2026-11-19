//! Downloading / importing cover images into the app's local `covers/` folder.
//! Once cached, covers are loaded from disk, so the app works fully offline.

use crate::db::Database;
use image::{DynamicImage, Rgba, RgbaImage};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

fn safe_stem(input: &str) -> String {
    let stem: String = input
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if stem.is_empty() {
        "cover".to_string()
    } else {
        stem
    }
}

/// Download a cover from the Open Library covers API and cache it locally.
/// Returns the relative path to store in `books.cover_path`.
pub fn download_cover(db: &Database, book_id: i64, isbn: &str) -> Result<String, Box<dyn Error>> {
    let isbn_clean: String = isbn.chars().filter(|c| c.is_ascii_alphanumeric()).collect();

    if isbn_clean.len() < 8 {
        return Err("Enter a valid ISBN (at least 8 digits) before downloading.".into());
    }

    let url = format!("https://covers.openlibrary.org/b/isbn/{isbn_clean}-L.jpg?default=false");
    let client = crate::openlibrary::http_client()?;
    let response = client
        .get(&url)
        .send()
        .map_err(|e| format!("cover download failed: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("cover download failed (HTTP {})", response.status()).into());
    }
    let bytes = response
        .bytes()
        .map_err(|e| format!("could not read cover data: {e}"))?
        .to_vec();

    let image = image::load_from_memory(&bytes)
        .map_err(|e| format!("downloaded data is not a valid image: {e}"))?;

    store_cover(db, book_id, &isbn_clean, image)
}

/// Copy a user-selected local image into the cache folder.
pub fn import_local_cover(
    db: &Database,
    book_id: i64,
    source: &Path,
) -> Result<String, Box<dyn Error>> {
    let image = image::open(source).map_err(|e| format!("could not open image: {e}"))?;
    store_cover(db, book_id, "local", image)
}

/// Cache a cover that was already downloaded as raw bytes.
pub fn save_cover_bytes(
    db: &Database,
    book_id: i64,
    tag: &str,
    bytes: &[u8],
) -> Result<String, Box<dyn Error>> {
    let image = image::load_from_memory(bytes)
        .map_err(|e| format!("downloaded data is not a valid image: {e}"))?;
    store_cover(db, book_id, tag, image)
}

/// Cache an RGBA image pasted from the system clipboard.
pub fn save_rgba_bytes(
    db: &Database,
    book_id: i64,
    tag: &str,
    width: u32,
    height: u32,
    bytes: &[u8],
) -> Result<String, Box<dyn Error>> {
    let image = RgbaImage::from_raw(width, height, bytes.to_vec())
        .ok_or("clipboard image has an unexpected size")?;
    store_cover(db, book_id, tag, DynamicImage::ImageRgba8(image))
}

/// Create and cache a clean 2:3 placeholder when a book has no cover online.
pub fn save_placeholder(db: &Database, book_id: i64) -> Result<String, Box<dyn Error>> {
    store_cover(db, book_id, "placeholder", placeholder_image())
}

fn placeholder_image() -> DynamicImage {
    const W: u32 = 300;
    const H: u32 = 450;
    let mut img = RgbaImage::new(W, H);

    // Vertical gradient: card grey -> app background.
    for y in 0..H {
        let t = y as f32 / H as f32;
        let r = lerp(44.0, 20.0, t);
        let g = lerp(52.0, 24.0, t);
        let b = lerp(64.0, 28.0, t);
        for x in 0..W {
            img.put_pixel(x, y, Rgba([r, g, b, 255]));
        }
    }

    let border = Rgba([58, 70, 86, 255]);
    fill_rect(&mut img, 8, 8, W - 8, 12, border);
    fill_rect(&mut img, 8, H - 12, W - 8, H - 8, border);
    fill_rect(&mut img, 8, 8, 12, H - 8, border);
    fill_rect(&mut img, W - 12, 8, W - 8, H - 8, border);

    // Simple open-book motif.
    let pages = Rgba([76, 90, 108, 255]);
    let rule = Rgba([36, 44, 56, 255]);
    fill_rect(&mut img, 88, 168, 146, 282, pages);
    fill_rect(&mut img, 154, 168, 212, 282, pages);
    fill_rect(&mut img, 148, 160, 152, 290, Rgba([0, 224, 84, 255]));
    for y in [186, 202, 218, 234, 250] {
        fill_rect(&mut img, 96, y, 138, y + 2, rule);
        fill_rect(&mut img, 162, y, 204, y + 2, rule);
    }

    // Three "title" lines near the bottom.
    for (i, half_width) in [62u32, 46, 30].iter().enumerate() {
        let y = 344 + i as u32 * 16;
        fill_rect(
            &mut img,
            W / 2 - half_width,
            y,
            W / 2 + half_width,
            y + 4,
            rule,
        );
    }

    DynamicImage::ImageRgba8(img)
}

fn lerp(a: f32, b: f32, t: f32) -> u8 {
    (a + (b - a) * t).round() as u8
}

fn fill_rect(img: &mut RgbaImage, x0: u32, y0: u32, x1: u32, y1: u32, color: Rgba<u8>) {
    for y in y0..y1 {
        for x in x0..x1 {
            img.put_pixel(x, y, color);
        }
    }
}

fn store_cover(
    db: &Database,
    book_id: i64,
    tag: &str,
    image: image::DynamicImage,
) -> Result<String, Box<dyn Error>> {
    let filename = format!("{book_id}_{}.jpg", safe_stem(tag));
    let absolute: PathBuf = db.covers_dir.join(&filename);

    // Bound the on-disk cover as well: 600x900 is ample for every UI surface.
    let image = if image.width() > 600 || image.height() > 900 {
        image.thumbnail(600, 900)
    } else {
        image
    };
    // Re-encode as JPEG so every cached cover has a predictable, valid format.
    image.save(&absolute)?;

    Ok(format!("covers/{filename}"))
}

/// Delete a cached cover file. Only ever touches files directly inside `covers/`.
pub fn remove_cover(db: &Database, relative: &str) {
    let path = Path::new(relative);
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        let target = db.covers_dir.join(name);
        if target.is_file() {
            let _ = fs::remove_file(target);
        }
    }
}
