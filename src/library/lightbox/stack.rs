//! Lightbox extras for the Main library: the ±10 tile preload and the stack strip.
//!
//! Opening a stack tile shows the Primary at once; `fetch_stack` then fills a
//! thumbnail strip above the action bar. Picking a member swaps the displayed
//! asset in place (no grid movement), and every lightbox action targets that
//! member. Tile navigation never enters a stack and resets it to the Primary.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use futures_util::StreamExt;
use gtk::prelude::*;

use crate::api_client::{StackResponse, ThumbnailSize, is_forbidden};
use crate::library::asset_model::build_asset_objects;
use crate::library::asset_object::AssetObject;

use super::{LOCAL_ID_PREFIX, Lightbox};

/// Tiles preloaded on each side of the current one.
const PRELOAD_RADIUS: u32 = 10;
/// Preloads in flight at once; low so the current picture's own load is not starved.
const PRELOAD_CONCURRENCY: usize = 2;
/// Edge length of a strip thumbnail.
const STRIP_THUMB_SIZE: i32 = 56;

/// Grid positions to preload around `pos`: nearest first, alternating next and
/// previous, clamped to `0..total`. Never includes `pos`.
pub(super) fn preload_positions(pos: u32, total: u32, radius: u32) -> Vec<u32> {
    let mut out = Vec::new();
    for distance in 1..=radius {
        if let Some(next) = pos.checked_add(distance).filter(|p| *p < total) {
            out.push(next);
        }
        if let Some(prev) = pos.checked_sub(distance) {
            out.push(prev);
        }
    }
    out
}

/// The members of the opened stack tile and which one is on screen.
pub(super) struct StackState {
    members: Vec<AssetObject>,
    selected: usize,
}

impl StackState {
    /// `None` unless there are two or more members and the tile's own asset (the
    /// Primary, shown first) is among them.
    pub(super) fn new(tile_id: &str, members: Vec<AssetObject>) -> Option<Self> {
        if members.len() < 2 {
            return None;
        }
        let selected = members.iter().position(|m| m.id() == tile_id)?;
        Some(Self { members, selected })
    }

    pub(super) fn selected_object(&self) -> AssetObject {
        self.members[self.selected].clone()
    }

    pub(super) fn selected(&self) -> usize {
        self.selected
    }

    pub(super) fn members(&self) -> &[AssetObject] {
        &self.members
    }

    /// Whether the strip is worth showing: a stack needs two or more members.
    pub(super) fn is_stack(&self) -> bool {
        self.members.len() >= 2
    }

    /// Select member `index`; false when out of range or already selected.
    pub(super) fn select(&mut self, index: usize) -> bool {
        if index >= self.members.len() || index == self.selected {
            return false;
        }
        self.selected = index;
        true
    }

    /// Drop the selected member; the next one takes its place (the previous one
    /// when it was last). False when no members remain.
    pub(super) fn remove_selected(&mut self) -> bool {
        self.members.remove(self.selected);
        if self.members.is_empty() {
            return false;
        }
        self.selected = self.selected.min(self.members.len() - 1);
        true
    }
}

/// Horizontal thumbnail strip, hidden until a stack's members have loaded.
pub(super) struct StackStrip {
    pub(super) scroller: gtk::ScrolledWindow,
    row: gtk::Box,
    buttons: RefCell<Vec<gtk::Button>>,
}

pub(super) fn build_stack_strip() -> StackStrip {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .halign(gtk::Align::Center)
        .build();
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .child(&row)
        .visible(false)
        .css_classes(["mimick-stack-strip"])
        .build();
    StackStrip {
        scroller,
        row,
        buttons: RefCell::new(Vec::new()),
    }
}

/// Generation counters and caches the stack/preload code keeps on the lightbox.
#[derive(Default)]
pub(super) struct StackTracking {
    /// Bumped whenever the displayed tile changes or the lightbox closes; a
    /// `fetch_stack` started for an older value is dropped.
    pub(super) stack_gen: Cell<u64>,
    /// Same discipline for preloads: a stale preload never starts new work.
    pub(super) preload_gen: Cell<u64>,
    /// Tile ids with preload work in flight, so overlapping windows don't repeat it.
    pub(super) preloading: RefCell<std::collections::HashSet<String>>,
}

