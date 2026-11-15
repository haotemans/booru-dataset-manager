use bdm_domain::tags::{NormalizedTags, TagRules};
use std::collections::BTreeSet;

const FORBIDDEN_QUALITY_TAGS: &[&str] = &[
    "masterpiece",
    "best_quality",
    "high_quality",
    "absurdres",
    "highres",
    "lowres",
    "aesthetic_score",
    "incredibly_absurdres",
    "commentary",
    "commentary_request",
    "chinese_commentary",
    "japanese_commentary",
    "english_commentary",
    "copyright_request",
    "artist_request",
    "character_request",
    "source_request",
    "translation_request",
    "tagme",
];

/// Danbooru's raw_tags.txt is provenance, not trusted model instructions.
/// Pass only locally verified general-category tags as a weak visual prior,
/// with underscores converted to spaces so V4 prompts/outputs stay space-only.
pub(super) fn generic_metadata_prior(
    raw: &str,
    normalized: &NormalizedTags,
    rules: &TagRules,
) -> String {
    let ordinary = normalized.ordinary.iter().collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    raw.split_whitespace()
        .map(|tag| rules.canonical(tag))
        .filter(|tag| {
            ordinary.contains(tag)
                && !FORBIDDEN_QUALITY_TAGS.contains(&tag.as_str())
                && seen.insert(tag.clone())
        })
        .map(|tag| tag.replace('_', " "))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The model cannot designate new identity tags or reclassify unknown tags.
/// Only tags already verified as Danbooru general (category 0) are accepted,
/// and all returned training/display tags use spaces instead of underscores.
pub(crate) fn verified_visual_tags(tags: &[String], rules: &TagRules) -> Vec<String> {
    let mut seen = BTreeSet::new();
    tags.iter()
        .filter_map(|tag| check_visual_tag(tag, rules).ok())
        .filter(|tag| seen.insert(tag.clone()))
        .collect()
}

/// One deterministic check shared by validation, vocabulary hints and rejection reports.
pub(crate) fn check_visual_tag(tag: &str, rules: &TagRules) -> Result<String, &'static str> {
    let candidate = tag
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
        .replace(' ', "_");
    if candidate.is_empty()
        || tag.chars().any(char::is_control)
        || candidate.contains('@')
        || candidate.contains(':')
        || candidate.contains(',')
    {
        return Err("invalid_syntax");
    }
    let mut canonical = rules.canonical(&candidate);
    // Protocol enum spellings are not Danbooru tag spellings. Resolve only
    // when the actual hyphenated term is already locally verified.
    if matches!(canonical.as_str(), "x_ray" | "cross_section") {
        let hyphenated = canonical.replace('_', "-");
        if !rules.categories.contains_key(&canonical)
            && rules.categories.get(&hyphenated) == Some(&0)
        {
            canonical = hyphenated;
        }
    }
    if FORBIDDEN_QUALITY_TAGS.contains(&candidate.as_str())
        || FORBIDDEN_QUALITY_TAGS.contains(&canonical.as_str())
    {
        return Err("non_visual_metadata");
    }
    match rules.categories.get(&canonical) {
        Some(0) if !rules.protected.contains(&canonical) => Ok(canonical.replace('_', " ")),
        Some(0) => Err("protected_tag"),
        Some(1 | 3 | 4) => Err("identity_merged_locally"),
        Some(_) => Err("not_general_category"),
        None => Err("unknown_category"),
    }
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
pub(crate) fn caption_mentions_identity(
    caption: &str,
    names: &NormalizedTags,
    rules: &TagRules,
    copyright_tags: &str,
) -> bool {
    let lowercase = caption.to_lowercase();
    if [
        "artist:",
        "character:",
        "copyright:",
        "franchise:",
        "series:",
        "@",
    ]
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
    fn special_tag_spellings_require_verified_hyphenated_categories() {
        let mut rules = TagRules::default();
        assert!(check_visual_tag("x ray", &rules).is_err());
        rules.categories.insert("x-ray".into(), 0);
        rules.categories.insert("cross-section".into(), 0);
        assert_eq!(check_visual_tag("x_ray", &rules), Ok("x-ray".into()));
        assert_eq!(
            check_visual_tag("cross section", &rules),
            Ok("cross-section".into())
        );
        rules.categories.insert("cross-section".into(), 4);
        assert!(check_visual_tag("cross section", &rules).is_err());
    }

    #[test]
    fn raw_danbooru_prior_excludes_artist_character_copyright_and_protected_tags_and_uses_spaces() {
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
            ("masterpiece", 0),
        ] {
            rules.categories.insert(name.into(), category);
        }
        rules.protected.insert("protected_x".into());
        let raw = "artist_x character_x franchise_x 1girl blue_hair smile protected_x masterpiece";
        let normalized = bdm_domain::tags::normalize(raw, "artist_x", "character_x", &rules);
        assert_eq!(
            generic_metadata_prior(raw, &normalized, &rules),
            "1girl, blue hair, smile"
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
                    "masterpiece".into(),
                ],
                &rules
            ),
            vec!["blue hair", "rim lighting"]
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
