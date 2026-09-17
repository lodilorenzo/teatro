# API reference

[Documentation home](../README.md#documentation) · [Configuration](configuration.md)

- [Authentication](#authentication)
- [Browse and read](#browse-and-read)
- [Downloads](#downloads)
- [Library management](#library-management)
- [Integrity](#integrity)
- [Integration endpoints](#integration-endpoints)
- [Transfers and jobs](#transfers-and-jobs)
- [Errors and compatibility](#errors-and-compatibility)

Examples use a server on `http://127.0.0.1:4440`; substitute your HTTPS origin for
remote access. `curl --user admin` prompts for the password instead of putting it
in command history. IDs are examples; obtain real IDs from your server.

## Authentication

Protected routes accept HTTP Basic Auth, a scoped API-token Bearer header, or a
browser-session Bearer header. Catalog, cover and game access requires read access;
all `/api/admin/*` operations require effective admin access.

```bash
curl --fail --user admin http://127.0.0.1:4440/api/users/me
```

Identity response: `{"id":1,"username":"admin","role":"admin"}`.
Roles are `admin` and `readonly`. A `read` token owned by an administrator still
has effective role `readonly`. An `admin` token requires its owner to remain an
administrator; `admin` scope includes read access.

| Method and path | Request or result |
| --- | --- |
| `GET /healthz` | Public process status, start time and version derived from Cargo. |
| `GET /api/setup` | Public `{"required":true}` until an administrator exists. |
| `POST /api/setup` | JSON `username`, `password`; atomic first-admin creation, then 409 on subsequent attempts. |
| `GET /api/users/me` | Current effective user identity. |
| `POST /api/auth/session` | Basic credentials plus `{"remember_me":false}`; 201 with `user`, one-time returned `token` and Unix-second `expires_at`. Bearer credentials cannot mint sessions. |
| `DELETE /api/auth/session` | Current browser-session Bearer token; revokes that session, returns 204. |
| `GET /api/admin/users` | Users without password hashes. |
| `POST /api/admin/users` | JSON `username`, `password`, optional `role`, default `readonly`. |
| `PATCH /api/admin/users/{id}/password` | JSON `password`; invalidates browser sessions for that account. |
| `PATCH /api/admin/users/{id}/role` | JSON `role`. |
| `DELETE /api/admin/users/{id}` | Delete the account; last-admin deletion/demotion is refused. |
| `GET /api/admin/tokens` | Owner, scopes, expiry, revocation and last-use metadata, never raw tokens. |
| `POST /api/admin/tokens` | JSON `name`, optional `scopes`, `user_id` or `username`, and future RFC3339 `expires_at`. Defaults to the actor and `["read"]`. |
| `DELETE /api/admin/tokens/{id}` | Revoke an API token. |

A created API token is returned once in `token`; save it securely. Send it in
`Authorization: Bearer <token>`, never the URL. Password resets do not replace
explicit API-token revocation. Passwords use Argon2id; session and API tokens are
stored as hashes. Sessions expire after 24 hours or 30 days with Remember me.

Static `/`, `/admin`, `/setup` shells and allowlisted assets are public. They do
not make catalog data public. Download-consumption endpoints use tickets instead
of an Authorization header, as described below.

## Browse and read

| Method and path | Result |
| --- | --- |
| `GET /api/platforms` | Platform array with `id`, `name`, `display_name`, `slug`, `fs_slug`, `rom_count`. Use these IDs and folder slugs. |
| `GET /api/roms` | Paginated `{"items":[...],"total":N,"limit":N,"offset":N}`. |
| `GET /api/roms/{id}` | One game, including platform, metadata, cover paths and registered files. |
| `GET /api/stats` | Read-scoped storage totals and uptime. |
| `GET /api/admin/stats` | Administrative catalog, platform and library-root totals. |
| `GET /assets/romm/resources/{path}` | Authenticated managed cover/asset bytes. |

Game-list query parameters are `limit`, `offset`, `platform_ids`, `search` or alias
`q`, `missing_cover=true`, and `sort=recent`. `platform_ids` accepts repeated keys,
comma-separated IDs, or both. Lists otherwise sort by title with stable ID ordering;
recent sorting uses descending creation time and ID. The limit is capped at 10,000.

```bash
curl --fail --user admin \
  'http://127.0.0.1:4440/api/roms?limit=24&offset=0&search=example'
```

A game has `id`, `name`, `slug`, `platform_id`, `platform_slug`,
`platform_display_name`, `regions`, `summary`, `metadatum`, `fs_name`,
`fs_size_bytes`, cover paths/URL and `files`. `metadatum` is the only metadata-object
key; no duplicate `metadata` alias is emitted. `fs_name` identifies the preferred
launcher, while `fs_size_bytes` sums all registered physical files. Files retain
`id`, `file_name`, `file_size_bytes` plus grouping/launch fields.

## Downloads

| Method and path | Behavior |
| --- | --- |
| `GET /api/roms/{id}/download-plan` | Read-scoped `rom_id`, `preferred_file_id`, ordered `files`, and `dependencies`. No server filesystem paths. |
| `GET /api/roms/{id}/content/{file_name}?file_ids={file_id}` | Stream one registered physical file. `file_ids` is required for multi-file games. The URL filename never selects a filesystem path. |
| `POST /api/roms/{id}/files/{file_id}/download-ticket` | Authenticate and open one file; return a single-use ticket. No request body. |
| `POST /api/roms/{id}/archive-ticket` | Authenticate and prepare a temporary ZIP of every registered file; requires at least two files. No request body. |
| `POST /api/downloads/file` | Consume a file ticket in form field `ticket`; return an attachment. |
| `POST /api/downloads/archive` | Consume an archive ticket in form field `ticket`; return the prepared ZIP. |

Ticket issuance returns `{"ticket":"<opaque value>","expires_in_seconds":60}`.
Consumption uses `application/x-www-form-urlencoded`, not query parameters, and
requires no Basic/Bearer header. Tickets are hashed, single-use, process-local and
separate for files/archives. Invalid, expired or reused tickets return 404.
An issued ticket remains usable until consumption/expiry even after logout;
logout prevents new issuance. The expiry limits consumption, not transfer duration.
Ticket downloads do not implement range/resume.

A plan's files include role, launchability, disc index and group ID. Edges contain
`parent_file_id`, `child_file_id`, `dependency_kind` and `sort_index`. A client
must compute the preferred file's transitive dependencies, fetch every required
file and preserve filenames together. A `.cue` alone is not a complete game.
Browser ZIPs preserve those names in store mode; they are temporary, not new library
records. Limits and disk requirements are in [configuration](configuration.md#uploads-and-downloads).

## Library management

All paths in this table start with `/api/admin`.

| Method and path | Request or behavior |
| --- | --- |
| `POST /uploads` | Multipart `file`, `platform_id` or `platform_slug`, optional `title` or `name`. Creates one game without extracting archives. |
| `POST /uploads/preview` | JSON platform selector and `files` containing `file_name`, optional `file_size_bytes` and text `manifest_contents`. Returns proposed games/groups/dependencies, warnings and errors; no writes. |
| `POST /upload-batches` | Repeated multipart `file` or `files`, platform selector, optional repeated `planned_title` JSON with `plan_id` and `title` from the preview. Stages and validates the complete grouped set. |
| `POST /library/scans` | No body; 202 admission and `status_url`. Scans the configured default root only. |
| `GET /library/scans/{id}` | Complete latest snapshot and per-file import/skip results. |
| `DELETE /library/scans/{id}` | Cancel active scan; committed batches remain. |
| `GET /library/sidecars` | Preview up to 256 unindexed sidecar targets and fingerprints. |
| `DELETE /library/sidecars` | JSON `confirm: "DELETE SIDECARS"` and exact preview `files`; 409 if the target list or fingerprints changed. |
| `PATCH /roms/{id}` | Optional `name`, platform selector, `summary`, `regions`, `genres`, `developers`, `publishers`, `release_year`. Null clears summary/year. Does not move files. |
| `PUT /roms/{id}/cover` | Raw JPEG/PNG/WebP body, at most 10 MiB. |
| `GET /roms/{id}/files` | Admin grouped file model, managed relative paths, roles, dependencies and stored hashes. |
| `DELETE /roms/{id}` | Remove record and managed files. `delete_files=false` is rejected. |
| `DELETE /platforms/{id}/roms?confirm={platform_slug}` | Delete all games/files for that platform. |
| `DELETE /roms?confirm=DELETE%20ALL` | Clear game library/files, retaining accounts and settings. |

Deletes can remove an exclusively owned game folder including unindexed sidecars.
Read [deletion semantics](library.md#delete-games-and-sidecars) before automating them.
Multipart names must be flat safe filenames; grouped ingest validates `.m3u`,
`.cue` and `.gdi` dependencies. Supply exactly one planned title per previewed game
when using that field; do not combine it with the legacy single-game `title`.

## Integrity

| Method and path | Request or result |
| --- | --- |
| `GET /api/admin/dats` | Imported DAT sources. |
| `POST /api/admin/dats` | Multipart `file`, Logiqx XML, maximum 64 MiB. Idempotent by uploaded SHA-256. |
| `GET /api/admin/integrity/jobs` | Latest 100 persisted jobs. |
| `POST /api/admin/integrity/jobs` | JSON `{}` for all files, or `{"rom_id":42,"force":false}`; returns 202 and the job. |
| `GET /api/admin/integrity/jobs/{id}` | State and processed/hashed/matched/error counters. |
| `GET /api/admin/roms/{id}/integrity` | Per-file hashes, errors and DAT match evidence. |

```bash
curl --fail --user admin -F file=@catalog.dat http://127.0.0.1:4440/api/admin/dats
curl --fail --user admin -H 'Content-Type: application/json' -d '{}' \
  http://127.0.0.1:4440/api/admin/integrity/jobs
```

Integrity states are `queued`, `running`, `completed`, `failed`. Completed hashes
are reused unless forced; matching checks every declared checksum and optional
size. See [integrity behavior](library.md#integrity-and-dat-verification).

## Integration endpoints

All paths below start with `/api/admin`. Setup, prerequisites and limitations are
in [integrations](integrations.md). Credential responses indicate presence, not
secret values.

| Method and path | Request or result |
| --- | --- |
| `GET /igdb/status` | Credential availability and token-cache state. |
| `GET /igdb/settings` | Effective credential sources and saved-settings status. |
| `PATCH /igdb/settings` | JSON `client_id`, optional `client_secret`; omitted/empty secret preserves an existing saved secret. |
| `DELETE /igdb/settings` | Clear saved overrides; environment credentials may become effective again. |
| `GET /igdb/search` | `q`, optional `limit` up to 25, `platform` or `platform_slug`, `require_platform_match`. Platform searches can fall back to all platforms. |
| `POST /roms/{id}/metadata/igdb` | Selected search result under `match` or `candidate`, optional `cache_cover`. |
| `GET /gog-import/status` | Enabled/configured status, bundled-extractor mode and limits. |
| `POST /gog-imports` | Multipart `title` or `name`, one `.exe` and matching `.bin` parts as repeated `file` or `files`; 202 after staging. |
| `GET /gog-imports/{id}` | Job snapshot and events. Optional `after` is a single unsigned cursor; use returned `next_event_seq` for the next poll. |
| `DELETE /gog-imports/{id}` | Cancel active work; 409 after publication begins. |
| `GET /sources/romm/status` | Source/index status, safe URL and username, secret-presence flag. |
| `PATCH /sources/romm/settings` | JSON `base_url`, `username`, optional `secret`, `auth_mode` of `token` or `basic`; HTTP needs `acknowledge_plaintext_http:true`. Omit secret to retain it. |
| `DELETE /sources/romm/settings` | Clear source credentials, access-token cache and browse index. |
| `POST /sources/romm/test` | Probe result: `reachable`, `unauthorized` or `unreachable`. No index refresh. |
| `POST /sources/romm/refresh` | Fetch complete bounded catalog; replace local index atomically on success. |
| `GET /sources/romm/platforms` | Indexed populated platforms and any matching local target. |
| `GET /sources/romm/roms` | Local snapshot query: `platform_id`, `search`, `limit` from 1 to 48, `offset`. |
| `GET /sources/romm/roms/{remote_id}` | Live remote detail and selectable physical file IDs. |
| `GET /sources/romm/roms/{remote_id}/cover` | Bounded remote cover proxy, no-store. |
| `POST /sources/romm/imports` | JSON `remote_rom_id`, `remote_file_ids`, `platform_id`, optional `title`; 202 with status URL. |
| `GET /sources/romm/imports/{id}` | Latest import snapshot; inspect `already_present` outcomes and `hash_conflicts`. |
| `DELETE /sources/romm/imports/{id}` | Cancel active import. |

Disabled RomM routes return 404. GOG import returns 503 if disabled or unconfigured.
An empty remote hash-conflict list may mean no hashes were supplied, not that
checksum verification occurred. Remote declared-size mismatches fail; declared-hash
mismatches remain warnings on an otherwise successful import.

## Transfers and jobs

Scan, GOG and RomM admissions return 202 with `job`, relative `status_url` and a
matching `Location` header. Poll the authenticated status URL. Workflow failures
can be HTTP 200 snapshots with `state:"failed"` and an `error`; inspect the body.
States include `queued`, `running`, `succeeded`, `failed`, `cancelled`. These jobs
are in memory, not durable queues. Unknown, expired or restart-lost IDs return 404.
Cancellation does not undo already committed library work.

For batch uploads and pre-admission GOG transfers, optional
`X-Teatro-Transfer-Id` enables bounded replay protection. Use a new ID for different
work, beginning with `upload_` or `gog_`, with a nonempty ASCII letter/digit/underscore
suffix and at most 128 total bytes. The key includes owner, method and endpoint;
every replay still requires current admin authorization.

- Running duplicate: `409 transfer_in_progress` and `Retry-After`.
- Retained successful duplicate: original response, without processing new bytes.
- Cancelled duplicate: `410 transfer_cancelled`.
- Failed requests release the key. Terminal entries share a 64-entry, one-hour
  process-local cache. Restart or eviction removes replay protection.

`DELETE /api/admin/background-transfers/{id}?operation=upload` cancels the caller's
batch upload; `operation=gog` targets the pre-admission GOG transfer. The selector
is required. Unknown/other-owner entries return 404, completed work 409, and active
or already-cancelled entries 204. Use the admitted-job endpoints after GOG admission.
Check the library before retrying uncertain work after cache loss; IDs are not a
durable exactly-once guarantee and payloads are not fingerprinted.

## Errors and compatibility

JSON API errors use `{"error":{"code":"not_found","message":"..."}}`.
Common statuses are 400 invalid input, 401 invalid credentials, 403 insufficient
access or unsafe paths, 404 missing resource, 409 conflict, 413 size limit, 422
invalid/changed file set, 429 capacity or authentication limits, 503 unavailable
integration, and 507 insufficient storage. Respect `Retry-After`; do not retry
all conflicts as if they meant `transfer_in_progress`.

This is a focused ROMM-compatible API, not a complete ROMM implementation.
Keep numeric IDs, safe platform slugs, `metadatum`, deterministic file ordering
and dependency handling intact in clients. The exact routes are in
[`src/api/mod.rs`](../src/api/mod.rs); management request/response structures are in
[`dto.rs`](../src/api/admin/dto.rs), with read/download translation in
[`romm.rs`](../src/api/romm.rs). Synthetic [contract tests](../tests/romm_contract.rs)
cover the supported subset without providing game content.
