//! Main library bucket state: which months are known, loading, loaded, or failed.
//!
//! The Main library (Remote `AllAssets` and `Unified`) is a list of timeline
//! **buckets** (one per month, see `CONTEXT.md`). Every bucket's count is known
//! up front from `/timeline/buckets`; its tiles load on demand, in any order, when
//! the bucket scrolls into the viewport band. This module is the pure state
//! machine behind that; the GTK/async glue lives in `main_library.rs`.
//!
//! Geometry (estimated vs exact heights, scroll anchoring) is owned by the canvas
//! layout (`masonry::layout::pack_sections`), since it depends on canvas width.

use std::collections::{HashMap, HashSet};

use chrono::{Datelike, Months, NaiveDate};

use crate::api_client::{LibraryAsset, TileAsset, TimeBucket};

/// Load state of one bucket.
#[derive(Debug, Clone, PartialEq)]
pub enum BucketState {
    /// Only the count is known; the canvas lays it out from an estimate.
    Estimated,
    Loading,
    Loaded(Vec<TileAsset>),
    /// The last fetch failed; retried when the bucket next enters the viewport band.
    Error(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct BucketSection {
    /// Opaque server bucket key, echoed back verbatim to `/timeline/bucket`.
    pub key: String,
    /// "June 2025".
    pub label: String,
    /// Tiles in the bucket (stacks count once). Corrected to the real tile count on load.
    pub count: u32,
    pub state: BucketState,
}

/// A bucket that should be fetched now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BucketFetch {
    pub index: usize,
    pub key: String,
    pub count: u32,
}

#[derive(Debug, Default)]
pub struct BucketStore {
    pub sections: Vec<BucketSection>,
    /// The `LibraryState.generation` this listing belongs to. Results tagged with
    /// any other generation are stale and must not apply.
    pub generation: u64,
    /// Synthetic grid sections shown ahead of the buckets (Unified's local-only
    /// section), so grid section `i` is bucket `i - leading`.
    pub leading: usize,
    /// Unified mode: loaded buckets hydrate eagerly for the sync overlay.
    pub unified: bool,
    active: bool,
    /// Buckets inside the viewport band at the last `demand` call.
    in_band: HashSet<usize>,
    /// Full asset records by asset id (see "Hydration" in `CONTEXT.md`).
    hydrated: HashMap<String, LibraryAsset>,
}

impl BucketStore {
    /// Start a fresh listing for `generation`: every bucket estimated, loaded tiles
    /// and the hydration cache dropped.
    pub fn begin(
        &mut self,
        generation: u64,
        buckets: &[TimeBucket],
        unified: bool,
        leading: usize,
    ) {
        *self = Self {
            sections: buckets
                .iter()
                .map(|bucket| BucketSection {
                    key: bucket.time_bucket.clone(),
                    label: bucket.label(),
                    count: bucket.count,
                    state: BucketState::Estimated,
                })
                .collect(),
            generation,
            leading,
            unified,
            active: true,
            in_band: HashSet::new(),
            hydrated: HashMap::new(),
        };
    }

    /// Leave the Main library: forget everything so late results cannot apply.
    pub fn deactivate(&mut self) {
        *self = Self {
            generation: self.generation,
            ..Self::default()
        };
    }

    pub fn is_current(&self, generation: u64) -> bool {
        self.active && self.generation == generation
    }

    /// Total tiles across all buckets.
    pub fn total_count(&self) -> u64 {
        self.sections.iter().map(|s| u64::from(s.count)).sum()
    }

    /// The viewport band now covers the unloaded buckets `in_band`. Estimated ones,
    /// and failed ones that just (re-)entered the band, move to `Loading`; they are
    /// returned for fetching. Loading/loaded buckets are never fetched twice.
    pub fn demand(&mut self, in_band: &[usize]) -> Vec<BucketFetch> {
        if !self.active {
            return Vec::new();
        }
        let mut fetches = Vec::new();
        for &index in in_band {
            let Some(section) = self.sections.get_mut(index) else {
                continue;
            };
            let fetch = match section.state {
                BucketState::Estimated => true,
                BucketState::Error(_) => !self.in_band.contains(&index),
                BucketState::Loading | BucketState::Loaded(_) => false,
            };
            if fetch {
                section.state = BucketState::Loading;
                fetches.push(BucketFetch {
                    index,
                    key: section.key.clone(),
                    count: section.count,
                });
            }
        }
        self.in_band = in_band.iter().copied().collect();
        fetches
    }

