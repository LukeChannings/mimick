# Phase 4 — Thin tiles, stack badge, hydration on interaction

Depends on Phase 3. See `CONTEXT.md` ("Hydration") and overview decision 4.

## Thin AssetObject

- `src/library/asset_object.rs`: tiles in the Main library are built from `TileAsset` —
  no filename, MIME, checksum or EXIF. Add what the tile needs: `ratio`, `stack-count`
  (0 = not stacked), and a `hydrated` notion. Width/height properties: derive a nominal
  `(w, h)` from `ratio` for layout compatibility, or switch the layout input to ratio
  (Phase 3 choice).
- Grid paint (`src/library/masonry/canvas.rs`): **stack badge** on tiles with
  `stack-count ≥ 2` — stack icon + count, top-right corner, same overlay style as the
  existing video/favorite badges. Thumbhash/placeholder, favorite heart, video duration
  badges keep working from thin data.

## Hydration

- Cache lives in the `BucketStore` (`HashMap<String /*asset id*/, LibraryAsset>`),
  cleared on refresh and on lightbox-driven reloads (`reload_on_close` edit path).
- `async fn hydrate(ui, asset_id) -> Result<LibraryAsset, String>`: cache hit or
  `fetch_asset_by_id` (`src/api_client/library.rs` ~L189), insert, return.
- **Never hydrate on scroll.** Interaction points that must hydrate first:
  - **Video tile activate** (`src/library/controls.rs` activate handler → `spawn_video_handoff`
    needs filename): hydrate, then handoff. `TileAsset.is_image == false` identifies videos.
  - **Context menu** (`src/library/context_menu.rs` `MenuAsset::from_item` reads
    filename/mime): hydrate before building the menu, or lazily per action — pick one and be
    consistent; copy/open/download need real filenames.
  - **Download / drag-out / share** paths that name files.
  - **Lightbox open** (Phase 5 wires the ±10 preload through this same `hydrate`).
- Favorite/archive/trash/tag bulk actions operate on IDs only — no hydration needed.
- Unified-mode eager per-bucket hydration (Phase 3) feeds this same cache.

## Acceptance

- Scrolling the Main library issues only bucket + thumbnail requests (verify via debug logs).
- Stack tiles show the badge with the server's member count.
- Clicking a video plays it (one extra round-trip, no UI regression); context-menu
  copy/open/download produce correctly named files.
- Non-Main-library sources are untouched (they still build fully-populated AssetObjects).
