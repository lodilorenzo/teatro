<h1 align="center">
  <a href="https://teatro.host/"><img src="docs/assets/readme-header.png" alt="Teatro" width="640"></a>
</h1>

<p align="center">
  <a href="Cargo.toml"><img src="https://img.shields.io/badge/version-v0.19.6-633436?style=flat-square" alt="Teatro version"></a>
  <a href="#limits-and-release-status"><img src="https://img.shields.io/badge/status-beta-bd4444?style=flat-square" alt="Status: beta"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-CC_BY--NC--SA_4.0-633436?style=flat-square" alt="License: CC BY-NC-SA 4.0"></a>
  <a href="docs/development.md#run-from-source"><img src="https://img.shields.io/badge/Rust-1.88%2B-bd4444?style=flat-square" alt="Rust: 1.88 or newer"></a>
  <a href="docs/docker.md"><img src="https://img.shields.io/badge/Docker-source_build-633436?style=flat-square" alt="Docker: build from source"></a>
</p>

Teatro is a self-hosted game library server with a player library,
browser administration, and a focused ROMM-compatible API. SQLite stores the
catalog; game files remain visible on disk. Teatro does not supply games or run
emulators. Import only content you have permission to use.

Teatro is beta, noncommercial source-available software under
[CC BY-NC-SA 4.0](LICENSE). Docker is the main deployment method, but no supported
binary or container distribution has been published. Build your own image from
this source. The current version is defined in [`Cargo.toml`](Cargo.toml).

## Features

- Authenticated browsing, search, game details, covers and browser downloads.
- Uploads and managed-library scans for single files, optical-disc descriptors,
  tracks and multi-disc playlists.
- Metadata and cover editing, optional IGDB matching, and explicit file deletion.
- Read-only and administrator accounts, browser sessions and scoped API tokens.
- CRC32, MD5, SHA-1 and SHA-256 hashing with Logiqx XML DAT verification.
- Optional GOG offline-installer extraction into Windows ZIPs.
- Optional browsing and selected-game imports from one remote RomM server.
- Optional same-link IPv4 discovery for compatible clients.

## Quick start with Docker

Use a Linux Docker engine with BuildKit, Compose v2 and local storage. From a
checkout of this repository:

```bash
docker compose up --build -d
```

Open `http://YOUR_SERVER:4440/setup` and immediately create the first administrator.
Then use `/` for the player library and `/admin` for administration. The default
named volume, `teatro-data`, holds the database, game files and covers.

**Complete setup on a trusted network. Do not expose port 4440 directly to the
Internet.** Passwords and tokens need HTTPS or a trusted VPN outside that boundary.
The default Compose file publishes the port on all host interfaces. For access
from the host only, set `TEATRO_HOST_PORT=127.0.0.1:4440` in a local Compose `.env`.

Read [Docker deployment](docs/docker.md) before using existing storage, changing
permissions, or upgrading. The public baseline supports fresh installations,
not databases from development snapshots with a different migration history.
For a native build, see [development](docs/development.md#run-from-source).

## Documentation

All documentation is included here; no wiki is required.

| Guide | Covers |
| --- | --- |
| [Docker deployment and operations](docs/docker.md) | Installation, volumes, HTTPS, users, backup, restore, upgrades and troubleshooting. |
| [Library guide](docs/library.md) | Playing and downloading, uploads, scans, metadata, deletion, Jobs and integrity checks. |
| [Integrations](docs/integrations.md) | IGDB, GOG, remote RomM, compatible clients and LAN discovery. |
| [Configuration](docs/configuration.md) | Environment variables, defaults, limits and credential storage. |
| [API reference](docs/api.md) | Authentication, read/download contract and all management endpoint groups. |
| [Development](docs/development.md) | Source builds, checks and project structure. |
| [Security policy](SECURITY.md) | Supported versions and private vulnerability reporting. |

## Limits and release status

Teatro implements a subset of the ROMM API for browsing and downloads, not the full API.
GOG and remote RomM imports are disabled by default. Remote RomM compatibility
has not been validated against a live server. Server-side import jobs do not
resume after a restart. Archives are hashed as files, not by their members;
split archives and ClrMamePro text DATs are unsupported.

The Docker recipe targets native `linux/amd64` and `linux/arm64` builds. Local
amd64 checks are not a guarantee for every host or architecture. Release-qualified
images, signed distributions and client validation across devices remain pending.

## License and security

Teatro's original code, documentation and artwork are licensed under
[CC BY-NC-SA 4.0](LICENSE). Copyright © 2026 Lorenzo Lodi. This is noncommercial
source-available software, not OSI-approved open source.

Third-party material retains its own licenses. See
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md),
[RUST_DEPENDENCY_LICENSES.tsv](RUST_DEPENDENCY_LICENSES.tsv), notices beside the
web assets, and [packaging notices](packaging/THIRD_PARTY_NOTICES.md).
Source-publication review does not qualify generated images for redistribution.

Report vulnerabilities through the private route in [SECURITY.md](SECURITY.md).
Do not post exploit details, credentials or personal library data in public issues.
