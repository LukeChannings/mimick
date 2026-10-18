//! Custom `gio::ListModel` backing the library grid.
//!
//! Replaces the previous `gio::ListStore`-of-`AssetObject` mirror. The model
//! owns a `Vec<AssetObject>` reconciled from `LibraryState.assets`.
//! `extend` is used for append-pagination (server-side date sort lets us emit
//! a precise `items_changed(prev_n, 0, added)` so the visible viewport doesn't
//! rebind); `reset` is used for source switches.
//!
//! The Main library uses the model in **sectioned** mode instead: every bucket
//! reserves `count` slots up front (so flat positions are stable across bucket
//! loads, in any order) and unloaded slots answer with a shared placeholder
//! object whose id is empty. `fill_section` swaps a bucket's placeholders for
//! real tiles. The canvas reads [`GridSection`]s to lay out month sections.

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashMap;
use std::ops::Range;

use gtk::gio;
use gtk::gio::prelude::ListModelExt;
use gtk::glib;
use gtk::glib::prelude::{Cast, ObjectExt, StaticType};
use gtk::subclass::prelude::{ObjectImpl, ObjectSubclass, ObjectSubclassIsExt};

use crate::api_client::{LibraryAsset, TileAsset, normalize_checksum_to_hex};
use crate::app_context::AppContext;
use crate::library::asset_object::{AssetInit, AssetObject};

/// One month section of the sectioned (Main library) model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridSection {
    /// Stable identity (the opaque bucket key, or a synthetic key).
    pub key: String,
    /// Inline label, e.g. "June 2025".
    pub label: String,
    /// Slots reserved for this section in the flat model.
    pub count: u32,
    /// Whether the slots hold real tiles (exact layout) or placeholders (estimate).
    pub loaded: bool,
}

mod imp {
    use super::{
        AssetObject, Cast, Cell, GridSection, ObjectImpl, ObjectSubclass, OnceCell, RefCell,
        StaticType, gio, glib,
    };
    use gio::subclass::prelude::ListModelImpl;

    #[derive(Default)]
    pub struct LibraryAssetModel {
        /// `None` = an unloaded slot of a sectioned model (served as `placeholder`).
        pub items: RefCell<Vec<Option<AssetObject>>>,
        /// `Some` in sectioned (Main library) mode.
        pub sections: RefCell<Option<Vec<GridSection>>>,
        /// Bumped on every `reset_sections`, so consumers can tell a new bucket list apart.
        pub epoch: Cell<u64>,
        pub placeholder: OnceCell<AssetObject>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LibraryAssetModel {
        const NAME: &str = "MimickLibraryAssetModel";
        type Type = super::LibraryAssetModel;
        type Interfaces = (gio::ListModel,);
    }

    impl ObjectImpl for LibraryAssetModel {}

    impl ListModelImpl for LibraryAssetModel {
        fn item_type(&self) -> glib::Type {
            AssetObject::static_type()
        }

        fn n_items(&self) -> u32 {
            self.items.borrow().len() as u32
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            let items = self.items.borrow();
            let slot = items.get(position as usize)?;
            Some(match slot {
                Some(object) => object.clone().upcast(),
                None => self
                    .placeholder
                    .get_or_init(AssetObject::placeholder)
                    .clone()
                    .upcast(),
            })
        }
    }
}

glib::wrapper! {
    pub struct LibraryAssetModel(ObjectSubclass<imp::LibraryAssetModel>) @implements gio::ListModel;
}

impl Default for LibraryAssetModel {
    fn default() -> Self {
        Self::new()
    }
}

/// Flat index of section `index`'s first slot.
fn section_offset(sections: &[GridSection], index: usize) -> u32 {
    sections.iter().take(index).map(|s| s.count).sum()
}

impl LibraryAssetModel {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Replace all slots (leaving sectioned mode) and emit a full reset.
    fn replace_flat(&self, objects: Vec<AssetObject>) {
        let prev_n = self.imp().items.borrow().len() as u32;
        let new_n = objects.len() as u32;
        *self.imp().sections.borrow_mut() = None;
        *self.imp().items.borrow_mut() = objects.into_iter().map(Some).collect();
        self.items_changed(0, prev_n, new_n);
    }

