//! Justified-row masonry layout for the photos grid.

use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use libadwaita::prelude::*;

use super::grid_view::AssetContextMenuHandler;
use crate::api_client::ThumbnailSize;
use crate::library::asset_model::LibraryAssetModel;
use crate::library::asset_object::AssetObject;
use crate::library::thumbnail_cache::ThumbnailCache;

type ActivateHandler = Rc<dyn Fn(u32)>;
type SelectModeChanger = Rc<dyn Fn(bool)>;
/// Receives the indices of unloaded sections intersecting the viewport band.
type SectionDemandHandler = Rc<dyn Fn(Vec<usize>)>;
/// Blocking hydration of a thin tile before drag-out names its file; false on failure.
type DragHydrator = Rc<dyn Fn(&AssetObject) -> bool>;

pub(super) const CORNER_RADIUS: f32 = 0.0;

const VIEWPORTS_BEHIND: f32 = 2.0;
const VIEWPORTS_AHEAD: f32 = 4.0;

use super::layout::{
    LaidItem, LaidRow, LaidSection, LayoutConfig, SECTION_LABEL_HEIGHT,
    SECTION_LABEL_HEIGHT_NARROW, SectionInput, anchor_delta, first_row_at_or_after, item_at_x,
    pack_rows, pack_sections, row_at_y, sections_overlapping,
};
pub use super::quality::GridQuality;
use super::quality::{bucket_for_row_height, fallback_bucket};