    /// Something outside the viewport band (lightbox navigation) needs bucket
    /// `index` now: fetch it unless it is already loading or loaded. Unlike
    /// [`demand`](Self::demand), a failed bucket is retried immediately and the
    /// band bookkeeping is left alone.
    pub fn request(&mut self, index: usize) -> Option<BucketFetch> {
        if !self.active {
            return None;
        }
        let section = self.sections.get_mut(index)?;
        match section.state {
            BucketState::Estimated | BucketState::Error(_) => {
                section.state = BucketState::Loading;
                Some(BucketFetch {
                    index,
                    key: section.key.clone(),
                    count: section.count,
                })
            }
            BucketState::Loading | BucketState::Loaded(_) => None,
        }
    }

    /// Store a fetched bucket. False (and no change) for stale generations or a
    /// bucket that is not loading.
    pub fn apply_loaded(&mut self, generation: u64, index: usize, tiles: Vec<TileAsset>) -> bool {
        let Some(section) = self.loading_section(generation, index) else {
            return false;
        };
        section.count = tiles.len() as u32;
        section.state = BucketState::Loaded(tiles);
        true
    }

    /// Record a failed fetch. False (and no change) for stale generations.
    pub fn apply_error(&mut self, generation: u64, index: usize, error: String) -> bool {
        let Some(section) = self.loading_section(generation, index) else {
            return false;
        };
        section.state = BucketState::Error(error);
        true
    }

    fn loading_section(&mut self, generation: u64, index: usize) -> Option<&mut BucketSection> {
        if !self.is_current(generation) {
            return None;
        }
        self.sections
            .get_mut(index)
            .filter(|s| s.state == BucketState::Loading)
    }

    /// Forget deleted/archived assets: drop them from loaded buckets (shrinking the
    /// count) and from the hydration cache.
    pub fn remove_ids(&mut self, ids: &[String]) {
        for section in &mut self.sections {
            if let BucketState::Loaded(tiles) = &mut section.state {
                tiles.retain(|tile| !ids.contains(&tile.id));
                section.count = tiles.len() as u32;
            }
        }
        self.hydrated.retain(|id, _| !ids.contains(id));
    }

    pub fn insert_hydrated(&mut self, assets: impl IntoIterator<Item = LibraryAsset>) {
        self.hydrated
            .extend(assets.into_iter().map(|asset| (asset.id.clone(), asset)));
    }

    pub fn hydrated(&self) -> &HashMap<String, LibraryAsset> {
        &self.hydrated
    }

    /// The cached full record for `id`, if it has been hydrated.
    pub fn hydrated_asset(&self, id: &str) -> Option<LibraryAsset> {
        self.hydrated.get(id).cloned()
    }

