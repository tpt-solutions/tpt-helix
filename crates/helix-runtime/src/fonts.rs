//! Task: Integrate `fontdb` for system font enumeration.
//!
//! `cosmic-text`'s [`crate::text`] module consumes a `fontdb::Database`
//! directly (via `cosmic_text::FontSystem::new_with_locale_and_db`), so this
//! module's job is just enumeration/lookup, not shaping.

use fontdb::{Database, Family, Query, Source, Style, Weight, ID};

/// Loads every font `fontdb` can find on the host system (platform font
/// directories) into a fresh [`Database`].
pub fn load_system_fonts() -> Database {
    let mut db = Database::new();
    db.load_system_fonts();
    db
}

/// Looks up the best matching face for `family` (e.g. `"sans-serif"` or a
/// specific family name), returning its [`ID`] if the database has one.
pub fn find_family(db: &Database, family: &str) -> Option<ID> {
    let family = match family {
        "serif" => Family::Serif,
        "sans-serif" => Family::SansSerif,
        "monospace" => Family::Monospace,
        "cursive" => Family::Cursive,
        "fantasy" => Family::Fantasy,
        other => Family::Name(other),
    };
    db.query(&Query {
        families: &[family],
        weight: Weight::NORMAL,
        style: Style::Normal,
        ..Query::default()
    })
}

/// Returns the family names for every face fontdb has enumerated, in
/// enumeration order (may contain duplicates across weights/styles of the
/// same family).
pub fn family_names(db: &Database) -> Vec<String> {
    db.faces()
        .filter_map(|face| face.families.first().map(|(name, _)| name.clone()))
        .collect()
}

/// Returns whether `id` refers to a face backed by an on-disk file, as
/// opposed to embedded/in-memory font data (relevant for cache invalidation
/// when a system font file changes on disk).
pub fn is_file_backed(db: &Database, id: ID) -> bool {
    matches!(db.face(id).map(|f| &f.source), Some(Source::File(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_at_least_one_system_font() {
        let db = load_system_fonts();
        assert!(db.faces().next().is_some(), "expected at least one system font to be found");
    }

    #[test]
    fn generic_family_query_resolves_to_some_face() {
        let db = load_system_fonts();
        assert!(find_family(&db, "sans-serif").is_some());
    }
}