impl Lightbox {
    /// The displayed asset is a stack member rather than the grid tile.
    pub(super) fn in_stack(&self) -> bool {
        self.stack.borrow().is_some()
    }

    /// Forget the open stack (tile navigation, close): strip hidden, fetch dropped.
    pub(super) fn reset_stack(self: &Rc<Self>) {
        let t = &self.tracking;
        t.stack_gen.set(t.stack_gen.get().wrapping_add(1));
        *self.stack.borrow_mut() = None;
        self.publish_members();
        self.rebuild_strip();
    }

    /// Stop background work when the page is hidden.
    pub(super) fn close_stack(self: &Rc<Self>) {
        self.reset_stack();
        let t = &self.tracking;
        t.preload_gen.set(t.preload_gen.get().wrapping_add(1));
    }

    /// Mirror the members into the window so Edit Info can reach them.
    fn publish_members(&self) {
        let members = self
            .stack
            .borrow()
            .as_ref()
            .map(|s| s.members().to_vec())
            .unwrap_or_default();
        *self.ui.lightbox_members.borrow_mut() = members;
    }

    /// For a stack tile, fetch its members in the background. The Primary is
    /// already on screen; the strip appears when this lands.
    pub(super) fn begin_stack_fetch(self: &Rc<Self>, tile: &AssetObject) {
        let stack_id = tile.stack_id();
        if !tile.is_stack() || stack_id.is_empty() {
            return;
        }
        let generation = self.tracking.stack_gen.get();
        let store_generation = self.ui.buckets.borrow().generation;
        let tile = tile.clone();
        let lb = self.clone();
        glib::MainContext::default().spawn_local(async move {
            let result = lb.ui.ctx.api_client.fetch_stack(&stack_id).await;
            if lb.tracking.stack_gen.get() != generation {
                return;
            }
            match result {
                Ok(stack) => lb.apply_stack(&tile, store_generation, stack),
                // The key lacks `stack.read`: Settings explains it; stay quiet here.
                Err(err) if is_forbidden(&err) => {
                    log::debug!("Stack {stack_id}: no stack.read permission, strip hidden");
                }
                Err(err) => log::warn!("Could not load stack {stack_id}: {err}"),
            }
        });
    }

    fn apply_stack(
        self: &Rc<Self>,
        tile: &AssetObject,
        store_generation: u64,
        stack: StackResponse,
    ) {
        let tile_id = tile.id();
        let mut members = Vec::with_capacity(stack.assets.len());
        for asset in &stack.assets {
            // Member DTOs are full records: later hydration of them is a cache hit.
            self.ui
                .buckets
                .borrow_mut()
                .cache_hydrated(store_generation, asset.clone());
            if asset.id == tile_id {
                // The tile stays the Primary's object, so its heart badge follows.
                members.push(tile.clone());
            } else {
                members.extend(build_asset_objects(
                    std::slice::from_ref(asset),
                    &self.ui.ctx,
                ));
            }
        }
        let Some(state) = StackState::new(&tile_id, members) else {
            log::debug!(
                "Stack {}: fewer than two members or Primary missing",
                stack.id
            );
            return;
        };
        *self.stack.borrow_mut() = Some(state);
        self.publish_members();
        self.rebuild_strip();
    }

