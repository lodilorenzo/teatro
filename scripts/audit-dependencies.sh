#!/usr/bin/env bash
# Run from the repository root. Pass --no-fetch for a prepared offline audit.
set -euo pipefail

# The RSA exception is valid only while SQLx's MySQL dependency stays disabled.
# Include every target and feature, matching the broadest CI build.
graph="$(cargo tree --locked --target all --all-features --prefix none --format '{p}')"
if grep -q '^rsa v' <<< "$graph"; then
  printf 'Refusing RUSTSEC-2023-0071 exception: rsa is in the enabled dependency graph.\n' >&2
  exit 1
fi
cargo audit "$@" --ignore RUSTSEC-2023-0071
