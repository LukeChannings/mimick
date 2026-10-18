# Stacked images: overview

Immich groups assets into **stacks** (burst shots, RAW+JPEG). Mimick currently shows every
asset individually. This plan collapses stacks to a single tile in the **Main library** and
adds a thumbnail strip in the lightbox for choosing among stack members.

Read first:

- `CONTEXT.md` — canonical terms: Main library, Bucket, Hydration, Stack, Primary, Stack tile.
- `docs/adr/0001-timeline-bucket-api-for-main-library.md` — why the main library moves to the
  timeline bucket API instead of `search/metadata`.

## Decisions already made (do not relitigate)

1. Stacks collapse **only in the Main library** (Remote and Unified modes). Albums, search,
   favorites, people, tags, places, trash and local views show every asset individually.
2. The Main library moves to `/api/timeline/buckets` + `/api/timeline/bucket?withStacked=true`
   (server-side collapse). Requires Immich ≥ v1.133 (columnar response); older servers get a
   clear error in the library view, sync unaffected.
3. The grid becomes a **virtual, month-sectioned** canvas: all bucket counts are known up
   front (truthful full-height scrollbar), buckets load on demand in any order, estimated
   heights are refined to exact on load with scroll anchoring.
4. **Lazy hydration**: tiles render from thin bucket data; full asset records are fetched only
   on interaction. Exception: Unified mode hydrates each loaded bucket eagerly (checksums for
   the sync overlay). Lightbox preloads ±10 tiles.
5. The separate **Timeline mode/toggle is removed**; Main library is always newest-first.
   **Filename/File Type sorts are removed app-wide**; grid sort dropdown deleted.
6. Grid grows **inline month labels** that scroll with content (not sticky); the floating
   month-year banner stays (becomes exact). The "Photos" source header (icon/title/counts) is
   dropped for the Main library.
7. Lightbox stack strip is **read-only** (no set-primary/unstack). Arrows/swipes navigate
   tiles, never into a stack; the strip is the only way to move within one. Lightbox actions
   target the currently displayed member.
8. Grid-level action semantics for a Stack tile: **archive/delete fan out to all members**
   (confirm dialog says "stack of N"); **favorite/download/copy act on the Primary only**;
   open shows the Primary with the strip.
9. `stack.read` is validated by **probing at settings time** (no Immich endpoint reports key
   permissions): warning, never a save blocker. At runtime, a 403 hides the strip silently and
   makes stack archive/delete fail with a permission error dialog (no partial action).

## Phases (work sequentially; each file is self-contained)

| File | Phase | Depends on |
| --- | --- | --- |
| `01-removals.md` | Remove Timeline mode + client-side sorts | — |
| `02-api-layer.md` | Timeline/stack endpoints, version gate, probe | — |
| `03-virtual-grid.md` | Bucket store + virtual sectioned canvas | 01, 02 |
| `04-tiles-and-hydration.md` | Thin tiles, stack badge, hydration | 03 |
| `05-lightbox.md` | ±10 preload + stack strip | 04 |
| `06-stack-actions.md` | Stack-aware grid/bulk actions | 04 |
| `07-settings-and-docs.md` | Settings probe UI, wiki, changelog | 02 |

Phase 3 is the riskiest (estimated-height scroll anchoring). A throwaway spike of the
anchoring math before full implementation is explicitly sanctioned.