    /// Rebuild the strip's thumbnails from the open stack (or hide it).
    fn rebuild_strip(self: &Rc<Self>) {
        let strip = &self.strip;
        while let Some(child) = strip.row.first_child() {
            strip.row.remove(&child);
        }
        strip.buttons.borrow_mut().clear();
        let ids: Vec<String> = match self.stack.borrow().as_ref() {
            Some(state) if state.is_stack() => state.members().iter().map(|m| m.id()).collect(),
            _ => Vec::new(),
        };
        strip.scroller.set_visible(!ids.is_empty());
        for (index, id) in ids.into_iter().enumerate() {
            let picture = gtk::Picture::builder()
                .content_fit(gtk::ContentFit::Cover)
                .width_request(STRIP_THUMB_SIZE)
                .height_request(STRIP_THUMB_SIZE)
                .build();
            let button = gtk::Button::builder()
                .child(&picture)
                .focusable(false)
                .focus_on_click(false)
                .css_classes(["flat", "mimick-stack-member"])
                .build();
            let lb = self.clone();
            button.connect_clicked(move |_| lb.select_member(index));
            strip.row.append(&button);
            strip.buttons.borrow_mut().push(button);
            self.load_strip_thumbnail(picture, id);
        }
        self.sync_strip();
    }

    /// Thumbnails come from the normal thumbnail cache.
    fn load_strip_thumbnail(&self, picture: gtk::Picture, id: String) {
        let lb_ui = self.ui.clone();
        glib::MainContext::default().spawn_local(async move {
            match lb_ui
                .ctx
                .thumbnail_cache
                .load_thumbnail(&id, ThumbnailSize::Thumbnail)
                .await
            {
                Ok(texture) => picture.set_paintable(Some(&texture)),
                Err(err) => log::debug!("Strip thumbnail {id}: {err}"),
            }
        });
    }

    /// Highlight the displayed member and bring it into view.
    fn sync_strip(&self) {
        let selected = self.stack.borrow().as_ref().map(StackState::selected);
        let buttons = self.strip.buttons.borrow();
        for (index, button) in buttons.iter().enumerate() {
            if Some(index) == selected {
                button.add_css_class("mimick-stack-member-current");
            } else {
                button.remove_css_class("mimick-stack-member-current");
            }
        }
        let Some(current) = selected.and_then(|i| buttons.get(i)).cloned() else {
            return;
        };
        let scroller = self.strip.scroller.clone();
        let row = self.strip.row.clone();
        // After layout, so the button has a position within the row.
        glib::idle_add_local_once(move || {
            let Some(bounds) = current.compute_bounds(&row) else {
                return;
            };
            let adjustment = scroller.hadjustment();
            let centre = f64::from(bounds.x()) + f64::from(bounds.width()) / 2.0;
            adjustment.set_value(centre - adjustment.page_size() / 2.0);
        });
    }

    /// Show strip member `index` in place: same crossfade as navigation, no slide,
    /// no grid movement.
    fn select_member(self: &Rc<Self>, index: usize) {
        let changed = self
            .stack
            .borrow_mut()
            .as_mut()
            .is_some_and(|state| state.select(index));
        if !changed {
            return;
        }
        self.nav_dir.set(0);
        self.sync_strip();
        self.render();
    }

    /// The displayed member is gone (deleted, or archived out of the Main
    /// library): show the next one, hide the strip when the stack drops below two.
    /// The grid reloads on close, since its tile may now stand for another Primary.
    pub(super) fn drop_displayed_member(self: &Rc<Self>) {
        self.reload_on_close.set(true);
        let any_left = self
            .stack
            .borrow_mut()
            .as_mut()
            .is_some_and(StackState::remove_selected);
        if !any_left {
            self.reset_stack();
            self.ui.nav.pop();
            return;
        }
        self.publish_members();
        self.rebuild_strip();
        self.nav_dir.set(0);
        self.render();
    }

