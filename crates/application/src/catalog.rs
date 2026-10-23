use bdm_domain::catalog::{AddCollectionResult, CollectionKind, SourceTag};
use bdm_ports::{CatalogError, CollectionRepository, TagProvider};

pub fn normalize_tag(input: &str) -> Result<String, CatalogError> {
    if input.chars().any(char::is_control) {
        return Err(CatalogError::InvalidTag);
    }
    let tag = input.trim().to_lowercase();
    if tag.is_empty()
        || tag.len() > 512
        || tag.chars().any(|c| {
            c.is_whitespace() || c.is_control() || matches!(c, '*' | '?' | '/' | '\\' | ':' | '#')
        })
    {
        return Err(CatalogError::InvalidTag);
    }
    Ok(tag)
}

pub fn validate_tag(kind: CollectionKind, tag: &SourceTag) -> Result<(), CatalogError> {
    if tag.id <= 0 || tag.post_count < 0 || normalize_tag(&tag.name)? != tag.name {
        return Err(CatalogError::InvalidResponse);
    }
    if tag.category != kind.category() {
        return Err(match tag.category {
            1 => CatalogError::ArtistTag,
            4 => CatalogError::CharacterTag,
            _ => CatalogError::WrongCategory,
        });
    }
    Ok(())
}

pub fn lookup_tag(
    provider: &dyn TagProvider,
    kind: CollectionKind,
    input: &str,
) -> Result<SourceTag, CatalogError> {
    let name = normalize_tag(input)?;
    let tag = provider.lookup(&name)?.ok_or(CatalogError::NotFound)?;
    if tag.name != name {
        return Err(CatalogError::InvalidResponse);
    }
    validate_tag(kind, &tag)?;
    Ok(tag)
}

pub fn add_collection(
    repo: &mut dyn CollectionRepository,
    kind: CollectionKind,
    tag: &SourceTag,
) -> Result<AddCollectionResult, CatalogError> {
    validate_tag(kind, tag)?;
    repo.add(kind, tag)
}

pub fn load_collections(
    repo: &dyn CollectionRepository,
) -> Result<Vec<bdm_domain::catalog::CollectionRecord>, CatalogError> {
    repo.list()
}

pub fn delete_empty_collections(
    repo: &mut dyn CollectionRepository,
    kind: CollectionKind,
    ids: &[i64],
) -> Result<(), CatalogError> {
    if ids.is_empty() || ids.iter().any(|id| *id <= 0) {
        return Err(CatalogError::Changed);
    }
    repo.delete_empty(kind, ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Provider(Option<SourceTag>);
    impl TagProvider for Provider {
        fn lookup(&self, _: &str) -> Result<Option<SourceTag>, CatalogError> {
            Ok(self.0.clone())
        }
    }
    #[test]
    fn single_tag_normalization_rejects_queries_urls_and_control_characters() {
        assert_eq!(
            normalize_tag("  Character_(Series) ").unwrap(),
            "character_(series)"
        );
        for input in [
            "",
            "two tags",
            "https://danbooru.donmai.us",
            "x*",
            "x?",
            "x/y",
            "x\\y",
            "x#y",
            "x\0y",
            "x\n",
        ] {
            assert_eq!(normalize_tag(input), Err(CatalogError::InvalidTag));
        }
    }
    #[test]
    fn source_identity_and_category_are_required_before_creating_records() {
        let mut tag = SourceTag {
            id: 1,
            name: "character_(series)".into(),
            category: 4,
            post_count: 8,
        };
        assert_eq!(
            lookup_tag(
                &Provider(Some(tag.clone())),
                CollectionKind::Character,
                " Character_(Series) "
            ),
            Ok(tag.clone())
        );
        assert_eq!(
            lookup_tag(&Provider(None), CollectionKind::Character, &tag.name),
            Err(CatalogError::NotFound)
        );
        assert_eq!(
            lookup_tag(
                &Provider(Some(tag.clone())),
                CollectionKind::Artist,
                &tag.name
            ),
            Err(CatalogError::CharacterTag)
        );
        tag.category = 1;
        assert_eq!(
            validate_tag(CollectionKind::Character, &tag),
            Err(CatalogError::ArtistTag)
        );
        tag.category = 0;
        assert_eq!(
            validate_tag(CollectionKind::Character, &tag),
            Err(CatalogError::WrongCategory)
        );
        tag.category = 4;
        assert_eq!(
            lookup_tag(
                &Provider(Some(tag.clone())),
                CollectionKind::Character,
                "another"
            ),
            Err(CatalogError::InvalidResponse)
        );
        tag.id = 0;
        assert_eq!(
            validate_tag(CollectionKind::Character, &tag),
            Err(CatalogError::InvalidResponse)
        );
    }
}
