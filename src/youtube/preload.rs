//! Plan a stable next-track choice before starting background network work.
use crate::model::{Repeat, Station, shuffle_candidates};
use uuid::Uuid;

pub fn next_index(
    queue: &[Station],
    current: usize,
    repeat: Repeat,
    shuffle: bool,
    history: &[Uuid],
    radio: bool,
    preferred: Option<Uuid>,
) -> Option<(usize, bool)> {
    if current >= queue.len() {
        return None;
    }
    if radio {
        return (current + 1 < queue.len()).then_some((current + 1, false));
    }
    if repeat == Repeat::One {
        return Some((current, false));
    }
    if shuffle && queue.len() > 1 {
        let mut candidates = shuffle_candidates(queue, current, history);
        let reset = candidates.is_empty() && repeat == Repeat::All;
        if reset {
            candidates = shuffle_candidates(queue, current, &[]);
        }
        let selected = preferred.and_then(|id| {
            candidates
                .iter()
                .copied()
                .find(|index| queue[*index].id == id)
        });
        use rand::seq::IndexedRandom;
        return selected
            .or_else(|| candidates.choose(&mut rand::rng()).copied())
            .map(|index| (index, reset));
    }
    if current + 1 < queue.len() {
        Some((current + 1, false))
    } else if repeat == Repeat::All {
        Some((0, false))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preload_selection_respects_repeat_radio_and_shuffle_history() {
        let queue: Vec<_> = ["A", "B", "C"]
            .iter()
            .map(|name| Station::new(*name, format!("https://youtube.com/watch?v={name}")))
            .collect();
        assert_eq!(
            next_index(&queue, 0, Repeat::Once, false, &[], false, None),
            Some((1, false))
        );
        assert_eq!(
            next_index(&queue, 2, Repeat::Once, false, &[], false, None),
            None
        );
        assert_eq!(
            next_index(&queue, 2, Repeat::All, false, &[], false, None),
            Some((0, false))
        );
        assert_eq!(
            next_index(&queue, 1, Repeat::One, true, &[], false, None),
            Some((1, false))
        );
        assert_eq!(
            next_index(&queue, 0, Repeat::One, true, &[], true, None),
            Some((1, false))
        );
        assert_eq!(
            next_index(&queue, 2, Repeat::All, true, &[], true, None),
            None
        );
        assert_eq!(
            next_index(
                &queue,
                0,
                Repeat::Once,
                true,
                &[queue[1].id],
                false,
                Some(queue[1].id)
            ),
            Some((2, false))
        );
        assert_eq!(
            next_index(
                &queue,
                2,
                Repeat::Once,
                true,
                &[queue[0].id, queue[1].id],
                false,
                None
            ),
            None
        );
        assert_eq!(
            next_index(
                &queue,
                2,
                Repeat::All,
                true,
                &[queue[0].id, queue[1].id],
                false,
                Some(queue[0].id)
            ),
            Some((0, true))
        );
        for _ in 0..20 {
            assert_eq!(
                next_index(&queue, 0, Repeat::All, true, &[], false, Some(queue[2].id)),
                Some((2, false))
            );
        }
    }
}
