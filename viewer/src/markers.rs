use crate::points::Mark;
use eframe::egui::{Color32, Stroke};

const LARGE_FILE: usize = 20_000;
const LIMIT: usize = 10_000;

pub fn displayed(candidates: Vec<Mark>, count: usize, selected: Option<usize>) -> Vec<Mark> {
    if count < LARGE_FILE || candidates.len() <= LIMIT {
        return candidates;
    }
    let (matching, filtered): (Vec<_>, Vec<_>) =
        candidates.into_iter().partition(|mark| mark.matched);
    let active = selected
        .and_then(|index| matching.iter().chain(&filtered).find(|m| m.index == index))
        .copied();
    let budget = LIMIT - usize::from(active.is_some());
    let filtered_count = filtered.len() - usize::from(active.is_some_and(|mark| !mark.matched));
    let filtered_reserve = filtered_count.min(LIMIT / 5);
    let matching_count = matching.len().min(budget - filtered_reserve);
    let mut result = Vec::with_capacity(LIMIT);
    for (marks, quota, matched) in [
        (&filtered, budget - matching_count, false),
        (&matching, matching_count, true),
    ] {
        let len = marks.len() - usize::from(active.is_some_and(|a| a.matched == matched));
        let quota = quota.min(len);
        if quota != 0 {
            let mut candidates = marks.iter().copied().filter(|m| Some(m.index) != selected);
            let mut previous = 0;
            for i in 0..quota {
                let index = i * len / quota;
                result.push(candidates.nth(index - previous).unwrap());
                previous = index + 1;
            }
        }
    }
    result.extend(active);
    result
}

pub fn style(
    zoom: f64,
    matched: bool,
    visible_count: usize,
    active: bool,
) -> (f32, Color32, Stroke) {
    let radius: f32 = if !matched {
        if zoom < 8. {
            2.
        } else if zoom < 11. {
            3.
        } else if zoom < 14. {
            5.
        } else {
            6.
        }
    } else if visible_count > 1000 && zoom < 11. {
        if zoom < 8. { 2. } else { 4. }
    } else {
        7.
    };
    if active {
        return (
            (radius + 4.).max(10.),
            Color32::from_rgb(255, 223, 239),
            Stroke::new(4_f32, Color32::from_rgb(192, 37, 117)),
        );
    }
    let (r, g, b, a) = if matched {
        (23, 107, 82, 217)
    } else {
        (133, 141, 150, 89)
    };
    let weight: f32 = if matched && visible_count <= 1000 {
        3.
    } else {
        1.
    };
    (
        radius,
        Color32::from_rgba_unmultiplied(r, g, b, a),
        Stroke::new(weight, Color32::from_rgb(r, g, b)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn html_marker_sizes() {
        for (zoom, matched, count, selected, radius) in [
            (7., true, 1001, false, 2.),
            (8., true, 1001, false, 4.),
            (11., true, 1001, false, 7.),
            (7., true, 1000, false, 7.),
            (10., false, 5, false, 3.),
            (13., false, 5, false, 5.),
            (14., false, 5, false, 6.),
            (5., false, 5, true, 10.),
            (5., true, 5, true, 11.),
        ] {
            assert_eq!(style(zoom, matched, count, selected).0, radius);
        }
    }
    #[test]
    fn large_file_sampling_keeps_selection_and_both_groups() {
        let marks = (0..24_000)
            .map(|index| Mark {
                index,
                world: [0.; 2],
                matched: index < 12_000,
            })
            .collect();
        let shown = displayed(marks, 24_000, Some(23_999));
        assert_eq!(shown.len(), LIMIT);
        assert_eq!(shown.iter().filter(|m| m.matched).count(), 7999);
        assert_eq!(shown.iter().filter(|m| m.index == 23_999).count(), 1);
    }
}
