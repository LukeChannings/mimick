# Phase 6 — Stack-aware grid and bulk actions

Depends on Phase 4. See overview decisions 8 and 9. Files: `src/library/context_menu.rs`,
`src/library/actions.rs`, `src/library/archive.rs`, `src/library/dialogs.rs`.

## Semantics (decided — do not change)

| Action | Stack tile target |
| --- | --- |
| Archive / unarchive | **All stack members** |
| Delete (trash) | **All stack members** |
| Favorite / unfavorite | Primary only |
| Download | Primary only |
| Copy to clipboard | Primary only |
| Open (activate) | Primary (strip shows members — Phase 5) |
| Add to shared link / album-style bulk ops | Primary only |

## Fan-out mechanics

- Member IDs are not in bucket data: at action time, for each selected stack tile, call
  `fetch_stack(stack_id)` (Phase 2) and collect member IDs; then use the existing bulk
  endpoints (`delete_assets`, `set_assets_visibility`) with the expanded ID list.
- **No partial action**: resolve *all* stacks first, then fire the mutation. If any
  `fetch_stack` fails with 403, abort the whole action and show the existing
  permission-error dialog pattern (`show_library_permission_error` /
  `show_alert_dialog`) with: "Archiving/deleting a stack requires the `stack.read` API
  permission." Non-403 failures: generic error, also aborted.
- **Confirmation copy**: the trash confirm (`confirm_trash` in `src/library/actions.rs`)
  must disclose scope — e.g. single stack tile: "Move stack of 4 to trash?"; mixed bulk
  selection: "Move 7 items (including 2 stacks, 11 files total) to trash?" Counts come from
  `stack-count` before fetch; exact IDs from the fetch.
- After a stack archive/delete: reload the affected bucket(s) (Phase 3 store) so the tile
  disappears and month counts stay truthful. A full bucket-list refresh is acceptable v1.

## Acceptance

- Trashing a stack tile trashes all members (verify in Immich trash) and the tile vanishes.
- Favoriting a stack tile favorites exactly one asset (the Primary).
- Downloading a stack tile produces one file.
- With a `stack.read`-less key: archive/delete on a stack tile shows the permission dialog
  and does nothing; the same actions on non-stacked tiles work normally.
- Unit tests: confirm-dialog copy composition; fan-out abort-on-403 logic (pure parts).
