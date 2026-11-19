//! Playful reader identity, badges and profile statistics derived from the
//! local library. No network calls here — everything comes from `books`.

use crate::models::{Book, Profile};

pub struct Archetype {
    pub emoji: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub evidence: String,
}

pub struct Badge {
    pub emoji: &'static str,
    pub label: String,
}

pub fn has_malayalam(text: &str) -> bool {
    text.chars().any(|c| ('\u{0D00}'..='\u{0D7F}').contains(&c))
}

fn tags(book: &Book) -> String {
    book.tags.to_lowercase()
}

pub fn archetype(books: &[Book], profile: &Profile) -> Archetype {
    let total = books.len().max(1);
    let malayalam = books
        .iter()
        .filter(|book| {
            has_malayalam(&book.title)
                || book
                    .malayalam_title
                    .as_deref()
                    .map(has_malayalam)
                    .unwrap_or(false)
                || tags(book).contains("malayalam")
        })
        .count();
    let with_cover = books
        .iter()
        .filter(|book| book.cover_path.is_some())
        .count();
    let half_stars = books
        .iter()
        .filter(|book| book.rating.map(|r| r.fract() == 0.5).unwrap_or(false))
        .count();
    let read = books
        .iter()
        .filter(|book| book.status == crate::models::ReadStatus::Read)
        .count();
    let queued = books
        .iter()
        .filter(|book| book.status == crate::models::ReadStatus::WantToRead)
        .count();
    let notes = books
        .iter()
        .filter(|book| !book.bookmark_note.trim().is_empty())
        .count();
    let pages: i64 = books
        .iter()
        .filter(|book| book.status == crate::models::ReadStatus::Read)
        .map(|book| book.total_pages)
        .sum();
    let all_tags: String = books.iter().map(tags).collect::<Vec<_>>().join(",");

    if malayalam * 100 / total >= 35 || profile.preferred_language == "Malayalam" {
        Archetype {
            emoji: "◆",
            name: "Malayalam Monsoon Reader",
            description: "You keep Malayalam stories close, whatever the weather.",
            evidence: format!("{malayalam} of {} books are Malayalam", books.len()),
        }
    } else if all_tags.contains("mystery") || all_tags.contains("crime") {
        Archetype {
            emoji: "◇",
            name: "Detective of Drawers",
            description: "You read books the way a detective reads a crime scene.",
            evidence: "Mystery / crime tags in your shelf".into(),
        }
    } else if all_tags.contains("poetry") {
        Archetype {
            emoji: "♪",
            name: "Verse Wanderer",
            description: "You like your pages short and your feelings long.",
            evidence: "Poetry tags in your shelf".into(),
        }
    } else if with_cover * 100 / total >= 85 {
        Archetype {
            emoji: "▣",
            name: "Cover Collector",
            description: "A tidy shelf is a happy shelf — and yours has faces.",
            evidence: format!("{with_cover} of {} books have covers", books.len()),
        }
    } else if half_stars >= 3 {
        Archetype {
            emoji: "★",
            name: "Half-Star Mystic",
            description: "Five stars is too easy; you live in the in-between.",
            evidence: format!("{half_stars} half-star ratings"),
        }
    } else if notes >= 3 {
        Archetype {
            emoji: "▪",
            name: "Bookmark Whisperer",
            description: "You leave a note on every page that speaks to you.",
            evidence: format!("{notes} bookmark notes"),
        }
    } else if pages >= 2000 {
        Archetype {
            emoji: "◆",
            name: "Page-Count Gladiator",
            description: "You look a 700-page novel in the eye and blink last.",
            evidence: format!("{pages} pages finished"),
        }
    } else if read * 100 / total >= 50 {
        Archetype {
            emoji: "✓",
            name: "Finish-Line Sprinter",
            description: "You don't start books — you complete them.",
            evidence: format!("{read} finished books"),
        }
    } else if queued * 100 / total >= 60 {
        Archetype {
            emoji: "◈",
            name: "Perpetual Beginner",
            description: "Your reading pile grows faster than your free time.",
            evidence: format!("{queued} books waiting"),
        }
    } else {
        Archetype {
            emoji: "▤",
            name: "Open-Shelf Explorer",
            description: "Every genre is a door and you keep opening them.",
            evidence: format!("{} books and counting", books.len()),
        }
    }
}

pub fn badges(books: &[Book]) -> Vec<Badge> {
    let mut badges = Vec::new();
    let malayalam = books
        .iter()
        .filter(|book| has_malayalam(&book.title))
        .count();
    if malayalam > 0 {
        badges.push(Badge {
            emoji: "◆",
            label: format!("{malayalam} Malayalam titles"),
        });
    }
    let favorites = books.iter().filter(|book| book.is_favorite).count();
    if favorites > 0 {
        badges.push(Badge {
            emoji: "♥",
            label: format!("{favorites} favorites"),
        });
    }
    let pages: i64 = books.iter().map(|book| book.total_pages).sum();
    if pages > 0 {
        badges.push(Badge {
            emoji: "▤",
            label: format!("{pages} pages tracked"),
        });
    }
    let notes = books
        .iter()
        .filter(|book| !book.bookmark_note.trim().is_empty())
        .count();
    if notes > 0 {
        badges.push(Badge {
            emoji: "▪",
            label: format!("{notes} bookmarks"),
        });
    }
    if badges.is_empty() {
        badges.push(Badge {
            emoji: "◈",
            label: "First chapter started".into(),
        });
    }
    badges
}

/// Query string used by the "find more like this" action.
pub fn recommendation_seed(books: &[Book], profile: &Profile) -> String {
    let mut ranked = books.to_vec();
    ranked.sort_by(|a, b| {
        b.rating
            .unwrap_or(0.0)
            .partial_cmp(&a.rating.unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    if let Some(top) = ranked.first() {
        if !top.author.trim().is_empty() {
            return top.author.clone();
        }
    }
    if !profile.favorite_publisher.trim().is_empty() {
        return profile.favorite_publisher.clone();
    }
    "Literary fiction".into()
}
