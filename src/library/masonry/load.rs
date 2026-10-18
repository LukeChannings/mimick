//! Async load plumbing for the masonry grid.
//!
//! `load_with_fallback` consults the shared `ThumbnailCache`, walking the
//! `quality::fallback_bucket` chain on 404 so opt-in server features (e.g.
//! fullsize generation) degrade gracefully to whatever the server has.
//! `collect_ratios` / `propagate_dimensions` keep the layout's aspect-ratio view
//! of each asset in sync with what eventually decodes.

use gdk4::Texture;
use gtk::prelude::*;

use crate::api_client::ThumbnailSize;
use crate::library::asset_model::LibraryAssetModel;
use crate::library::masonry::quality::fallback_bucket;
use crate::library::thumbnail_cache::ThumbnailCache;

/// Layout ratio of every slot in the model.
pub(crate) fn collect_ratios(model: &LibraryAssetModel) -> Vec<f32> {
    model.ratios(0..model.n_items())
}

/// Try requested bucket, then walk `fallback_bucket` chain on 404. Only the
/// 404 path is retried — auth, network, etc. surface unchanged.
pub(crate) async fn load_with_fallback<F: Fn() -> bool>(
    cache: &ThumbnailCache,
    asset_id: &str,
    requested: ThumbnailSize,
    is_cancelled: &F,
) -> Result<Texture, String> {
    let mut current = requested;
    loop {
        match cache
            .load_thumbnail_cancellable(asset_id, current, is_cancelled)
            .await
        {
            Ok(tex) => return Ok(tex),
            Err(e) if e.contains("404") => match fallback_bucket(current) {
                Some(next) => {
                    log::debug!(
                        "masonry fallback id={} {:?} -> {:?} ({})",
                        asset_id,
                        current,
                        next,
                        e
                    );
                    current = next;
                }
                None => return Err(e),
            },
            Err(e) => return Err(e),
        }
    }
}

/// Returns true if the AssetObject dimensions were filled in (relayout needed).
/// Assets with a server-provided ratio (Main library tiles) already lay out
/// exactly, so their decode never triggers a relayout.
pub(crate) fn propagate_dimensions(
    model: &LibraryAssetModel,
    asset_id: &str,
    tex: &Texture,
) -> bool {
    let Some(obj) = model.find_by_id(asset_id) else {
        return false;
    };
    if obj.width() != 0 && obj.height() != 0 {
        return false;
    }
    obj.set_width(tex.width() as u32);
    obj.set_height(tex.height() as u32);
    obj.ratio() <= 0.0
}
