//! Pure data transforms behind bar positions (`stack`, `normalize`, `group`) and pie slices.
//! They know nothing about pixels: input is rows of category codes and a value, output is
//! the value range each mark occupies, so they are easy to test exhaustively.
//!
//! Series order is the order of the colour field's dictionary (first appearance in the
//! column), which is what the legend lists too.

use std::collections::HashMap;

/// One input row: category codes (indices into the layer's dictionaries) and a value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Row {
    pub x: usize,
    pub series: usize,
    /// The depth category in 3D; 0 otherwise.
    pub z: usize,
    pub value: f64,
    /// Index of the row in the layer's table.
    pub row: usize,
}

/// All rows of one (x, series, z) combination, summed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cell {
    pub x: usize,
    pub series: usize,
    pub z: usize,
    /// The sum of the rows' values.
    pub value: f64,
    /// The first row that fed this cell; hover and click report this row's data (with the
    /// y value replaced by the sum when `rows > 1`).
    pub row: usize,
    /// How many rows were summed.
    pub rows: usize,
}

impl Cell {
    /// The summed value if more than one row was merged (the datum must then say so).
    pub fn sum(&self) -> Option<f64> {
        (self.rows > 1).then_some(self.value)
    }
}

/// Sums rows with the same (x, series, z). Cells come out in order of first appearance, so
/// the result is stable when rows are appended. Rows whose value is not finite are dropped
/// (a missing number is simply no bar).
pub fn aggregate(rows: impl IntoIterator<Item = Row>) -> Vec<Cell> {
    let mut index: HashMap<(usize, usize, usize), usize> = HashMap::new();
    let mut cells: Vec<Cell> = Vec::new();
    for r in rows.into_iter().filter(|r| r.value.is_finite()) {
        match index.get(&(r.x, r.series, r.z)) {
            Some(&i) => {
                cells[i].value += r.value;
                cells[i].rows += 1;
            }
            None => {
                index.insert((r.x, r.series, r.z), cells.len());
                cells.push(Cell {
                    x: r.x,
                    series: r.series,
                    z: r.z,
                    value: r.value,
                    row: r.row,
                    rows: 1,
                });
            }
        }
    }
    cells
}

/// The value range one cell covers: from `y0` to `y1` (`y0 <= y1`, in data units, or in
/// `0..=1` when normalised).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub y0: f64,
    pub y1: f64,
}

/// Stacks cells: one [`Span`] per cell, in the same order as `cells`.
///
/// Within each (x, z) the series are visited in ascending series order. Positive values pile
/// up above zero, negative values below it, and the two piles are independent, so a negative
/// value never cuts into the positive stack. A zero value gets an empty span at the top of
/// the positive pile.
///
/// With `normalize` each span is a share of that (x, z)'s positive total instead, so the
/// positive pile always ends at exactly 1. Negative values have no share: they count as 0
/// (`validate` rejects them before this runs). A column whose total is 0 gets empty spans.
pub fn stack(cells: &[Cell], normalize: bool) -> Vec<Span> {
    let mut order: Vec<usize> = (0..cells.len()).collect();
    order.sort_by_key(|&i| (cells[i].x, cells[i].z, cells[i].series));
    let mut spans = vec![Span { y0: 0.0, y1: 0.0 }; cells.len()];
    let mut start = 0;
    while start < order.len() {
        let key = |i: usize| (cells[i].x, cells[i].z);
        let mut end = start;
        while end < order.len() && key(order[end]) == key(order[start]) {
            end += 1;
        }
        let group = &order[start..end];
        let total: f64 = group.iter().map(|&i| cells[i].value.max(0.0)).sum();
        let (mut up, mut down) = (0.0_f64, 0.0_f64);
        for &i in group {
            let v = cells[i].value;
            spans[i] = if normalize {
                let share = if total > 0.0 { v.max(0.0) / total } else { 0.0 };
                let y0 = up;
                up += share;
                Span { y0, y1: up }
            } else if v >= 0.0 {
                let y0 = up;
                up += v;
                Span { y0, y1: up }
            } else {
                let y1 = down;
                down += v;
                Span { y0: down, y1 }
            };
        }
        // Rounding in the running sum could leave a hair under 1; the top of a normalised
        // stack is exactly 1 by definition.
        if normalize && total > 0.0 {
            if let Some(&last) = group.iter().rev().find(|&&i| cells[i].value > 0.0) {
                spans[last].y1 = 1.0;
            }
        }
        start = end;
    }
    spans
}

/// The smallest and largest value any span reaches, or `None` when there are none.
pub fn extent(spans: &[Span]) -> Option<(f64, f64)> {
    spans
        .iter()
        .map(|s| (s.y0, s.y1))
        .reduce(|(lo, hi), (a, b)| (lo.min(a), hi.max(b)))
}

