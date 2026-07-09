#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchResult {
    pub score: i64,
    pub positions: Vec<usize>,
}

pub fn fuzzy_match(candidate: &str, query: &str) -> Option<MatchResult> {
    let candidate_chars = candidate.chars().collect::<Vec<_>>();
    let query_chars = query.chars().collect::<Vec<_>>();
    if query_chars.is_empty() {
        return Some(MatchResult {
            score: 0,
            positions: Vec::new(),
        });
    }
    if candidate_chars.is_empty() || query_chars.len() > candidate_chars.len() {
        return None;
    }

    let lower_candidate = candidate_chars
        .iter()
        .map(|ch| ch.to_lowercase().to_string())
        .collect::<Vec<_>>();
    let lower_query = query_chars
        .iter()
        .map(|ch| ch.to_lowercase().to_string())
        .collect::<Vec<_>>();

    let n = candidate_chars.len();
    let m = query_chars.len();
    let mut scores = vec![vec![i64::MIN / 4; n]; m];
    let mut prevs = vec![vec![None; n]; m];

    for query_idx in 0..m {
        let mut best_prev = i64::MIN / 4;
        let mut best_prev_idx = None;

        for cand_idx in 0..n {
            // best_prev covers indices strictly less than cand_idx: the fold
            // for the current index happens at the END of the body, after the
            // match handling, so one candidate position can never satisfy two
            // consecutive query characters.
            if lower_candidate[cand_idx] == lower_query[query_idx] {
                let char_score = char_score(&candidate_chars, cand_idx, query_idx);
                if query_idx == 0 {
                    scores[query_idx][cand_idx] = char_score;
                } else if let Some(prev_idx) = best_prev_idx {
                    let gap = cand_idx.saturating_sub(prev_idx + 1) as i64;
                    let consecutive_bonus = if prev_idx + 1 == cand_idx { 35 } else { 0 };
                    scores[query_idx][cand_idx] =
                        best_prev + char_score + consecutive_bonus - gap * 3;
                    prevs[query_idx][cand_idx] = Some(prev_idx);
                }
            }

            if query_idx > 0 && scores[query_idx - 1][cand_idx] > best_prev {
                best_prev = scores[query_idx - 1][cand_idx];
                best_prev_idx = Some(cand_idx);
            }
        }
    }

    let (mut cand_idx, mut score) = scores[m - 1]
        .iter()
        .copied()
        .enumerate()
        .max_by_key(|(_, score)| *score)?;
    if score <= i64::MIN / 8 {
        return None;
    }

    let mut positions = vec![0; m];
    for query_idx in (0..m).rev() {
        positions[query_idx] = cand_idx;
        if query_idx > 0 {
            cand_idx = prevs[query_idx][cand_idx]?;
        }
    }

    score += adjacency_run_bonus(&positions);
    Some(MatchResult { score, positions })
}

fn char_score(candidate: &[char], index: usize, query_index: usize) -> i64 {
    let mut score = 10;
    if index == 0 {
        score += 45;
    } else if is_word_start(candidate[index - 1], candidate[index]) {
        score += 40;
    }
    if candidate[index].is_uppercase() {
        score += 2;
    }
    score - (index as i64 - query_index as i64).max(0)
}

fn is_word_start(prev: char, current: char) -> bool {
    matches!(prev, '-' | '_' | '/' | '\\' | ' ' | '.' | ':' | '>')
        || (prev.is_lowercase() && current.is_uppercase())
        || (prev.is_ascii_digit() && current.is_alphabetic())
}

fn adjacency_run_bonus(positions: &[usize]) -> i64 {
    let mut bonus = 0;
    let mut run_len = 1;
    for pair in positions.windows(2) {
        if pair[1] == pair[0] + 1 {
            run_len += 1;
            bonus += i64::from(run_len * 4);
        } else {
            run_len = 1;
        }
    }
    bonus
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_query_chars_need_distinct_positions() {
        // Regression: one candidate position must never satisfy two query chars.
        assert!(fuzzy_match("alpha", "ll").is_none());
        assert!(fuzzy_match("ab", "bb").is_none());
        assert!(fuzzy_match("shell", "lll").is_none());
        let m = fuzzy_match("shell", "ll").expect("shell has two l's");
        assert_eq!(m.positions, vec![3, 4]);
    }

    #[test]
    fn matches_case_insensitive_subsequence() {
        let result = fuzzy_match("Workspace > Build Screen > npm test", "wbt").unwrap();
        assert_eq!(result.positions.len(), 3);
        assert_eq!(
            positions_text("Workspace > Build Screen > npm test", &result),
            "WBt"
        );
    }

    #[test]
    fn rejects_non_subsequences() {
        assert!(fuzzy_match("alpha", "az").is_none());
    }

    #[test]
    fn rewards_word_starts_over_late_gaps() {
        let word_start = fuzzy_match("foo-bar", "fb").unwrap();
        let late_gap = fuzzy_match("foooooob", "fb").unwrap();
        assert!(word_start.score > late_gap.score);
    }

    #[test]
    fn rewards_consecutive_runs() {
        let consecutive = fuzzy_match("workspace", "work").unwrap();
        let scattered = fuzzy_match("w_o_r_k", "work").unwrap();
        assert!(consecutive.score > scattered.score);
    }

    #[test]
    fn empty_query_matches_without_positions() {
        let result = fuzzy_match("anything", "").unwrap();
        assert_eq!(result.score, 0);
        assert!(result.positions.is_empty());
    }

    fn positions_text(candidate: &str, result: &MatchResult) -> String {
        let chars = candidate.chars().collect::<Vec<_>>();
        result.positions.iter().map(|idx| chars[*idx]).collect()
    }
}