    /// Cache one on-interaction hydration fetched during `generation`. False (and
    /// no change) if the listing was refreshed or left meanwhile: the record
    /// may predate the refresh, which exists to drop stale records.
    pub fn cache_hydrated(&mut self, generation: u64, asset: LibraryAsset) -> bool {
        if !self.is_current(generation) {
            return false;
        }
        self.hydrated.insert(asset.id.clone(), asset);
        true
    }
}

/// `(takenAfter, takenBefore)` for hydrating a bucket via `search/metadata`: the
/// bucket's calendar month widened by one day on each side, so assets whose
/// timezone puts them in a neighbouring UTC day are still found. `None` when the
/// key does not start with `YYYY-MM`.
pub fn month_window(bucket_key: &str) -> Option<(String, String)> {
    let year: i32 = bucket_key.get(0..4)?.parse().ok()?;
    if bucket_key.as_bytes().get(4) != Some(&b'-') {
        return None;
    }
    let month: u32 = bucket_key.get(5..7)?.parse().ok()?;
    let first = NaiveDate::from_ymd_opt(year, month, 1)?;
    let next = first.checked_add_months(Months::new(1))?;
    let after = first.pred_opt()?;
    let before = next.succ_opt()?;
    let iso = |d: NaiveDate| {
        format!(
            "{:04}-{:02}-{:02}T00:00:00.000Z",
            d.year(),
            d.month(),
            d.day()
        )
    };
    Some((iso(after), iso(before)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bucket(key: &str, count: u32) -> TimeBucket {
        TimeBucket {
            time_bucket: key.to_string(),
            count,
        }
    }

    fn tile(id: &str) -> TileAsset {
        TileAsset {
            id: id.to_string(),
            ratio: 1.5,
            thumbhash: None,
            is_image: true,
            duration: None,
            is_favorite: false,
            visibility: "timeline".to_string(),
            file_created_at: "2025-06-03T10:00:00.000Z".to_string(),
            stack: None,
        }
    }

    fn full(id: &str) -> LibraryAsset {
        LibraryAsset {
            id: id.into(),
            filename: format!("{id}.jpg"),
            mime_type: "image/jpeg".into(),
            created_at: "2025-06-03T10:00:00.000Z".into(),
            asset_type: "IMAGE".into(),
            thumbhash: None,
            width: None,
            height: None,
            checksum: None,
            exif_info: None,
            is_favorite: false,
            visibility: "timeline".into(),
        }
    }

    fn store() -> BucketStore {
        let mut store = BucketStore::default();
        store.begin(
            7,
            &[
                bucket("2025-06-01", 3),
                bucket("2025-05-01", 2),
                bucket("2019-01-01", 4),
            ],
            false,
            0,
        );
        store
    }

    fn keys(fetches: &[BucketFetch]) -> Vec<usize> {
        fetches.iter().map(|f| f.index).collect()
    }

    #[test]
    fn begin_estimates_every_bucket_with_labels() {
        let store = store();
        assert_eq!(store.sections.len(), 3);
        assert!(
            store
                .sections
                .iter()
                .all(|s| s.state == BucketState::Estimated)
        );
        assert_eq!(store.sections[0].label, "June 2025");
        assert_eq!(store.total_count(), 9);
        assert!(store.is_current(7));
        assert!(!store.is_current(6));
    }

    #[test]
    fn demand_loads_only_the_buckets_in_band_in_any_order() {
        let mut store = store();
        // Jump straight to 2019: only that bucket is fetched.
        let fetches = store.demand(&[2]);
        assert_eq!(
            fetches,
            vec![BucketFetch {
                index: 2,
                key: "2019-01-01".into(),
                count: 4
            }]
        );
        assert_eq!(store.sections[2].state, BucketState::Loading);
        assert_eq!(store.sections[0].state, BucketState::Estimated);
    }

    #[test]
    fn loading_and_loaded_buckets_are_not_fetched_again() {
        let mut store = store();
        assert_eq!(keys(&store.demand(&[0, 1])), vec![0, 1]);
        assert!(store.demand(&[0, 1]).is_empty());
        assert!(store.apply_loaded(7, 0, vec![tile("a"), tile("b"), tile("c")]));
        assert!(store.demand(&[0, 1, 2]).iter().all(|f| f.index == 2));
        assert!(matches!(store.sections[0].state, BucketState::Loaded(ref t) if t.len() == 3));
    }

    #[test]
    fn loaded_count_follows_the_real_tiles() {
        let mut store = store();
        store.demand(&[1]);
        assert!(store.apply_loaded(7, 1, vec![tile("x")]));
        assert_eq!(store.sections[1].count, 1);
        assert_eq!(store.total_count(), 8);
    }

    #[test]
    fn stale_generation_results_do_not_apply() {
        let mut store = store();
        store.demand(&[0]);
        assert!(!store.apply_loaded(6, 0, vec![tile("a")]));
        assert!(!store.apply_error(6, 0, "boom".into()));
        assert_eq!(store.sections[0].state, BucketState::Loading);

        // A refresh starts a new generation; the old fetch must not land in it.
        store.begin(8, &[bucket("2025-06-01", 3)], false, 0);
        assert!(!store.apply_loaded(7, 0, vec![tile("a")]));
        assert_eq!(store.sections[0].state, BucketState::Estimated);
    }

    #[test]
    fn results_for_buckets_that_are_not_loading_are_ignored() {
        let mut store = store();
        assert!(!store.apply_loaded(7, 0, vec![tile("a")]));
        assert!(!store.apply_loaded(7, 99, vec![tile("a")]));
        assert_eq!(store.sections[0].state, BucketState::Estimated);
    }

    #[test]
    fn failed_buckets_retry_on_the_next_viewport_intersection() {
        let mut store = store();
        store.demand(&[0]);
        assert!(store.apply_error(7, 0, "HTTP 500".into()));
        assert_eq!(
            store.sections[0].state,
            BucketState::Error("HTTP 500".into())
        );
        // Still in band: no hammering on every report.
        assert!(store.demand(&[0, 1]).iter().all(|f| f.index != 0));
        // Scrolled away, then back: retried.
        store.demand(&[2]);
        assert_eq!(keys(&store.demand(&[0])), vec![0]);
        assert_eq!(store.sections[0].state, BucketState::Loading);
    }

    #[test]
    fn explicit_requests_fetch_once_and_retry_failures_at_once() {
        let mut store = store();
        assert_eq!(store.request(1).map(|f| f.index), Some(1));
        assert_eq!(store.sections[1].state, BucketState::Loading);
        assert!(store.request(1).is_none());
        assert!(store.request(99).is_none());
        assert!(store.apply_error(7, 1, "HTTP 500".into()));
        assert_eq!(store.request(1).map(|f| f.index), Some(1));
        assert!(store.apply_loaded(7, 1, vec![tile("a")]));
        assert!(store.request(1).is_none());
        store.deactivate();
        assert!(store.request(0).is_none());
    }

    #[test]
    fn deactivated_store_ignores_everything() {
        let mut store = store();
        store.demand(&[0]);
        store.deactivate();
        assert!(store.sections.is_empty());
        assert!(!store.is_current(7));
        assert!(store.demand(&[0]).is_empty());
        assert!(!store.apply_loaded(7, 0, vec![tile("a")]));
    }

    #[test]
    fn begin_clears_loaded_buckets_and_hydration_cache() {
        let mut store = store();
        store.demand(&[0]);
        store.apply_loaded(7, 0, vec![tile("a")]);
        store.insert_hydrated([full("a")]);
        assert_eq!(store.hydrated().len(), 1);
        store.begin(8, &[bucket("2025-06-01", 3)], true, 1);
        assert_eq!(store.sections[0].state, BucketState::Estimated);
        assert!(store.hydrated().is_empty());
        assert!(store.unified);
        assert_eq!(store.leading, 1);
    }

    #[test]
    fn hydrations_cache_only_into_the_listing_they_started_in() {
        let mut store = store();
        assert!(store.hydrated_asset("a").is_none());
        assert!(store.cache_hydrated(7, full("a")));
        assert_eq!(
            store.hydrated_asset("a").map(|a| a.filename),
            Some("a.jpg".to_string())
        );
        // Fetched before a refresh, landing after it: dropped.
        assert!(!store.cache_hydrated(6, full("b")));
        store.begin(8, &[bucket("2025-06-01", 3)], false, 0);
        assert!(store.hydrated_asset("a").is_none());
        assert!(!store.cache_hydrated(7, full("c")));
        store.deactivate();
        assert!(!store.cache_hydrated(8, full("d")));
        assert!(store.hydrated().is_empty());
    }

    #[test]
    fn remove_ids_shrinks_loaded_buckets() {
        let mut store = store();
        store.demand(&[0]);
        store.apply_loaded(7, 0, vec![tile("a"), tile("b"), tile("c")]);
        store.insert_hydrated([full("b")]);
        store.remove_ids(&["b".to_string()]);
        assert_eq!(store.sections[0].count, 2);
        assert!(store.hydrated().is_empty());
        // Estimated buckets keep their server count.
        assert_eq!(store.sections[2].count, 4);
    }

    #[test]
    fn month_window_widens_by_a_day_each_side() {
        assert_eq!(
            month_window("2025-06-01"),
            Some((
                "2025-05-31T00:00:00.000Z".to_string(),
                "2025-07-02T00:00:00.000Z".to_string()
            ))
        );
        assert_eq!(
            month_window("2025-06-01T00:00:00.000Z"),
            month_window("2025-06-01")
        );
    }

    #[test]
    fn month_window_crosses_year_and_month_lengths() {
        assert_eq!(
            month_window("2024-12-01"),
            Some((
                "2024-11-30T00:00:00.000Z".to_string(),
                "2025-01-02T00:00:00.000Z".to_string()
            ))
        );
        assert_eq!(
            month_window("2025-01-01"),
            Some((
                "2024-12-31T00:00:00.000Z".to_string(),
                "2025-02-02T00:00:00.000Z".to_string()
            ))
        );
        // Leap February ends on the 29th, so the window runs to March 2nd.
        assert_eq!(
            month_window("2024-02-01").map(|w| w.0),
            Some("2024-01-31T00:00:00.000Z".to_string())
        );
        assert_eq!(
            month_window("2024-03-01").map(|w| w.0),
            Some("2024-02-29T00:00:00.000Z".to_string())
        );
    }

    #[test]
    fn month_window_rejects_garbage() {
        assert_eq!(month_window(""), None);
        assert_eq!(month_window("garbage"), None);
        assert_eq!(month_window("2025-13-01"), None);
        assert_eq!(month_window("2025/06/01"), None);
    }
}
