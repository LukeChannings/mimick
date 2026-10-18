# Phase 5 — Lightbox: ±10 preload and the stack strip

Depends on Phase 4. See overview decisions 4, 7 and 9. Main file: `src/library/lightbox.rs`.

## ±10 tile preload

- On lightbox open and on every tile navigation, preload the **10 tiles on each side** of the
  current position: hydration (via Phase 4 `hydrate`) + preview texture through the existing
  thumbnail/preview cache. Replaces the current immediate-neighbour prefetch.
- Preloads are cancellable/ignorable: reuse the `load_gen` generation pattern already in the
  lightbox so stale completions never touch the UI. Positions near the ends clamp.
- Preload counts **tiles**, not stack members.

## Stack strip

- When the opened tile has `stack-count ≥ 2`: show the Primary immediately (normal path),
  kick off `fetch_stack(stack_id)` (Phase 2).
- On success, reveal a horizontal thumbnail strip at the bottom of the viewer, above the
  action bar (a `gtk::ScrolledWindow` of thumbnails; thumbnails via the normal thumbnail
  cache). Primary first (server returns it first), current member highlighted.
- **Selecting a member** swaps the displayed asset in place: same crossfade machinery as tile
  navigation but `nav_dir = 0`, no grid-position change. Title, details pane
  (`fetch_asset_details` is already per-asset), favorite heart, archive state, drag-out path
  all re-render for that member. Member DTOs from `fetch_stack` are full `LibraryAsset`s —
  insert them into the hydration cache.
- **Tile navigation never enters a stack**: arrows/swipe/keys move between grid tiles exactly
  as today. Navigating away and back resets to the Primary. Non-stacked assets never show the
  strip.
- **All lightbox actions target the currently displayed member** (favorite, download, delete,
  edit, tag, archive, open-external, drag-out). Deleting the displayed member advances the
  strip to the next member; if the stack shrinks below 2, hide the strip. After any member
  delete, mark the grid for reload on close (existing `reload_on_close` pattern).
- **403 from `fetch_stack`** (key lacks `stack.read`): hide the strip silently — settings
  (Phase 7) is where the permission story is explained. Other errors: log, no strip.

## Acceptance

- Opening a stack tile shows the Primary instantly; the strip appears when the fetch lands.
- Strip selection swaps image + details + action targets; arrows still move between tiles and
  reset the stack to Primary on return.
- Holding an arrow key through 30 tiles stays smooth (preload generation discipline).
- With a `stack.read`-less key: stack tiles open normally, no strip, no error dialogs.
