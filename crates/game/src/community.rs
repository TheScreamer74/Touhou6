//! Completed-run metadata for a hosting page. Networking and accounts belong
//! to the independent Blackoutburst platform, outside the game repository.
use crate::{result::RunStats, selection::RunMode};

pub fn result_payload(stats: &RunStats, mode: RunMode) -> Option<serde_json::Value> {
    if stats.frames == 0 || stats.character > 3 || stats.score > 999_999_999 {
        return None;
    }
    let valid = match mode {
        RunMode::Extra => stats.difficulty == 4 && stats.stage == 7,
        RunMode::Practice => stats.difficulty < 4 && (1..=6).contains(&stats.stage),
        RunMode::Story => stats.difficulty < 4 && (1..=6).contains(&stats.stage),
    };
    if !valid {
        return None;
    }
    Some(serde_json::json!({
        "score": stats.score, "character": stats.character,
        "difficulty": stats.difficulty, "stage": stats.stage,
        "mode": match mode { RunMode::Story => "normal", RunMode::Extra => "extra", RunMode::Practice => "practice" },
        "cleared": stats.completed, "continues": stats.continues,
        "frames": stats.frames,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extra_and_practice_keep_separate_score_categories() {
        let stats = RunStats {
            score: 123_450,
            character: 3,
            difficulty: 4,
            stage: 7,
            completed: true,
            frames: 54_692,
            ..Default::default()
        };
        assert_eq!(
            result_payload(&stats, RunMode::Extra).unwrap()["mode"],
            "extra"
        );
        assert!(result_payload(&stats, RunMode::Story).is_none());
        let practice = RunStats {
            difficulty: 2,
            stage: 5,
            ..stats
        };
        let payload = result_payload(&practice, RunMode::Practice).unwrap();
        assert_eq!(payload["mode"], "practice");
        assert_eq!(payload["score"], 123_450);
    }
    #[test]
    fn menu_or_unstarted_run_has_no_score_to_publish() {
        assert!(result_payload(&RunStats::default(), RunMode::Story).is_none());
        assert!(
            result_payload(
                &RunStats {
                    frames: 30,
                    stage: 1,
                    score: u32::MAX,
                    ..Default::default()
                },
                RunMode::Story
            )
            .is_none()
        );
    }
}
