//! Pure layout math for the justified-row masonry grid.
//!
//! `pack_rows` greedily fills rows up to `canvas_w` then scales each row to
//! fit. Includes `O(log n)` row lookup helpers used by the snapshot + hit-test
//! paths.
//!
//! The Main library lays out as a list of month **sections** (`pack_sections`):
//! each section is an inline label row followed by its packed tiles, or by an
//! estimated block (`estimate_rows_height`) while its bucket is not loaded yet.
//! `anchor_delta` is the scroll correction that keeps visible content still when
//! sections above the viewport change height.

use std::ops::Range;

const FALLBACK_W: f32 = 4.0;
const FALLBACK_H: f32 = 3.0;

/// Aspect ratio assumed for every tile of a section that has not loaded yet.
pub(crate) const ASSUMED_AVG_RATIO: f32 = 4.0 / 3.0;

/// Height of the inline month label row above each section's tiles.
pub(crate) const SECTION_LABEL_HEIGHT: f32 = 44.0;
/// Label row height for narrow (phone-sized) layouts.
pub(crate) const SECTION_LABEL_HEIGHT_NARROW: f32 = 36.0;

pub(crate) const MIN_ROW_HEIGHT_COMPACT: f32 = 48.0;
pub(crate) const MAX_ROW_HEIGHT_COMPACT: f32 = 80.0;
pub(crate) const MIN_ROW_HEIGHT_NARROW: f32 = 60.0;
pub(crate) const MAX_ROW_HEIGHT_NARROW: f32 = 120.0;
pub(crate) const MIN_ROW_HEIGHT_WIDE: f32 = 180.0;
pub(crate) const MAX_ROW_HEIGHT_WIDE: f32 = 360.0;

pub(crate) const GAP: f32 = 0.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LaidItem {
    pub asset_index: u32,
    pub x: f32,
    pub w: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LaidRow {
    pub y: f32,
    pub h: f32,
    pub items: Vec<LaidItem>,
}

#[derive(Debug, Clone, Copy)]
pub struct LayoutConfig {
    pub min_row_height: f32,
    pub max_row_height: f32,
    pub gap: f32,
}

impl LayoutConfig {
    /// Compact: very small tiles for sub-400px widths (phone screens).
    pub(crate) fn compact() -> Self {
        Self {
            min_row_height: MIN_ROW_HEIGHT_COMPACT,
            max_row_height: MAX_ROW_HEIGHT_COMPACT,
            gap: GAP,
        }
    }

    pub(crate) fn narrow() -> Self {
        Self {
            min_row_height: MIN_ROW_HEIGHT_NARROW,
            max_row_height: MAX_ROW_HEIGHT_NARROW,
            gap: GAP,
        }
    }

    pub(crate) fn wide() -> Self {
        Self {
            min_row_height: MIN_ROW_HEIGHT_WIDE,
            max_row_height: MAX_ROW_HEIGHT_WIDE,
            gap: GAP,
        }
    }

    /// Override the gap for this config (used for configurable border width).
    pub(crate) fn with_gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }
}

/// Width / height for pixel dimensions; unknown (zero) dimensions fall back to 4:3.
pub(crate) fn aspect(width: u32, height: u32) -> f32 {
    if width == 0 || height == 0 {
        FALLBACK_W / FALLBACK_H
    } else {
        (width as f32) / (height as f32)
    }
}

/// The usable ratio of asset `i`: non-finite or non-positive ratios fall back to 4:3.
fn ratio_at(ratios: &[f32], i: usize) -> f32 {
    let r = ratios[i];
    if r.is_finite() && r > 0.0 {
        r
    } else {
        FALLBACK_W / FALLBACK_H
    }
}

