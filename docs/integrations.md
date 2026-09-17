# Integrations

[Documentation home](../README.md#documentation) · [Configuration](configuration.md)

- [IGDB metadata](#igdb-metadata)
- [GOG offline installers](#gog-offline-installers)
- [Remote RomM imports](#remote-romm-imports)
- [Compatible clients](#compatible-clients)
- [LAN discovery](#lan-discovery)

## IGDB metadata

Obtain a Twitch application client ID and client secret for IGDB access, following
the [IGDB authentication instructions](https://api-docs.igdb.com/#authentication).
Save both in administration's Settings, or supply `TEATRO_IGDB_CLIENT_ID` and
`TEATRO_IGDB_CLIENT_SECRET` in the server environment. No credentials ship with Teatro.

Saved database values override environment values. Clearing saved settings restores
any environment fallback; remove those too if you want IGDB disabled. Credential
status is visible, but stored values are not returned through the API. They still
live in SQLite, so protect the database and backups.

Teatro contacts Twitch for an OAuth token, IGDB for metadata, and IGDB's image
service for covers. Allow outbound HTTPS to those services. Credentials remain on
the server. Automatic and manual matching first try mapped platform IDs, then fall
back to all platforms if no result is found or the platform has no mapping.
Review matches when editions or titles are ambiguous. Metadata/cover failures do
not roll back imported games. Local cached covers remain readable without IGDB.

## GOG offline installers

This is an opt-in extraction tool for offline installers you already own. It does
not sign into GOG, download purchases, run setup scripts or install prerequisites.

For the Docker image, add to Compose's local `.env`, then recreate the service:

```dotenv
TEATRO_GOG_IMPORT_ENABLED=true
```

```bash
docker compose up -d
```

Leave `TEATRO_INNOEXTRACT_PATH` and `TEATRO_INNOEXTRACT_SHA256` unset. The image
contains a sidecar at `/opt/teatro/tools/innoextract` with a checksum manifest.
Teatro verifies its hash for every import. The pinned source and declared Inno
Setup support ceiling are in [version.env](../packaging/innoextract/version.env);
a declared supported data version is not a guarantee for every installer.

For a native source build, supply an absolute path to a reviewed `innoextract`
executable and its lowercase SHA-256 in those two variables. Both must be set.
A binary with the wrong hash or invalid configuration is rejected.

In Import, select exactly one `.exe` and all matching `.bin` parts, review the
suggested title, and start. Teatro tests the installer, extracts to a private
workspace, validates files, and creates a Windows ZIP under platform `win`.
Watch Jobs for the terminal result, not just upload completion.

Defaults allow 30 minutes of processing, 50 GiB extracted content and 20,000 files.
Budget disk space for uploaded parts, extracted content, ZIP creation and staged
publication, not just the final archive. Incoming parts also use the normal upload
limits. The resulting ZIP must also stay within 50 GiB uncompressed, 20,000 entries
and a 1,000:1 per-entry compression ratio. Raising extraction settings does not
remove those fixed limits or guarantee that the result will launch.

Warnings, unsupported installer data, unsafe filenames/links, missing parts and
extractor failures abort the job. The image contains no RAR helpers. If a specific
installer needs one, an operator must separately review and mount a dedicated
helper directory and set `TEATRO_GOG_IMPORT_HELPER_PATH`; do not point this at a
general executable directory. Separate redistribution obligations apply.
Server restart does not resume a job. A produced ZIP is extracted game content,
not a reproduction of registry changes, services or all installer behavior.

## Remote RomM imports

This makes Teatro a client of one RomM server; it is separate from Teatro's
ROMM-compatible API. Compatibility against a live RomM deployment remains
unvalidated. Start with a small owned test game and retain your source library.

1. Set `TEATRO_ROMM_SOURCE_ENABLED=true` in Compose's local `.env` and run
   `docker compose up -d`.
2. In Settings, enter the remote base URL, username and secret. Use HTTPS where
   possible. Plain HTTP requires explicit acknowledgement that credentials and
   files cross the network unencrypted.
3. Choose token authentication for the OAuth2 password-grant flow, or Basic Auth
   if appropriate for your server. Test the connection.
4. Open the RomM page. A reachable new connection loads its initial catalog;
   **Refresh remote list** explicitly replaces it afterward. Searching and filtering
   use Teatro's local snapshot, not a live query per keystroke.
5. Select games, inspect file selections and the destination platform/title, then
   import and follow Jobs. Detail and download requests use current remote data.

The remote secret is configured through Settings, not environment variables, and
is write-only through the API. Clearing the source removes its stored credentials
and browse index, not already imported local games. Avoid credential-bearing URLs.
Teatro makes outbound requests to the configured server and refuses redirects;
use a direct endpoint rather than one that relies on login-page redirects.

Imports validate current remote file IDs, enforce size/file/time limits, compute
local hashes and use normal managed ingest. Duplicate checks use content hashes,
normalized target-platform filenames, and title/platform identity. `already_present`
is a normal skip, with no force-import override.

A declared size mismatch fails. A declared hash mismatch is a **warning**, and the
import keeps the received bytes and their computed hashes. Review every reported
conflict. No declared hashes means no upstream checksum comparison took place.
A usable remote cover is stored; otherwise configured IGDB matching can supply it.
Cover failures are non-fatal. The default browse snapshot ceiling is 100,000 games;
imports default to 64 files and a one-hour timeout. Durable resume is unavailable.

## Compatible clients

Create a dedicated `readonly` account. In a client that supports Teatro's
[API subset](api.md), configure the Teatro base URL and that account's credentials,
then check browsing, covers and a small download.
Do not use administrator credentials for ordinary play. Use HTTPS or a trusted VPN
when the connection leaves a trusted LAN.

Teatro supplies the catalog and files, not emulators or launch settings. For grouped
games, compatible clients use [download plans](api.md#downloads) to fetch every
required file and preserve names/order. Windows ZIP/7z games use platform `win`.
Automated API tests are not a complete Windows/Linux/Steam Deck release matrix;
validate your actual client build and target before depending on it.

## LAN discovery

Discovery is off by default. Set `TEATRO_LAN_DISCOVERY_ENABLED=true` and optionally
`TEATRO_DISCOVERY_NAME`, a nonempty name of at most 48 UTF-8 bytes. It advertises
`_teatro-games._tcp.local.` on the same IPv4 link and needs a non-loopback IPv4
listener. Discovery support depends on the client; manual URL entry remains valid.

Docker's ordinary bridge network usually advertises an address clients cannot use.
On a native Linux host, edit the service to use `network_mode: host` and remove
`ports`, then enable discovery. Host networking ignores `TEATRO_HOST_PORT`; the
server binds directly using `TEATRO_BIND_ADDR`. Permit TCP 4440 and UDP 5353 only
on the intended trusted LAN. Docker Desktop and other host environments may differ.

The advertised identity persists under the data directory. Discovery publishes
connection information, not credentials or the library, and does not authenticate
the server or encrypt traffic. A discovery failure does not stop HTTP service.
