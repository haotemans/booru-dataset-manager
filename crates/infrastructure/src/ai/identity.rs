use bdm_domain::tags::{NormalizedTags, TagRules};
use std::collections::BTreeSet;

/// Danbooru's raw_tags.txt is provenance, not trusted model instructions.
/// Pass only locally verified general-category tags as a weak visual prior.
pub(super) fn generic_metadata_prior(
    raw: &str,
    normalized: &NormalizedTags,
    rules: &TagRules,
) -> String {
    let ordinary = normalized.ordinary.iter().collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    raw.split_whitespace()
        .map(|tag| rules.canonical(tag))
        .filter(|tag| ordinary.contains(tag) && seen.insert(tag.clone()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The model cannot designate new identity tags or reclassify unknown tags.
/// Only tags already verified as Danbooru general (category 0) are accepted.
/// Unknown candidates are omitted pending local vocabulary enrichment.
pub(super) fn verified_visual_tags(tags: &[String], rules: &TagRules) -> Vec<String> {
    let mut seen = BTreeSet::new();
    tags.iter()
        .filter_map(|tag| {
            let candidate = tag.trim().to_lowercase().replace(' ', "_");
            if candidate.is_empty()
                || candidate.contains('@')
                || candidate.contains(':')
                || candidate.chars().any(char::is_control)
            {
                return None;
            }
            let canonical = rules.canonical(&candidate);
            if rules.categories.get(&canonical) != Some(&0)
                || rules.protected.contains(&canonical)
                || !seen.insert(canonical.clone())
            {
                return None;
            }
            Some(canonical)
        })
        .collect()
}

fn tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn identity_key(name: &str) -> String {
    tokens(name).join("_")
}

/// Prevent *known* local identities from being repeated in AI prose.
/// Unknown visual identities cannot be identified reliably without an
/// authoritative identity catalogue; all captions still need review.
pub(super) fn caption_mentions_identity(
    caption: &str,
    names: &NormalizedTags,
    rules: &TagRules,
    copyright_tags: &str,
) -> bool {
    let lowercase = caption.to_lowercase();
    if ["artist:", "character:", "copyright:", "franchise:", "series:", "@"]
        .iter()
        .any(|prefix| lowercase.contains(prefix))
    {
        return true;
    }
    let mut identity = rules
        .categories
        .iter()
        .filter(|(_, category)| matches!(**category, 1 | 3 | 4))
        .map(|(name, _)| identity_key(name))
        .filter(|name| !name.is_empty())
        .collect::<BTreeSet<_>>();
    for name in names
        .artists
        .iter()
        .chain(names.characters.iter())
        .map(String::as_str)
        .chain(copyright_tags.split_whitespace())
    {
        identity.insert(identity_key(name));
    }
    for (alias, target) in &rules.aliases {
        if identity.contains(&identity_key(target)) {
            identity.insert(identity_key(alias));
        }
    }
    let words = tokens(caption);
    for start in 0..words.len() {
        let mut phrase = String::new();
        for word in words.iter().take((start + 12).min(words.len())).skip(start) {
            if !phrase.is_empty() {
                phrase.push('_');
            }
            phrase.push_str(word);
            if identity.contains(&phrase) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_danbooru_prior_excludes_artist_character_copyright_and_protected_tags() {
        let mut rules = TagRules::default();
        for (name, category) in [
            ("artist_x", 1),
            ("character_x", 4),
            ("franchise_x", 3),
            ("1girl", 0),
            ("blue_hair", 0),
            ("smile", 0),
            ("rim_lighting", 0),
            ("protected_x", 0),
        ] {
            rules.categories.insert(name.into(), category);
        }
        rules.protected.insert("protected_x".into());
        let raw = "artist_x character_x franchise_x 1girl blue_hair smile protected_x";
        let normalized = bdm_domain::tags::normalize(
            raw,
            "artist_x",
            "character_x",
            &rules,
        );
        assert_eq!(
            generic_metadata_prior(raw, &normalized, &rules),
            "1girl, blue_hair, smile"
        );
        assert_eq!(
            verified_visual_tags(
                &[
                    "blue hair".into(),
                    "rim_lighting".into(),
                    "character_x".into(),
                    "unknown_person".into(),
                    "protected_x".into(),
                    "blue_hair".into(),
                ],
                &rules
            ),
            vec!["blue_hair", "rim_lighting"]
        );
        assert!(caption_mentions_identity(
            "A portrait of Character X in soft light.",
            &normalized,
            &rules,
            "franchise_x"
        ));
        assert!(!caption_mentions_identity(
            "A girl with blue hair smiling in soft light.",
            &normalized,
            &rules,
            "franchise_x"
        ));
    }
}
