# Changelog

This changelog tracks user-visible functionality in public Teatro versions. It
omits internal refactors and publication-process changes unless they affect
installation or operation.

## 0.19.9 — 2026-09-28

Changes from 0.19.7:

### Metadata

- IGDB searches now remove file extensions and tags in parentheses or brackets
  from game titles before searching.
- Applying an IGDB match now updates the saved game title to the matched IGDB
  name as well as saving metadata and covers.

### Player library

- Search now updates automatically after a short typing delay while preserving
  input focus and cursor position.
- Clearing a global search returns to the library home page.
- Search results show each game's platform icon; platform-filtered results show
  icons when a title search is active.
- Game-card action spacing now adjusts whenever a platform icon is present.

### Container deployment

- The published image target is `ghcr.io/lodilorenzo/teatro:0.19.9` for amd64 and
  arm64; there is no `latest` tag.
- The bundled innoextract build uses only its required Boost components and
  supports an explicit parallel-build limit.

Version 0.19.8 was not published.

## 0.19.7 — 2026-09-18

Changes from 0.19.6:

- Added native amd64 and arm64 beta-image publication through GHCR.
- Added signed image indexes, SBOM and provenance output, vulnerability gates,
  retained notices and Debian source packages.
- Added fresh-volume, authentication, persistence and clean-shutdown checks for
  published images.
- Application functionality is unchanged from 0.19.6.

## 0.19.6 — 2026-09-17

First public beta source baseline. It includes:

- Authenticated player and administrator interfaces.
- Uploads, managed-library scans, browser downloads and file deletion.
- IGDB metadata and cover matching.
- Integrity hashing and Logiqx XML DAT verification.
- Optional GOG offline-installer extraction.
- Optional remote RomM browsing and imports.
- Read-only and administrator accounts, browser sessions and scoped API tokens.
- Optional same-link IPv4 discovery.
