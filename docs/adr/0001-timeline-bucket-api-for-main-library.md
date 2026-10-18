# Main library browses via the timeline bucket API, not search/metadata

Stacks must always collapse to a single tile (the primary) in the main library, and Immich's
`search/metadata` endpoint cannot do that: `withStacked: false` drops *all* stacked assets
(including primaries), and its responses carry no stack information. Only
`/api/timeline/buckets` + `/api/timeline/bucket?withStacked=true` collapse server-side. We
therefore move the main library (Remote and Unified modes) off the shared page-number
`search/metadata` pager and onto the timeline bucket API, while every other grid view (albums,
search, favorites, people, tags, places, trash, local) stays on the existing pager and does not
collapse stacks.

Consequences:

- The main library becomes a virtualized, month-sectioned grid: `/timeline/buckets` gives
  per-month totals up front (full-height scrollbar from day one, estimated heights refined as
  buckets load), and buckets load on demand in any order.
- Bucket responses are thin (no filename, MIME type, checksum, EXIF), so assets hydrate lazily
  via `GET /api/assets/{id}` when something needs more than a tile. Exception: Unified mode
  hydrates each loaded bucket eagerly, because its sync overlay cannot work without checksums.
- Filename/File Type sorting is unimplementable on bucket data; it was removed app-wide rather
  than kept alive for non-bucket views. The main library is always newest-first, and the
  separate "Timeline" mode/toggle (whose only remaining difference was the month banner) was
  removed with it.
- Requires Immich ≥ v1.133 (columnar bucket format). Older servers get a clear "server too
  old" error in the library view; sync is unaffected.

Rejected alternatives:

- **Client-side collapse over the existing pager** (fetch `GET /api/stacks` once, filter
  non-primary members out of each page): kept all existing plumbing but made pages shrink
  unpredictably, added a staleness window, and left the scrollbar lying about library size.
- **A legacy search/metadata fallback for pre-v1.133 servers**: two complete main-library code
  paths to maintain for a shrinking audience.
