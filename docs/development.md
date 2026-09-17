# Development

[Documentation home](../README.md#documentation) · [API reference](api.md)

## Run from source

Use Rust 1.88 or newer, Cargo, a native C build toolchain/linker and local writable
storage. Web assets are embedded at build time; no frontend build or Node install
is required to run Teatro. Node.js 22 is used for tests.

From the repository root:

```bash
cargo run --locked -- users create-admin --username admin
cargo run --locked -- serve
```

Plain Cargo builds bundle SQLite 3.46.0 through `libsqlite3-sys`. That C library
has [known upstream vulnerabilities](https://www.sqlite.org/cves.html), including
CVE-2025-6965. A clean Rust dependency scan does not cover them. The Docker recipe
instead links Debian's patched SQLite; its image review does not qualify native
builds. For a native system-library build, install your distribution's maintained
SQLite development package and `pkg-config`, then set
`LIBSQLITE3_SYS_USE_PKG_CONFIG=1` for Cargo. Review that library and test the result
on your host before deployment.

The first command prompts for a password. Open `http://127.0.0.1:4440/` or
`http://127.0.0.1:4440/admin`. Alternatively start the server first and complete
`/setup` locally. Running without a subcommand also starts the server.

Native defaults put runtime files under `./data` and bind to loopback.
[Configuration](configuration.md) explains the environment; `.env` is not loaded
automatically. Keep development data separate from any running instance. Source
changes to embedded web assets require rebuilding/restarting the executable.

Build a standalone executable with:

```bash
cargo build --release --locked
./target/release/teatro --help
```

The executable embeds migrations and web assets, but not the GOG extractor.
Use [Docker](docker.md) for the main deployment path. Native extraction requires
the explicit pin described in [GOG setup](integrations.md#gog-offline-installers).

## Checks

Install stable Rust with rustfmt/Clippy, Rust 1.88.0, Node.js 22, Bash, Python 3,
ShellCheck and cargo-audit. Container checks also need Docker/Compose. No tests
below need actual game content or integration credentials.

```bash
cargo +stable fmt --all -- --check
cargo +stable clippy --locked --all-targets --all-features -- -D warnings
cargo +stable test --locked --all-targets
cargo +stable test --locked --doc
cargo +1.88.0 test --locked --all-targets
find web -type f -name '*.js' -print0 | sort -z | xargs -0 -n1 node --check
node --check scripts/test-browser-downloads.mjs
(cd web/admin && npm test)
(cd web/public && npm test)
bash -n run-dev.sh packaging/innoextract/version.env
find scripts -type f -name '*.sh' -print0 | sort -z | xargs -0 bash -n
shellcheck run-dev.sh
find scripts -type f -name '*.sh' -print0 | sort -z | xargs -0 shellcheck
scripts/test-audit-dependencies.sh
scripts/audit-dependencies.sh
docker compose config --quiet
docker build --progress=plain --tag teatro:check .
scripts/test-docker-image.sh teatro:check
```

The [GitHub workflow](../.github/workflows/ci.yml) is the maintained CI command list.
The audit wrapper permits `RUSTSEC-2023-0071` only after checking that RSA is absent
from every enabled target/feature dependency graph. It fails if graph resolution
fails or RSA becomes active. This is not a general vulnerability waiver or an
audit of Docker's operating-system packages.

For an already prepared offline Rust audit, use `CARGO_NET_OFFLINE=true` and
`scripts/audit-dependencies.sh --no-fetch`; dependencies, tools and the RustSec
database must already be cached. Docker builds fetch pinned inputs and are separate
from offline smoke testing of an existing image. A passing local image test does
not establish reproducibility, redistribution clearance or support on other hosts.

Browser suites use Node's test runner; they do not replace real-browser or client
acceptance tests. A separate optional browser-download check is available through
`scripts/test-browser-downloads.mjs` with an installed Playwright/Chromium and
`TEATRO_PLAYWRIGHT_MODULE` pointing to its module. The 10,000-ROM benchmark remains
ignored in the normal Rust gate; see [large_library.rs](../tests/large_library.rs).

The public `scripts/` directory contains only these checks:

- `audit-dependencies.sh`: guarded dependency audit used by CI.
- `test-audit-dependencies.sh`: regression test for the advisory exception.
- `test-docker-image.sh`: isolated local-image startup and persistence check.
- `test-browser-downloads.mjs`: optional real-browser download check.

Build Docker images with `docker compose build` or `docker build`; no remote-host
or repository-publication helpers are required.

## Project structure

| Path | Responsibility |
| --- | --- |
| [`src/api/`](../src/api/) | Axum routes, authentication and wire translation. |
| [`src/services/`](../src/services/) | Library workflows, ingest, integrations, integrity and background jobs. |
| [`src/repositories/`](../src/repositories/) | SQLx access to SQLite records. |
| [`src/storage/`](../src/storage/) | Managed paths, files, streaming and filesystem safety. |
| [`src/domain/`](../src/domain/) | Domain records and ingest/workflow models. |
| [`src/ops/`](../src/ops/) | Operational reporting. |
| [`src/cli.rs`](../src/cli.rs), [`src/config.rs`](../src/config.rs), [`src/state.rs`](../src/state.rs) | CLI, environment configuration and application state/startup. |
| [`migrations/`](../migrations/) | Embedded SQLite schema and platform catalog. |
| [`web/`](../web/) | Build-free player, setup and administrator interfaces. |
| [`tests/`](../tests/) | Synthetic HTTP, persistence, recovery, integration-boundary and contract tests. |
| [`packaging/`](../packaging/), [`Dockerfile`](../Dockerfile) | Container sidecar pin and redistribution notices. |

SQLite stores identity/catalog metadata. Managed files remain under configured
roots. Startup obtains the instance lock before migrations and recovery; only the
server performs startup reconciliation. File operations use staging, locks and a
persistent journal to reconcile interrupted writes/deletes. Scan/GOG/RomM job
status is in memory, while integrity jobs persist in SQLite. Browser IndexedDB
transfer storage is separate from both.
