#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")"

export RUST_LOG="${RUST_LOG:-teatro=debug,tower_http=debug,sqlx=warn}"

exec cargo run -- "$@"
