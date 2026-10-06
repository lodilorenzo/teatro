#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT
cat > "$WORK_DIR/cargo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
case "$1" in
  tree)
    if [[ "$*" == "tree --locked --target all --all-features --invert rsa --depth 1 --prefix none --format {p}" ]]; then
      printf '%s\n' "${TEST_RSA_GRAPH:-}"
    else
      [[ "$*" == "tree --locked --target all --all-features --prefix none --format {p}" ]]
      printf '%s\n' "$TEST_GRAPH"
    fi
    exit "$TEST_TREE_EXIT"
    ;;
  audit)
    printf '%s\n' "$*" > "$TEST_AUDIT_LOG"
    exit "$TEST_AUDIT_EXIT"
    ;;
  *) exit 99 ;;
esac
EOF
chmod +x "$WORK_DIR/cargo"
export PATH="$WORK_DIR:$PATH" TEST_AUDIT_LOG="$WORK_DIR/audit.log"
export TEST_GRAPH='teatro v0.0.0' TEST_TREE_EXIT=0 TEST_AUDIT_EXIT=0
bash "$SCRIPT_DIR/audit-dependencies.sh" --no-fetch
[[ "$(< "$TEST_AUDIT_LOG")" == 'audit --no-fetch --ignore RUSTSEC-2023-0071' ]]
rm "$TEST_AUDIT_LOG"
export TEST_GRAPH=$'teatro v0.0.0\nrsa v0.9.8'
if bash "$SCRIPT_DIR/audit-dependencies.sh"; then exit 1; fi
test ! -e "$TEST_AUDIT_LOG"
export TEST_GRAPH=$'teatro v0.0.0\nrsa v0.9.10'
export TEST_RSA_GRAPH=$'rsa v0.9.10\nrom-converto-lib v0.22.0 (https://github.com/DevYukine/rom-converto.git?rev=0bc5e29ce2c4afc3ce30b64de73f7d673f4f9817#0bc5e29c)'
bash "$SCRIPT_DIR/audit-dependencies.sh" --no-fetch
test -f "$TEST_AUDIT_LOG"
rm "$TEST_AUDIT_LOG"
export TEST_RSA_GRAPH=$'rsa v0.9.10\nother v1.0.0'
if bash "$SCRIPT_DIR/audit-dependencies.sh"; then exit 1; fi
test ! -e "$TEST_AUDIT_LOG"
export TEST_GRAPH=$'teatro v0.0.0\nsqlx-mysql v0.8.6'
if bash "$SCRIPT_DIR/audit-dependencies.sh"; then exit 1; fi
test ! -e "$TEST_AUDIT_LOG"
export TEST_GRAPH='teatro v0.0.0' TEST_TREE_EXIT=42
if bash "$SCRIPT_DIR/audit-dependencies.sh"; then exit 1; fi
test ! -e "$TEST_AUDIT_LOG"
export TEST_TREE_EXIT=0 TEST_AUDIT_EXIT=43
if bash "$SCRIPT_DIR/audit-dependencies.sh"; then exit 1; fi
printf 'Audit guard passed: inactive RSA, pinned WUA-only RSA, unexpected RSA callers, MySQL, graph failure, and audit failure.\n'
