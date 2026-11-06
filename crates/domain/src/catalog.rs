#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionKind {
    Artist,
    Character,
    /// A saved conjunction of exact tags, not a remote tag identity.
    Query,
}

impl CollectionKind {
    pub fn category(self) -> u8 {
        match self {
            Self::Artist => 1,
            Self::Character => 4,
            Self::Query => 0,
        }
    }
}

/// Download queries support exact tags joined with spaces (AND).
/// Keep search operators out until their result validation is implemented.
pub fn normalize_download_query(input: &str) -> Option<String> {
    if input.len() > 512 || input.chars().any(char::is_control) {
        return None;
    }
    let mut terms = Vec::new();
    for term in input.split_whitespace() {
        let term = term.to_lowercase();
        if term.starts_with(['-', '~'])
            || matches!(term.as_str(), "or" | "(" | ")")
            || term
                .chars()
                .any(|c| matches!(c, '*' | '?' | '/' | '\\' | ':' | '#' | '{' | '}'))
        {
            return None;
        }
        if !terms.contains(&term) {
            terms.push(term);
        }
    }
    if terms.is_empty() || terms.len() > 16 {
        return None;
    }
    Some(terms.join(" "))
}

pub fn matches_download_query(query: &str, tags: &str) -> bool {
    !query.trim().is_empty()
        && query
            .split_whitespace()
            .all(|term| tags.split_whitespace().any(|tag| tag == term))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceTag {
    pub provider: crate::sources::SourceProvider,
    pub id: i64,
    pub name: String,
    pub category: u8,
    pub post_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionRecord {
    pub provider: crate::sources::SourceProvider,
    pub id: i64,
    pub kind: CollectionKind,
    pub name: String,
    /// Queries and discovered artists have no remote tag identity/count.
    pub source: Option<SourceTag>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddCollectionResult {
    pub record: CollectionRecord,
    pub created: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_queries_match_all_exact_tags_across_categories() {
        let query =
            normalize_download_query(" Official_Art  zenless_zone_zero official_art ").unwrap();
        assert_eq!(query, "official_art zenless_zone_zero");
        assert!(matches_download_query(
            &query,
            "1girl zenless_zone_zero official_art sample_artist"
        ));
        assert!(!matches_download_query(
            &query,
            "zenless_zone_zero official_artwork"
        ));
        assert!(!matches_download_query(&query, "official_art another_game"));
        for invalid in [
            "",
            " ",
            "x\ny",
            "rating:g",
            "-official_art",
            "x*",
            "{ x y }",
            "https://example.com",
        ] {
            assert!(normalize_download_query(invalid).is_none(), "{invalid}");
        }
    }
}
