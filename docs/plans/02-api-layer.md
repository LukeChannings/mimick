# Phase 2 — API layer: timeline buckets, stacks, version gate, permission probe

All in `src/api_client/`. No UI changes. Independent of Phase 1. See
`docs/adr/0001-timeline-bucket-api-for-main-library.md`.

## Timeline bucket endpoints (new module, e.g. `src/api_client/timeline.rs`)

- `fetch_timeline_buckets(visibility: &str) -> Result<Vec<TimeBucket>, String>`
  - `GET /api/timeline/buckets?visibility=timeline&withStacked=true&order=desc`
  - `TimeBucket { time_bucket: String, count: u32 }`. Treat `time_bucket` as an **opaque key**
    (it is an ISO date string like `2025-06-01`); never do local date math on it beyond
    deriving the "June 2025" display label.
- `fetch_timeline_bucket(bucket: &str, visibility: &str) -> Result<Vec<TileAsset>, String>`
  - `GET /api/timeline/bucket?timeBucket={bucket}&visibility=timeline&withStacked=true&order=desc`
  - The response is **columnar** (arrays per field, same length):
    `id, ownerId, ratio, isFavorite, visibility, isTrashed, isImage, thumbhash, fileCreatedAt,
    localOffsetHours, duration, stack, projectionType, livePhotoVideoId, city, country,
    latitude, longitude`. `stack` entries are `null` or a `[stackId, assetCount]` tuple of
    strings (parse count from string).
  - Transpose into row structs:
    `TileAsset { id, ratio: f64, thumbhash: Option<String>, is_image: bool,
    duration: Option<String>, is_favorite: bool, visibility: String, file_created_at: String,
    stack: Option<(String, u32)> }` (extend if later phases need more columns).
  - Mismatched column lengths → `Err`, never a panic.
- Follow the house style of `fetch_search_assets` in `src/api_client/search.rs`: api-key
  header, 10 s timeout, `classify_http_issue`/`classify_network_issue` with a new
  `RequestContext` variant, `clear_issue` on success.

## Stack endpoints

- `fetch_stack(id: &str) -> Result<StackResponse, String>` — `GET /api/stacks/{id}`.
  `StackResponse { id, primary_asset_id, assets: Vec<LibraryAsset> }` (members come as full
  asset DTOs, primary first; reuse `LibraryAsset` deserialization so checksums normalize).
  Surface 403 distinctly (callers branch on missing `stack.read`).

## Server version gate

- Parse the semver from the existing `/api/server/about` fetch (`ServerAbout`).
- `supports_timeline_buckets(&ServerAbout) -> bool` — true iff version ≥ **1.133.0**.
  Unparseable version → `true` (fail open; the endpoint call itself will error loudly).
- Unit-test the comparison (1.132.x false, 1.133.0 true, 2.x true, garbage true).

## Permission probe

- `probe_permissions() -> Vec<PermissionProbeResult>` driven by a small table so future
  permissions can be added. One row today:
  - `stack.read` → `GET /api/stacks?primaryAssetId={random-uuid-v4}` — `200` ⇒ granted
    (empty array), `403` ⇒ missing, anything else ⇒ `Unknown` (do not warn on Unknown).
- `PermissionProbeResult { permission: &'static str, feature_hint: &'static str, state: Granted|Missing|Unknown }`.

## Acceptance

- `cargo test` with unit tests for: columnar transpose (including null/short columns →
  error), stack tuple parsing, version comparison, probe classification of 200/403/timeout.
- No behavior change anywhere in the UI yet.
