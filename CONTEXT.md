# Mimick

A desktop client for an Immich server: it uploads watched local folders and lets you browse the remote library.

## Language

### Library browsing

**Main library**:
The unfiltered view of everything in the user's Immich timeline, always sorted newest-first. It has three source modes: Remote, Local, and Unified.
_Avoid_: Timeline, all assets, home

### Library browsing (cont.)

**Bucket**:
One calendar month of the **Main library**, as served by Immich's timeline API. The unit of loading and of layout: the grid knows every bucket's asset count up front, but fetches a bucket's contents only when scrolled into view.
_Avoid_: Page (buckets are random-access; pages are sequential)

**Hydration**:
Upgrading a thin bucket entry (ID, ratio, thumbhash, flags) to a full asset record (filename, MIME type, checksum, EXIF). Happens on interaction, never on scroll — except in Unified mode, where every loaded bucket hydrates eagerly for the sync overlay.

### Stacks

**Stack**:
An Immich grouping of two or more assets, such as burst shots or RAW+JPEG pairs. Exactly one of them is the **Primary**.
_Avoid_: Group, bundle, set

**Primary**:
The one asset in a **Stack** that represents the whole stack.
_Avoid_: Cover, lead, top asset

**Stack member**:
Any asset that belongs to a **Stack**, including the **Primary**.

**Stack tile**:
The single grid tile that stands in for a whole **Stack** in the **Main library**. It shows the **Primary**. Stacks always collapse into stack tiles in the Main library's server-backed modes (Remote, Unified); Local mode has no stacks because stacks exist only on the server. Every other view (search, albums, favorites, people, etc.) shows each matching asset individually.

## Flagged ambiguities

- "Timeline" was used to mean the whole main library, and also named a separate date-banner mode. Resolved: the separate Timeline mode is removed; the umbrella term is **Main library**. Avoid "timeline" except when naming the Immich server's timeline API.
