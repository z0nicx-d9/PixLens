/// A box of terminal cells an image is drawn into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellBox {
    pub columns: u32,
    pub rows: u32,
}

/// Fits a `width` x `height` image inside `max_columns` x `max_rows` cells,
/// keeping its aspect ratio.
///
/// A terminal cell is about twice as tall as it is wide, so one cell holds
/// two square pixels stacked (the half-block model). The image is never
/// scaled above one pixel per half cell, so a small icon stays small.
pub fn fit_cells(width: u32, height: u32, max_columns: u32, max_rows: u32) -> CellBox {
    let (w, h) = (width.max(1) as f64, height.max(1) as f64);
    let (max_w, max_h) = (max_columns.max(1) as f64, (max_rows.max(1) * 2) as f64);
    let scale = (max_w / w).min(max_h / h).min(1.0);

    let columns = (w * scale).round().clamp(1.0, max_w) as u32;
    let rows = ((h * scale) / 2.0)
        .ceil()
        .clamp(1.0, max_rows.max(1) as f64) as u32;

    CellBox { columns, rows }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_image_is_bounded_by_columns() {
        assert_eq!(
            fit_cells(1920, 1080, 60, 40),
            CellBox {
                columns: 60,
                rows: 17
            }
        );
    }

    #[test]
    fn tall_image_is_bounded_by_rows() {
        assert_eq!(
            fit_cells(1000, 3000, 80, 20),
            CellBox {
                columns: 13,
                rows: 20
            }
        );
    }

    #[test]
    fn small_image_is_not_scaled_up() {
        assert_eq!(
            fit_cells(16, 16, 80, 24),
            CellBox {
                columns: 16,
                rows: 8
            }
        );
    }

    #[test]
    fn degenerate_sizes_give_at_least_one_cell() {
        assert_eq!(
            fit_cells(0, 0, 0, 0),
            CellBox {
                columns: 1,
                rows: 1
            }
        );
        assert_eq!(
            fit_cells(10_000, 1, 40, 10),
            CellBox {
                columns: 40,
                rows: 1
            }
        );
    }
}
