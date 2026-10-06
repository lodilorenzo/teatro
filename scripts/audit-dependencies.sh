#!/usr/bin/env bash
# Run from the repository root. Pass --no-fetch for a prepared offline audit.
set -euo pipefail

# RSA is allowed only through the pinned, key-free WUA/GameCube RVZ integration.
# No keys/decryption endpoints; SQLx/MySQL remains forbidden. See the integration plan.
graph="$(cargo tree --locked --target all --all-features --prefix none --format '{p}')"
if grep -q '^sqlx-mysql v' <<< "$graph"; then
  printf 'Refusing RUSTSEC-2023-0071 exception: SQLx/MySQL is enabled.\n' >&2
  exit 1
fi
if grep -q '^rsa v' <<< "$graph"; then
  parents="$(cargo tree --locked --target all --all-features --invert rsa --depth 1 --prefix none --format '{p}')"
  expected=$'rsa v0.9.10\nrom-converto-lib v0.22.0 (https://github.com/DevYukine/rom-converto.git?rev=0bc5e29ce2c4afc3ce30b64de73f7d673f4f9817#0bc5e29c)'
  if [[ "$parents" != "$expected" ]]; then
    printf 'Refusing RUSTSEC-2023-0071 exception: RSA has unreviewed versions or callers.\n' >&2
    exit 1
  fi
fi
cargo audit "$@" --ignore RUSTSEC-2023-0071
