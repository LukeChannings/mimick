//! Main library driver: bucket listing, viewport-driven bucket loads, Unified hydration.
//!
//! Entering the Main library (Remote `AllAssets` or `Unified`) checks the server
//! version, then fetches every bucket's count at once and installs one grid section
//! per bucket (plus, in Unified mode, a local-only section on top). The canvas then
//! reports which unloaded sections intersect the viewport band; those (and only
//! those) are fetched, in whatever order the user scrolls to them. See ADR 0001.

use std::collections::HashSet;
use std::rc::Rc;

use crate::api_client::{
    LibraryAsset, MetadataSearchFilters, SortOrder, TileAsset, TimeBucket,
    supports_timeline_buckets,
};
use crate::app_context::AppContext;
use crate::library::asset_model::{GridSection, build_asset_objects, build_tile_objects};
use crate::library::bucket_store::{BucketFetch, month_window};
use crate::library::local_source::{LocalAsset, enumerate_local};
use crate::library::state::LibrarySource;

use super::{LibraryWindowUi, local_to_library_asset, navigation, source_header};

pub(super) const SERVER_TOO_OLD: &str =
    "This library view requires Immich v1.133 or newer. Syncing is unaffected.";

/// Timeline visibility: the Main library excludes archived and locked assets.
const VISIBILITY: &str = "timeline";
/// Key/label of Unified mode's synthetic section of local files not yet uploaded.
const LOCAL_SECTION_KEY: &str = "local-not-uploaded";
const LOCAL_SECTION_LABEL: &str = "Not Yet Uploaded";
/// `search/metadata` page-size cap, and a bound on pages per bucket hydration.
const HYDRATION_PAGE_SIZE_MAX: u32 = 1000;
const HYDRATION_MAX_PAGES: u32 = 50;

/// Sources served by the timeline bucket API rather than the page-number pager.
pub(super) fn is_main_library(source: &LibrarySource) -> bool {
    matches!(source, LibrarySource::AllAssets | LibrarySource::Unified)
}

/// Enter (or refresh) the Main library for `generation`: refetch the bucket list,
/// dropping loaded buckets and the hydration cache.
pub(super) fn load(ui: Rc<LibraryWindowUi>, generation: u64, source: LibrarySource) {
    ui.buckets.borrow_mut().deactivate();
    let unified = matches!(source, LibrarySource::Unified);
    log::debug!("Loading Main library bucket list (unified={unified})");
    glib::MainContext::default().spawn_local(async move {
        if !server_supports_buckets(&ui).await {
            show_error(&ui, generation, SERVER_TOO_OLD.to_string());
            return;
        }
        let buckets = ui.ctx.api_client.fetch_timeline_buckets(VISIBILITY);
        let locals = async {
            if unified {
                enumerate_local(ui.ctx.clone()).await
            } else {
                Vec::new()
            }
        };
        let (buckets, locals) = futures_util::future::join(buckets, locals).await;
        match buckets {
            Ok(buckets) => apply_listing(&ui, generation, unified, &buckets, locals),
            Err(err) => show_error(
                &ui,
                generation,
                format!("Could not load library assets: {err}"),
            ),
        }
    });
}

/// Version gate. Without a known server version (offline `about`), fail open: the
/// bucket call itself reports the real problem.
async fn server_supports_buckets(ui: &LibraryWindowUi) -> bool {
    let known = ui.ctx.library_state.lock().status.about.clone();
    let about = match known {
        Some(about) => Some(about),
        None => ui.ctx.api_client.fetch_server_about().await.ok(),
    };
    about.is_none_or(|about| supports_timeline_buckets(&about))
}

fn show_error(ui: &Rc<LibraryWindowUi>, generation: u64, message: String) {
    let current = {
        let mut state = ui.ctx.library_state.lock();
        state.mark_error(generation, message.clone());
        state.generation == generation
    };
    if current && !navigation::on_page(ui) {
        source_header::set_status(ui, "error", Some(&message));
        ui.content_stack.set_visible_child_name("grid");
    }
}

