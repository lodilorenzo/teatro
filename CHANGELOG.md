# Changelog

This changelog tracks user-visible functionality in public Teatro versions. It
omits internal refactors and publication-process changes unless they affect
installation or operation.

## 0.22.1 - 2026-10-15

Changes since 0.21.3. Version 0.22.0 was not published.

### Player library

- Browse without a title query and combine play mode, year, score, genre, developer, publisher, region, language, tag and total-size filters before pagination.
- Sort by title, date added, release year, rating or total size in either direction. Applied controls stay in the URL.
- Use two-ended year, score and size sliders with exact inputs and bounds derived from matching games. Untouched ranges include missing metadata; unknown values sort last.
- Show scores and play modes in game cards and details. Reapply IGDB metadata to older games to populate play modes.

### Collections and access

- Create, edit and delete collections in Admin, selecting games across search pages. Each collection has a dedicated URL and appears on the player home.
- Regular collections require login. Enabling LAN Party Zone explicitly allows anonymous browsing, covers and downloads for selected games only. This is not an IP/subnet restriction.
- Membership is checked before listing, filtering, details, covers and download-ticket issuance. Disabling a zone stops new anonymous requests; existing single-use tickets keep their 60-second lifetime.
- Deleting a collection keeps its games and files; changing its URL name breaks the old link.

### Upgrade notes

- Startup migrations add collection tables and remove stored age classifications from metadata and RomM provenance. Other metadata and game files are preserved. Age classifications are no longer imported or displayed.
- Back up the complete stopped-instance data set before upgrading. Rollback requires the matching pre-upgrade backup.
- Container publication remains beta with scoped security exceptions. The runtime UID/GID remains 10001:10001.

## 0.21.3 — 2026-10-06

Changes from 0.19.9:

### Experimental compression on import

Disabled unless `TEATRO_CONVERSION_ENABLED=true`. Outputs are verified against
their inputs but not yet qualified with emulators. See the
[library guide](docs/library.md#compress-on-import-experimental).

- Wii U: decrypted base game, update and DLC folders become one `.wua`, with
  folder drag-and-drop, local family checks and IGDB metadata after import.
- GameCube and Wii: plain ISO/GCM and single-file WBFS discs become `.rvz`.
- PS1, PS2, PSP, Saturn, Sega CD, PC Engine CD and Neo Geo CD: supported
  ISO, IMG and CUE/BIN discs become one `.chd` per disc, with disc sets grouped
  into one job per game and `.m3u` playlists pointing to the compressed discs.
- Other non-disc platforms: uncompressed single-file ROMs become one `.7z` each;
  existing archives in the same selection upload unchanged.
- Jobs shows upload, compression and verification progress for these imports
  and can cancel them until publication begins.

### Building from source

- Rust 1.93 or newer is required, along with a C/C++ toolchain, libclang and
  zlib development headers. The linked conversion library needs libstdc++ and
  zlib at runtime. The Docker image includes these.

### Container deployment

- The published image target is `ghcr.io/lodilorenzo/teatro:0.21.3` for amd64 and
  arm64; there is no `latest` tag.
- Debian packages come from the 2026-10-05 snapshot, which includes the PCRE2
  and OpenSSL security updates.

Versions 0.19.10 to 0.21.2 were not published.

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
