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
            || is_search_metatag(&term)
            || term
                .chars()
                .any(|c| matches!(c, '*' | '?' | '/' | '\\' | '#' | '{' | '}' | '"'))
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

fn is_search_metatag(term: &str) -> bool {
    let Some((prefix, _)) = term.split_once(':') else {
        return false;
    };
    // A colon alone does not make a metatag (reverse:1999 and re:zero are
    // literal tags). Recognize the providers' reserved search prefixes.
    // https://github.com/danbooru/danbooru/blob/master/app/logical/post_query_builder.rb
    // https://gelbooru.com/index.php?page=wiki&s=view&id=26263
    const PREFIXES: &str = "user approver commenter comm noter noteupdater artcomm
        commentaryupdater flagger appealer upvote downvote fav ordvote ordfav
        favgroup ordfavgroup reacted pool ordpool note comment commentary id
        rating source status filetype disapproved parent child search embedded
        md5 pixelhash width height mpixels ratio score upvotes downvotes favcount
        filesize date age order limit tagcount pixiv_id pixiv unaliased exif
        duration random is has ai sort cid locked newpool newfavgroup
        gentags arttags copytags chartags metatags
        comment_count deleted_comment_count active_comment_count
        note_count deleted_note_count active_note_count flag_count
        child_count deleted_child_count active_child_count
        pool_count deleted_pool_count active_pool_count series_pool_count collection_pool_count
        appeal_count approval_count replacement_count disapproval_count
        comments deleted_comments active_comments notes deleted_notes active_notes
        flags children deleted_children active_children pools deleted_pools active_pools
        series_pools collection_pools appeals approvals replacements disapprovals";
    PREFIXES.split_whitespace().any(|name| name == prefix)
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

    #[test]
    fn literal_colons_are_preserved_without_enabling_search_metatags() {
        let query = normalize_download_query(" Reverse:1999  Official_Art reverse:1999 ").unwrap();
        assert_eq!(query, "reverse:1999 official_art");
        assert!(matches_download_query(
            &query,
            "1girl official_art reverse:1999 sample_artist"
        ));
        assert!(!matches_download_query(
            &query,
            "official_art reverse:19990"
        ));
        assert!(!matches_download_query(&query, "reverse:1999"));
        assert_eq!(normalize_download_query("re:zero"), Some("re:zero".into()));
        assert_eq!(
            normalize_download_query("regulus_(reverse:1999)"),
            Some("regulus_(reverse:1999)".into())
        );
        for invalid in [
            "rating:g",
            "sort:score",
            "order:id",
            "id:123",
            "width:1000",
            "comments:1",
            "chartags:2",
            "official_art pool:2",
            "\"rating:g\"",
        ] {
            assert!(normalize_download_query(invalid).is_none(), "{invalid}");
        }
    }
}