fn apply_listing(
    ui: &Rc<LibraryWindowUi>,
    generation: u64,
    unified: bool,
    buckets: &[TimeBucket],
    locals: Vec<LocalAsset>,
) {
    let mut local_rows: Vec<LibraryAsset> = not_yet_uploaded(&ui.ctx, locals)
        .into_iter()
        .map(local_to_library_asset)
        .collect();
    local_rows.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    let empty = buckets.is_empty() && local_rows.is_empty();
    if !ui
        .ctx
        .library_state
        .lock()
        .finish_bucket_listing(generation, empty)
    {
        return;
    }
    let leading = usize::from(unified);
    ui.buckets
        .borrow_mut()
        .begin(generation, buckets, unified, leading);

    let mut sections = Vec::with_capacity(buckets.len() + leading);
    if unified {
        let section = GridSection {
            key: LOCAL_SECTION_KEY.to_string(),
            label: LOCAL_SECTION_LABEL.to_string(),
            count: 0,
            loaded: true,
        };
        sections.push((section, Some(build_asset_objects(&local_rows, &ui.ctx))));
    }
    sections.extend(buckets.iter().map(|bucket| {
        let section = GridSection {
            key: bucket.time_bucket.clone(),
            label: bucket.label(),
            count: bucket.count,
            loaded: false,
        };
        (section, None)
    }));
    log::debug!(
        "Main library: {} buckets, {} tiles, {} local-only",
        buckets.len(),
        ui.buckets.borrow().total_count(),
        local_rows.len()
    );
    ui.grid.model.reset_sections(sections);
    super::sync_content_state(ui);
    super::reload_sidebar(ui);
}

/// Unified's local-only section: watched files whose checksum does not map to a
/// synced path (i.e. not uploaded yet, as far as the sync index knows).
fn not_yet_uploaded(ctx: &AppContext, locals: Vec<LocalAsset>) -> Vec<LocalAsset> {
    locals
        .into_iter()
        .filter(|local| {
            ctx.sync_index
                .stored_checksum(&local.path.display().to_string())
                .and_then(|checksum| ctx.sync_index.local_path_for_checksum(&checksum))
                .is_none()
        })
        .collect()
}

/// The canvas reports the unloaded grid sections inside the viewport band; fetch
/// the buckets among them that need it.
pub(super) fn on_sections_demanded(ui: &Rc<LibraryWindowUi>, wanted: Vec<usize>) {
    let (generation, unified, fetches) = {
        let mut store = ui.buckets.borrow_mut();
        let leading = store.leading;
        let in_band: Vec<usize> = wanted
            .iter()
            .filter_map(|section| section.checked_sub(leading))
            .collect();
        (store.generation, store.unified, store.demand(&in_band))
    };
    for fetch in fetches {
        spawn_bucket_load(ui.clone(), generation, unified, fetch);
    }
}

/// Make sure the tile at flat `position` is (or is becoming) real. Returns true
/// when it is still an unloaded slot: its bucket is then fetched (if it was not
/// already in flight) and the caller should wait for the model to fill it.
pub(super) fn ensure_loaded(ui: &Rc<LibraryWindowUi>, position: u32) -> bool {
    let model = &ui.grid.model;
    if !model.is_placeholder(position) {
        return false;
    }
    let Some(section) = model.section_at(position) else {
        return false;
    };
    let (generation, unified, fetch) = {
        let mut store = ui.buckets.borrow_mut();
        let fetch = section
            .checked_sub(store.leading)
            .and_then(|bucket| store.request(bucket));
        (store.generation, store.unified, fetch)
    };
    if let Some(fetch) = fetch {
        spawn_bucket_load(ui.clone(), generation, unified, fetch);
    }
    true
}

