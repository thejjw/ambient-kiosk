use serde::{Deserialize, Serialize};

/// Represents a 2D bounding rectangle in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LogicalRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Dynamically calculates a scroll-free, full-bleed grid layout for `count` endpoints.
/// Evaluates candidate row counts $r \in [1, count]$ against `target_aspect` (default 1.4).
pub fn calculate_grid_layout(
    win_width: f64,
    win_height: f64,
    count: usize,
    target_aspect: f64,
    explicit_rows: Option<&[usize]>,
    header_height: f64,
    padding: f64,
    gap: f64,
) -> (Vec<LogicalRect>, Vec<usize>) {
    if count == 0 {
        return (Vec::new(), Vec::new());
    }

    let avail_w = (win_width - 2.0 * padding).max(0.0);
    let avail_h = (win_height - header_height - 2.0 * padding).max(0.0);

    let row_counts: Vec<usize> = if let Some(rows) = explicit_rows {
        rows.to_vec()
    } else {
        let mut _best_r = 1;
        let mut best_score = f64::MAX;
        let mut best_partition = vec![count];

        for r in 1..=count {
            let q = count / r;
            let m = count % r;
            let cols: Vec<usize> = (0..r).map(|i| if i < m { q + 1 } else { q }).collect();

            let mut aspect_cost = 0.0;
            for &c in &cols {
                let tile_w = avail_w / c as f64;
                let tile_h = avail_h / r as f64;
                let alpha = tile_w / tile_h;
                aspect_cost += (alpha / target_aspect).ln().abs();
            }
            aspect_cost /= r as f64;
            let uneven_penalty = if m > 0 { 0.25 } else { 0.0 } + (m as f64 / r as f64) * 0.1;
            let score = aspect_cost + uneven_penalty;

            if score < best_score {
                best_score = score;
                _best_r = r;
                best_partition = cols;
            }
        }
        best_partition
    };

    let num_rows = row_counts.len();
    let total_gaps_h = (num_rows as f64 - 1.0).max(0.0) * gap;
    let slot_h = (avail_h - total_gaps_h) / num_rows as f64;

    let mut rects = Vec::with_capacity(count);
    for (r_idx, &cols_in_row) in row_counts.iter().enumerate() {
        let total_gaps_w = (cols_in_row as f64 - 1.0).max(0.0) * gap;
        let slot_w = (avail_w - total_gaps_w) / cols_in_row as f64;
        let y = header_height + padding + r_idx as f64 * (slot_h + gap);

        for c_idx in 0..cols_in_row {
            let x = padding + c_idx as f64 * (slot_w + gap);
            rects.push(LogicalRect {
                x,
                y,
                width: slot_w,
                height: slot_h,
            });
        }
    }

    (rects, row_counts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_layout_scorer_16_9() {
        let w = 1920.0;
        let h = 1080.0;
        let alpha = 1.4;

        // N = 4 -> [2, 2]
        let (_, rows_4) = calculate_grid_layout(w, h, 4, alpha, None, 0.0, 0.0, 0.0);
        assert_eq!(rows_4, vec![2, 2]);

        // N = 5 -> [3, 2]
        let (_, rows_5) = calculate_grid_layout(w, h, 5, alpha, None, 0.0, 0.0, 0.0);
        assert_eq!(rows_5, vec![3, 2]);

        // N = 6 -> [3, 3] (3x2)
        let (_, rows_6) = calculate_grid_layout(w, h, 6, alpha, None, 0.0, 0.0, 0.0);
        assert_eq!(rows_6, vec![3, 3]);

        // N = 8 -> [4, 4] (4x2)
        let (_, rows_8) = calculate_grid_layout(w, h, 8, alpha, None, 0.0, 0.0, 0.0);
        assert_eq!(rows_8, vec![4, 4]);

        // N = 9 -> [3, 3, 3] (3x3)
        let (_, rows_9) = calculate_grid_layout(w, h, 9, alpha, None, 0.0, 0.0, 0.0);
        assert_eq!(rows_9, vec![3, 3, 3]);

        // N = 12 -> [4, 4, 4] (4x3)
        let (_, rows_12) = calculate_grid_layout(w, h, 12, alpha, None, 0.0, 0.0, 0.0);
        assert_eq!(rows_12, vec![4, 4, 4]);
    }

    #[test]
    fn test_full_bleed_coverage() {
        let w = 1920.0;
        let h = 1080.0;
        let (rects, rows) = calculate_grid_layout(w, h, 5, 1.4, None, 0.0, 0.0, 0.0);
        assert_eq!(rects.len(), 5);
        assert_eq!(rows, vec![3, 2]);

        // Row 0 height + Row 1 height == h
        let h_row0 = rects[0].height;
        let h_row1 = rects[3].height;
        assert!((h_row0 + h_row1 - h).abs() < 1e-6);

        // Row 0 columns sum to w
        let w_row0 = rects[0].width + rects[1].width + rects[2].width;
        assert!((w_row0 - w).abs() < 1e-6);

        // Row 1 columns sum to w
        let w_row1 = rects[3].width + rects[4].width;
        assert!((w_row1 - w).abs() < 1e-6);
    }

    #[test]
    fn test_explicit_rows_override() {
        let (rects, rows) = calculate_grid_layout(1920.0, 1080.0, 5, 1.4, Some(&[2, 3]), 0.0, 0.0, 0.0);
        assert_eq!(rows, vec![2, 3]);
        assert_eq!(rects.len(), 5);
        assert_eq!(rects[0].width, 960.0);
        assert!((rects[2].width - (1920.0 / 3.0)).abs() < 1e-6);
    }
}
