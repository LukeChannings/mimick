# Phase 1 — Remove Timeline mode and client-side sorts

Standalone cleanup that shrinks the surface before the bucket-grid rewrite. Shippable on its
own. See `docs/plans/00-overview.md` decisions 5 and 6.

## Remove Filename/File Type sorting (app-wide)

- `src/library/state.rs`: collapse `LibrarySortMode` — `Filename`, `FileType` and the
  UI-unreachable `OldestFirst` all go. Either delete the enum entirely or reduce it to a
  unit `NewestFirst`; prefer deletion if `apply_sort` / `sort_mode` plumbing can go with it.
  `server_order()` simplifies to always-`Desc`.
- `src/library/asset_model.rs`: delete the sort machinery (`build_sorted_asset_objects`
  sorting arms at ~L85–150); the model keeps server order.
- `src/library/controls.rs`: delete the grid branch of `sort_labels_for_view` (~L219) and
  `apply_sort_selection` (~L229). The sort dropdown must no longer appear for grid views.
  Albums-list ("Newest", "Name", "Most assets") and Explore ("Default") dropdowns stay.
- `src/library/window_layout.rs`: remove/hide the `sort_mode` dropdown for grid views
  accordingly.

## Remove the Timeline mode

- `src/library/state.rs`: delete `LibrarySource::Timeline`. `AllAssets` is now the single
  remote Main-library source.
- `src/library/controls.rs`: delete the timeline toggle wiring (~L264–290) and the
  `timeline_active` arm of `source_for_selection` (~L317); fix its tests (~L520).
- `src/library/window_layout.rs`: delete `timeline_toggle` (~L320–355) and its struct fields.
- `src/library/mod.rs`:
  - `apply_timeline_ui_state` (~L758): remove toggle/sort-forcing logic. Keep the function if
    the rest (source-mode dropdown sync, trash mode, upload tooltip) is still needed; rename
    it (e.g. `apply_source_ui_state`).
  - `update_timeline_banner_if_active` (~L823): the month-year banner now applies to
    `LibrarySource::AllAssets` (interim behavior until the bucket grid rewires it).
  - `remote_filters` (~L1149) and `fetch_unified_page` (~L1218): replace
    `Timeline` references with `AllAssets`.
- `src/library/navigation.rs` (~L150, ~L234, ~L262): the `"photos"` nav-restore key maps to
  `AllAssets`. Keep *accepting* any previously saved timeline key from old state.
- `src/library/features.rs` (~L17): fallback navigation target becomes `AllAssets`.
- `src/library/archive.rs`, `src/library/source_header.rs`, `src/library/nav_history.rs`:
  update matches; fix tests.

## Drop the "Photos" source header

- `src/library/source_header.rs` `basics()` (~L190): the `AllAssets`/`Unified` arm no longer
  produces an icon/title/photo-video-count header. The Main library grid has no source header;
  all other sources keep theirs.

## Acceptance

- `cargo build && cargo test && cargo clippy` clean.
- Grid views show no sort dropdown; Albums list and Explore still do.
- No Timeline toggle; Photos sidebar entry lands on the (still page-based, for now) main grid,
  sorted newest-first, with the month banner working and no "Photos" header above the grid.
- Grep proof: no `LibrarySource::Timeline`, no `LibrarySortMode::Filename`, no
  `LibrarySortMode::FileType` anywhere.

## Out of scope

Anything bucket/stack related. The main library still uses `search/metadata` paging after
this phase.