    /// Replace all items and emit a `items_changed(0, prev, new)` reset.
    /// Use when the caller can't promise that existing positions are stable
    /// (source switch, dedup that affected the head).
    pub fn reset(&self, ctx: &AppContext, assets: &[LibraryAsset]) {
        self.replace_flat(build_asset_objects(assets, ctx));
    }

    /// Append-style update for paginated loads. Server order keeps existing
    /// positions stable, so we emit a precise tail-only
    /// `items_changed(prev_n, 0, added)`; if the list shrank (dedup) we fall
    /// back to a full reset.
    pub fn extend(&self, ctx: &AppContext, assets: &[LibraryAsset]) {
        let prev_n = self.imp().items.borrow().len() as u32;
        let new_items = build_asset_objects(assets, ctx);
        let new_n = new_items.len() as u32;

        *self.imp().sections.borrow_mut() = None;
        *self.imp().items.borrow_mut() = new_items.into_iter().map(Some).collect();

        if new_n >= prev_n {
            let added = new_n - prev_n;
            if added > 0 {
                self.items_changed(prev_n, 0, added);
            }
        } else {
            self.items_changed(0, prev_n, new_n);
        }
    }

    /// Replace all items with pre-built `AssetObject`s and emit a full reset.
    /// Used by the staging view, which builds local-only objects from file paths
    /// rather than going through the `LibraryAsset` pipeline.
    pub fn reset_with_objects(&self, objects: Vec<AssetObject>) {
        self.replace_flat(objects);
    }

    /// Append `AssetObject`s with a tail-only `items_changed`, leaving the viewport alone.
    /// Used by the staging view drop handler.
    pub fn append_objects(&self, objects: &[AssetObject]) {
        if objects.is_empty() {
            return;
        }
        let prev_n = {
            let mut items = self.imp().items.borrow_mut();
            let prev = items.len() as u32;
            items.extend(objects.iter().cloned().map(Some));
            prev
        };
        self.items_changed(prev_n, 0, objects.len() as u32);
    }

    /// Enter sectioned mode. Sections given objects are loaded (their count is the
    /// object count); the rest reserve `count` placeholder slots.
    pub fn reset_sections(&self, sections: Vec<(GridSection, Option<Vec<AssetObject>>)>) {
        let prev_n = self.imp().items.borrow().len() as u32;
        let mut items = Vec::new();
        let mut specs = Vec::with_capacity(sections.len());
        for (mut spec, objects) in sections {
            match objects {
                Some(objects) => {
                    spec.count = objects.len() as u32;
                    spec.loaded = true;
                    items.extend(objects.into_iter().map(Some));
                }
                None => {
                    spec.loaded = false;
                    items.extend(std::iter::repeat_n(None, spec.count as usize));
                }
            }
            specs.push(spec);
        }
        let new_n = items.len() as u32;
        *self.imp().items.borrow_mut() = items;
        *self.imp().sections.borrow_mut() = Some(specs);
        self.imp().epoch.set(self.imp().epoch.get().wrapping_add(1));
        self.items_changed(0, prev_n, new_n);
    }

    /// Swap section `index`'s slots for `objects` and mark it loaded. Positions
    /// before the section never move; later ones shift only if the real tile count
    /// differs from the announced one. Returns false outside sectioned mode.
    pub fn fill_section(&self, index: usize, objects: Vec<AssetObject>) -> bool {
        let (offset, old_count) = {
            let mut sections = self.imp().sections.borrow_mut();
            let Some(sections) = sections.as_mut() else {
                return false;
            };
            let Some(spec) = sections.get(index) else {
                return false;
            };
            let old_count = spec.count;
            let offset = section_offset(sections, index);
            sections[index].count = objects.len() as u32;
            sections[index].loaded = true;
            (offset, old_count)
        };
        let new_count = objects.len() as u32;
        {
            let mut items = self.imp().items.borrow_mut();
            let start = offset as usize;
            items.splice(
                start..start + old_count as usize,
                objects.into_iter().map(Some),
            );
        }
        self.items_changed(offset, old_count, new_count);
        true
    }

