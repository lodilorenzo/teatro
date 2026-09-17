# Library guide

[Documentation home](../README.md#documentation) · [Integrations](integrations.md)

- [Browse and download](#browse-and-download)
- [Upload games](#upload-games)
- [Scan existing files](#scan-existing-files)
- [Edit metadata and covers](#edit-metadata-and-covers)
- [Jobs and cancellation](#jobs-and-cancellation)
- [Delete games and sidecars](#delete-games-and-sidecars)
- [Integrity and DAT verification](#integrity-and-dat-verification)

## Browse and download

Sign in at `/` with a player or administrator account. Browse populated platforms,
search across the library or within a platform, and open a game for metadata,
covers and files. The home page also shows recent additions and storage figures.
Compatible clients connect to the same server using a [read-only account](docker.md#accounts-and-administration).

Individual downloads contain the selected file unchanged. A multi-file game's
package download creates a temporary ZIP containing all registered files with
filenames preserved, including playlist and track references. Extract the complete
package together. Downloading only a `.cue`, `.gdi` or `.m3u` omits its dependencies.
Teatro serves files; the client supplies emulators and launch configuration.

Browser transfers use the normal download manager. Temporary ZIPs consume server
disk space and an archive slot until consumed or cleaned up. Downloads use
single-use tickets that must be consumed within 60 seconds; that is not a limit
on transfer duration. Interrupted browser downloads need a new download request;
range/resume is not supported on ticket endpoints.

Browser passwords are not saved. Sign-in lasts up to 24 hours with tab-scoped
session storage, or 30 days with **Remember me** and origin-scoped local storage.
Player and admin pages share the session. Use Remember me only on trusted devices.
Sign out explicitly; closing a browser may preserve its restored tab storage.

## Upload games

In `/admin`, open Import, select a platform and choose files. Review the preview,
its warnings and proposed titles before starting. Uploads appear in Jobs and clear
the preparation form so another batch can be prepared.

- A single file becomes one game. Normal uploads do not extract ZIP or 7z content.
- Select a `.cue` or `.gdi` together with every referenced track.
- Select a supplied `.m3u` together with all referenced discs and their dependencies.
  Teatro preserves playlist order.
- Recognizable multi-disc sets can receive a generated `.m3u` when no supplied
  playlist owns them. Check the preview rather than assuming a naming convention
  was recognized correctly.
- Select flat filenames, not absolute paths or parent-directory references.
  Missing dependencies, unsafe names and split archive volumes are rejected.
- Multi-file games use a per-game folder. Existing managed filenames may be skipped
  in the preview; other collisions receive safe suffixes rather than overwriting data.

Use platform `win` for Windows games packaged as ZIP/7z archives.
Teatro stores those archives unchanged. [GOG import](integrations.md#gog-offline-installers)
is a separate extraction workflow.

Defaults allow 128 GiB per uploaded file, 256 files and 128 GiB per batch, with two
concurrent uploads. Browser storage, available disk and proxy limits also apply.
See [configuration](configuration.md#uploads-and-downloads) before raising limits.

## Scan existing files

Place files under the configured default library root. Use `fs_slug` values from
`GET /api/platforms`, not display names or guessed folder names. Supported layouts:

```text
roms/
  snes/
    Example.sfc
  psx/
    Example Game/
      Example.cue
      Example (Track 01).bin
```

Standalone files may be at the platform root or one direct game-folder level.
Keep grouped files together in a direct game folder; arbitrary nesting is not
supported. Start a library scan from administration and read the confirmation.

The scan imports new valid files in place. It does not copy or rewrite their
contents, follow symlinks, repair indexed games, attach new files to existing games,
or remove records for missing files. It can generate a playlist and rename a
complete multi-disc folder to the managed playlist-based layout. Back up first if
you need the original folder names.

Already indexed files, unknown platforms, incomplete groups, unsupported nesting
and sidecars appear in the result. Sidecars stay on disk; scanning does not delete
them. Configured IGDB matching may add metadata and covers after ingest. A cover
failure does not undo a successful import.

## Edit metadata and covers

Open a game in administration to change its title, logical platform, summary,
regions, genres, developers, publishers and release year. Logical title/platform
changes do not move the physical files. Use [IGDB](integrations.md#igdb-metadata)
for lookup and selection, including correction of an automatic match.

Replace a cover with JPEG, PNG or WebP, up to 10 MiB. The Missing cover filter helps
find games needing artwork. Cached covers are private managed assets and require
the same read authentication as the catalog.

## Jobs and cancellation

Jobs shows uploads, scans, GOG imports and remote RomM imports, with progress,
results and warnings. Keep successful results long enough to inspect skipped files,
cover failures and remote hash conflicts.

Launched upload and GOG payloads are temporarily retained in browser IndexedDB.
They can continue through navigation between Teatro pages in the same tab, not
through a guaranteed background service. Storage quotas or blocked IndexedDB can
prevent admission. Treat site storage as sensitive; do not start imports on an
untrusted/shared device. Closing the tab or clearing storage is not a reliable
cancellation method. Use Cancel and wait for its result.

Admitted server jobs continue without polling. Cancelling a scan retains batches
already committed. GOG cancellation is refused once archive publication begins.
Completed work is not rolled back by dismissing a Jobs card.

Scan, GOG and RomM status resources are process-local. Restart loses those IDs;
terminal records also expire. Managed-file recovery can complete publication even
when its status ID is gone. Check the library before resubmitting after a restart,
missing job or uncertain response. There is no durable import resume.

## Delete games and sidecars

**Deleting a game removes managed files from disk, not just its catalog entry.**
If a multi-file game exclusively owns its managed folder, deletion also removes
unindexed files inside it. Do not keep unrelated files in game folders. The
operation journal recovers interrupted deletes; it is not an undo or recycle bin.

Platform-wide deletion requires the platform slug. Clearing the entire library
requires `DELETE ALL`; it retains accounts and server settings. Take a backup
before bulk deletion. The API does not support keeping orphaned files with
`delete_files=false`.

Sidecar cleanup is separate: preview the exact unindexed files, inspect them, then
confirm `DELETE SIDECARS`. Recognized text, metadata and download sidecars may be
useful to you, so do not approve solely because they are listed. Changed previews
are rejected. Cleanup removes files only, at most 256 per preview; repeat the
preview for remaining batches.

## Integrity and DAT verification

Integrity management is available through the [admin API](api.md#integrity).
It is optional and does not block uploads or downloads.

1. Import a Logiqx XML DAT from a source you trust. No-Intro/Redump-style `game` or
   `machine` entries are supported. The limit is 64 MiB; entries without supported
   checksums are skipped. Reimporting identical DAT bytes is idempotent.
2. Start an integrity job for one game or the full library.
3. Poll the job, then inspect each game's integrity report.

Teatro streams each physical file once to compute CRC32, MD5, SHA-1 and SHA-256.
It hashes an archive itself, not its members. Completed hashes are reused for DAT
rematching; use `force: true` after out-of-band file changes to reread the bytes.

A DAT match requires the declared size, when present, and every declared checksum
to agree. Matching records may supply verified serial, language and region metadata;
unmatched files retain filename-derived identity. A mismatch is not proof of
corruption if the DAT describes another edition or archive representation.

Integrity jobs are stored in SQLite. Interrupted queued/running jobs become failed
on restart; start another job to continue. ClrMamePro text DATs and archive-member
hashing are not supported.