    /// Preload the tiles within ±[`PRELOAD_RADIUS`] of the current one: hydrate
    /// them and warm their preview textures. Newer navigation supersedes older
    /// preloads, which stop starting work (the item in flight finishes, as it
    /// only fills caches). Counts tiles, not stack members.
    pub(super) fn preload_neighbours(self: &Rc<Self>) {
        let t = &self.tracking;
        let generation = t.preload_gen.get().wrapping_add(1);
        t.preload_gen.set(generation);

        let model = &self.ui.grid.model;
        let items: Vec<AssetObject> =
            preload_positions(self.pos.get(), model.n_items(), PRELOAD_RADIUS)
                .into_iter()
                .filter_map(|p| model.item(p).and_downcast::<AssetObject>())
                .filter(|item| !item.id().is_empty() && !item.id().starts_with(LOCAL_ID_PREFIX))
                .collect();
        if items.is_empty() {
            return;
        }
        let lb = self.clone();
        glib::MainContext::default().spawn_local(async move {
            futures_util::stream::iter(items)
                .map(|item| {
                    let lb = lb.clone();
                    async move { lb.preload_item(&item, generation).await }
                })
                .buffer_unordered(PRELOAD_CONCURRENCY)
                .collect::<Vec<()>>()
                .await;
        });
    }

    async fn preload_item(&self, item: &AssetObject, generation: u64) {
        let id = item.id();
        if self.tracking.preload_gen.get() != generation {
            return;
        }
        if !self.tracking.preloading.borrow_mut().insert(id.clone()) {
            return;
        }
        if let Err(err) = crate::library::hydration::hydrate_object(&self.ui, item).await {
            log::debug!("Preload: could not hydrate {id}: {err}");
        }
        // Not cancellable: an abandoned load would hand its "cancelled" error to a
        // later load of the same preview waiting on it.
        let _ = self
            .ui
            .ctx
            .thumbnail_cache
            .load_thumbnail(&id, ThumbnailSize::Preview)
            .await;
        self.tracking.preloading.borrow_mut().remove(&id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::asset_object::AssetInit;

    fn member(id: &str) -> AssetObject {
        AssetObject::new(AssetInit {
            id,
            filename: "a.jpg",
            mime_type: "image/jpeg",
            created_at: "",
            asset_type: "IMAGE",
            sync_state: 0,
            thumbhash: None,
            width: 0,
            height: 0,
            is_favorite: false,
            is_archived: false,
        })
    }

    fn state(ids: &[&str], tile: &str) -> Option<StackState> {
        StackState::new(tile, ids.iter().map(|id| member(id)).collect())
    }

    #[test]
    fn preload_window_alternates_nearest_first() {
        assert_eq!(preload_positions(5, 100, 2), vec![6, 4, 7, 3]);
    }

    #[test]
    fn preload_window_clamps_to_both_ends() {
        assert_eq!(preload_positions(0, 3, 10), vec![1, 2]);
        assert_eq!(preload_positions(2, 3, 10), vec![1, 0]);
        assert_eq!(preload_positions(0, 1, 10), Vec::<u32>::new());
        assert_eq!(preload_positions(50, 100, 10).len(), 20);
    }

    #[test]
    fn stack_needs_two_members_and_the_tile_among_them() {
        assert!(state(&["a"], "a").is_none());
        assert!(state(&["a", "b"], "x").is_none());
        let s = state(&["a", "b"], "a").unwrap();
        assert_eq!(s.selected(), 0);
        assert_eq!(s.selected_object().id(), "a");
    }

    #[test]
    fn selecting_a_member_changes_the_displayed_object() {
        let mut s = state(&["a", "b", "c"], "a").unwrap();
        assert!(s.select(2));
        assert_eq!(s.selected_object().id(), "c");
        assert!(!s.select(2), "already selected");
        assert!(!s.select(3), "out of range");
    }

    #[test]
    fn removing_the_selected_member_advances_to_the_next() {
        let mut s = state(&["a", "b", "c"], "a").unwrap();
        assert!(s.remove_selected());
        assert_eq!(s.selected_object().id(), "b");
        assert!(s.select(1));
        assert!(s.remove_selected());
        assert_eq!(s.selected_object().id(), "b", "last member steps back");
    }

    #[test]
    fn stack_below_two_members_is_no_longer_a_stack() {
        let mut s = state(&["a", "b"], "a").unwrap();
        assert!(s.is_stack());
        assert!(s.remove_selected());
        assert!(!s.is_stack());
        assert_eq!(s.selected_object().id(), "b");
        assert!(!s.remove_selected(), "nothing left");
    }
}
