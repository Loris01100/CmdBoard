//! Fuzzy ranking shared by the `/` search and Tab completion.

use fuzzy_matcher::{FuzzyMatcher, skim::SkimMatcherV2};

/// Indices of the `items` matching `query`, best match first (ties keep their order).
/// An empty query matches everything, in order. Case is ignored.
pub fn rank<'a>(query: &str, items: impl IntoIterator<Item = &'a str>) -> Vec<usize> {
    let query = query.trim();
    let items = items.into_iter();
    if query.is_empty() {
        return items.enumerate().map(|(i, _)| i).collect();
    }
    let matcher = SkimMatcherV2::default().ignore_case();
    let mut scored: Vec<(usize, i64)> = items
        .enumerate()
        .filter_map(|(i, item)| matcher.fuzzy_match(item, query).map(|score| (i, score)))
        .collect();
    // Stable sort: equal scores keep the input order.
    scored.sort_by_key(|&(_, score)| std::cmp::Reverse(score));
    scored.into_iter().map(|(i, _)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_best_match_first_ignoring_case() {
        let items = ["Steam", "Windows Terminal", "Bloc-notes", "Stellaris"];
        assert_eq!(rank("ste", items)[..2], [0, 3]); // prefixes first, then "Windows Terminal"
        assert_eq!(rank("WT", items), [1]);
        assert_eq!(rank("wterm", items), [1]);
        assert!(rank("zzz", items).is_empty());
        assert_eq!(rank("  ", items), [0, 1, 2, 3]);
    }
}
