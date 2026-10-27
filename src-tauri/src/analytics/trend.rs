use super::types::{Bucket, Turn, UsageEvent};
use std::collections::HashMap;

#[cfg(test)]
use super::metrics;

#[cfg(test)]
pub fn buckets(
    events: &[UsageEvent],
    hours: u32,
    before: Option<i64>,
    now: i64,
) -> (Vec<Bucket>, i64, bool) {
    let bins = match hours {
        168 => 14,
        720 => 15,
        _ => 12,
    };
    let seconds = i64::from(hours) * 3600 / bins;
    let before = before.filter(|v| *v <= now);
    let end = before.unwrap_or(now + 1);
    // Three viewport widths per load, then fetch earlier contiguous chunks on scroll.
    let start = before.unwrap_or(now) - seconds * bins * 3;
    let rows = (0..bins * 3)
        .map(|i| {
            let a = start + i * seconds;
            let b = if i == bins * 3 - 1 { end } else { a + seconds };
            Bucket {
                start: a,
                end: b,
                metrics: metrics(events.iter().filter(|e| e.at >= a && e.at < b)),
            }
        })
        .collect();
    (rows, seconds, events.iter().any(|e| e.at < start))
}

pub fn buckets_by_completion(
    events: &[UsageEvent],
    turns: &HashMap<String, Turn>,
    hours: u32,
    before: Option<i64>,
    now: i64,
    card: &super::pricing::RateCard,
) -> (Vec<Bucket>, i64, bool) {
    let bins = match hours {
        168 => 14,
        720 => 15,
        _ => 12,
    };
    let seconds = i64::from(hours) * 3600 / bins;
    let before = before.filter(|v| *v <= now);
    let end = before.unwrap_or(now + 1);
    // Keep three viewport widths loaded; older pages remain contiguous.
    let start = before.unwrap_or(now) - seconds * bins * 3;
    let rows = (0..bins * 3)
        .map(|i| {
            let a = start + i * seconds;
            let b = if i == bins * 3 - 1 { end } else { a + seconds };
            Bucket {
                start: a,
                end: b,
                metrics: super::completed_task_metrics(events, turns, a, b, card),
            }
        })
        .collect();
    let has_older = events.iter().any(|event| {
        turns.get(&event.root_turn).is_some_and(|turn| {
            !turn.is_subagent && turn.completed.is_some_and(|completed| completed < start)
        })
    });
    (rows, seconds, has_older)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn historical_chunks_are_contiguous_and_keep_the_same_resolution() {
        for hours in [1, 12, 24, 168, 720] {
            let (current, seconds, _) = buckets(&[], hours, None, 9000000);
            let (older, previous_seconds, _) = buckets(&[], hours, Some(current[0].start), 9000000);
            assert_eq!(seconds, previous_seconds);
            assert_eq!(older.last().unwrap().end, current[0].start);
            assert!(older.windows(2).all(|pair| pair[0].end == pair[1].start));
            assert_eq!(current.last().unwrap().end, 9000001);
        }
    }
}