fn spawn_bucket_load(ui: Rc<LibraryWindowUi>, generation: u64, unified: bool, fetch: BucketFetch) {
    log::debug!(
        "Loading timeline bucket {} ({} tiles)",
        fetch.key,
        fetch.count
    );
    glib::MainContext::default().spawn_local(async move {
        let result = ui
            .ctx
            .api_client
            .fetch_timeline_bucket(&fetch.key, VISIBILITY)
            .await;
        let hydrated =
            match &result {
                Ok(tiles) if unified => hydrate_bucket(&ui, &fetch.key, tiles)
                    .await
                    .unwrap_or_else(|err| {
                        log::warn!("Could not hydrate bucket {}: {}", fetch.key, err);
                        Vec::new()
                    }),
                _ => Vec::new(),
            };
        apply_bucket(&ui, generation, fetch.index, result, hydrated);
    });
}

/// Unified mode's eager hydration: one `search/metadata` query windowed on the
/// bucket's month (widened a day each side), intersected with the bucket's ids.
async fn hydrate_bucket(
    ui: &LibraryWindowUi,
    key: &str,
    tiles: &[TileAsset],
) -> Result<Vec<LibraryAsset>, String> {
    if tiles.is_empty() {
        return Ok(Vec::new());
    }
    let (after, before) =
        month_window(key).ok_or_else(|| format!("unrecognised bucket key {key}"))?;
    let filters = MetadataSearchFilters {
        taken_after: Some(after),
        taken_before: Some(before),
        visibility: Some(VISIBILITY.to_string()),
        order: Some(SortOrder::Desc),
        ..MetadataSearchFilters::default()
    };
    let size = u32::try_from(tiles.len())
        .unwrap_or(u32::MAX)
        .clamp(1, HYDRATION_PAGE_SIZE_MAX);
    let mut wanted: HashSet<&str> = tiles.iter().map(|tile| tile.id.as_str()).collect();
    let mut found = Vec::with_capacity(tiles.len());
    for page in 1..=HYDRATION_MAX_PAGES {
        let (assets, has_more) = ui
            .ctx
            .api_client
            .search_metadata_with_filters(&filters, page, size)
            .await?;
        found.extend(
            assets
                .into_iter()
                .filter(|asset| wanted.remove(asset.id.as_str())),
        );
        if wanted.is_empty() || !has_more {
            break;
        }
    }
    if !wanted.is_empty() {
        log::debug!(
            "Bucket {}: {} tile(s) not found by hydration",
            key,
            wanted.len()
        );
    }
    Ok(found)
}

fn apply_bucket(
    ui: &Rc<LibraryWindowUi>,
    generation: u64,
    index: usize,
    result: Result<Vec<TileAsset>, String>,
    hydrated: Vec<LibraryAsset>,
) {
    if ui.ctx.library_state.lock().generation != generation {
        return;
    }
    let (section, objects) = {
        let mut store = ui.buckets.borrow_mut();
        if !store.is_current(generation) {
            return;
        }
        let tiles = match result {
            Ok(tiles) => tiles,
            Err(err) => {
                log::warn!("Timeline bucket {} failed: {}", index, err);
                store.apply_error(generation, index, err);
                return;
            }
        };
        store.insert_hydrated(hydrated);
        let objects = build_tile_objects(&tiles, store.hydrated(), &ui.ctx);
        if !store.apply_loaded(generation, index, tiles) {
            return;
        }
        (store.leading + index, objects)
    };
    // The store borrow is released: items_changed handlers may run arbitrary UI code.
    ui.grid.model.fill_section(section, objects);
}

/// Drop deleted/archived assets from the Main library in place. False when the
/// grid is not showing the Main library (callers fall back to the pager path).
pub(super) fn remove_ids(ui: &LibraryWindowUi, ids: &[String]) -> bool {
    if !ui.grid.model.is_sectioned() {
        return false;
    }
    ui.buckets.borrow_mut().remove_ids(ids);
    ui.grid.model.remove_ids(ids)
}