mod interaction;

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};
    use std::collections::HashSet;
    use std::sync::Arc;

    use gtk::glib;
    use gtk::graphene::{Rect, Size};
    use gtk::gsk::RoundedRect;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;

    use super::{
        ActivateHandler, AssetContextMenuHandler, AssetObject, CORNER_RADIUS, DragHydrator,
        GridQuality, LaidItem, LaidRow, LaidSection, LayoutConfig, LibraryAssetModel,
        SECTION_LABEL_HEIGHT, SECTION_LABEL_HEIGHT_NARROW, SectionDemandHandler, SectionInput,
        SelectModeChanger, ThumbnailCache, ThumbnailSize, VIEWPORTS_AHEAD, VIEWPORTS_BEHIND,
        accent_bg_color, anchor_delta, bucket_for_asset, bucket_for_row_height, cached_texture,
        collect_ratios, finish_masonry_load, first_row_at_or_after, load_masonry_asset,
        load_target_for, pack_rows, pack_sections, placeholder_color, resolve_symbolic_icon,
        resolve_video_icon, sections_overlapping, stack_badge_geometry, stack_badge_icon_size,
    };

    pub(super) enum PaintResult {
        Hit,
        Miss,
    }

    struct SnapshotCtx<'a> {
        model: &'a LibraryAssetModel,
        cache: &'a Arc<ThumbnailCache>,
        placeholder: gdk4::RGBA,
        select_tint: gdk4::RGBA,
        selection: Option<&'a gtk::MultiSelection>,
    }

    #[derive(Default)]
    pub(super) struct SnapshotStats {
        pub(super) hits: usize,
        pub(super) misses: usize,
        pub(super) painted: usize,
    }

    pub(super) struct SnapshotViewport {
        scroll_y: f32,
        center: f32,
        top: f32,
        bottom: f32,
        band_top: f32,
        band_bottom: f32,
    }

    pub(super) struct LoadRequest {
        pub(super) priority: f32,
        pub(super) asset_id: String,
        pub(super) bucket: ThumbnailSize,
        pub(super) local_path: String,
        pub(super) is_local: bool,
    }

    #[derive(Default)]
    pub struct MasonryCanvas {
        pub model: OnceCell<LibraryAssetModel>,
        pub cache: OnceCell<Arc<ThumbnailCache>>,
        pub selection: OnceCell<gtk::MultiSelection>,
        pub narrow: Cell<bool>,
        pub select_mode: Cell<bool>,
        pub quality: Cell<GridQuality>,
        pub rows: RefCell<Vec<LaidRow>>,
        pub cached_width: Cell<f32>,
        pub layout_h: Cell<f32>,
        pub pending: RefCell<HashSet<String>>,
        /// Permanently failed ids; skipped to avoid re-queueing every frame.
        pub failed: RefCell<HashSet<String>>,
        pub vadjustment: RefCell<Option<gtk::Adjustment>>,
        pub activate_handler: RefCell<Option<ActivateHandler>>,
        pub context_menu_handler: RefCell<Option<AssetContextMenuHandler>>,
        pub select_mode_changer: RefCell<Option<SelectModeChanger>>,
        /// Anchor position for Shift+Click range selection.
        pub last_selected: Cell<Option<u32>>,
        pub permission_warned: Cell<bool>,
        /// Cached video badge icon paintable, resolved lazily from the icon theme.
        pub video_icon: OnceCell<gdk4::Paintable>,
        /// Cached favorite heart icon, painted on favorited tiles.
        pub heart_icon: OnceCell<gdk4::Paintable>,
        /// Cached stack icon, painted with the member count on stack tiles.
        pub stack_icon: OnceCell<gdk4::Paintable>,
        /// Cached checkbox icon (selected state).
        pub check_icon: OnceCell<gdk4::Paintable>,
        /// Cached checkbox icon (unselected state).
        pub uncheck_icon: OnceCell<gdk4::Paintable>,
        /// True while a drag-out operation is in progress; suppresses click-to-activate.
        pub drag_active: Cell<bool>,
        /// Configurable gap between grid tiles, in pixels.
        pub border_gap: Cell<f32>,
        /// Color painted behind tiles so the gap appears as a border.
        pub border_color: Cell<Option<gdk4::RGBA>>,
        /// Laid-out month sections (Main library only; empty for flat sources).
        pub sections: RefCell<Vec<LaidSection>>,
        /// Inline label of each entry in `sections`.
        pub section_labels: RefCell<Vec<String>>,
        pub section_demand_handler: RefCell<Option<SectionDemandHandler>>,
        pub drag_hydrator: RefCell<Option<DragHydrator>>,
        /// Last reported `(model epoch, unloaded sections in band)`, to report changes only.
        pub last_demand: RefCell<Option<(u64, Vec<usize>)>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MasonryCanvas {
        const NAME: &str = "MimickMasonryCanvas";
        type Type = super::MasonryCanvas;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for MasonryCanvas {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().add_css_class("mimick-masonry-canvas");
            self.cached_width.set(-1.0);
        }
    }

    impl WidgetImpl for MasonryCanvas {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            match orientation {
                gtk::Orientation::Horizontal => (120, 200, -1, -1),
                _ => {
                    let width = if for_size > 0 {
                        for_size as f32
                    } else {
                        self.cached_width.get().max(120.0)
                    };
                    let h = self.layout_for_width(width);
                    let h_i = h.ceil() as i32;
                    (h_i, h_i, -1, -1)
                }
            }
        }

        fn size_allocate(&self, width: i32, _height: i32, _baseline: i32) {
            let w = width.max(0) as f32;
            if (w - self.cached_width.get()).abs() > 0.5 {
                self.cached_width.set(-1.0);
            }
            let _ = self.layout_for_width(w);
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let canvas_w = widget.width() as f32;
            if canvas_w <= 0.0 {
                return;
            }
            let _ = self.layout_for_width(canvas_w);

            let rows = self.rows.borrow();
            let Some(model) = self.model.get() else {
                return;
            };
            let Some(cache) = self.cache.get() else {
                return;
            };

            let viewport = self.snapshot_viewport();

            // Fill the visible area with the border color so gaps between
            // tiles appear as a coloured border.
            let gap = self.border_gap.get();
            if gap > 0.0 {
                let bc = self
                    .border_color
                    .get()
                    .unwrap_or_else(|| gdk4::RGBA::new(1.0, 1.0, 1.0, 1.0));
                let canvas_h = self.layout_h.get();
                let vis_rect = Rect::new(
                    0.0,
                    viewport.scroll_y.max(0.0),
                    canvas_w,
                    (viewport.bottom - viewport.scroll_y.max(0.0)).min(canvas_h),
                );
                snapshot.append_color(&bc, &vis_rect);
            }

            let sctx = self.snapshot_ctx(model, cache);
            self.paint_section_chrome(snapshot, &sctx, &viewport, canvas_w);
            let (stats, mut to_load) = self.paint_snapshot_rows(snapshot, &sctx, &rows, &viewport);
            drop(rows);
            self.paint_section_labels(snapshot, &viewport, canvas_w);
            self.finish_snapshot(cache, &viewport, &stats, &mut to_load);
            self.report_section_demand(model, &viewport);
        }
    }

    /// Compact layout threshold: canvas widths below this use the
    /// smallest tile tier for phone-sized screens.
    const COMPACT_WIDTH_THRESHOLD: f32 = 400.0;

    impl MasonryCanvas {
        fn cfg(&self) -> LayoutConfig {
            let gap = self.border_gap.get();
            if self.narrow.get() {
                let w = self.cached_width.get();
                if w > 0.0 && w < COMPACT_WIDTH_THRESHOLD {
                    LayoutConfig::compact().with_gap(gap)
                } else {
                    LayoutConfig::narrow().with_gap(gap)
                }
            } else {
                LayoutConfig::wide().with_gap(gap)
            }
        }

        fn label_height(&self) -> f32 {
            if self.narrow.get() {
                SECTION_LABEL_HEIGHT_NARROW
            } else {
                SECTION_LABEL_HEIGHT
            }
        }

        fn layout_for_width(&self, width: f32) -> f32 {
            let cached = self.cached_width.get();
            if (width - cached).abs() < 0.5 && cached >= 0.0 {
                return self.layout_h.get();
            }
            let Some(model) = self.model.get() else {
                self.cached_width.set(width);
                self.layout_h.set(0.0);
                return 0.0;
            };
            let (rows, h) = match model.sections() {
                Some(sections) => self.layout_sections(model, sections, width),
                None => {
                    self.sections.borrow_mut().clear();
                    self.section_labels.borrow_mut().clear();
                    pack_rows(&collect_ratios(model), width, self.cfg())
                }
            };
            *self.rows.borrow_mut() = rows;
            self.cached_width.set(width);
            self.layout_h.set(h);
            h
        }

        /// Sectioned (Main library) layout: loaded sections pack exactly from their
        /// tiles' ratios, unloaded ones contribute an estimate.
        fn layout_sections(
            &self,
            model: &LibraryAssetModel,
            sections: Vec<crate::library::asset_model::GridSection>,
            width: f32,
        ) -> (Vec<LaidRow>, f32) {
            let mut offset = 0_u32;
            let inputs: Vec<SectionInput> = sections
                .iter()
                .map(|section| {
                    let range = offset..offset + section.count;
                    offset = range.end;
                    SectionInput {
                        key: section.key.clone(),
                        offset: range.start,
                        count: section.count,
                        ratios: section.loaded.then(|| model.ratios(range)),
                    }
                })
                .collect();
            let (laid, rows, h) = pack_sections(&inputs, width, self.cfg(), self.label_height());
            *self.sections.borrow_mut() = laid;
            *self.section_labels.borrow_mut() = sections.into_iter().map(|s| s.label).collect();
            (rows, h)
        }

        pub(super) fn invalidate_layout(&self) {
            // Pending and textures are NOT cleared here — they are independent
            // of layout and are owned per-asset across reflows.
            self.cached_width.set(-1.0);
            self.layout_h.set(0.0);
            self.rows.borrow_mut().clear();
            self.sections.borrow_mut().clear();
            self.obj().queue_resize();
        }

        /// Relayout after the model changed. Sectioned layouts reflow immediately and
        /// anchor the scroll: height changes of sections entirely above the viewport
        /// are added to the scroll offset, so visible content does not move.
        pub(super) fn relayout(&self) {
            let width = self.cached_width.get();
            let sectioned = self
                .model
                .get()
                .is_some_and(LibraryAssetModel::is_sectioned);
            if !sectioned || width < 0.0 {
                self.invalidate_layout();
                return;
            }
            let old_sections = std::mem::take(&mut *self.sections.borrow_mut());
            let old_h = self.layout_h.get();
            let scroll_y = self.anchor_scroll_y();
            self.cached_width.set(-1.0);
            let new_h = self.layout_for_width(width);
            let delta = anchor_delta(&old_sections, &self.sections.borrow(), scroll_y);
            if delta.abs() > 0.01
                && let Some(adj) = self.find_vadjustment()
            {
                // Grow/shrink `upper` alongside so the new value is not clamped to the
                // stale range; the viewport sets the same upper on its next allocation.
                let upper = (adj.upper() + f64::from(new_h - old_h)).max(adj.page_size());
                let value = (adj.value() + f64::from(delta))
                    .clamp(adj.lower(), (upper - adj.page_size()).max(adj.lower()));
                log::debug!(
                    "masonry anchor delta={:.1} value {:.0} -> {:.0}",
                    delta,
                    adj.value(),
                    value
                );
                adj.configure(
                    value,
                    adj.lower(),
                    upper,
                    adj.step_increment(),
                    adj.page_increment(),
                    adj.page_size(),
                );
            }
            self.obj().queue_resize();
            self.obj().queue_draw();
        }

        /// Canvas-relative scroll offset for anchoring, read from the adjustment
        /// itself. `viewport()` derives it from the last allocation instead, which
        /// is stale between an anchor correction and the next layout; reading the
        /// adjustment keeps several bucket loads landing in one frame correct.
        fn anchor_scroll_y(&self) -> f32 {
            let Some(adj) = self.find_vadjustment() else {
                return 0.0;
            };
            // Offset of the canvas inside the scrolled content (header rows above
            // it), which does not change with scrolling.
            let offset = self
                .obj()
                .parent()
                .and_then(|parent| super::scrolled_child(&parent))
                .and_then(|child| match child.downcast::<gtk::Viewport>() {
                    Ok(viewport) => viewport.child(),
                    Err(child) => Some(child),
                })
                .and_then(|content| {
                    self.obj()
                        .compute_point(&content, &gtk::graphene::Point::zero())
                })
                .map_or(0.0, |origin| origin.y());
            adj.value() as f32 - offset
        }

        /// Placeholder blocks for every unloaded section (painted under the tiles).
        fn paint_section_chrome(
            &self,
            snapshot: &gtk::Snapshot,
            sctx: &SnapshotCtx<'_>,
            viewport: &SnapshotViewport,
            canvas_w: f32,
        ) {
            let sections = self.sections.borrow();
            if sections.is_empty() {
                return;
            }
            let gap = self.border_gap.get();
            for i in sections_overlapping(&sections, viewport.top, viewport.bottom) {
                let section = &sections[i];
                if !section.loaded {
                    let top = section.content_top() + gap;
                    let h = (section.bottom() - top - gap).max(0.0);
                    let w = (canvas_w - 2.0 * gap).max(0.0);
                    snapshot.append_color(&sctx.placeholder, &Rect::new(gap, top, w, h));
                }
            }
        }

        /// Month labels, painted over the tiles. Each label is sticky: it pins to
        /// the viewport top while its section scrolls past, until the next
        /// section's label pushes it out of the way.
        fn paint_section_labels(
            &self,
            snapshot: &gtk::Snapshot,
            viewport: &SnapshotViewport,
            canvas_w: f32,
        ) {
            let sections = self.sections.borrow();
            if sections.is_empty() {
                return;
            }
            let labels = self.section_labels.borrow();
            let gap = self.border_gap.get();
            let widget = self.obj();
            let color = widget.color();
            // Backdrop for a pinned label floating over photos: dark in dark
            // themes, light in light ones (derived from the foreground colour).
            let dark_theme = color.red() + color.green() + color.blue() > 1.5;
            let backdrop = if dark_theme {
                gdk4::RGBA::new(0.07, 0.07, 0.07, 0.82)
            } else {
                gdk4::RGBA::new(0.98, 0.98, 0.98, 0.82)
            };
            for i in sections_overlapping(&sections, viewport.top, viewport.bottom) {
                let section = &sections[i];
                if section.label_h <= 0.0 {
                    continue;
                }
                let Some(label) = labels.get(i) else {
                    continue;
                };
                let y = viewport
                    .top
                    .max(section.top)
                    .min(section.bottom() - section.label_h);
                if y > section.top {
                    snapshot.append_color(&backdrop, &Rect::new(0.0, y, canvas_w, section.label_h));
                }
                let layout = widget.create_pango_layout(Some(label));
                let attrs = gtk::pango::AttrList::new();
                attrs.insert(gtk::pango::AttrInt::new_weight(gtk::pango::Weight::Bold));
                attrs.insert(gtk::pango::AttrFloat::new_scale(1.15));
                layout.set_attributes(Some(&attrs));
                let (_, text_h) = layout.pixel_size();
                snapshot.save();
                snapshot.translate(&gtk::graphene::Point::new(
                    12.0 + gap,
                    y + (section.label_h - text_h as f32) * 0.5,
                ));
                snapshot.append_layout(&layout, &color);
                snapshot.restore();
            }
        }

        /// Tell the demand handler which unloaded sections intersect the viewport band
        /// (the same `VIEWPORTS_BEHIND`/`VIEWPORTS_AHEAD` margins as thumbnails), once
        /// per change. Deferred to idle so the handler never runs inside a snapshot.
        fn report_section_demand(&self, model: &LibraryAssetModel, viewport: &SnapshotViewport) {
            let Some(handler) = self.section_demand_handler.borrow().clone() else {
                return;
            };
            let wanted: Vec<usize> = {
                let sections = self.sections.borrow();
                if sections.is_empty() {
                    return;
                }
                sections_overlapping(&sections, viewport.band_top, viewport.band_bottom)
                    .filter(|&i| !sections[i].loaded)
                    .collect()
            };
            let report = (model.sections_epoch(), wanted);
            if self.last_demand.borrow().as_ref() == Some(&report) {
                return;
            }
            *self.last_demand.borrow_mut() = Some(report.clone());
            glib::idle_add_local_once(move || handler(report.1));
        }

        fn snapshot_ctx<'a>(
            &'a self,
            model: &'a LibraryAssetModel,
            cache: &'a Arc<ThumbnailCache>,
        ) -> SnapshotCtx<'a> {
            SnapshotCtx {
                model,
                cache,
                placeholder: placeholder_color(),
                select_tint: gdk4::RGBA::new(0.30, 0.55, 0.95, 0.35),
                selection: self.selection.get(),
            }
        }

        fn snapshot_viewport(&self) -> SnapshotViewport {
            let (scroll_y, viewport_h) = self.viewport();
            SnapshotViewport {
                scroll_y,
                center: scroll_y + viewport_h * 0.5,
                top: scroll_y,
                bottom: scroll_y + viewport_h,
                band_top: scroll_y - viewport_h * VIEWPORTS_BEHIND,
                band_bottom: scroll_y + viewport_h * (1.0 + VIEWPORTS_AHEAD),
            }
        }

        fn paint_snapshot_rows(
            &self,
            snapshot: &gtk::Snapshot,
            sctx: &SnapshotCtx<'_>,
            rows: &[LaidRow],
            viewport: &SnapshotViewport,
        ) -> (SnapshotStats, Vec<LoadRequest>) {
            let mut stats = SnapshotStats::default();
            let mut to_load = Vec::new();
            let start_idx = first_row_at_or_after(rows, viewport.band_top);

            for row in rows[start_idx..]
                .iter()
                .take_while(|row| row.y <= viewport.band_bottom)
            {
                self.paint_row(snapshot, sctx, row, viewport, &mut stats, &mut to_load);
            }
            (stats, to_load)
        }

        fn paint_row(
            &self,
            snapshot: &gtk::Snapshot,
            sctx: &SnapshotCtx<'_>,
            row: &LaidRow,
            viewport: &SnapshotViewport,
            stats: &mut SnapshotStats,
            to_load: &mut Vec<LoadRequest>,
        ) {
            let row_in_viewport = row.y + row.h > viewport.top && row.y < viewport.bottom;
            // Auto quality compares against the thumbnail's pixel edge, so feed it
            // device pixels: logical row height × the monitor's scale factor.
            let scale = self.obj().scale_factor().max(1) as f32;
            let bucket = bucket_for_row_height(row.h * scale, self.quality.get());

            for it in &row.items {
                stats.record(self.paint_item(snapshot, sctx, row, it, row_in_viewport));
                self.queue_load_if_needed(sctx.model, it, row, viewport.center, bucket, to_load);
            }
        }

        fn paint_item(
            &self,
            snapshot: &gtk::Snapshot,
            sctx: &SnapshotCtx<'_>,
            row: &LaidRow,
            it: &LaidItem,
            row_in_viewport: bool,
        ) -> PaintResult {
            let asset = sctx
                .model
                .item(it.asset_index)
                .and_downcast::<AssetObject>();
            let rect = Rect::new(it.x, row.y, it.w, row.h);

            let Some(asset) = asset else {
                snapshot.append_color(&sctx.placeholder, &rect);
                return PaintResult::Miss;
            };

            let asset_id = asset.property::<String>("id");
            let local_path = asset.property::<String>("local-path");
            let scale = self.obj().scale_factor().max(1) as f32;
            let bucket =
                bucket_for_asset(&asset_id, &local_path, row.h * scale, self.quality.get());
            let cached = cached_texture(sctx.cache, &asset_id, bucket, row_in_viewport);

            let clipped = CORNER_RADIUS > 0.0;
            if clipped {
                let corner = Size::new(CORNER_RADIUS, CORNER_RADIUS);
                let rounded = RoundedRect::new(rect, corner, corner, corner, corner);
                snapshot.push_rounded_clip(&rounded);
            }

            let result = if let Some(tex) = cached.as_ref() {
                snapshot.append_texture(tex, &rect);
                PaintResult::Hit
            } else {
                snapshot.append_color(&sctx.placeholder, &rect);
                PaintResult::Miss
            };

            let selected = sctx
                .selection
                .map(|s| s.is_selected(it.asset_index))
                .unwrap_or(false);
            if selected {
                snapshot.append_color(&sctx.select_tint, &rect);
            }

            self.paint_badges(snapshot, &asset, it, row, selected);

            if clipped {
                snapshot.pop();
            }
            result
        }

        /// Overlays on top of the thumbnail: select-mode checkbox, video play badge,
        /// stack badge, and the favorite heart.
        fn paint_badges(
            &self,
            snapshot: &gtk::Snapshot,
            asset: &AssetObject,
            it: &LaidItem,
            row: &LaidRow,
            selected: bool,
        ) {
            if self.select_mode.get() {
                self.paint_checkbox(snapshot, it.x, row.y, row.h, selected);
            }
            let is_video = asset
                .property::<String>("asset-type")
                .eq_ignore_ascii_case("VIDEO");
            if is_video {
                self.paint_video_badge(snapshot, it.x, row.y, it.w, row.h);
            }
            if asset.is_stack() {
                let cell = Rect::new(it.x, row.y, it.w, row.h);
                self.paint_stack_badge(snapshot, asset.stack_count(), &cell);
            }
            if asset.is_favorite() {
                self.paint_favorite_badge(snapshot, it.x + it.w, row.y + row.h, row.h);
            }
        }

        /// Paint the stack icon and member count as a pill in the top-right corner of
        /// a stack tile, on the same dark backing as the video badge.
        fn paint_stack_badge(&self, snapshot: &gtk::Snapshot, count: u32, cell: &Rect) {
            let icon = self
                .stack_icon
                .get_or_init(|| resolve_symbolic_icon(&self.obj(), "mimick-stack-symbolic"));
            let Some(symbolic) = icon.dynamic_cast_ref::<gtk::SymbolicPaintable>() else {
                return;
            };
            let icon_size = stack_badge_icon_size(cell.height());
            let layout = self.obj().create_pango_layout(Some(&count.to_string()));
            let attrs = gtk::pango::AttrList::new();
            attrs.insert(gtk::pango::AttrInt::new_weight(gtk::pango::Weight::Bold));
            attrs.insert(gtk::pango::AttrSize::new_size_absolute(
                (icon_size * 0.8 * gtk::pango::SCALE as f32) as i32,
            ));
            layout.set_attributes(Some(&attrs));
            let (text_w, text_h) = layout.pixel_size();
            let Some(badge) = stack_badge_geometry(cell.width(), cell.height(), text_w as f32)
            else {
                return;
            };

            let (x0, y0) = (cell.x(), cell.y());
            let pill = Rect::new(
                x0 + badge.pill.0,
                y0 + badge.pill.1,
                badge.pill.2,
                badge.pill.3,
            );
            let radius = Size::new(badge.pill.3 * 0.5, badge.pill.3 * 0.5);
            snapshot.push_rounded_clip(&RoundedRect::new(pill, radius, radius, radius, radius));
            snapshot.append_color(&gdk4::RGBA::new(0.0, 0.0, 0.0, 0.55), &pill);
            snapshot.pop();

            let white = gdk4::RGBA::new(1.0, 1.0, 1.0, 0.95);
            snapshot.save();
            snapshot.translate(&gtk::graphene::Point::new(
                x0 + badge.icon.0,
                y0 + badge.icon.1,
            ));
            symbolic.snapshot_symbolic(
                snapshot.upcast_ref::<gdk4::Snapshot>(),
                badge.icon_size as f64,
                badge.icon_size as f64,
                &[white],
            );
            snapshot.restore();

            if let Some((text_x, text_center_y)) = badge.text {
                snapshot.save();
                snapshot.translate(&gtk::graphene::Point::new(
                    x0 + text_x,
                    y0 + text_center_y - text_h as f32 * 0.5,
                ));
                snapshot.append_layout(&layout, &white);
                snapshot.restore();
            }
        }

        /// Paint the video play-icon badge centred on the cell.
        ///
        /// A semi-transparent dark circle is drawn behind the icon for contrast,
        /// then the cached `mimick-video-symbolic` icon is rendered on top.
        fn paint_video_badge(
            &self,
            snapshot: &gtk::Snapshot,
            cell_x: f32,
            cell_y: f32,
            cell_w: f32,
            cell_h: f32,
        ) {
            let icon = self
                .video_icon
                .get_or_init(|| resolve_video_icon(&self.obj()));

            let icon_size = (cell_h * 0.22).clamp(16.0, 48.0);
            let bg_size = icon_size * 1.6;

            // Centre the background circle.
            let bg_x = cell_x + (cell_w - bg_size) * 0.5;
            let bg_y = cell_y + (cell_h - bg_size) * 0.5;
            let bg_rect = Rect::new(bg_x, bg_y, bg_size, bg_size);
            let half = bg_size * 0.5;
            let corner = Size::new(half, half);
            let rounded = RoundedRect::new(bg_rect, corner, corner, corner, corner);

            let bg_color = gdk4::RGBA::new(0.0, 0.0, 0.0, 0.55);
            snapshot.push_rounded_clip(&rounded);
            snapshot.append_color(&bg_color, &bg_rect);
            snapshot.pop();

            // Centre the icon inside the circle.
            let icon_x = cell_x + (cell_w - icon_size) * 0.5;
            let icon_y = cell_y + (cell_h - icon_size) * 0.5;
            snapshot.save();
            snapshot.translate(&gtk::graphene::Point::new(icon_x, icon_y));
            icon.snapshot(
                snapshot.upcast_ref::<gdk4::Snapshot>(),
                icon_size as f64,
                icon_size as f64,
            );
            snapshot.restore();
        }

        /// Paint a white heart in the bottom-right corner (`right`, `bottom`) of a
        /// favorited tile. A dark copy offset by one pixel keeps it legible on bright photos.
        fn paint_favorite_badge(
            &self,
            snapshot: &gtk::Snapshot,
            right: f32,
            bottom: f32,
            cell_h: f32,
        ) {
            let icon = self
                .heart_icon
                .get_or_init(|| resolve_symbolic_icon(&self.obj(), "mimick-heart-filled-symbolic"));
            let Some(symbolic) = icon.dynamic_cast_ref::<gtk::SymbolicPaintable>() else {
                return;
            };
            let size = (cell_h * 0.12).clamp(14.0, 22.0);
            let margin = (size * 0.4).max(6.0);
            let shadow = gdk4::RGBA::new(0.0, 0.0, 0.0, 0.45);
            let white = gdk4::RGBA::new(1.0, 1.0, 1.0, 0.95);
            for (offset, color) in [(1.0, shadow), (0.0, white)] {
                snapshot.save();
                snapshot.translate(&gtk::graphene::Point::new(
                    right - margin - size + offset,
                    bottom - margin - size + offset,
                ));
                symbolic.snapshot_symbolic(
                    snapshot.upcast_ref::<gdk4::Snapshot>(),
                    size as f64,
                    size as f64,
                    &[color],
                );
                snapshot.restore();
            }
        }

        /// Paint a checkbox indicator in the top-left corner of a tile.
        fn paint_checkbox(
            &self,
            snapshot: &gtk::Snapshot,
            cell_x: f32,
            cell_y: f32,
            cell_h: f32,
            selected: bool,
        ) {
            let box_size = (cell_h * 0.14).clamp(16.0, 26.0);
            let margin = 6.0_f32;
            let bx = cell_x + margin;
            let by = cell_y + margin;
            if selected {
                self.paint_checked_box(snapshot, bx, by, box_size);
            } else {
                Self::paint_unchecked_box(snapshot, bx, by, box_size);
            }
        }

        /// Accent-filled square with a checkmark icon.
        fn paint_checked_box(&self, snapshot: &gtk::Snapshot, bx: f32, by: f32, box_size: f32) {
            let r = 4.0_f32;
            let corner = Size::new(r, r);
            let rect = Rect::new(bx, by, box_size, box_size);
            let rounded = RoundedRect::new(rect, corner, corner, corner, corner);
            snapshot.push_rounded_clip(&rounded);
            snapshot.append_color(&accent_bg_color(), &rect);
            snapshot.pop();

            let icon = self
                .check_icon
                .get_or_init(|| resolve_symbolic_icon(&self.obj(), "object-select-symbolic"));
            let inset = 3.0_f32;
            let icon_size = box_size - inset * 2.0;
            snapshot.save();
            snapshot.translate(&gtk::graphene::Point::new(bx + inset, by + inset));
            icon.snapshot(
                snapshot.upcast_ref::<gdk4::Snapshot>(),
                icon_size as f64,
                icon_size as f64,
            );
            snapshot.restore();
        }

        /// Bordered empty square (outline only).
        fn paint_unchecked_box(snapshot: &gtk::Snapshot, bx: f32, by: f32, box_size: f32) {
            let r = 4.0_f32;
            let border_w = 2.0_f32;
            let corner = Size::new(r, r);
            let outer_rect = Rect::new(bx, by, box_size, box_size);
            let outer_round = RoundedRect::new(outer_rect, corner, corner, corner, corner);

            snapshot.push_rounded_clip(&outer_round);
            snapshot.append_color(&gdk4::RGBA::new(1.0, 1.0, 1.0, 0.8), &outer_rect);

            let ir = (r - border_w).max(0.0);
            let ic = Size::new(ir, ir);
            let inner = Rect::new(
                bx + border_w,
                by + border_w,
                box_size - border_w * 2.0,
                box_size - border_w * 2.0,
            );
            let inner_round = RoundedRect::new(inner, ic, ic, ic, ic);
            snapshot.push_rounded_clip(&inner_round);
            snapshot.append_color(&gdk4::RGBA::new(0.0, 0.0, 0.0, 0.3), &inner);
            snapshot.pop();
            snapshot.pop();
        }

        fn queue_load_if_needed(
            &self,
            model: &LibraryAssetModel,
            it: &LaidItem,
            row: &LaidRow,
            viewport_center: f32,
            bucket: ThumbnailSize,
            to_load: &mut Vec<LoadRequest>,
        ) {
            let Some((asset_id, local_path, is_local_only)) = load_target_for(model, it) else {
                return;
            };
            let lookup_bucket = if is_local_only {
                ThumbnailSize::Thumbnail
            } else {
                bucket
            };
            if self.load_is_blocked(&asset_id, lookup_bucket) {
                return;
            }
            let mut pending = self.pending.borrow_mut();
            if pending.contains(&asset_id) {
                return;
            }
            pending.insert(asset_id.to_string());
            let cell_center = row.y + row.h * 0.5;
            let priority = (cell_center - viewport_center).abs();
            to_load.push(LoadRequest {
                priority,
                asset_id: asset_id.to_string(),
                bucket,
                local_path,
                is_local: is_local_only,
            });
        }

        fn load_is_blocked(&self, asset_id: &str, bucket: ThumbnailSize) -> bool {
            self.failed.borrow().contains(asset_id)
                || self
                    .cache
                    .get()
                    .is_none_or(|cache| cache.peek_cached(asset_id, bucket).is_some())
        }

        fn finish_snapshot(
            &self,
            cache: &ThumbnailCache,
            viewport: &SnapshotViewport,
            stats: &SnapshotStats,
            to_load: &mut Vec<LoadRequest>,
        ) {
            to_load.sort_by(|a, b| {
                a.priority
                    .partial_cmp(&b.priority)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            self.log_snapshot(cache, viewport, stats, to_load.len());
            for request in to_load.drain(..) {
                self.spawn_load(request);
            }
        }

        fn log_snapshot(
            &self,
            cache: &ThumbnailCache,
            viewport: &SnapshotViewport,
            stats: &SnapshotStats,
            queued: usize,
        ) {
            let (cache_n, cache_bytes, cache_max, cache_evicts) = cache.cache_stats();
            log::debug!(
                "masonry snapshot y={:.0} painted={} hits={} misses={} pending={} queued={} cache={}/{}/{}MB evicts={}",
                viewport.scroll_y,
                stats.painted,
                stats.hits,
                stats.misses,
                self.pending.borrow().len(),
                queued,
                cache_n,
                cache_bytes / (1024 * 1024),
                cache_max / (1024 * 1024),
                cache_evicts,
            );
        }

        pub(super) fn viewport(&self) -> (f32, f32) {
            let Some(adj) = self.find_vadjustment() else {
                return (0.0, self.obj().height() as f32);
            };
            let scroll_y = self
                .obj()
                .parent()
                .and_then(|parent| super::scrolled_child(&parent))
                .and_then(|child| {
                    self.obj()
                        .compute_point(&child, &gtk::graphene::Point::new(0.0, 0.0))
                })
                .map(|origin| -origin.y())
                .unwrap_or(adj.value() as f32);
            (scroll_y, adj.page_size() as f32)
        }

        pub(super) fn find_vadjustment(&self) -> Option<gtk::Adjustment> {
            if let Some(adj) = self.vadjustment.borrow().clone() {
                return Some(adj);
            }
            let mut node: Option<gtk::Widget> = self.obj().parent();
            while let Some(w) = node {
                if let Some(sw) = w.downcast_ref::<gtk::ScrolledWindow>() {
                    let adj = sw.vadjustment();
                    let weak = self.obj().downgrade();
                    adj.connect_value_changed(move |_| {
                        if let Some(canvas) = weak.upgrade() {
                            canvas.queue_draw();
                        }
                    });
                    *self.vadjustment.borrow_mut() = Some(adj.clone());
                    return Some(adj);
                }
                node = w.parent();
            }
            None
        }

        fn spawn_load(&self, request: LoadRequest) {
            let Some(cache) = self.cache.get().cloned() else {
                return;
            };
            let Some(model) = self.model.get().cloned() else {
                return;
            };
            let widget = self.obj().clone();
            log::trace!(
                "masonry spawn_load id={} bucket={:?} local={}",
                request.asset_id,
                request.bucket,
                request.is_local
            );
            glib::MainContext::default().spawn_local(async move {
                let asset_id = request.asset_id.clone();
                let result = load_masonry_asset(&cache, &request).await;
                finish_masonry_load(&widget, &model, &asset_id, result);
            });
        }
    }
}