    /// Drop loaded tiles whose id (or remote id) is in `ids` from a sectioned model,
    /// shrinking their sections. Returns false outside sectioned mode.
    pub fn remove_ids(&self, ids: &[String]) -> bool {
        let prev_n = {
            let mut sections = self.imp().sections.borrow_mut();
            let Some(sections) = sections.as_mut() else {
                return false;
            };
            let mut items = self.imp().items.borrow_mut();
            let prev_n = items.len() as u32;
            let mut kept = Vec::with_capacity(items.len());
            let mut slots = std::mem::take(&mut *items).into_iter();
            for spec in sections.iter_mut() {
                let mut count = 0;
                for slot in slots.by_ref().take(spec.count as usize) {
                    let removed = slot.as_ref().is_some_and(|object| {
                        ids.contains(&object.id()) || ids.contains(&object.remote_id())
                    });
                    if !removed {
                        kept.push(slot);
                        count += 1;
                    }
                }
                spec.count = count;
            }
            *items = kept;
            prev_n
        };
        let new_n = self.imp().items.borrow().len() as u32;
        self.items_changed(0, prev_n, new_n);
        true
    }

    /// Month sections in sectioned (Main library) mode, `None` for flat sources.
    pub fn sections(&self) -> Option<Vec<GridSection>> {
        self.imp().sections.borrow().clone()
    }

    /// Whether the model is in sectioned (Main library) mode.
    pub fn is_sectioned(&self) -> bool {
        self.imp().sections.borrow().is_some()
    }

    /// Changes whenever a new section list is installed.
    pub fn sections_epoch(&self) -> u64 {
        self.imp().epoch.get()
    }

    /// Layout ratios for the slots in `range` (placeholders count as unknown, 0).
    pub fn ratios(&self, range: Range<u32>) -> Vec<f32> {
        let items = self.imp().items.borrow();
        let start = (range.start as usize).min(items.len());
        let end = (range.end as usize).clamp(start, items.len());
        items[start..end]
            .iter()
            .map(|slot| slot.as_ref().map_or(0.0, AssetObject::layout_ratio))
            .collect()
    }

    /// Every real (non-placeholder) object, in model order.
    pub fn loaded_objects(&self) -> Vec<AssetObject> {
        self.imp()
            .items
            .borrow()
            .iter()
            .flatten()
            .cloned()
            .collect()
    }

    /// The first real object with this id.
    pub fn find_by_id(&self, id: &str) -> Option<AssetObject> {
        self.imp()
            .items
            .borrow()
            .iter()
            .flatten()
            .find(|object| object.id() == id)
            .cloned()
    }

    /// Whether `position` is an unloaded slot of a sectioned model.
    pub fn is_placeholder(&self, position: u32) -> bool {
        matches!(self.imp().items.borrow().get(position as usize), Some(None))
    }

