use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CharacterRelation {
    pub base: String,
    pub variant: String,
    pub verified: bool,
    /// Retained for saved-rule compatibility. Tag co-occurrence cannot establish
    /// a conflict: an image can depict several characters or character variants.
    #[serde(default)]
    pub exclusive_group: String,
    pub evidence: String,
}

pub const DANBOORU_CHARACTER_IMPLICATION: &str = "danbooru-active-character-implication-v1";
const LEGACY_CHARACTER_IMPLICATION: &str =
    "Danbooru active implication; semantic hierarchy awaits confirmation";

impl CharacterRelation {
    pub fn source_verified(&self) -> bool {
        self.verified && self.evidence == DANBOORU_CHARACTER_IMPLICATION
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct TagRules {
    pub aliases: BTreeMap<String, String>,
    pub categories: BTreeMap<String, u8>,
    pub characters: Vec<CharacterRelation>,
    pub protected: BTreeSet<String>,
    pub rejected_characters: BTreeSet<(String, String)>,
    pub resolved_aliases: BTreeSet<String>,
    pub discovered_characters: BTreeSet<String>,
}

impl TagRules {
    /// Called only with an active, exact-source implication. Character category checks
    /// and graph validation are required; implication does not establish exclusivity.
    pub fn accept_character_implication(&mut self, base: &str, variant: &str) -> bool {
        let base = self.canonical(base);
        let variant = self.canonical(variant);
        if base == variant
            || self.categories.get(&base) != Some(&4)
            || self.categories.get(&variant) != Some(&4)
            || self
                .rejected_characters
                .iter()
                .any(|(b, v)| self.canonical(b) == base && self.canonical(v) == variant)
        {
            return false;
        }
        let mut candidate = self.clone();
        if let Some(relation) = candidate
            .characters
            .iter_mut()
            .find(|r| self.canonical(&r.base) == base && self.canonical(&r.variant) == variant)
        {
            // Preserve manual decisions and manually assigned groups.
            if relation.verified {
                return false;
            }
            relation.verified = true;
            relation.evidence = DANBOORU_CHARACTER_IMPLICATION.into();
        } else {
            candidate.characters.push(CharacterRelation {
                base: base.clone(),
                variant: variant.clone(),
                verified: true,
                exclusive_group: String::new(),
                evidence: DANBOORU_CHARACTER_IMPLICATION.into(),
            });
        }
        if candidate.validate_characters().is_err() {
            if !self
                .characters
                .iter()
                .any(|r| self.canonical(&r.base) == base && self.canonical(&r.variant) == variant)
            {
                self.characters.push(CharacterRelation {
                    base,
                    variant,
                    verified: false,
                    exclusive_group: String::new(),
                    evidence: DANBOORU_CHARACTER_IMPLICATION.into(),
                });
            }
            return false;
        }
        self.characters = candidate.characters;
        true
    }

    /// Upgrade only the provenance written by the previous source-discovery code.
    /// Arbitrary inferred or manually entered candidates are never auto-approved.
    pub fn process_discovered_characters(&mut self) -> usize {
        let candidates: Vec<_> = self
            .characters
            .iter()
            .filter(|r| {
                !r.verified
                    && matches!(
                        r.evidence.as_str(),
                        DANBOORU_CHARACTER_IMPLICATION | LEGACY_CHARACTER_IMPLICATION
                    )
            })
            .map(|r| (r.base.clone(), r.variant.clone()))
            .collect();
        candidates
            .into_iter()
            .filter(|(base, variant)| self.accept_character_implication(base, variant))
            .count()
    }

    pub fn canonical(&self, name: &str) -> String {
        let original = name.to_owned();
        let mut name = name.to_owned();
        let mut seen = BTreeSet::new();
        while let Some(next) = self.aliases.get(&name) {
            if !seen.insert(name.clone()) {
                // An ambiguous alias cycle must not invent a canonical identity.
                return original;
            }
            name = next.clone();
        }
        name
    }

    fn redundant_character_bases<'a>(
        &self,
        names: impl IntoIterator<Item = &'a str>,
    ) -> BTreeSet<String> {
        if self.validate_characters().is_err() {
            return BTreeSet::new();
        }
        let present: BTreeSet<_> = names.into_iter().map(|name| self.canonical(name)).collect();
        self.characters
            .iter()
            .filter(|r| r.verified && present.contains(&self.canonical(&r.variant)))
            .map(|r| self.canonical(&r.base))
            .collect()
    }

    pub fn validate_characters(&self) -> Result<(), &'static str> {
        let mut edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for relation in self.characters.iter().filter(|r| r.verified) {
            let base = self.canonical(&relation.base);
            let variant = self.canonical(&relation.variant);
            if base == variant || base.is_empty() || variant.is_empty() {
                return Err("本体和变体必须是不同的角色 Tag。");
            }
            if !edges.entry(base).or_default().insert(variant) {
                return Err("角色关系已存在，请修改已有关系。");
            }
        }
        for start in edges.keys() {
            let mut pending = edges.get(start).cloned().unwrap_or_default();
            let mut seen = BTreeSet::new();
            while let Some(next) = pending.pop_first() {
                if &next == start {
                    return Err("角色分级存在循环，请检查本体和变体的方向。");
                }
                if seen.insert(next.clone()) {
                    pending.extend(edges.get(&next).into_iter().flatten().cloned());
                }
            }
        }
        Ok(())
    }
}

