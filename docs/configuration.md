# Configuration

[Documentation home](../README.md#documentation) · [Docker operations](docker.md)

Teatro reads environment variables at startup. It does **not** load `.env` files.
Docker Compose reads its own local `.env` for interpolation into the committed
[Compose file](../docker-compose.yml). Run `docker compose up -d` after changing
values so Compose recreates the container. A plain container restart keeps its
old environment.

[`.env.example`](../.env.example) lists native-development values; do not copy it
unchanged into Docker. Its loopback listener and relative paths are wrong for the
container defaults. Empty values are not universally equivalent to unset values.
Use explicit `true` or `false` for feature flags and decimal integers for limits.
Byte limits below use binary units for readability; enter the integer byte count.

## Paths and networking

| Variable | Native default | Docker/Compose default or meaning |
| --- | --- | --- |
| `TEATRO_BIND_ADDR` | `127.0.0.1:4440` | `0.0.0.0:4440`; server listener, not the host port mapping. |
| `TEATRO_DATA_DIR` | `./data` | `/data`; runtime state and temporary archive work. |
| `TEATRO_DATABASE_URL` | `sqlite://data/teatro.sqlite3` | `sqlite:///data/teatro.sqlite3`; does not automatically follow a changed data directory. |
| `TEATRO_DEFAULT_LIBRARY_ROOT` | `DATA_DIR/roms` | `/data/roms`; writable managed game files. |
| `TEATRO_ASSET_ROOT` | `DATA_DIR/assets` | `/data/assets`; writable cached/manual covers. |
| `TEATRO_LOG_FORMAT` | `pretty` | `json`; accepted values are `pretty` and `json`. |
| `RUST_LOG` | See `.env.example` | `teatro=info,tower_http=info` tracing filter. |
| `TEATRO_LAN_DISCOVERY_ENABLED` | `false` | Opt-in IPv4 advertisement; see [discovery](integrations.md#lan-discovery). |
| `TEATRO_DISCOVERY_NAME` | `Teatro` | Friendly discovery name, at most 48 UTF-8 bytes. |
| `TEATRO_TRUSTED_PROXY_IPS` | Empty | Exact comma-separated proxy peer IPs allowed to supply forwarding headers. |

Relative native paths resolve from the process working directory. Native library
and asset defaults follow `TEATRO_DATA_DIR` when unset; Compose explicitly supplies
all paths, so changing its data directory alone does not move the others. Keep
mounts, database URL and all root paths consistent. Changing a path does not migrate
existing rows or files. See [storage and restore](docker.md#storage).

Compose-only interpolation values, not application settings:

| Variable | Default | Purpose |
| --- | --- | --- |
| `TEATRO_IMAGE` | `teatro:local` | Locally built image name. This is not a published registry image. |
| `TEATRO_HOST_PORT` | `4440` | Host-side port, or `127.0.0.1:4440` to publish on host loopback only. |
| `TEATRO_DATA_VOLUME` | `teatro-data` | Persistent named volume. Use distinct names for isolated instances. |

## Uploads and downloads

| Variable | Default | Limits |
| --- | --- | --- |
| `TEATRO_MAX_UPLOAD_BYTES` | `137438953472` | 128 GiB per incoming file. |
| `TEATRO_MAX_UPLOAD_BATCH_FILES` | `256` | Files per upload batch. |
| `TEATRO_MAX_UPLOAD_BATCH_BYTES` | `137438953472` | 128 GiB aggregate input per batch. |
| `TEATRO_MAX_CONCURRENT_UPLOADS` | `2` | Admitted upload/import work; GOG holds its permit through processing. |
| `TEATRO_MAX_MULTIPART_TEXT_BYTES` | `4096` | Each multipart text field. |
| `TEATRO_UPLOAD_FREE_SPACE_MARGIN_BYTES` | `536870912` | Keep 512 MiB free while uploading. |
| `TEATRO_UPLOAD_DISK_CHECK_INTERVAL_BYTES` | `67108864` | Recheck space per 64 MiB of writes. |
| `TEATRO_STALE_UPLOAD_AGE_SECONDS` | `86400` | Startup cleanup threshold for abandoned upload parts. |
| `TEATRO_MAX_DOWNLOAD_ARCHIVE_FILES` | `256` | Registered files per browser ZIP. |
| `TEATRO_MAX_DOWNLOAD_ARCHIVE_BYTES` | `53687091200` | 50 GiB aggregate source size per browser ZIP. |
| `TEATRO_MAX_CONCURRENT_DOWNLOAD_ARCHIVES` | `1` | ZIP preparation/download slots. |
| `TEATRO_DOWNLOAD_ARCHIVE_FREE_SPACE_MARGIN_BYTES` | `536870912` | Keep 512 MiB free on the temporary archive filesystem. |

These are ceilings, not disk reservations. Keep space for source uploads,
extraction, ZIPs, staging and final files that may coexist. Match proxy limits to
intended uploads. Cover uploads have a fixed 10 MiB limit and DAT uploads a fixed
64 MiB limit. Single-use download tickets expire after 60 seconds and their file
and archive registries each hold at most 64 pending tickets.

## Integration settings

Read [integration setup](integrations.md) before enabling imports.

| Variable | Default | Purpose |
| --- | --- | --- |
| `TEATRO_IGDB_CLIENT_ID`, `TEATRO_IGDB_CLIENT_SECRET` | Unset | Optional server-side IGDB credentials; saved Settings values take precedence. |
| `TEATRO_GOG_IMPORT_ENABLED` | `false` | Enable reviewed offline-installer extraction. |
| `TEATRO_INNOEXTRACT_PATH` | Unset | Explicit absolute extractor path; pair with its SHA-256. |
| `TEATRO_INNOEXTRACT_SHA256` | Unset | Lowercase SHA-256 of that exact executable. Leave both unset for Docker's bundled discovery. |
| `TEATRO_GOG_IMPORT_HELPER_PATH` | Unset | Separately reviewed, dedicated optional helper directory. |
| `TEATRO_GOG_IMPORT_TIMEOUT_SECONDS` | `1800` | Processing timeout. |
| `TEATRO_GOG_IMPORT_MAX_EXTRACTED_BYTES` | `53687091200` | 50 GiB extracted content ceiling. |
| `TEATRO_GOG_IMPORT_MAX_EXTRACTED_FILES` | `20000` | Extracted file ceiling. |
| `TEATRO_ROMM_SOURCE_ENABLED` | `false` | Enable the remote RomM source routes. |
| `TEATRO_ROMM_SOURCE_TIMEOUT_SECONDS` | `30` | Remote metadata/probe request timeout. |
| `TEATRO_ROMM_IMPORT_TIMEOUT_SECONDS` | `3600` | Import timeout. |
| `TEATRO_ROMM_IMPORT_MAX_BYTES` | Upload byte limit | Blank/unset falls back to `TEATRO_MAX_UPLOAD_BYTES`. |
| `TEATRO_ROMM_IMPORT_MAX_FILES` | `64` | Selected files per remote import. |

RomM's URL, username, authentication mode and secret are runtime Settings, not
environment variables. IGDB and RomM saved secrets are write-only through HTTP,
but stored in SQLite. They are not protected by hashing like login passwords.
Protect database copies, container administration and the host.

## Authentication limits

| Variable | Default | Purpose |
| --- | --- | --- |
| `TEATRO_AUTH_RATE_LIMIT_MAX_FAILURES` | `10` | Failures before lockout; `0` disables this protection. |
| `TEATRO_AUTH_RATE_LIMIT_WINDOW_SECONDS` | `300` | Failure-count window. |
| `TEATRO_AUTH_RATE_LIMIT_LOCKOUT_SECONDS` | `60` | Lockout duration. |
| `TEATRO_AUTH_RATE_LIMIT_MAX_ENTRIES` | `10000` | Bounded in-memory authentication keys. |
| `TEATRO_PASSWORD_CONCURRENCY` | `2` | Concurrent Argon2 workers. |
| `TEATRO_PASSWORD_QUEUE_DEPTH` | `16` | Maximum queued password checks. |
| `TEATRO_PASSWORD_QUEUE_TIMEOUT_SECONDS` | `5` | Maximum wait for a worker. |
| `TEATRO_MAX_USERNAME_BYTES` | `128` | HTTP username bound. |
| `TEATRO_MAX_PASSWORD_BYTES` | `1024` | HTTP password bound. |
| `TEATRO_TOKEN_TOUCH_INTERVAL_SECONDS` | `300` | Minimum interval for persisting API-token last-use updates. |

Excess work returns HTTP 429 with `Retry-After`; do not disable limits to hide a
bad client retry loop. Rate-limit state is process-local. Browser sessions have
fixed 24-hour or 30-day expiry and a maximum of 64 active sessions per account.

For noninteractive CLI use only, `TEATRO_ADMIN_PASSWORD` supplies `users create-admin`
and `TEATRO_USER_PASSWORD` supplies `users create` or `users reset-password`.
Prefer interactive prompts. Do not place these variables in the long-lived service
environment, source tree or command logs. They do not reset accounts merely by
starting the server.