    /// The section holding flat `position` (sectioned models only).
    pub fn section_at(&self, position: u32) -> Option<usize> {
        let sections = self.imp().sections.borrow();
        let mut end = 0_u32;
        sections.as_ref()?.iter().position(|section| {
            end += section.count;
            position < end
        })
    }
}

/// Grid objects for one loaded bucket. `hydrated` (Unified mode) supplies the full
/// records: filenames, MIME types and the checksums that drive the sync overlay.
pub fn build_tile_objects(
    tiles: &[TileAsset],
    hydrated: &HashMap<String, LibraryAsset>,
    ctx: &AppContext,
) -> Vec<AssetObject> {
    tiles
        .iter()
        .map(|tile| {
            let full = hydrated.get(&tile.id);
            let local_match = full
                .and_then(|asset| asset.checksum.as_deref())
                .and_then(normalize_checksum_to_hex)
                .and_then(|hex| ctx.sync_index.local_path_for_checksum(&hex));
            let (width, height) = full.map(asset_dimensions).unwrap_or((0, 0));
            let object = AssetObject::new(AssetInit {
                id: &tile.id,
                filename: full.map_or("", |a| a.filename.as_str()),
                mime_type: full.map_or("", |a| a.mime_type.as_str()),
                created_at: &tile.file_created_at,
                asset_type: if tile.is_image { "IMAGE" } else { "VIDEO" },
                sync_state: if local_match.is_some() { 2 } else { 0 },
                thumbhash: tile.thumbhash.as_deref(),
                width,
                height,
                is_favorite: tile.is_favorite,
                is_archived: tile.visibility == "archive",
            });
            object.set_ratio(tile.ratio);
            if let Some(path) = local_match {
                object.set_local_path(path);
            }
            object
        })
        .collect()
}

pub(crate) fn build_asset_objects(assets: &[LibraryAsset], ctx: &AppContext) -> Vec<AssetObject> {
    use super::{LOCAL_ID_PREFIX, immich_checksum_to_hex};
    use crate::library::local_source::local_sync_state;

    assets
        .iter()
        .map(|asset| {
            if let Some(local_path) = asset.id.strip_prefix(LOCAL_ID_PREFIX) {
                let sync_state =
                    local_sync_state(&ctx.sync_index, std::path::Path::new(local_path));
                let object = AssetObject::new_local(
                    &asset.id,
                    &asset.filename,
                    &asset.mime_type,
                    &asset.created_at,
                    &asset.asset_type,
                    local_path,
                );
                if sync_state != 1 {
                    object.set_property("sync-state", sync_state);
                }
                return object;
            }
            let local_match = asset
                .checksum
                .as_deref()
                .and_then(immich_checksum_to_hex)
                .as_deref()
                .and_then(|hex| ctx.sync_index.local_path_for_checksum(hex));
            let sync_state = if local_match.is_some() { 2 } else { 0 };
            let (width, height) = asset_dimensions(asset);
            let object = AssetObject::new(AssetInit {
                id: &asset.id,
                filename: &asset.filename,
                mime_type: &asset.mime_type,
                created_at: &asset.created_at,
                asset_type: &asset.asset_type,
                sync_state,
                thumbhash: asset.thumbhash.as_deref(),
                width,
                height,
                is_favorite: asset.is_favorite,
                is_archived: asset.is_archived(),
            });
            if let Some(path) = local_match {
                object.set_property("local-path", path);
            }
            object
        })
        .collect()
}

/// Pixel size from the asset, falling back to EXIF (where Immich often keeps it); 0 = unknown.
fn asset_dimensions(asset: &LibraryAsset) -> (u32, u32) {
    let (exif_w, exif_h) = asset
        .exif_info
        .as_ref()
        .map(|e| (e.exif_image_width, e.exif_image_height))
        .unwrap_or((None, None));
    (
        asset.width.filter(|v| *v > 0).or(exif_w).unwrap_or(0),
        asset.height.filter(|v| *v > 0).or(exif_h).unwrap_or(0),
    )
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gtk::prelude::CastNone;

    use super::*;

    fn object(id: &str, ratio: f64) -> AssetObject {
        let object = AssetObject::new(AssetInit {
            id,
            filename: "",
            mime_type: "",
            created_at: "",
            asset_type: "IMAGE",
            sync_state: 0,
            thumbhash: None,
            width: 0,
            height: 0,
            is_favorite: false,
            is_archived: false,
        });
        object.set_ratio(ratio);
        object
    }

    fn section(key: &str, count: u32) -> GridSection {
        GridSection {
            key: key.to_string(),
            label: key.to_string(),
            count,
            loaded: false,
        }
    }

    fn id_at(model: &LibraryAssetModel, pos: u32) -> String {
        model.item(pos).and_downcast::<AssetObject>().unwrap().id()
    }

    /// `(position, removed, added)` of every `items_changed` emission.
    fn record_changes(model: &LibraryAssetModel) -> Rc<RefCell<Vec<(u32, u32, u32)>>> {
        let log = Rc::new(RefCell::new(Vec::new()));
        let sink = log.clone();
        model.connect_items_changed(move |_, pos, removed, added| {
            sink.borrow_mut().push((pos, removed, added));
        });
        log
    }

    fn sectioned() -> LibraryAssetModel {
        let model = LibraryAssetModel::new();
        model.reset_sections(vec![
            (section("local", 0), Some(vec![object("l1", 1.0)])),
            (section("2025-06", 3), None),
            (section("2025-05", 2), None),
        ]);
        model
    }

    #[test]
    fn sections_reserve_slots_for_every_bucket() {
        let model = sectioned();
        assert!(model.is_sectioned());
        assert_eq!(model.n_items(), 6);
        let sections = model.sections().unwrap();
        assert_eq!(sections[0].count, 1);
        assert!(sections[0].loaded);
        assert!(!sections[1].loaded);
        assert_eq!(id_at(&model, 0), "l1");
        // Unloaded slots answer with an id-less placeholder.
        assert_eq!(id_at(&model, 1), "");
        assert_eq!(id_at(&model, 5), "");
        assert!(model.item(6).is_none());
        assert_eq!(model.loaded_objects().len(), 1);
    }

    #[test]
    fn filling_a_bucket_keeps_other_positions_stable() {
        let model = sectioned();
        let changes = record_changes(&model);
        assert!(model.fill_section(2, vec![object("b1", 1.5), object("b2", 0.5)]));
        assert_eq!(*changes.borrow(), vec![(4, 2, 2)]);
        assert_eq!(id_at(&model, 4), "b1");
        assert_eq!(id_at(&model, 5), "b2");
        assert_eq!(id_at(&model, 1), "");
        assert_eq!(model.ratios(4..6), vec![1.5, 0.5]);

        // A bucket that turns out smaller than announced shifts only later slots.
        assert!(model.fill_section(1, vec![object("a1", 1.0), object("a2", 1.0)]));
        assert_eq!(changes.borrow()[1], (1, 3, 2));
        assert_eq!(id_at(&model, 3), "b1");
        assert_eq!(model.sections().unwrap()[1].count, 2);
        assert!(model.find_by_id("a2").is_some());
    }

    #[test]
    fn positions_map_to_sections_and_placeholders() {
        let model = sectioned();
        assert_eq!(model.section_at(0), Some(0));
        assert_eq!(model.section_at(1), Some(1));
        assert_eq!(model.section_at(3), Some(1));
        assert_eq!(model.section_at(4), Some(2));
        assert_eq!(model.section_at(6), None);
        assert!(!model.is_placeholder(0));
        assert!(model.is_placeholder(4));
        assert!(!model.is_placeholder(6));
        model.fill_section(2, vec![object("b1", 1.0), object("b2", 1.0)]);
        assert!(!model.is_placeholder(4));

        model.reset_with_objects(vec![object("x", 1.0)]);
        assert_eq!(model.section_at(0), None);
        assert!(!model.is_placeholder(0));
    }

    #[test]
    fn remove_ids_shrinks_sections() {
        let model = sectioned();
        model.fill_section(
            1,
            vec![object("a1", 1.0), object("a2", 1.0), object("a3", 1.0)],
        );
        assert!(model.remove_ids(&["a2".to_string(), "l1".to_string()]));
        let counts: Vec<u32> = model.sections().unwrap().iter().map(|s| s.count).collect();
        assert_eq!(counts, vec![0, 2, 2]);
        assert_eq!(model.n_items(), 4);
        assert_eq!(id_at(&model, 1), "a3");
    }

    #[test]
    fn flat_resets_leave_sectioned_mode() {
        let model = sectioned();
        let epoch = model.sections_epoch();
        model.reset_with_objects(vec![object("x", 1.0)]);
        assert!(!model.is_sectioned());
        assert!(!model.fill_section(0, vec![]));
        assert!(!model.remove_ids(&["x".to_string()]));
        assert_eq!(model.n_items(), 1);
        model.reset_sections(vec![(section("2025-06", 1), None)]);
        assert_ne!(model.sections_epoch(), epoch);
    }
}