/// Apply confirmed character hierarchy to a view or export without rewriting its
/// source artifact. A base survives whenever its specific variant is absent.
pub fn filter_character_tags(content: &str, comma_separated: bool, rules: &TagRules) -> String {
    let tags: Vec<_> = if comma_separated {
        content.split(',').map(str::trim).collect()
    } else {
        content.split_whitespace().collect()
    };
    let name = |tag: &str| {
        if tag.starts_with("artist:") || tag.starts_with("@artist ") {
            return String::new();
        }
        let tag = tag.strip_prefix("character:").unwrap_or(tag);
        rules.canonical(&tag.trim_start_matches('@').replace(' ', "_"))
    };
    let names: Vec<_> = tags.iter().map(|tag| name(tag)).collect();
    let redundant = rules.redundant_character_bases(names.iter().map(String::as_str));
    let retained: Vec<_> = tags
        .iter()
        .zip(&names)
        .filter(|(_, name)| !redundant.contains(*name))
        .map(|(tag, _)| *tag)
        .collect();
    if retained.len() == tags.len() {
        content.to_owned()
    } else {
        retained.join(if comma_separated { ", " } else { " " })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NormalizedTags {
    pub tags: Vec<String>,
    pub artists: Vec<String>,
    pub characters: Vec<String>,
    pub ordinary: Vec<String>,
    pub protected: Vec<String>,
    pub warnings: Vec<String>,
}

pub fn normalize(raw: &str, artists: &str, characters: &str, rules: &TagRules) -> NormalizedTags {
    let canonical = |name: &str| rules.canonical(name);
    let artist_names: BTreeSet<_> = artists.split_whitespace().map(canonical).collect();
    let character_names: BTreeSet<_> = characters.split_whitespace().map(canonical).collect();
    let protected_names: BTreeSet<_> = rules.protected.iter().map(|name| canonical(name)).collect();
    let mut tags: Vec<_> = raw.split_whitespace().map(canonical).collect();
    let mut seen = BTreeSet::new();
    tags.retain(|tag| seen.insert(tag.clone()));
    let mut result = NormalizedTags::default();
    if rules.validate_characters().is_ok() {
        let redundant = rules.redundant_character_bases(tags.iter().map(String::as_str));
        tags.retain(|tag| !redundant.contains(tag));
    } else {
        result
            .warnings
            .push("角色分级无效，已保留所有角色标签供审核。".into());
    }
    // Every specific character tag survives. Co-occurrence alone does not tell us
    // whether tags describe one depiction or several depictions in the same image.
    for tag in tags {
        if artist_names.contains(&tag) || rules.categories.get(&tag) == Some(&1) {
            result.artists.push(tag.clone());
            result.tags.push(format!("@artist {tag}"));
        } else if character_names.contains(&tag) || rules.categories.get(&tag) == Some(&4) {
            result.characters.push(tag.clone());
            result.tags.push(tag);
        } else {
            if rules.categories.get(&tag) == Some(&0) && !protected_names.contains(&tag) {
                result.ordinary.push(tag.clone());
            } else {
                result.protected.push(tag.clone());
                if !rules.categories.contains_key(&tag) {
                    result.warnings.push(format!("未解析标签: {tag}"));
                }
            }
            result.tags.push(tag);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn character_filter_keeps_a_lone_base_and_only_removes_confirmed_redundancy() {
        let mut rules = TagRules::default();
        rules.characters.push(CharacterRelation {
            base: "phoenix_reffaella".into(),
            variant: "phoenix_reffaella_(the_illness_called_life)".into(),
            verified: true,
            exclusive_group: String::new(),
            evidence: "human-confirmed".into(),
        });
        let base = "phoenix_reffaella";
        let variant = "phoenix_reffaella_(the_illness_called_life)";
        assert_eq!(filter_character_tags(base, false, &rules), base);
        assert_eq!(filter_character_tags(variant, false, &rules), variant);
        let raw = format!("solo {base} {variant} another_character");
        assert_eq!(
            filter_character_tags(&raw, false, &rules),
            format!("solo {variant} another_character")
        );
        for prefix in ["", "character:", "@"] {
            let text = format!("{prefix}{base}, {prefix}{variant}, other_variant");
            assert_eq!(
                filter_character_tags(&text, true, &rules),
                format!("{prefix}{variant}, other_variant")
            );
        }
        rules.aliases.insert("old_name".into(), base.into());
        assert_eq!(
            filter_character_tags(&format!("old_name {variant}"), false, &rules),
            variant
        );
        rules.characters[0].verified = false;
        assert_eq!(filter_character_tags(&raw, false, &rules), raw);
        // A matching prefix alone must never be interpreted as a relationship.
        assert_eq!(
            filter_character_tags(&format!("{base} {base}_(unknown)"), false, &rules),
            format!("{base} {base}_(unknown)")
        );
    }

    #[test]
    fn source_implications_auto_normalize_without_inventing_exclusivity() {
        let mut rules = TagRules::default();
        for name in ["aria", "aria_human", "aria_robot", "unrelated"] {
            rules.categories.insert(name.into(), 4);
        }
        assert!(rules.accept_character_implication("aria", "aria_human"));
        assert!(rules.accept_character_implication("aria", "aria_robot"));
        assert!(!rules.accept_character_implication("aria", "aria_human"));
        assert!(rules
            .characters
            .iter()
            .all(|r| r.source_verified() && r.exclusive_group.is_empty()));
        let result = normalize(
            "aria aria_human aria_robot unrelated",
            "",
            "aria aria_human aria_robot unrelated",
            &rules,
        );
        assert_eq!(
            result.characters,
            vec!["aria_human", "aria_robot", "unrelated"]
        );
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn source_processing_respects_rejections_categories_aliases_and_cycles() {
        let mut rules = TagRules::default();
        rules.categories.extend([
            ("base".into(), 4),
            ("variant".into(), 4),
            ("scenery".into(), 0),
        ]);
        rules.aliases.insert("old_base".into(), "base".into());
        rules
            .rejected_characters
            .insert(("old_base".into(), "variant".into()));
        assert!(!rules.accept_character_implication("base", "variant"));
        assert!(!rules.accept_character_implication("base", "scenery"));
        assert!(!rules.accept_character_implication("base", "base"));
        rules.rejected_characters.clear();
        assert!(rules.accept_character_implication("old_base", "variant"));
        assert!(!rules.accept_character_implication("variant", "base"));
        assert!(rules.validate_characters().is_ok());
        assert!(rules
            .characters
            .iter()
            .any(|r| r.base == "variant" && !r.verified));
    }

    #[test]
    fn cached_discovery_upgrade_preserves_manual_rules_and_unknown_candidates() {
        let mut rules = TagRules::default();
        for name in ["base", "costume", "other", "unknown"] {
            rules.categories.insert(name.into(), 4);
        }
        rules.characters = vec![
            CharacterRelation {
                base: "base".into(),
                variant: "costume".into(),
                verified: false,
                exclusive_group: String::new(),
                evidence: LEGACY_CHARACTER_IMPLICATION.into(),
            },
            CharacterRelation {
                base: "base".into(),
                variant: "other".into(),
                verified: true,
                exclusive_group: "outfit".into(),
                evidence: "human-confirmed".into(),
            },
            CharacterRelation {
                base: "base".into(),
                variant: "unknown".into(),
                verified: false,
                exclusive_group: String::new(),
                evidence: "inferred from name".into(),
            },
        ];
        assert_eq!(rules.process_discovered_characters(), 1);
        assert_eq!(rules.process_discovered_characters(), 0);
        assert!(rules.characters[0].source_verified());
        assert_eq!(rules.characters[1].exclusive_group, "outfit");
        assert_eq!(rules.characters[1].evidence, "human-confirmed");
        assert!(!rules.characters[2].verified);
    }
    #[test]
    fn verified_variants_replace_only_their_base_and_coexist_without_conflicts() {
        let mut rules = TagRules {
            characters: vec![
                CharacterRelation {
                    base: "ganyu".into(),
                    variant: "ganyu_costume".into(),
                    verified: true,
                    exclusive_group: "costume".into(),
                    evidence: "human".into(),
                },
                CharacterRelation {
                    base: "ganyu".into(),
                    variant: "ganyu_other".into(),
                    verified: true,
                    exclusive_group: "costume".into(),
                    evidence: "human".into(),
                },
            ],
            ..Default::default()
        };
        rules.categories.extend([
            ("ganyu".into(), 4),
            ("ganyu_costume".into(), 4),
            ("ganyu_other".into(), 4),
            ("unrelated".into(), 4),
            ("special_term".into(), 0),
        ]);
        rules.protected.insert("special_term".into());
        let raw = "ganyu ganyu_costume ganyu_other unrelated special_term";
        let output = normalize(raw, "", "ganyu ganyu_costume ganyu_other unrelated", &rules);
        assert!(!output.characters.contains(&"ganyu".into()));
        assert_eq!(
            output.characters,
            vec!["ganyu_costume", "ganyu_other", "unrelated"]
        );
        assert!(output.warnings.is_empty());
        assert!(output.protected.contains(&"special_term".into()));
        rules.characters.iter_mut().for_each(|r| r.verified = false);
        assert!(normalize(raw, "", "ganyu", &rules)
            .characters
            .contains(&"ganyu".into()));
        assert_eq!(
            raw,
            "ganyu ganyu_costume ganyu_other unrelated special_term"
        );
    }

    #[test]
    fn multiple_characters_keep_their_own_variants_and_unrelated_base_tags() {
        let mut rules = TagRules::default();
        rules.aliases.insert("old_a".into(), "a".into());
        for name in ["a", "a_costume", "b", "b_costume", "third"] {
            rules.categories.insert(name.into(), 4);
        }
        for (base, variant) in [("a", "a_costume"), ("b", "b_costume")] {
            rules.characters.push(CharacterRelation {
                base: base.into(),
                variant: variant.into(),
                verified: true,
                exclusive_group: "costume".into(),
                evidence: "human".into(),
            });
        }
        for (raw, expected) in [
            ("old_a a_costume b third", vec!["a_costume", "b", "third"]),
            (
                "old_a a_costume b b_costume third",
                vec!["a_costume", "b_costume", "third"],
            ),
        ] {
            let output = normalize(raw, "", raw, &rules);
            assert_eq!(output.characters, expected);
            assert!(output.warnings.is_empty());
        }
    }

    #[test]
    fn hierarchy_uses_aliases_and_rejects_cycles_without_losing_characters() {
        let mut rules = TagRules::default();
        rules.aliases.insert("old_base".into(), "base".into());
        rules.characters.push(CharacterRelation {
            base: "old_base".into(),
            variant: "variant".into(),
            verified: true,
            exclusive_group: String::new(),
            evidence: "human".into(),
        });
        assert_eq!(
            normalize(
                "old_base variant unrelated_base",
                "",
                "old_base variant unrelated_base",
                &rules
            )
            .characters,
            vec!["variant", "unrelated_base"]
        );
        rules.characters.push(CharacterRelation {
            base: "variant".into(),
            variant: "base".into(),
            verified: true,
            exclusive_group: String::new(),
            evidence: "human".into(),
        });
        assert!(rules.validate_characters().is_err());
        let output = normalize("old_base variant", "", "old_base variant", &rules);
        assert_eq!(output.characters, vec!["base", "variant"]);
        assert!(!output.warnings.is_empty());
    }
    #[test]
    fn unverified_hierarchy_and_cyclic_aliases_preserve_source_terms() {
        let mut rules = TagRules::default();
        rules.aliases.insert("a".into(), "b".into());
        rules.aliases.insert("b".into(), "a".into());
        assert_eq!(rules.canonical("a"), "a");
        assert_eq!(rules.canonical("b"), "b");
        rules.characters.push(CharacterRelation {
            base: "a".into(),
            variant: "b".into(),
            verified: false,
            exclusive_group: String::new(),
            evidence: "candidate".into(),
        });
        assert_eq!(
            normalize("a b", "", "a b", &rules).characters,
            vec!["a", "b"]
        );
    }
}
