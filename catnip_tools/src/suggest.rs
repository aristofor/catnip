// FILE: catnip_tools/src/suggest.rs
//! Name suggestion utilities based on Damerau-Levenshtein distance.

/// Optimal string alignment distance (restricted Damerau-Levenshtein).
///
/// Counts insertions, deletions, substitutions, and adjacent transpositions
/// each as a single edit operation.
pub fn damerau_levenshtein(a: &str, b: &str) -> usize {
    let a_bytes = a.as_bytes();
    let b_bytes = b.as_bytes();
    let m = a_bytes.len();
    let n = b_bytes.len();

    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }

    // Full matrix needed for transposition lookback
    let mut d = vec![vec![0usize; n + 1]; m + 1];
    for (i, row) in d.iter_mut().enumerate().take(m + 1) {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate().take(n + 1) {
        *cell = j;
    }

    for i in 1..=m {
        for j in 1..=n {
            let cost = if a_bytes[i - 1] == b_bytes[j - 1] { 0 } else { 1 };
            d[i][j] = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);

            // Transposition
            if i > 1 && j > 1 && a_bytes[i - 1] == b_bytes[j - 2] && a_bytes[i - 2] == b_bytes[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[m][n]
}

/// Find similar names from candidates.
///
/// Filters out names starting with `_`, computes normalized similarity
/// (1 - dist/max_len), returns up to `max` results above `cutoff`.
pub fn suggest_similar(name: &str, candidates: &[&str], max: usize, cutoff: f64) -> Vec<String> {
    let mut scored: Vec<(f64, &str)> = candidates
        .iter()
        .filter(|c| !c.starts_with('_') && !c.is_empty())
        .filter_map(|&c| {
            let max_len = name.len().max(c.len());
            if max_len == 0 {
                return None;
            }
            let dist = damerau_levenshtein(name, c);
            let ratio = 1.0 - (dist as f64 / max_len as f64);
            if ratio >= cutoff { Some((ratio, c)) } else { None }
        })
        .collect();

    // Sort by similarity descending, then alphabetically for ties
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap().then_with(|| a.1.cmp(b.1)));
    scored.truncate(max);
    scored.into_iter().map(|(_, s)| s.to_string()).collect()
}

/// Format a suggestion message.
///
/// - 0 suggestions: None
/// - 1 suggestion: "Did you mean 'x'?"
/// - N suggestions: "Did you mean one of: 'x', 'y'?"
pub fn format_suggestion(suggestions: &[String]) -> Option<String> {
    match suggestions.len() {
        0 => None,
        1 => Some(format!("Did you mean '{}'?", suggestions[0])),
        _ => {
            let quoted: Vec<String> = suggestions.iter().map(|s| format!("'{s}'")).collect();
            Some(format!("Did you mean one of: {}?", quoted.join(", ")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_damerau_levenshtein_identical() {
        assert_eq!(damerau_levenshtein("abc", "abc"), 0);
    }

    #[test]
    fn test_damerau_levenshtein_empty() {
        assert_eq!(damerau_levenshtein("", "abc"), 3);
        assert_eq!(damerau_levenshtein("abc", ""), 3);
    }

    #[test]
    fn test_damerau_levenshtein_substitution() {
        assert_eq!(damerau_levenshtein("kitten", "sitten"), 1);
    }

    #[test]
    fn test_damerau_levenshtein_transposition() {
        // "naem" -> "name" is 1 transposition
        assert_eq!(damerau_levenshtein("naem", "name"), 1);
    }

    #[test]
    fn test_damerau_levenshtein_full() {
        assert_eq!(damerau_levenshtein("kitten", "sitting"), 3);
    }

    #[test]
    fn test_suggest_similar_basic() {
        let candidates = vec!["factorial", "factor", "factory", "unrelated"];
        let result = suggest_similar("factoral", &candidates, 3, 0.6);
        assert!(result.contains(&"factorial".to_string()));
    }

    #[test]
    fn test_suggest_similar_filters_private() {
        let candidates = vec!["_private", "public"];
        let result = suggest_similar("publc", &candidates, 3, 0.6);
        assert!(!result.iter().any(|s| s.starts_with('_')));
    }

    #[test]
    fn test_suggest_similar_no_match() {
        let candidates = vec!["completely", "different"];
        let result = suggest_similar("xyz", &candidates, 3, 0.6);
        assert!(result.is_empty());
    }

    #[test]
    fn test_format_suggestion_none() {
        assert_eq!(format_suggestion(&[]), None);
    }

    #[test]
    fn test_format_suggestion_one() {
        let sugg = vec!["factorial".to_string()];
        assert_eq!(format_suggestion(&sugg), Some("Did you mean 'factorial'?".to_string()));
    }

    #[test]
    fn test_format_suggestion_multiple() {
        let sugg = vec!["foo".to_string(), "bar".to_string()];
        assert_eq!(
            format_suggestion(&sugg),
            Some("Did you mean one of: 'foo', 'bar'?".to_string())
        );
    }
}