/// Greedy justified-row pack. `ratios[i]` is the width / height of asset i.
pub(crate) fn pack_rows(ratios: &[f32], canvas_w: f32, cfg: LayoutConfig) -> (Vec<LaidRow>, f32) {
    if ratios.is_empty() || canvas_w <= 0.0 {
        return (Vec::new(), 0.0);
    }

    // When a border gap is configured, inset the content area so the same
    // gap appears between tiles AND between tiles and the window edges.
    let edge = cfg.gap;
    let inner_w = (canvas_w - 2.0 * edge).max(1.0);

    let mut rows: Vec<LaidRow> = Vec::new();
    let mut y_cursor = edge;
    let mut i = 0_usize;

    while i < ratios.len() {
        let (indices, next_i) = collect_row_indices(ratios, i, inner_w, cfg);
        i = next_i;

        let last_row = i >= ratios.len();
        let mut row = build_row(ratios, indices, inner_w, y_cursor, last_row, cfg);

        // Offset each item's x position by the edge inset.
        if edge > 0.0 {
            for item in &mut row.items {
                item.x += edge;
            }
        }

        y_cursor += row.h + cfg.gap;
        rows.push(row);
    }

    let total_height = (y_cursor - cfg.gap + edge).max(0.0);
    (rows, total_height)
}

/// Greedily collect item indices that fit into a single row at max height.
fn collect_row_indices(
    ratios: &[f32],
    start: usize,
    canvas_w: f32,
    cfg: LayoutConfig,
) -> (Vec<usize>, usize) {
    let mut indices: Vec<usize> = Vec::new();
    let mut summed_w = 0.0_f32;
    let mut i = start;
    while i < ratios.len() {
        let w_at_max = ratio_at(ratios, i) * cfg.max_row_height;
        let gap_before = if indices.is_empty() { 0.0 } else { cfg.gap };
        if !indices.is_empty() && summed_w + gap_before + w_at_max > canvas_w {
            break;
        }
        indices.push(i);
        summed_w += w_at_max + gap_before;
        i += 1;
    }
    (indices, i)
}

/// Scale, clamp, and place items into a single laid-out row.
fn build_row(
    ratios: &[f32],
    mut indices: Vec<usize>,
    canvas_w: f32,
    y: f32,
    last_row: bool,
    cfg: LayoutConfig,
) -> LaidRow {
    let mut row_h = scale_to_fit(&indices, ratios, canvas_w, cfg);

    // Pop the trailing item if the row is too short -- it spills to the next row
    // (handled by the caller via the returned next index).
    if indices.len() > 1 && row_h < cfg.min_row_height {
        indices.pop();
        row_h = scale_to_fit(&indices, ratios, canvas_w, cfg);
    }

    if last_row {
        row_h = row_h.clamp(cfg.min_row_height, cfg.max_row_height);
    }

    let mut placed = Vec::with_capacity(indices.len());
    let mut x_cursor = 0.0_f32;
    for &idx in &indices {
        let w = ratio_at(ratios, idx) * row_h;
        placed.push(LaidItem {
            asset_index: idx as u32,
            x: x_cursor,
            w,
        });
        x_cursor += w + cfg.gap;
    }

    LaidRow {
        y,
        h: row_h,
        items: placed,
    }
}

fn scale_to_fit(indices: &[usize], ratios: &[f32], canvas_w: f32, cfg: LayoutConfig) -> f32 {
    let total_gap = if indices.len() > 1 {
        cfg.gap * (indices.len() as f32 - 1.0)
    } else {
        0.0
    };
    let sum: f32 = indices
        .iter()
        .map(|&idx| ratio_at(ratios, idx) * cfg.max_row_height)
        .sum();
    if sum <= 0.0 {
        return cfg.max_row_height;
    }
    let scale = ((canvas_w - total_gap) / sum).max(0.0);
    cfg.max_row_height * scale
}

pub(crate) fn first_row_at_or_after(rows: &[LaidRow], y: f32) -> usize {
    let mut lo = 0;
    let mut hi = rows.len();
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if rows[mid].y + rows[mid].h < y {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    lo
}

pub(crate) fn row_at_y(rows: &[LaidRow], y: f32) -> Option<usize> {
    if rows.is_empty() {
        return None;
    }
    let mut lo = 0;
    let mut hi = rows.len();
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let r = &rows[mid];
        if y < r.y {
            hi = mid;
        } else if y >= r.y + r.h {
            lo = mid + 1;
        } else {
            return Some(mid);
        }
    }
    None
}