/// The sum of the positive values of each (x, z) (the height of its stack), by `(x, z)`.
/// Used to turn a stacked segment into a percentage of its stack.
pub fn column_totals(cells: &[Cell]) -> HashMap<(usize, usize), f64> {
    let mut totals = HashMap::new();
    for c in cells {
        *totals.entry((c.x, c.z)).or_insert(0.0) += c.value.max(0.0);
    }
    totals
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(x: usize, series: usize, value: f64) -> Row {
        Row {
            x,
            series,
            z: 0,
            value,
            row: 0,
        }
    }

    fn cell(x: usize, series: usize, value: f64) -> Cell {
        Cell {
            x,
            series,
            z: 0,
            value,
            row: 0,
            rows: 1,
        }
    }

    #[test]
    fn duplicate_rows_are_summed_in_first_seen_order() {
        let rows = [
            Row {
                row: 0,
                ..row(1, 0, 5.0)
            },
            Row {
                row: 1,
                ..row(0, 0, 1.0)
            },
            Row {
                row: 2,
                ..row(1, 0, 2.5)
            },
            Row {
                row: 3,
                ..row(1, 1, 4.0)
            },
        ];
        let cells = aggregate(rows);
        assert_eq!(cells.len(), 3);
        assert_eq!((cells[0].x, cells[0].value, cells[0].rows), (1, 7.5, 2));
        assert_eq!(cells[0].row, 0, "the first row represents the group");
        assert_eq!(cells[0].sum(), Some(7.5));
        assert_eq!(cells[1].sum(), None, "a lone row is not a sum");
        assert_eq!((cells[2].x, cells[2].series), (1, 1));
    }

    #[test]
    fn missing_values_are_not_cells() {
        let cells = aggregate([
            row(0, 0, f64::NAN),
            row(0, 1, 2.0),
            row(0, 0, f64::INFINITY),
        ]);
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].series, 1);
    }

    #[test]
    fn stacking_follows_series_order_not_row_order() {
        // Rows arrive series 2, 0, 1; the stack is 0 at the bottom, then 1, then 2.
        let cells = [cell(0, 2, 3.0), cell(0, 0, 1.0), cell(0, 1, 2.0)];
        let s = stack(&cells, false);
        assert_eq!(s[1], Span { y0: 0.0, y1: 1.0 });
        assert_eq!(s[2], Span { y0: 1.0, y1: 3.0 });
        assert_eq!(s[0], Span { y0: 3.0, y1: 6.0 });
    }

    #[test]
    fn negatives_stack_downward_separately_from_positives() {
        let cells = [
            cell(0, 0, 4.0),
            cell(0, 1, -1.0),
            cell(0, 2, 2.0),
            cell(0, 3, -3.0),
        ];
        let s = stack(&cells, false);
        assert_eq!(s[0], Span { y0: 0.0, y1: 4.0 });
        assert_eq!(s[1], Span { y0: -1.0, y1: 0.0 });
        assert_eq!(s[2], Span { y0: 4.0, y1: 6.0 });
        assert_eq!(s[3], Span { y0: -4.0, y1: -1.0 });
        assert_eq!(extent(&s), Some((-4.0, 6.0)));
    }

    #[test]
    fn every_x_has_its_own_stack() {
        let cells = [
            cell(0, 0, 1.0),
            cell(1, 0, 5.0),
            cell(0, 1, 1.0),
            cell(1, 1, 5.0),
        ];
        let s = stack(&cells, false);
        assert_eq!(s[2], Span { y0: 1.0, y1: 2.0 });
        assert_eq!(s[3], Span { y0: 5.0, y1: 10.0 });
    }

    #[test]
    fn depth_cells_stack_separately() {
        let mut a = cell(0, 0, 1.0);
        let mut b = cell(0, 0, 9.0);
        a.z = 0;
        b.z = 1;
        let s = stack(&[a, b], false);
        assert_eq!(s[0], Span { y0: 0.0, y1: 1.0 });
        assert_eq!(s[1], Span { y0: 0.0, y1: 9.0 });
    }

    #[test]
    fn normalising_gives_shares_that_end_at_one() {
        let cells = [cell(0, 0, 1.0), cell(0, 1, 1.0), cell(0, 2, 2.0)];
        let s = stack(&cells, true);
        assert_eq!(s[0], Span { y0: 0.0, y1: 0.25 });
        assert_eq!(s[1], Span { y0: 0.25, y1: 0.5 });
        assert_eq!(s[2], Span { y0: 0.5, y1: 1.0 });
        // Thirds do not add up exactly in floating point; the top is still exactly 1.
        let thirds = [cell(0, 0, 1.0), cell(0, 1, 1.0), cell(0, 2, 1.0)];
        assert_eq!(stack(&thirds, true)[2].y1, 1.0);
    }

    #[test]
    fn normalising_an_all_zero_column_gives_empty_spans() {
        let s = stack(&[cell(0, 0, 0.0), cell(0, 1, 0.0)], true);
        assert!(s.iter().all(|s| s.y0 == 0.0 && s.y1 == 0.0));
    }

    #[test]
    fn normalising_ignores_negatives_instead_of_producing_nonsense() {
        let s = stack(&[cell(0, 0, 3.0), cell(0, 1, -5.0), cell(0, 2, 1.0)], true);
        assert_eq!(s[0], Span { y0: 0.0, y1: 0.75 });
        assert_eq!(s[1], Span { y0: 0.75, y1: 0.75 });
        assert_eq!(s[2], Span { y0: 0.75, y1: 1.0 });
    }

    #[test]
    fn column_totals_count_positive_values_only() {
        let t = column_totals(&[cell(0, 0, 3.0), cell(0, 1, -5.0), cell(1, 0, 2.0)]);
        assert_eq!(t[&(0, 0)], 3.0);
        assert_eq!(t[&(1, 0)], 2.0);
    }

    #[cfg(not(target_arch = "wasm32"))]
    mod properties {
        use proptest::prelude::*;

        use super::*;

        fn rows() -> impl Strategy<Value = Vec<Row>> {
            prop::collection::vec(
                (0usize..5, 0usize..4, 0usize..3, -50.0f64..100.0).prop_map(
                    |(x, series, z, value)| Row {
                        x,
                        series,
                        z,
                        value,
                        row: 0,
                    },
                ),
                0..60,
            )
        }

        proptest! {
            /// Summing loses nothing and leaves one cell per combination.
            #[test]
            fn aggregation_keeps_the_total(rows in rows()) {
                let total: f64 = rows.iter().map(|r| r.value).sum();
                let cells = aggregate(rows.clone());
                let after: f64 = cells.iter().map(|c| c.value).sum();
                prop_assert!((total - after).abs() < 1e-6);
                let mut keys: Vec<_> = cells.iter().map(|c| (c.x, c.series, c.z)).collect();
                keys.sort_unstable();
                keys.dedup();
                prop_assert_eq!(keys.len(), cells.len());
                prop_assert_eq!(cells.iter().map(|c| c.rows).sum::<usize>(), rows.len());
            }

            /// Within a stack the segments are contiguous, ordered by series, and never
            /// overlap; every segment is as tall as its value.
            #[test]
            fn stacks_are_contiguous_and_non_overlapping(rows in rows()) {
                let cells = aggregate(rows);
                let spans = stack(&cells, false);
                for (c, s) in cells.iter().zip(&spans) {
                    prop_assert!(s.y0 <= s.y1);
                    prop_assert!((s.y1 - s.y0 - c.value.abs()).abs() < 1e-6);
                    // Positive segments sit above zero, negative ones below.
                    if c.value >= 0.0 { prop_assert!(s.y0 >= 0.0) } else { prop_assert!(s.y1 <= 0.0) }
                }
                // Per (x, z), sorted by series: each positive segment starts where the
                // previous positive one ended.
                for x in 0..5 { for z in 0..3 {
                    let mut group: Vec<_> = cells.iter().zip(&spans)
                        .filter(|(c, _)| c.x == x && c.z == z && c.value >= 0.0).collect();
                    group.sort_by_key(|(c, _)| c.series);
                    let mut top = 0.0;
                    for (_, s) in group {
                        prop_assert!((s.y0 - top).abs() < 1e-9);
                        top = s.y1;
                    }
                }}
            }

            /// Normalised stacks end at exactly 1 (or are empty) and stay inside 0..=1.
            #[test]
            fn normalised_stacks_are_shares(rows in rows()) {
                let rows: Vec<Row> = rows.into_iter().map(|r| Row { value: r.value.abs(), ..r }).collect();
                let cells = aggregate(rows);
                let spans = stack(&cells, true);
                for s in &spans {
                    prop_assert!(s.y0 >= 0.0 && s.y1 <= 1.0 && s.y0 <= s.y1);
                }
                for x in 0..5 { for z in 0..3 {
                    let group: Vec<_> = cells.iter().zip(&spans)
                        .filter(|(c, _)| c.x == x && c.z == z).collect();
                    if group.is_empty() { continue; }
                    let total: f64 = group.iter().map(|(c, _)| c.value).sum();
                    let top = group.iter().map(|(_, s)| s.y1).fold(0.0, f64::max);
                    if total > 0.0 { prop_assert_eq!(top, 1.0) } else { prop_assert_eq!(top, 0.0) }
                    let share: f64 = group.iter().map(|(_, s)| s.y1 - s.y0).sum();
                    let expected = if total > 0.0 { 1.0 } else { 0.0 };
                    prop_assert!((share - expected).abs() < 1e-9);
                }}
            }
        }
    }
}
