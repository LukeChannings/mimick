# Phase 3 — Bucket store and the virtual month-sectioned grid

The big one. Depends on Phases 1 and 2. The Main library (Remote `AllAssets` and `Unified`)
moves off the page-number pager onto bucket-based virtual scrolling. **Every other source
keeps the existing `LibraryState.assets` pager untouched.**

Riskiest part: estimated-height scroll anchoring. A throwaway spike (hard-coded bucket data,
no network) to validate the anchoring math before full wiring is sanctioned and encouraged.

## Bucket store (new module, e.g. `src/library/bucket_store.rs`)

```rust
enum BucketState { Estimated, Loading, Loaded(Vec<TileAsset>), Error(String) }
struct BucketSection { key: String, label: String, count: u32, state: BucketState, height: f32 }
struct BucketStore { sections: Vec<BucketSection>, generation: u64, /* hydration cache: HashMap<String, LibraryAsset> */ }
```

- Entering the Main library: version-gate check first (`supports_timeline_buckets`; if false,
  show the existing error view with "This library view requires Immich v1.133 or newer.
  Syncing is unaffected."). Then `fetch_timeline_buckets` populates all sections at once.
- Generation counter mirrors `LibraryState.generation` semantics: stale fetches must not
  apply (see `replace_assets_with_more` for the pattern).
- Refresh (`source_header` refresh / re-entry) refetches the bucket list and clears loaded
  buckets + hydration cache.

## Layout (`src/library/masonry/`)

- `layout.rs`: layout becomes a list of sections. Per loaded bucket: a month-label row
  (scrolls with content, not sticky — label e.g. "June 2025") followed by `pack_rows` over
  that bucket's tiles, using `ratio` from `TileAsset` (pack_rows currently takes `(w, h)`
  dims; `aspect()` already reduces to ratio — adapt the input type).
- Unloaded bucket height estimate: `rows ≈ ceil(count / avg_tiles_per_row)` where
  `avg_tiles_per_row = canvas_w / (target_row_height × assumed_avg_ratio 4/3)`, times
  `(row_height + gap)`, plus the label row. Deterministic and unit-tested.
- Total canvas height = Σ section heights ⇒ the scrollbar is truthful from first paint.
- On bucket load, replace estimate with exact packed height and **anchor the scroll**: if the
  changed section is entirely above the viewport, adjust `vadjustment` by the height delta so
  visible content does not move. Unit-test the pure anchoring math.

## Loading driver (`src/library/masonry/canvas.rs` / `controls.rs`)

- Replace the 50%-scroll `load_next_page_if_needed` trigger (Main library only) with
  viewport-driven bucket loading: any `Estimated` section intersecting the viewport (plus the
  existing `VIEWPORTS_AHEAD`/`VIEWPORTS_BEHIND` margins) → `Loading` → fetch. Random access:
  dragging the scrollbar to 2019 loads 2019 only.
- Failed bucket loads → `Error` state, retried on next viewport intersection.
- The floating month banner (`update_timeline_banner_if_active`) now reads the section under
  the viewport midpoint — exact, works over unloaded months.

## Model bridge

- The canvas currently reads `LibraryAssetModel` (flat `GListModel`). Options: teach the
  canvas to read the `BucketStore` directly for Main-library sources, or project loaded
  buckets into the model with section offsets. Prefer whichever keeps `interaction.rs`
  (hit-testing, selection, activate→position) coherent; tile identity for the lightbox is
  `(section_index, item_index)` or a flattened index — keep it stable across bucket loads.

## Unified mode

- A synthetic section **above** the first month: local files not yet uploaded (reuse the
  `merge_unified_page` logic from `src/library/mod.rs` ~L1582: enumerate locals, drop those
  whose checksum maps to a synced path). Exact height (locals fully enumerable).
- Each loaded bucket **hydrates eagerly**: one `search/metadata` call windowed on the bucket's
  month (`takenAfter`/`takenBefore` widened by ±1 day for timezone edges, size up to the
  bucket count), intersect by ID, store in the hydration cache; then overlay sync badges by
  checksum as today.

## Acceptance

- Main library: scrollbar reflects the whole library immediately; jumping anywhere loads that
  month within one fetch; no visible jump when buckets above the viewport load.
- Stack tiles render once per stack (server collapses; nothing to do client-side here).
- Unified shows the local-only section and correct sync badges; albums/search/favorites etc.
  behave exactly as before this phase.
- Unit tests: estimation, anchoring delta, bucket-state transitions, Unified month-window
  widening.

## Out of scope

Stack badge, hydration-on-interaction (Phase 4); lightbox (Phase 5).