pub(crate) fn item_at_x(row: &LaidRow, x: f32) -> Option<&LaidItem> {
    row.items.iter().find(|it| x >= it.x && x < it.x + it.w)
}

/// One month section to lay out. `ratios` is `Some` once the bucket has loaded
/// (exact layout) and `None` while only its `count` is known (estimated layout).
#[derive(Debug, Clone, PartialEq)]
pub struct SectionInput {
    pub key: String,
    /// Flat model index of the section's first tile.
    pub offset: u32,
    pub count: u32,
    pub ratios: Option<Vec<f32>>,
}

/// A laid-out section: a label row at `top`, then tiles (or an estimate) below it.
#[derive(Debug, Clone, PartialEq)]
pub struct LaidSection {
    pub key: String,
    pub top: f32,
    /// Label row plus tiles; 0 for an empty section (no label is drawn).
    pub height: f32,
    pub label_h: f32,
    pub loaded: bool,
}

impl LaidSection {
    pub(crate) fn bottom(&self) -> f32 {
        self.top + self.height
    }

    /// Top of the tile area, just below the label row.
    pub(crate) fn content_top(&self) -> f32 {
        self.top + self.label_h
    }
}

/// Row height that a run of uniform `ASSUMED_AVG_RATIO` tiles packs to at `canvas_w`.
/// `pack_rows` scales full rows *up* to fill the width, so this sits at or above
/// `max_row_height`; using it keeps estimates close to what loading will produce.
pub(crate) fn target_row_height(canvas_w: f32, cfg: LayoutConfig) -> f32 {
    let inner_w = (canvas_w - 2.0 * cfg.gap).max(1.0);
    let w_at_max = ASSUMED_AVG_RATIO * cfg.max_row_height;
    let per_row = ((inner_w + cfg.gap) / (w_at_max + cfg.gap))
        .floor()
        .max(1.0);
    (inner_w - cfg.gap * (per_row - 1.0)).max(1.0) / (per_row * ASSUMED_AVG_RATIO)
}

/// Estimated tile-area height of an unloaded section holding `count` tiles:
/// `rows = ceil(count / avg_tiles_per_row)` with
/// `avg_tiles_per_row = canvas_w / (target_row_height × 4/3)`, times `(row_h + gap)`,
/// plus the trailing edge gap that `pack_rows` also adds.
pub(crate) fn estimate_rows_height(count: u32, canvas_w: f32, cfg: LayoutConfig) -> f32 {
    if count == 0 || canvas_w <= 0.0 {
        return 0.0;
    }
    let inner_w = (canvas_w - 2.0 * cfg.gap).max(1.0);
    let row_h = target_row_height(canvas_w, cfg);
    let avg_tiles_per_row = (inner_w / (row_h * ASSUMED_AVG_RATIO)).max(1.0);
    // The epsilon keeps float noise (2.9999 tiles per row) from adding a whole row.
    let rows = (count as f32 / avg_tiles_per_row - 1e-3).ceil().max(1.0);
    rows * (row_h + cfg.gap) + cfg.gap
}

