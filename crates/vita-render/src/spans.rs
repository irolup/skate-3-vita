//! The park draw's index runs once the Object Dropper's hidden pieces are left out (host-testable).

/// The index ranges of `draw` (first index, count) once `hidden`'s spans in it are left out.
pub fn visible_runs(draw: u32, first: u32, count: u32, hidden: &[[u32; 3]], out: &mut Vec<(u32, u32)>) {
    out.clear();
    let start = hidden.partition_point(|h| h[0] < draw);
    let mut at = first;
    for h in hidden[start..].iter().take_while(|h| h[0] == draw) {
        if h[1] > at {
            out.push((at, h[1] - at));
        }
        at = at.max(h[1] + h[2]);
    }
    if first + count > at {
        out.push((at, first + count - at));
    }
}


#[cfg(test)]
mod tests {
    #[test]
    fn hidden_spans_leave_runs_around_them() {
        let mut runs = Vec::new();
        // Draw 2 spans indices 100..400; spans 160..190 and 300..330 hidden (one in another draw is ignored).
        let hidden = [[1, 0, 30], [2, 160, 30], [2, 300, 30]];
        super::visible_runs(2, 100, 300, &hidden, &mut runs);
        assert_eq!(runs, vec![(100, 60), (190, 110), (330, 70)]);
        super::visible_runs(3, 0, 30, &hidden, &mut runs);
        assert_eq!(runs, vec![(0, 30)]);
        // A whole draw hidden draws nothing.
        super::visible_runs(1, 0, 30, &hidden, &mut runs);
        assert!(runs.is_empty());
    }
}
