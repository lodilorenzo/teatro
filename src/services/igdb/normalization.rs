use chrono::{DateTime, Datelike, Utc};
use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

use super::models::{CompanyRole, IgdbGameCandidate, IgdbInvolvedCompany, IgdbNamedEntity};

pub(super) fn entity_names(entities: Vec<IgdbNamedEntity>) -> Vec<String> {
    normalize_string_vec(
        entities
            .into_iter()
            .filter_map(|entity| entity.name)
            .collect(),
    )
}

pub(super) fn company_names(companies: &[IgdbInvolvedCompany], role: CompanyRole) -> Vec<String> {
    let names = companies
        .iter()
        .filter(|company| match role {
            CompanyRole::Developer => company.developer.unwrap_or(false),
            CompanyRole::Publisher => company.publisher.unwrap_or(false),
        })
        .filter_map(|company| company.company.as_ref())
        .filter_map(|company| company.name.clone())
        .collect();

    normalize_string_vec(names)
}

pub(super) fn normalize_string_vec(values: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::new();

    for value in values {
        let value = value.trim();
        if value.is_empty() || normalized.iter().any(|existing| existing == value) {
            continue;
        }
        normalized.push(value.to_string());
    }

    normalized
}

pub(super) fn trimmed_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(super) fn normalize_igdb_url(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else if value.starts_with("//") {
        Some(format!("https:{value}"))
    } else {
        Some(value.to_string())
    }
}

pub(super) fn release_year_from_timestamp(timestamp: i64) -> Option<i32> {
    DateTime::<Utc>::from_timestamp(timestamp, 0).map(|date| date.year())
}

pub(super) fn rank_candidates(
    game_name: &str,
    platform: Option<(&str, &str)>,
    candidates: &mut [IgdbGameCandidate],
) {
    let game_name = normalize_title_for_distance(game_name);
    candidates.sort_by_cached_key(|candidate| {
        (
            usize::from(!platform_matches(platform, candidate)),
            levenshtein_distance(
                game_name.as_bytes(),
                normalize_title_for_distance(&candidate.name).as_bytes(),
            ),
        )
    });
}

fn platform_matches(platform: Option<(&str, &str)>, candidate: &IgdbGameCandidate) -> bool {
    let Some((slug, display_name)) = platform else {
        return false;
    };
    [slug, display_name].iter().any(|target| {
        let target = normalize_title_for_distance(target);
        candidate.platform_keys.iter().any(|candidate_key| {
            platform_key_matches(&target, &normalize_title_for_distance(candidate_key))
        })
    })
}

fn platform_key_matches(left: &str, right: &str) -> bool {
    left == right
        || (left.len() >= 3
            && right
                .strip_suffix(left)
                .is_some_and(|prefix| prefix.ends_with(' ')))
        || (right.len() >= 3
            && left
                .strip_suffix(right)
                .is_some_and(|prefix| prefix.ends_with(' ')))
}

fn normalize_title_for_distance(title: &str) -> String {
    let mut normalized = String::new();
    let mut closing_bracket = None;
    for character in title
        .to_lowercase()
        .nfkd()
        .filter(|ch| !is_combining_mark(*ch))
    {
        if let Some(closing) = closing_bracket {
            if character == closing {
                closing_bracket = None;
            }
            continue;
        }
        closing_bracket = match character {
            '(' => Some(')'),
            '[' => Some(']'),
            '{' => Some('}'),
            _ => None,
        };
        if closing_bracket.is_some() {
            continue;
        }
        if character.is_ascii_alphanumeric() {
            normalized.push(character);
        } else if !normalized.is_empty() && !normalized.ends_with(' ') {
            normalized.push(' ');
        }
    }
    if normalized.ends_with(' ') {
        normalized.pop();
    }
    normalized
}

fn levenshtein_distance(left: &[u8], right: &[u8]) -> usize {
    if left == right {
        return 0;
    }
    let mut previous = (0..=right.len()).collect::<Vec<_>>();
    let mut current = vec![0; right.len() + 1];
    for (left_index, left_character) in left.iter().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_character) in right.iter().enumerate() {
            current[right_index + 1] = (previous[right_index + 1] + 1)
                .min(current[right_index] + 1)
                .min(previous[right_index] + usize::from(left_character != right_character));
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{levenshtein_distance, normalize_title_for_distance, rank_candidates};
    use crate::services::igdb::models::IgdbGameCandidate;

    fn candidate(name: &str, platforms: &[&str]) -> IgdbGameCandidate {
        let mut candidate: IgdbGameCandidate = serde_json::from_value(json!({
            "id": 1,
            "name": name
        }))
        .unwrap();
        candidate.platform_keys = platforms.iter().map(|value| value.to_string()).collect();
        candidate
    }

    #[test]
    fn igdb_ranking_uses_platform_fallback_title_distance_and_stable_ties() {
        assert_eq!(
            normalize_title_for_distance("Pokémon (USA) [Rev 1]"),
            "pokemon"
        );
        assert_eq!(levenshtein_distance(b"sonic", b"sonic 2"), 2);

        let exact_title = candidate("Sonic the Hedgehog", &["Super Nintendo"]);
        let platform_match = candidate("Sonic the Hedgehog 2", &["PlayStation"]);
        let mut candidates = vec![exact_title.clone(), platform_match.clone()];
        rank_candidates(
            "Sonic the Hedgehog",
            Some(("psx", "Sony PlayStation")),
            &mut candidates,
        );
        assert_eq!(candidates[0], platform_match);

        rank_candidates(
            "Sonic the Hedgehog",
            Some(("sgb", "Nintendo Super Game Boy")),
            &mut candidates,
        );
        assert_eq!(candidates[0], exact_title);

        let mut first_tie = candidate("Game", &[]);
        first_tie.id = 1;
        let mut second_tie = candidate("Game", &[]);
        second_tie.id = 2;
        let mut ties = vec![first_tie, second_tie];
        rank_candidates("Game", None, &mut ties);
        assert_eq!(
            ties.iter()
                .map(|candidate| candidate.id)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
    }
}