/// Lay out every section top to bottom. Returns the sections, the flat list of
/// tile rows (absolute `y`, `asset_index` = flat model index), and the total height.
pub(crate) fn pack_sections(
    inputs: &[SectionInput],
    canvas_w: f32,
    cfg: LayoutConfig,
    label_h: f32,
) -> (Vec<LaidSection>, Vec<LaidRow>, f32) {
    let mut sections = Vec::with_capacity(inputs.len());
    let mut rows = Vec::new();
    let mut y = 0.0_f32;
    if canvas_w <= 0.0 {
        return (sections, rows, 0.0);
    }
    for input in inputs {
        let label = if input.count > 0 { label_h } else { 0.0 };
        let tiles_h = match &input.ratios {
            Some(ratios) => {
                let (packed, h) = pack_rows(ratios, canvas_w, cfg);
                rows.extend(packed.into_iter().map(|mut row| {
                    row.y += y + label;
                    for item in &mut row.items {
                        item.asset_index += input.offset;
                    }
                    row
                }));
                h
            }
            None => estimate_rows_height(input.count, canvas_w, cfg),
        };
        let height = if input.count > 0 {
            label + tiles_h
        } else {
            0.0
        };
        sections.push(LaidSection {
            key: input.key.clone(),
            top: y,
            height,
            label_h: label,
            loaded: input.ratios.is_some(),
        });
        y += height;
    }
    (sections, rows, y)
}

/// Scroll correction after a relayout, so content in view stays put.
///
/// The anchor is the section straddling the viewport top (`scroll_y`): the
/// correction is how far its top moved, i.e. the summed height change of every
/// section entirely above the viewport (`old.bottom <= scroll_y`), including
/// sections that appeared or disappeared above it (a refreshed bucket list).
/// Changes to the anchor section itself or anything below never move the view.
/// Zero at the very top (nothing is above the viewport), past the end, or when the
/// anchor section is gone from the new layout.
pub(crate) fn anchor_delta(old: &[LaidSection], new: &[LaidSection], scroll_y: f32) -> f32 {
    if scroll_y <= 0.0 {
        return 0.0;
    }
    let index = old.partition_point(|s| s.bottom() <= scroll_y);
    let Some(anchor) = old.get(index) else {
        return 0.0;
    };
    let moved = match new.get(index) {
        Some(same) if same.key == anchor.key => Some(same),
        _ => new.iter().find(|s| s.key == anchor.key),
    };
    moved.map_or(0.0, |s| s.top - anchor.top)
}

/// The non-empty section containing `y`. Production code no longer needs this
/// lookup (the sticky labels pin inside the snapshot pass), but the tests keep
/// it as the spec for `pack_sections` boundary behavior.
#[cfg(test)]
pub(crate) fn section_at_y(sections: &[LaidSection], y: f32) -> Option<usize> {
    let idx = sections.partition_point(|s| s.bottom() <= y);
    sections
        .get(idx)
        .filter(|s| s.height > 0.0 && y >= s.top)
        .map(|_| idx)
}

