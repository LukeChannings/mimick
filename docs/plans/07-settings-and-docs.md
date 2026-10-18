# Phase 7 — Settings permission validation, wiki, changelog

Depends on Phase 2 (probe). Independent of Phases 3–6 (can run in parallel with them).

## Settings probe UI

- Wire `probe_permissions()` (Phase 2) into **Test Connection** and run it once on **Save**
  (`src/settings_window/connectivity.rs`, `src/settings_window/apply.rs`,
  `src/settings_window/wiring.rs`).
- Result handling:
  - All granted → no change to today's success feedback.
  - `Missing` entries → append a **warning** to the result: "Connection OK — note: the key
    lacks `stack.read`. Stack contents won't be viewable in the lightbox, and stack
    archive/delete will be unavailable." Use each probe row's `feature_hint`.
  - `Unknown` (network blip, unexpected status) → say nothing.
- **Never block saving.** The key stays saveable with missing optional permissions.
- Update the getting-started description in `src/settings_window/wiring.rs` (~L113), which
  lists required permissions, to mention `stack.read` as optional-for-stacks.

## Wiki (`wiki/`)

- `Configuration-and-First-Run.md` — permissions table: add `stack.read` → "View stack
  contents in the lightbox; archive/delete whole stacks".
- `Library-View-User-Guide.md`:
  - Remove the **Timeline View** section (~L106) and the Timeline toggle mention (~L76);
    describe the Main library as a single month-sectioned, newest-first view.
  - Remove sort-dropdown documentation for grid views.
  - New **Stacks** section: what a stack tile is, the badge, lightbox strip, and the action
    semantics table from `docs/plans/06-stack-actions.md` — including the explicit note that
    trashing a stack removes *all* members, while favorite/download act on the primary only.
  - Note the Immich ≥ v1.133 requirement for the library view.
- `Troubleshooting.md`: "library says server too old" entry; "stack strip missing" →
  check `stack.read`.

## Changelog

- `CHANGELOG.md`: user-facing entries — stacked assets collapse in the main library; lightbox
  stack strip; virtual scrolling with full-height scrollbar; Timeline toggle and
  Filename/File-Type sorts removed; minimum Immich version for the library view.

## Acceptance

- Test Connection against a key without `stack.read` shows the warning and still saves.
- Wiki contains no references to the Timeline toggle or grid sort dropdown.
