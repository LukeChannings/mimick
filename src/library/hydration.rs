//! Hydration on interaction: upgrading a thin Main library tile to its full asset
//! record (see "Hydration" in `CONTEXT.md`).
//!
//! Bucket tiles carry no filename, MIME type, checksum or EXIF. Anything that
//! names a file (video handoff, context menu copy/open/download, bulk download,
//! drag-out, the lightbox) hydrates first; scrolling never does. Records are
//! cached in the `BucketStore`, so they share one cache with Unified mode's eager
//! per-bucket hydration, and are dropped whenever the bucket list is refreshed.
//! Objects from every other source are built hydrated, so for them these calls
//! are free no-ops.

use futures_util::StreamExt;

use crate::api_client::LibraryAsset;
use crate::library::asset_model::apply_full_record;
use crate::library::asset_object::AssetObject;

use super::LibraryWindowUi;

/// Concurrent `GET /assets/{id}` requests when hydrating a selection.
const BULK_CONCURRENCY: usize = 6;

/// The full record for `asset_id`: from the cache, or fetched and cached.
pub(super) async fn hydrate(ui: &LibraryWindowUi, asset_id: &str) -> Result<LibraryAsset, String> {
    let generation = {
        let store = ui.buckets.borrow();
        if let Some(asset) = store.hydrated_asset(asset_id) {
            return Ok(asset);
        }
        store.generation
    };
    log::debug!("Hydrating asset {asset_id}");
    let asset = ui.ctx.api_client.fetch_asset_by_id(asset_id).await?;
    ui.buckets
        .borrow_mut()
        .cache_hydrated(generation, asset.clone());
    Ok(asset)
}

/// Make `object` carry its full record before something names its file.
pub(super) async fn hydrate_object(
    ui: &LibraryWindowUi,
    object: &AssetObject,
) -> Result<(), String> {
    let Some(asset_id) = needs_hydration(object) else {
        return Ok(());
    };
    let asset = hydrate(ui, &asset_id).await?;
    apply_full_record(object, &asset, &ui.ctx);
    Ok(())
}

/// Hydrate every object in `objects` (a bulk selection), a few at a time.
/// Failures are logged and leave that object thin.
pub(super) async fn hydrate_objects(ui: &LibraryWindowUi, objects: &[AssetObject]) {
    futures_util::stream::iter(objects)
        .map(|object| async move {
            if let Err(err) = hydrate_object(ui, object).await {
                log::warn!("Could not hydrate asset {}: {}", object.id(), err);
            }
        })
        .buffer_unordered(BULK_CONCURRENCY)
        .collect::<Vec<()>>()
        .await;
}

/// Blocking [`hydrate_object`] for GTK's synchronous drag-prepare callback, which
/// already downloads originals on a worker thread the same way.
pub(super) fn hydrate_object_blocking(
    ui: &LibraryWindowUi,
    object: &AssetObject,
) -> Result<(), String> {
    let Some(asset_id) = needs_hydration(object) else {
        return Ok(());
    };
    let (cached, generation) = {
        let store = ui.buckets.borrow();
        (store.hydrated_asset(&asset_id), store.generation)
    };
    let asset = match cached {
        Some(asset) => asset,
        None => {
            log::debug!("Hydrating asset {asset_id} for drag-out");
            let api = ui.ctx.api_client.clone();
            let asset = std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().map_err(|err| err.to_string())?;
                rt.block_on(api.fetch_asset_by_id(&asset_id))
            })
            .join()
            .map_err(|_| "hydration worker panicked".to_string())??;
            ui.buckets
                .borrow_mut()
                .cache_hydrated(generation, asset.clone());
            asset
        }
    };
    apply_full_record(object, &asset, &ui.ctx);
    Ok(())
}

/// The server id to hydrate `object` by, or `None` when it is already complete
/// (or is an unloaded placeholder, which has nothing to hydrate).
fn needs_hydration(object: &AssetObject) -> Option<String> {
    if object.hydrated() {
        return None;
    }
    let remote_id = object.remote_id();
    let id = if remote_id.is_empty() {
        object.id()
    } else {
        remote_id
    };
    (!id.is_empty()).then_some(id)
}