/// Indices of the non-empty sections that intersect `[top, bottom)`.
pub(crate) fn sections_overlapping(
    sections: &[LaidSection],
    top: f32,
    bottom: f32,
) -> Range<usize> {
    let start = sections.partition_point(|s| s.bottom() <= top);
    let end = sections.partition_point(|s| s.top < bottom).max(start);
    start..end
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> LayoutConfig {
        LayoutConfig {
            min_row_height: 100.0,
            max_row_height: 200.0,
            gap: 0.0,
        }
    }

    /// Ratios for `(w, h)` pixel dimensions, as the canvas derives them.
    fn ratios(dims: &[(u32, u32)]) -> Vec<f32> {
        dims.iter().map(|&(w, h)| aspect(w, h)).collect()
    }

    #[test]
    fn empty_input_yields_empty_layout() {
        let (rows, h) = pack_rows(&[], 1000.0, cfg());
        assert!(rows.is_empty());
        assert_eq!(h, 0.0);
    }

    #[test]
    fn zero_canvas_width_yields_empty() {
        let (rows, h) = pack_rows(&[1.0], 0.0, cfg());
        assert!(rows.is_empty());
        assert_eq!(h, 0.0);
    }

    #[test]
    fn fallback_aspect_when_dimensions_zero() {
        let (rows, _) = pack_rows(&ratios(&[(0, 0), (0, 0), (0, 0)]), 1200.0, cfg());
        assert_eq!(rows.len(), 1);
        assert!((rows[0].h - 200.0).abs() < 0.01);
    }

    #[test]
    fn full_row_fills_canvas_width_within_a_pixel() {
        let dims = ratios(&[(1600, 900), (1600, 900), (1600, 900), (1600, 900)]);
        let (rows, _) = pack_rows(&dims, 1200.0, cfg());
        let r1 = &rows[0];
        let last = r1.items.last().unwrap();
        let fill = last.x + last.w;
        assert!((fill - 1200.0).abs() < 1.0);
    }

    #[test]
    fn all_items_placed_across_rows() {
        let dims: Vec<f32> = (0..8).map(|_| 3.0).collect();
        let (rows, _) = pack_rows(&dims, 1200.0, cfg());
        let total: usize = rows.iter().map(|r| r.items.len()).sum();
        assert_eq!(total, 8);
    }

    #[test]
    fn last_row_clamped_to_max_height() {
        let mut dims: Vec<(u32, u32)> = (0..6).map(|_| (4000, 3000)).collect();
        dims.push((1000, 1500));
        let (rows, _) = pack_rows(&ratios(&dims), 1200.0, cfg());
        let last = rows.last().unwrap();
        assert!(last.h <= 200.0 + 0.01);
    }

    #[test]
    fn binary_search_finds_correct_row() {
        let rows = vec![
            LaidRow {
                y: 0.0,
                h: 100.0,
                items: vec![],
            },
            LaidRow {
                y: 100.0,
                h: 150.0,
                items: vec![],
            },
            LaidRow {
                y: 250.0,
                h: 80.0,
                items: vec![],
            },
        ];
        assert_eq!(row_at_y(&rows, 0.0), Some(0));
        assert_eq!(row_at_y(&rows, 100.0), Some(1));
        assert_eq!(row_at_y(&rows, 329.9), Some(2));
        assert_eq!(row_at_y(&rows, 330.0), None);
    }

    #[test]
    fn item_hit_test_within_row() {
        let row = LaidRow {
            y: 0.0,
            h: 100.0,
            items: vec![
                LaidItem {
                    asset_index: 5,
                    x: 0.0,
                    w: 50.0,
                },
                LaidItem {
                    asset_index: 6,
                    x: 50.0,
                    w: 80.0,
                },
                LaidItem {
                    asset_index: 7,
                    x: 130.0,
                    w: 40.0,
                },
            ],
        };
        assert_eq!(item_at_x(&row, 0.0).map(|i| i.asset_index), Some(5));
        assert_eq!(item_at_x(&row, 50.0).map(|i| i.asset_index), Some(6));
        assert_eq!(item_at_x(&row, 130.0).map(|i| i.asset_index), Some(7));
        assert!(item_at_x(&row, 200.0).is_none());
    }

    #[test]
    fn gap_increases_total_layout_height() {
        let dims = &[1.0, 1.0, 1.0, 1.0];
        let (_, h0) = pack_rows(dims, 1200.0, LayoutConfig { gap: 0.0, ..cfg() });
        let (_, h1) = pack_rows(dims, 1200.0, LayoutConfig { gap: 10.0, ..cfg() });
        assert!(h1 > h0);
    }

    #[test]
    fn first_row_skip_lands_on_intersecting_row() {
        let rows = vec![
            LaidRow {
                y: 0.0,
                h: 100.0,
                items: vec![],
            },
            LaidRow {
                y: 100.0,
                h: 100.0,
                items: vec![],
            },
            LaidRow {
                y: 200.0,
                h: 100.0,
                items: vec![],
            },
        ];
        assert_eq!(first_row_at_or_after(&rows, -50.0), 0);
        assert_eq!(first_row_at_or_after(&rows, 0.0), 0);
        assert_eq!(first_row_at_or_after(&rows, 150.0), 1);
        assert_eq!(first_row_at_or_after(&rows, 250.0), 2);
        assert_eq!(first_row_at_or_after(&rows, 500.0), 3);
    }

    #[test]
    fn non_finite_or_non_positive_ratios_fall_back_to_four_thirds() {
        let (a, _) = pack_rows(&[f32::NAN, 0.0, -1.0], 1200.0, cfg());
        let (b, _) = pack_rows(&[4.0 / 3.0; 3], 1200.0, cfg());
        assert_eq!(a, b);
    }

    // --- sections ---------------------------------------------------------

    fn estimated(key: &str, offset: u32, count: u32) -> SectionInput {
        SectionInput {
            key: key.to_string(),
            offset,
            count,
            ratios: None,
        }
    }

    fn loaded(key: &str, offset: u32, ratios: Vec<f32>) -> SectionInput {
        SectionInput {
            key: key.to_string(),
            offset,
            count: ratios.len() as u32,
            ratios: Some(ratios),
        }
    }

    #[test]
    fn target_row_height_is_the_uniform_four_thirds_packing() {
        // 1200 / (200 * 4/3) = 4.5 -> four tiles per row, scaled up to fill: 225px.
        assert!((target_row_height(1200.0, cfg()) - 225.0).abs() < 0.01);
        // A tile wider than the canvas still yields one (shrunk) tile per row.
        assert!((target_row_height(200.0, cfg()) - 150.0).abs() < 0.01);
    }

    #[test]
    fn estimate_is_rows_times_row_height() {
        assert_eq!(estimate_rows_height(0, 1200.0, cfg()), 0.0);
        assert_eq!(estimate_rows_height(10, 0.0, cfg()), 0.0);
        // Four tiles per row at 225px: 1 -> 1 row, 4 -> 1 row, 5 -> 2 rows, 10 -> 3 rows.
        let est = |n| estimate_rows_height(n, 1200.0, cfg());
        assert!((est(1) - 225.0).abs() < 0.01);
        assert!((est(4) - 225.0).abs() < 0.01);
        assert!((est(5) - 450.0).abs() < 0.01);
        assert!((est(10) - 675.0).abs() < 0.01);
    }

    #[test]
    fn estimate_includes_gaps_like_pack_rows() {
        let gapped = LayoutConfig { gap: 10.0, ..cfg() };
        let est = estimate_rows_height(8, 1200.0, gapped);
        let row_h = target_row_height(1200.0, gapped);
        // Two rows, each followed by a gap, plus the leading edge gap.
        assert!((est - (2.0 * (row_h + 10.0) + 10.0)).abs() < 0.01);
    }

    #[test]
    fn estimate_is_deterministic_and_monotonic() {
        let mut prev = 0.0;
        for n in [1, 3, 10, 100, 1000, 10_000] {
            let a = estimate_rows_height(n, 1000.0, cfg());
            assert_eq!(a, estimate_rows_height(n, 1000.0, cfg()));
            assert!(a >= prev);
            prev = a;
        }
        // Narrower canvases fit fewer tiles per row, so the estimate grows.
        assert!(estimate_rows_height(500, 600.0, cfg()) > estimate_rows_height(500, 1800.0, cfg()));
    }

    #[test]
    fn estimate_tracks_the_packed_height_of_four_thirds_tiles() {
        for &(count, width) in &[(40_u32, 1200.0_f32), (97, 900.0), (500, 1600.0)] {
            let est = estimate_rows_height(count, width, cfg());
            let (_, actual) = pack_rows(&vec![4.0 / 3.0; count as usize], width, cfg());
            // Only the (clamped) last row may differ.
            assert!(
                (est - actual).abs() <= target_row_height(width, cfg()),
                "count={count} width={width} est={est} actual={actual}"
            );
        }
    }

    #[test]
    fn sections_stack_labels_and_tiles() {
        let inputs = vec![
            loaded("a", 0, vec![1.5; 6]),
            estimated("b", 6, 10),
            loaded("c", 16, vec![1.0; 2]),
        ];
        let (sections, rows, total) = pack_sections(&inputs, 1200.0, cfg(), 40.0);
        assert_eq!(sections.len(), 3);
        assert_eq!(sections[0].top, 0.0);
        let (_, a_h) = pack_rows(&[1.5; 6], 1200.0, cfg());
        assert!((sections[0].height - (40.0 + a_h)).abs() < 0.01);
        assert_eq!(sections[1].top, sections[0].bottom());
        assert!(
            (sections[1].height - (40.0 + estimate_rows_height(10, 1200.0, cfg()))).abs() < 0.01
        );
        assert!(!sections[1].loaded);
        assert_eq!(sections[2].top, sections[1].bottom());
        assert!((total - sections[2].bottom()).abs() < 0.01);

        // Tile rows are absolute and carry flat model indices.
        assert!((rows[0].y - 40.0).abs() < 0.01);
        let c_rows: Vec<_> = rows.iter().filter(|r| r.y >= sections[2].top).collect();
        assert!((c_rows[0].y - sections[2].content_top()).abs() < 0.01);
        let c_indices: Vec<u32> = c_rows
            .iter()
            .flat_map(|r| r.items.iter().map(|i| i.asset_index))
            .collect();
        assert_eq!(c_indices, vec![16, 17]);
        let all: usize = rows.iter().map(|r| r.items.len()).sum();
        assert_eq!(all, 8, "estimated sections contribute no tile rows");
    }

    #[test]
    fn empty_sections_take_no_space() {
        let inputs = vec![loaded("local", 0, vec![]), estimated("b", 0, 3)];
        let (sections, _, _) = pack_sections(&inputs, 1200.0, cfg(), 40.0);
        assert_eq!(sections[0].height, 0.0);
        assert_eq!(sections[1].top, 0.0);
        assert_eq!(section_at_y(&sections, 0.0), Some(1));
    }

    #[test]
    fn section_lookup_and_band_queries() {
        let inputs = vec![
            estimated("a", 0, 4),
            estimated("empty", 4, 0),
            estimated("b", 4, 4),
            estimated("c", 8, 4),
        ];
        let (s, _, total) = pack_sections(&inputs, 1200.0, cfg(), 40.0);
        // Each section: 40 label + 225 row = 265.
        assert_eq!(section_at_y(&s, 0.0), Some(0));
        assert_eq!(section_at_y(&s, 264.0), Some(0));
        assert_eq!(section_at_y(&s, 265.0), Some(2));
        assert_eq!(section_at_y(&s, total - 1.0), Some(3));
        assert_eq!(section_at_y(&s, total + 1.0), None);
        assert_eq!(section_at_y(&s, -5.0), None);

        assert_eq!(sections_overlapping(&s, 0.0, 10.0), 0..1);
        assert_eq!(sections_overlapping(&s, 300.0, 310.0), 2..3);
        assert_eq!(sections_overlapping(&s, 200.0, 600.0), 0..4);
        assert_eq!(sections_overlapping(&s, -1000.0, total + 1000.0), 0..4);
        assert!(sections_overlapping(&s, total + 1.0, total + 50.0).is_empty());
    }

    fn laid(heights: &[f32]) -> Vec<LaidSection> {
        let mut top = 0.0;
        heights
            .iter()
            .enumerate()
            .map(|(i, &height)| {
                let s = LaidSection {
                    key: format!("k{i}"),
                    top,
                    height,
                    label_h: 40.0,
                    loaded: false,
                };
                top += height;
                s
            })
            .collect()
    }

    #[test]
    fn anchor_compensates_sections_entirely_above_the_viewport() {
        let old = laid(&[100.0, 100.0, 100.0, 100.0]);
        // Viewport starts at 250: sections 0 and 1 are entirely above it.
        let new = laid(&[150.0, 100.0, 100.0, 100.0]);
        assert_eq!(anchor_delta(&old, &new, 250.0), 50.0);
        let new = laid(&[150.0, 70.0, 100.0, 100.0]);
        assert_eq!(anchor_delta(&old, &new, 250.0), 20.0);
        // A section ending exactly at the viewport top counts as above it.
        assert_eq!(anchor_delta(&old, &new, 200.0), 20.0);
    }

    #[test]
    fn anchor_ignores_visible_and_lower_sections() {
        let old = laid(&[100.0, 100.0, 100.0, 100.0]);
        // Section 2 straddles the viewport top; section 3 is below it.
        let new = laid(&[100.0, 100.0, 180.0, 300.0]);
        assert_eq!(anchor_delta(&old, &new, 250.0), 0.0);
    }

    #[test]
    fn anchor_is_zero_at_the_top_past_the_end_or_without_the_anchor() {
        let old = laid(&[0.0, 100.0, 100.0]);
        let new = laid(&[120.0, 100.0, 100.0]);
        assert_eq!(anchor_delta(&old, &new, 0.0), 0.0);
        // Scrolled past the end of the old layout: nothing to anchor on.
        assert_eq!(anchor_delta(&old, &new, 250.0), 0.0);
        // The anchor section (k2 at y=150) is gone from the new layout.
        let mut renamed = new.clone();
        renamed[2].key = "other".into();
        assert_eq!(anchor_delta(&old, &renamed, 150.0), 0.0);
        assert_eq!(anchor_delta(&old, &new, 150.0), 120.0);
    }

    #[test]
    fn anchor_follows_sections_added_or_removed_above_the_viewport() {
        let old = laid(&[100.0, 100.0, 100.0]);
        // A refreshed bucket list gains a new month on top.
        let mut grown = laid(&[80.0, 100.0, 100.0, 100.0]);
        grown[0].key = "new".into();
        for (i, s) in grown.iter_mut().enumerate().skip(1) {
            s.key = format!("k{}", i - 1);
        }
        assert_eq!(anchor_delta(&old, &grown, 250.0), 80.0);
        // ...or loses the first month entirely.
        let mut shrunk = laid(&[100.0, 100.0]);
        for (i, s) in shrunk.iter_mut().enumerate() {
            s.key = format!("k{}", i + 1);
        }
        assert_eq!(anchor_delta(&old, &shrunk, 250.0), -100.0);
    }

    /// Spike: load buckets above and below the viewport in arbitrary order; the
    /// content under the viewport must never move on screen.
    #[test]
    fn loading_buckets_in_any_order_never_moves_visible_content() {
        let counts = [12_u32, 40, 7, 25, 60, 18, 33, 5, 90, 14];
        let ratios_for = |i: usize| -> Vec<f32> {
            (0..counts[i])
                .map(|j| [0.75, 1.5, 1.0, 1.78, 0.56][(i + j as usize) % 5])
                .collect()
        };
        let mut loaded_set = vec![false; counts.len()];
        let build = |loaded_set: &[bool]| {
            let mut offset = 0;
            let inputs: Vec<SectionInput> = counts
                .iter()
                .enumerate()
                .map(|(i, &count)| {
                    let input = SectionInput {
                        key: format!("2025-{i:02}"),
                        offset,
                        count,
                        ratios: loaded_set[i].then(|| ratios_for(i)),
                    };
                    offset += count;
                    input
                })
                .collect();
            pack_sections(&inputs, 1100.0, cfg(), 40.0).0
        };

        let watched = 5;
        let mut sections = build(&loaded_set);
        let mut scroll_y = sections[watched].content_top() + 30.0;
        let on_screen = |s: &[LaidSection], scroll: f32| s[watched].content_top() - scroll;
        let before = on_screen(&sections, scroll_y);

        for i in [8, 1, 9, 0, 3, 6, 2, 4, 7] {
            loaded_set[i] = true;
            let next = build(&loaded_set);
            scroll_y += anchor_delta(&sections, &next, scroll_y);
            sections = next;
            assert!(
                (on_screen(&sections, scroll_y) - before).abs() < 0.01,
                "loading bucket {i} moved the visible content"
            );
        }
    }
}