use super::load::{collect_ratios, load_with_fallback, propagate_dimensions};

glib::wrapper! {
    pub struct MasonryCanvas(ObjectSubclass<imp::MasonryCanvas>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl imp::SnapshotStats {
    fn record(&mut self, result: imp::PaintResult) {
        match result {
            imp::PaintResult::Hit => self.hits += 1,
            imp::PaintResult::Miss => self.misses += 1,
        }
        self.painted += 1;
    }
}

/// Get the current theme accent background colour from libadwaita.
fn scrolled_child(widget: &gtk::Widget) -> Option<gtk::Widget> {
    let mut node = Some(widget.clone());
    while let Some(current) = node {
        if let Some(scrolled) = current.downcast_ref::<gtk::ScrolledWindow>() {
            return scrolled.child();
        }
        node = current.parent();
    }
    None
}

fn accent_bg_color() -> gdk4::RGBA {
    let accent = libadwaita::StyleManager::default().accent_color();
    accent.to_rgba()
}

fn placeholder_color() -> gdk4::RGBA {
    if libadwaita::StyleManager::default().is_dark() {
        gdk4::RGBA::new(0.20, 0.20, 0.22, 1.0)
    } else {
        gdk4::RGBA::new(0.90, 0.90, 0.92, 1.0)
    }
}

/// Resolve the video badge icon from the icon theme. The result is cached
/// per-canvas via a `OnceCell` so the theme lookup only happens once.
/// Stack badge icon edge for a row of height `cell_h`.
fn stack_badge_icon_size(cell_h: f32) -> f32 {
    (cell_h * 0.11).clamp(12.0, 18.0)
}

/// Where the stack badge goes inside a tile, relative to the tile's top-left corner.
#[derive(Debug, Clone, Copy, PartialEq)]
struct StackBadge {
    /// Backing pill `(x, y, w, h)`.
    pill: (f32, f32, f32, f32),
    /// Icon top-left `(x, y)`.
    icon: (f32, f32),
    icon_size: f32,
    /// Count text `(x, vertical centre)`; `None` when only the icon fits.
    text: Option<(f32, f32)>,
}

/// Lay out the stack badge (icon + member count, top-right corner) for a
/// `cell_w` × `cell_h` tile whose count text is `text_w` wide. Narrow tiles drop
/// the count, then the whole badge, rather than paint over their neighbours.
fn stack_badge_geometry(cell_w: f32, cell_h: f32, text_w: f32) -> Option<StackBadge> {
    let icon_size = stack_badge_icon_size(cell_h);
    let pad = (icon_size * 0.35).round();
    let gap = (icon_size * 0.25).round();
    let margin = (icon_size * 0.4).max(6.0);
    let pill_h = icon_size + pad * 2.0;
    let with_text = pad + icon_size + gap + text_w + pad * 1.2;
    let icon_only = pad * 2.0 + icon_size;
    let fits = |w: f32| w + margin * 2.0 <= cell_w && pill_h + margin * 2.0 <= cell_h;
    let (pill_w, show_text) = if fits(with_text) {
        (with_text, true)
    } else if fits(icon_only) {
        (icon_only, false)
    } else {
        return None;
    };
    let pill_x = cell_w - margin - pill_w;
    Some(StackBadge {
        pill: (pill_x, margin, pill_w, pill_h),
        icon: (pill_x + pad, margin + pad),
        icon_size,
        text: show_text.then_some((pill_x + pad + icon_size + gap, margin + pill_h * 0.5)),
    })
}

fn resolve_video_icon(widget: &MasonryCanvas) -> gdk4::Paintable {
    resolve_symbolic_icon(widget, "mimick-video-symbolic")
}

fn resolve_symbolic_icon(widget: &MasonryCanvas, icon_name: &str) -> gdk4::Paintable {
    let display = widget
        .native()
        .map(|n| n.display())
        .unwrap_or_else(|| gtk::gdk::Display::default().expect("default GDK display"));
    let theme = gtk::IconTheme::for_display(&display);
    let icon = theme.lookup_icon(
        icon_name,
        &[],
        48,
        1,
        gtk::TextDirection::None,
        gtk::IconLookupFlags::FORCE_SYMBOLIC,
    );
    icon.upcast::<gdk4::Paintable>()
}

/// `device_row_h` is in device pixels (logical row height × scale factor).
fn bucket_for_asset(
    asset_id: &str,
    local_path: &str,
    device_row_h: f32,
    quality: GridQuality,
) -> ThumbnailSize {
    if !local_path.is_empty() && asset_id.starts_with(crate::library::LOCAL_ID_PREFIX) {
        ThumbnailSize::Thumbnail
    } else {
        bucket_for_row_height(device_row_h, quality)
    }
}

fn cached_texture(
    cache: &ThumbnailCache,
    asset_id: &str,
    bucket: ThumbnailSize,
    touch_lru: bool,
) -> Option<gdk4::Texture> {
    let get = |size| {
        if touch_lru {
            cache.get_cached(asset_id, size)
        } else {
            cache.peek_cached(asset_id, size)
        }
    };
    get(bucket).or_else(|| fallback_bucket(bucket).and_then(get))
}

fn load_target_for(model: &LibraryAssetModel, it: &LaidItem) -> Option<(String, String, bool)> {
    let asset = model.item(it.asset_index).and_downcast::<AssetObject>()?;
    let asset_id = asset.property::<String>("id");
    let local_path = asset.property::<String>("local-path");
    let is_local = !local_path.is_empty() && asset_id.starts_with(crate::library::LOCAL_ID_PREFIX);
    Some((asset_id, local_path, is_local))
}

async fn load_masonry_asset(
    cache: &ThumbnailCache,
    request: &imp::LoadRequest,
) -> Result<gdk4::Texture, String> {
    let is_cancelled = || false;
    if request.is_local {
        let path = std::path::PathBuf::from(&request.local_path);
        cache
            .load_local_thumbnail_cancellable(&request.asset_id, &path, &is_cancelled)
            .await
    } else {
        load_with_fallback(cache, &request.asset_id, request.bucket, &is_cancelled).await
    }
}

fn finish_masonry_load(
    widget: &MasonryCanvas,
    model: &LibraryAssetModel,
    asset_id: &str,
    result: Result<gdk4::Texture, String>,
) {
    let imp = widget.imp();
    let dims_changed = match &result {
        Ok(tex) => {
            log::trace!(
                "masonry load OK id={} dims=({}x{})",
                asset_id,
                tex.width(),
                tex.height(),
            );
            propagate_dimensions(model, asset_id, tex)
        }
        Err(e) => {
            if e != "cancelled" {
                imp.failed.borrow_mut().insert(asset_id.to_string());

                if (e.contains("HTTP 401") || e.contains("HTTP 403"))
                    && !imp.permission_warned.get()
                {
                    imp.permission_warned.set(true);
                    if let Some(window) = widget
                        .root()
                        .and_downcast::<libadwaita::ApplicationWindow>()
                    {
                        let dialog = libadwaita::AlertDialog::builder()
                            .heading("Missing API Permissions")
                            .body("Your API key is missing permissions required to view thumbnails. Please ensure the key has 'asset.view' enabled.")
                            .build();
                        dialog.add_response("ok", "OK");
                        dialog.present(Some(&window));
                    }
                }
            }
            log::warn!("masonry load ERR id={} err={}", asset_id, e);
            false
        }
    };
    imp.pending.borrow_mut().remove(asset_id);
    if dims_changed {
        imp.relayout();
    }
    widget.queue_draw();
}

#[cfg(test)]
mod tests {
    use super::{StackBadge, stack_badge_geometry};

    fn inside(badge: &StackBadge, cell_w: f32, cell_h: f32) -> bool {
        let (x, y, w, h) = badge.pill;
        x >= 0.0 && y >= 0.0 && x + w <= cell_w && y + h <= cell_h
    }

    #[test]
    fn stack_badge_sits_in_the_top_right_corner_with_its_count() {
        let badge = stack_badge_geometry(300.0, 200.0, 10.0).expect("fits");
        assert!(inside(&badge, 300.0, 200.0));
        let (x, y, w, _) = badge.pill;
        // Same margin from the top and the right edge.
        assert!((300.0 - (x + w) - y).abs() < 1e-3);
        assert!(x > 150.0, "badge hugs the right edge, got x={x}");
        let (text_x, _) = badge.text.expect("count shown");
        assert!(text_x > badge.icon.0 + badge.icon_size);
        assert!(text_x + 10.0 <= x + w);
    }

    #[test]
    fn narrow_tiles_drop_the_count_then_the_badge() {
        let full = stack_badge_geometry(300.0, 200.0, 30.0).unwrap();
        let narrow_w = full.pill.2 + 2.0 * full.pill.1 - 1.0;
        let icon_only = stack_badge_geometry(narrow_w, 200.0, 30.0).expect("icon fits");
        assert!(icon_only.text.is_none());
        assert!(inside(&icon_only, narrow_w, 200.0));
        assert!(stack_badge_geometry(10.0, 200.0, 30.0).is_none());
        assert!(stack_badge_geometry(300.0, 10.0, 30.0).is_none());
    }

    #[test]
    fn stack_badge_scales_with_row_height_within_bounds() {
        let small = stack_badge_geometry(1000.0, 60.0, 8.0).unwrap();
        let large = stack_badge_geometry(1000.0, 600.0, 8.0).unwrap();
        assert_eq!(small.icon_size, 12.0);
        assert_eq!(large.icon_size, 18.0);
    }
}
